use std::cmp::Ordering;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, RwLock};
use std::time::Instant;

use log::{debug, info};
#[cfg(feature = "notify")]
use notify_rust::Notification;

use rand::prelude::*;
use strum_macros::Display;

use crate::config::Config;
use crate::library::Library;
use crate::model::playable::Playable;
use crate::spotify::PlayerEvent;
use crate::spotify::Spotify;
use crate::traits::ListItem;

/// Repeat behavior for the [Queue].
#[derive(Display, Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum RepeatSetting {
    #[serde(rename = "off")]
    None,
    #[serde(rename = "playlist")]
    RepeatPlaylist,
    #[serde(rename = "track")]
    RepeatTrack,
}

/// When the sleep timer pauses playback.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SleepTimer {
    /// When the current track ends: the next one is loaded paused.
    EndOfTrack,
    /// At this time.
    At(Instant),
}

/// Events that are specific to the [Queue].
#[derive(Clone, Debug)]
pub enum QueueEvent {
    /// Request the player to 'preload' a track, basically making sure that
    /// transitions between tracks can be uninterrupted.
    PreloadTrackRequest,
    /// Tracks from autoplay to follow the track with URI `after`; see [Queue::request_autoplay].
    Autoplay {
        after: String,
        tracks: Vec<Playable>,
    },
}

/// The source ("Next from: …") of tracks added by autoplay.
pub const AUTOPLAY_SOURCE: &str = "Autoplay";

/// The queue determines the playback order of [Playable] items, and is also used to control
/// playback itself.
pub struct Queue {
    /// The internal data, which doesn't change with shuffle or repeat. This is
    /// the raw data only.
    pub queue: Arc<RwLock<Vec<Playable>>>,
    /// The playback order of the queue, as indices into `self.queue`. Shuffling reorders
    /// `self.queue` itself (see [Queue::generate_random_order]), so this is only set by queue
    /// state saved by older versions.
    random_order: RwLock<Option<Vec<usize>>>,
    /// The queue as it was before shuffling, so turning shuffle off can restore that order.
    unshuffled: RwLock<Option<Vec<Playable>>>,
    /// Where each item in `self.queue` was queued from (an album or playlist name), shown as
    /// "Next from: …" in the queue panel. Kept parallel to `self.queue`; `None` for items queued
    /// individually or restored from a previous session.
    sources: RwLock<Vec<Option<Arc<str>>>>,
    current_track: RwLock<Option<usize>>,
    sleep_timer: RwLock<Option<SleepTimer>>,
    /// When the current track was loaded, and how many tracks in a row "finished" moments after
    /// loading, as ones that fail to play do; see [Queue::failing].
    loaded_at: RwLock<Option<Instant>>,
    failed_in_a_row: RwLock<u32>,
    /// The URI of the track autoplay was last asked to follow, so it's asked once per track.
    autoplay_after: RwLock<Option<String>>,
    /// The queue ran out while autoplay was still looking: play what it finds.
    waiting_for_autoplay: RwLock<bool>,
    spotify: Spotify,
    cfg: Arc<Config>,
    /// Formats notifications.
    #[cfg_attr(not(feature = "notify"), allow(dead_code))]
    library: Arc<Library>,
}

impl Queue {
    pub fn new(spotify: Spotify, cfg: Arc<Config>, library: Arc<Library>) -> Self {
        let queue_state = cfg.state().queuestate.clone();
        // Saved sources only line up if there's one per saved item.
        let sources = if queue_state.sources.len() == queue_state.queue.len() {
            queue_state
                .sources
                .iter()
                .map(|s| s.as_deref().map(Arc::from))
                .collect()
        } else {
            vec![None; queue_state.queue.len()]
        };

        Self {
            queue: Arc::new(RwLock::new(queue_state.queue)),
            spotify: spotify.clone(),
            current_track: RwLock::new(queue_state.current_track),
            sleep_timer: RwLock::new(None),
            loaded_at: RwLock::new(None),
            failed_in_a_row: RwLock::new(0),
            autoplay_after: RwLock::new(None),
            waiting_for_autoplay: RwLock::new(false),
            random_order: RwLock::new(queue_state.random_order),
            unshuffled: RwLock::new(None),
            sources: RwLock::new(sources),
            cfg,
            library,
        }
    }

    /// Position of the current track within the play order (shuffle-aware).
    /// None if nothing is playing.
    fn current_order_pos(&self) -> Option<usize> {
        let current = (*self.current_track.read().unwrap())?;
        Some(match self.random_order.read().unwrap().as_ref() {
            Some(order) => order.iter().position(|&i| i == current).unwrap(),
            None => current,
        })
    }

    /// The `self.queue` index at play-order position `pos` (shuffle-aware).
    fn queue_index_at(&self, pos: usize) -> usize {
        match self.random_order.read().unwrap().as_ref() {
            Some(order) => order[pos],
            None => pos,
        }
    }

    /// The index of the next item in `self.queue` that should be played. None
    /// if at the end of the queue.
    pub fn next_index(&self) -> Option<usize> {
        let next = self.current_order_pos()? + 1;
        (next < self.queue.read().unwrap().len()).then(|| self.queue_index_at(next))
    }

    /// The index of the previous item in `self.queue` that should be played.
    /// None if at the start of the queue.
    pub fn previous_index(&self) -> Option<usize> {
        let pos = self.current_order_pos()?;
        (pos > 0).then(|| self.queue_index_at(pos - 1))
    }

    /// The `self.queue` index `offset` items from the current one in play
    /// order (negative = backward), clamped to the queue bounds. None if
    /// nothing is playing or the queue is empty.
    fn skip_index(&self, offset: isize) -> Option<usize> {
        let pos = self.current_order_pos()?;
        let len = self.queue.read().unwrap().len();
        (len > 0).then(|| {
            let target = (pos as isize + offset).clamp(0, len as isize - 1) as usize;
            self.queue_index_at(target)
        })
    }

    /// Skip `offset` items forward (positive) or backward (negative) in play
    /// order and play the destination. Lets a count-prefixed next/previous
    /// load one track instead of stepping through (and loading) `offset`.
    ///
    /// Intentionally ignores repeat mode: it clamps to the queue ends rather
    /// than wrapping like `next()` under `RepeatPlaylist`, and does not flip
    /// `RepeatTrack` the way `next(true)` does. At an end it plays the last
    /// (or first) track rather than stopping as stepping off the end would.
    /// A bulk count-skip is a jump to a position, not a sequence of
    /// end-of-queue transitions.
    pub fn skip(&self, offset: isize) {
        if let Some(index) = self.skip_index(offset) {
            self.play(index, false, false);
        }
    }

    /// The currently playing item from `self.queue`.
    pub fn get_current(&self) -> Option<Playable> {
        self.get_current_index()
            .map(|index| self.queue.read().unwrap()[index].clone())
    }

    /// Whether the current item's ID is `id` (`None` for items without one, like local files).
    pub fn is_current_uri(&self, uri: &str) -> bool {
        let Some(index) = self.get_current_index() else {
            return false;
        };
        self.queue
            .read()
            .unwrap()
            .get(index)
            .is_some_and(|current| current.uri() == uri)
    }

    pub fn is_current_id(&self, id: Option<&str>) -> bool {
        let Some(index) = self.get_current_index() else {
            return false;
        };
        self.queue
            .read()
            .unwrap()
            .get(index)
            .is_some_and(|current| current.id_ref() == id)
    }

    /// Whether the queue is exactly the items with `ids` (ignoring ones without an ID), and
    /// there's at least one: i.e. an album or playlist with these tracks is what's playing.
    pub fn consists_of<'a>(&self, ids: impl Iterator<Item = &'a str> + Clone) -> bool {
        ids.clone().next().is_some()
            && self
                .queue
                .read()
                .unwrap()
                .iter()
                .filter_map(Playable::id_ref)
                .eq(ids)
    }

    /// The index of the currently playing item from `self.queue`.
    pub fn get_current_index(&self) -> Option<usize> {
        *self.current_track.read().unwrap()
    }

    /// Insert `track` as the item that should logically follow the currently
    /// playing item, taking into account shuffle status.
    pub fn insert_after_current(&self, track: Playable) {
        if let Some(index) = self.get_current_index() {
            let mut random_order = self.random_order.write().unwrap();
            if let Some(order) = random_order.as_mut() {
                let next_i = order.iter().position(|&i| i == index).unwrap();
                // shift everything after the insertion in order
                for item in order.iter_mut() {
                    if *item > index {
                        *item += 1;
                    }
                }
                // finally, add the next track index
                order.insert(next_i + 1, index + 1);
            }
            let mut q = self.queue.write().unwrap();
            q.insert(index + 1, track);
            insert_source(&mut self.sources.write().unwrap(), index + 1, None);
        } else {
            self.append(track);
        }
    }

    /// Add `track` to the end of the queue.
    pub fn append(&self, track: Playable) {
        let mut random_order = self.random_order.write().unwrap();
        if let Some(order) = random_order.as_mut() {
            order.push(order.len());
        }

        let mut q = self.queue.write().unwrap();
        let mut sources = self.sources.write().unwrap();
        // Ahead of what autoplay added, as in Spotify's apps (not with the old shuffle order,
        // which only appends).
        let index = match random_order.as_ref() {
            Some(_) => q.len(),
            None => self.autoplay_start(&q, &sources),
        };
        q.insert(index, track);
        insert_source(&mut sources, index, None);
    }

    /// Where the tracks autoplay added after the current one start (the end of the queue if there
    /// are none).
    fn autoplay_start(&self, q: &[Playable], sources: &[Option<Arc<str>>]) -> usize {
        let first = self.current_track.read().unwrap().map_or(0, |i| i + 1);
        let mut start = q.len();
        while start > first
            && sources
                .get(start - 1)
                .is_some_and(|s| s.as_deref() == Some(AUTOPLAY_SOURCE))
        {
            start -= 1;
        }
        start
    }

    /// Append `tracks` after the currently playing item, taking into account
    /// shuffle status. Returns the first index(in `self.queue`) of added items.
    pub fn append_next(&self, tracks: &[Playable]) -> usize {
        self.append_next_from(tracks, None)
    }

    /// [Queue::append_next], recording that the tracks came from `source` (an album or playlist
    /// name) for the queue panel.
    pub fn append_next_from(&self, tracks: &[Playable], source: Option<&str>) -> usize {
        let mut q = self.queue.write().unwrap();

        {
            let mut random_order = self.random_order.write().unwrap();
            if let Some(order) = random_order.as_mut() {
                order.extend((q.len().saturating_sub(1))..(q.len() + tracks.len()));
            }
        }

        let first = match *self.current_track.read().unwrap() {
            Some(index) => index + 1,
            None => q.len(),
        };

        let source: Option<Arc<str>> = source.map(Arc::from);
        let mut sources = self.sources.write().unwrap();
        for (i, track) in (first..).zip(tracks.iter()) {
            q.insert(i, track.clone());
            insert_source(&mut sources, i, source.clone());
        }

        first
    }

    /// Remove the item at `index`. This doesn't take into account shuffle
    /// status, and will literally remove the item at `index` in `self.queue`.
    pub fn remove(&self, index: usize) {
        {
            let mut q = self.queue.write().unwrap();
            if q.is_empty() {
                info!("queue is empty");
                return;
            }
            q.remove(index);
            let mut sources = self.sources.write().unwrap();
            if index < sources.len() {
                sources.remove(index);
            }
        }

        // if the queue is empty stop playback
        let len = self.queue.read().unwrap().len();
        if len == 0 {
            self.stop();
            return;
        }

        // if we are deleting the currently playing track, play the track with
        // the same index again, because the next track is now at the position
        // of the one we deleted
        let current = *self.current_track.read().unwrap();
        if let Some(current_track) = current {
            match current_track.cmp(&index) {
                Ordering::Equal => {
                    // if we have deleted the last item and it was playing
                    // stop playback, unless repeat playlist is on, play next
                    if current_track == len {
                        if self.get_repeat() == RepeatSetting::RepeatPlaylist {
                            self.next(false);
                        } else {
                            self.stop();
                        }
                    } else {
                        self.play(index, false, false);
                    }
                }
                Ordering::Greater => {
                    let mut current = self.current_track.write().unwrap();
                    current.replace(current_track - 1);
                }
                _ => (),
            }
        }

        // Shuffling reorders the queue itself, so a removal leaves the remaining (already
        // shuffled) order intact rather than reshuffling everything.
    }

    /// Clear all the items from the queue and stop playback.
    pub fn clear(&self) {
        self.stop();

        let mut q = self.queue.write().unwrap();
        q.clear();

        let mut random_order = self.random_order.write().unwrap();
        if let Some(o) = random_order.as_mut() {
            o.clear()
        }
        *self.unshuffled.write().unwrap() = None;
        self.sources.write().unwrap().clear();
    }

    /// The amount of items in `self.queue`.
    pub fn len(&self) -> usize {
        self.queue.read().unwrap().len()
    }

    /// Shift the item at `from` in `self.queue` to `to`.
    pub fn shift(&self, from: usize, to: usize) {
        let mut queue = self.queue.write().unwrap();
        let item = queue.remove(from);
        queue.insert(to, item);
        {
            let mut sources = self.sources.write().unwrap();
            sources.resize(queue.len(), None);
            let source = sources.remove(from);
            sources.insert(to, source);
        }

        // if the currently playing track is affected by the shift, update its
        // index
        let mut current = self.current_track.write().unwrap();
        if let Some(index) = *current {
            if index == from {
                current.replace(to);
            } else if index == to && from > index {
                current.replace(to + 1);
            } else if index == to && from < index {
                current.replace(to - 1);
            }
        }
    }

    /// Play the item at `index` in `self.queue`.
    ///
    /// `reshuffle`: Reshuffle the current order of the queue.
    /// `shuffle_index`: If this is true, `index` isn't actually used, but is
    /// chosen at random as a valid index in the queue.
    pub fn play(&self, index: usize, reshuffle: bool, shuffle_index: bool) {
        self.load(index, reshuffle, shuffle_index, true);
    }

    /// Note the current track in the listening history (see [crate::stats]) as far as it was
    /// listened to, as it stops or another starts.
    pub fn record_listened(&self) {
        if let Some(Playable::Track(track)) = self.get_current() {
            let listened = match self.spotify.get_current_status() {
                PlayerEvent::Playing(_) | PlayerEvent::Paused(_) => {
                    self.spotify.get_current_progress()
                }
                _ => return,
            };
            crate::stats::record(&track, listened);
        }
    }

    /// Whether tracks keep failing to play: one that "finishes" moments after it was loaded didn't
    /// play, and after a few in a row Spotify is refusing them (it does when asked for too many
    /// too fast). Then playback stops rather than going on to the next track, which would be
    /// refused too and skip through the whole queue. Counts the track that just finished.
    fn failing(&self) -> bool {
        const QUICK: std::time::Duration = std::time::Duration::from_secs(5);
        const MAX_FAILURES: u32 = 3;
        let quick = self
            .loaded_at
            .read()
            .unwrap()
            .is_some_and(|at| at.elapsed() < QUICK);
        let mut failed = self.failed_in_a_row.write().unwrap();
        *failed = if quick { *failed + 1 } else { 0 };
        if *failed < MAX_FAILURES {
            return false;
        }
        *failed = 0;
        drop(failed);
        self.spotify.stop();
        crate::tui::report_error(
            "Spotify isn't letting songs play right now: wait a minute, then play again".into(),
        );
        true
    }

    /// Note the current track in the listening history as played to the end.
    pub fn record_finished(&self) {
        if let Some(Playable::Track(track)) = self.get_current() {
            let length = std::time::Duration::from_millis(u64::from(track.duration));
            crate::stats::record(&track, length);
        }
    }

    /// [Queue::play], but the track only starts playing if `start_playing`.
    fn load(&self, mut index: usize, reshuffle: bool, shuffle_index: bool, start_playing: bool) {
        self.record_listened();
        *self.loaded_at.write().unwrap() = Some(Instant::now());
        let queue_length = self.queue.read().unwrap().len();
        // The length of the queue must be bigger than 0 or gen_range panics!
        if queue_length > 0 && shuffle_index && self.get_shuffle() {
            let mut rng = rand::rng();
            index = rng.random_range(0..queue_length);
        }

        if let Some(track) = &self.queue.read().unwrap().get(index) {
            self.spotify.load(track, start_playing, 0);
            self.current_track.write().unwrap().replace(index);
            self.spotify.update_track();

            #[cfg(feature = "notify")]
            if start_playing && self.cfg.values().notify.unwrap_or(false) {
                std::thread::spawn({
                    // use same parser as track_format, Playable::format
                    let format = self
                        .cfg
                        .values()
                        .notification_format
                        .clone()
                        .unwrap_or_default();
                    let default_title = crate::config::NotificationFormat::default().title.unwrap();
                    let title = format.title.unwrap_or_else(|| default_title.clone());

                    let default_body = crate::config::NotificationFormat::default().body.unwrap();
                    let body = format.body.unwrap_or_else(|| default_body.clone());

                    let summary_txt = Playable::format(track, &title, &self.library);
                    let body_txt = Playable::format(track, &body, &self.library);
                    let cover_url = track.cover_url();
                    move || send_notification(&summary_txt, &body_txt, cover_url)
                });
            }

            // Send a Seeked signal at start of new track
            #[cfg(feature = "mpris")]
            self.spotify.notify_seeked(0);
        }

        if reshuffle && self.get_shuffle() {
            self.generate_random_order()
        }
        *self.waiting_for_autoplay.write().unwrap() = false;
        self.request_autoplay(index);
    }

    /// Toggle the playback. If playback is currently stopped, this will either
    /// play the next song if one is available, or restart from the start.
    pub fn toggleplayback(&self) {
        // A track still loading starts, or doesn't, once it's loaded: rather than going on to the
        // next one, which (pressed while waiting) would skip through the queue.
        if let Some(starts) = self.spotify.loading() {
            if starts {
                self.spotify.pause();
            } else {
                self.spotify.play();
            }
            return;
        }
        match self.spotify.get_current_status() {
            PlayerEvent::Playing(_) | PlayerEvent::Paused(_) => {
                self.spotify.toggleplayback();
            }
            PlayerEvent::Stopped => match self.next_index() {
                Some(_) => self.next(false),
                None => self.play(0, false, false),
            },
            _ => (),
        }
    }

    /// Stop playback.
    pub fn stop(&self) {
        self.record_listened();
        let mut current = self.current_track.write().unwrap();
        *current = None;
        self.spotify.stop();
    }

    /// Play the next song in the queue. Stops playback if there are no playable tracks
    /// remaining.
    ///
    /// `manual`: If this is true, normal queue logic like repeat will not be
    /// used, and the next track will actually be played. This should be used
    /// when going to the next entry in the queue is the wanted behavior.
    pub fn next(&self, manual: bool) {
        if !manual && self.failing() {
            return;
        }
        let q = self.queue.read().unwrap();
        let current = *self.current_track.read().unwrap();
        let repeat = self.cfg.state().repeat;
        // A track that ended with the sleep timer set to its end loads the next one paused.
        let start_playing = manual || !self.take_sleep_timer(SleepTimer::EndOfTrack);

        if repeat == RepeatSetting::RepeatTrack && !manual {
            if let Some(index) = current
                && q[index].is_playable()
            {
                self.load(index, false, false, start_playing);
            }
        } else if let Some(index) = self.next_index() {
            self.load(index, false, false, start_playing);
            if repeat == RepeatSetting::RepeatTrack && manual {
                self.set_repeat(RepeatSetting::RepeatPlaylist);
            }
        } else if repeat == RepeatSetting::RepeatPlaylist
            && !q.is_empty()
            && q.iter().any(|track| track.is_playable())
        {
            let random_order = self.random_order.read().unwrap();
            self.load(
                random_order.as_ref().map(|o| o[0]).unwrap_or(0),
                false,
                false,
                start_playing,
            );
        } else {
            // Autoplay may still be looking for what comes next: it plays it when it's found.
            let current_uri = current.and_then(|i| q.get(i)).map(Playable::uri);
            *self.waiting_for_autoplay.write().unwrap() = !manual
                && current_uri.is_some()
                && *self.autoplay_after.read().unwrap() == current_uri;
            self.spotify.stop();
        }
    }

    /// Whether autoplay is on: the `autoplay` setting, and only without repeat.
    fn autoplay_enabled(&self) -> bool {
        self.cfg.values().autoplay.unwrap_or(true) && self.cfg.state().repeat == RepeatSetting::None
    }

    /// When the track at `index` is the last one to play, ask autoplay for tracks to follow it,
    /// based on its album and the tracks played before it. Asked once per track.
    fn request_autoplay(&self, index: usize) {
        if !self.autoplay_enabled() || self.next_index().is_some() {
            return;
        }
        let q = self.queue.read().unwrap();
        // Podcasts have no autoplay.
        let Some(Playable::Track(track)) = q.get(index) else {
            return;
        };
        let Some(album_id) = &track.album_id else {
            return;
        };
        {
            let mut after = self.autoplay_after.write().unwrap();
            if after.as_deref() == Some(track.uri.as_str()) {
                return;
            }
            *after = Some(track.uri.clone());
        }
        // The tracks played up to this one, oldest first, so autoplay doesn't repeat them.
        let recent: Vec<String> = q[..=index]
            .iter()
            .rev()
            .filter_map(Playable::track)
            .map(|track| track.uri)
            .take(50)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        debug!("Asking autoplay for tracks after {}", track.uri);
        self.spotify.request_autoplay(
            format!("spotify:album:{album_id}"),
            recent,
            track.uri.clone(),
        );
    }

    /// Add `tracks` from autoplay after the track `after`, if it's still the last one to play.
    /// Plays the first if playback stopped at the end of the queue meanwhile.
    fn add_autoplay(&self, after: &str, tracks: Vec<Playable>) {
        let still_last = self.next_index().is_none()
            && self.get_current().map(|t| t.uri()).as_deref() == Some(after);
        let waiting = std::mem::take(&mut *self.waiting_for_autoplay.write().unwrap());
        if !still_last || !self.autoplay_enabled() || tracks.is_empty() {
            return;
        }
        let first = {
            let mut q = self.queue.write().unwrap();
            let mut sources = self.sources.write().unwrap();
            if let Some(order) = self.random_order.write().unwrap().as_mut() {
                order.extend(q.len()..q.len() + tracks.len());
            }
            let first = q.len();
            for track in tracks {
                q.push(track);
                let len = q.len();
                insert_source(&mut sources, len - 1, Some(Arc::from(AUTOPLAY_SOURCE)));
            }
            first
        };
        if waiting {
            self.play(first, false, false);
        }
    }

    pub fn sleep_timer(&self) -> Option<SleepTimer> {
        *self.sleep_timer.read().unwrap()
    }

    /// Set the sleep timer, or turn it off with `None`.
    pub fn set_sleep_timer(&self, timer: Option<SleepTimer>) {
        *self.sleep_timer.write().unwrap() = timer;
    }

    /// Turn the sleep timer off if it's set to `timer`. Returns whether it was.
    fn take_sleep_timer(&self, timer: SleepTimer) -> bool {
        self.sleep_timer
            .write()
            .unwrap()
            .take_if(|set| *set == timer)
            .is_some()
    }

    /// Pause playback if the sleep timer's time has come. Returns whether it had.
    pub fn sleep_if_due(&self) -> bool {
        let Some(SleepTimer::At(at)) = self.sleep_timer() else {
            return false;
        };
        if Instant::now() < at || !self.take_sleep_timer(SleepTimer::At(at)) {
            return false;
        }
        if let PlayerEvent::Playing(_) = self.spotify.get_current_status() {
            self.spotify.pause();
        }
        true
    }

    /// Play the previous item in the queue.
    pub fn previous(&self) {
        let q = self.queue.read().unwrap();
        let current = *self.current_track.read().unwrap();
        let repeat = self.cfg.state().repeat;

        if let Some(index) = self.previous_index() {
            self.play(index, false, false);
        } else if repeat == RepeatSetting::RepeatPlaylist && !q.is_empty() {
            if self.get_shuffle() {
                let random_order = self.random_order.read().unwrap();
                self.play(
                    random_order.as_ref().map(|o| o[q.len() - 1]).unwrap_or(0),
                    false,
                    false,
                );
            } else {
                self.play(q.len() - 1, false, false);
            }
        } else if let Some(index) = current {
            self.play(index, false, false);
        }
    }

    /// Get the current repeat behavior.
    pub fn get_repeat(&self) -> RepeatSetting {
        self.cfg.state().repeat
    }

    /// Set the current repeat behavior and save it to the configuration.
    pub fn set_repeat(&self, new: RepeatSetting) {
        self.cfg.with_state_mut(|s| s.repeat = new);
    }

    /// Get the current shuffle behavior.
    pub fn get_shuffle(&self) -> bool {
        self.cfg.state().shuffle
    }

    /// Get the current order that is used to shuffle.
    pub fn get_random_order(&self) -> Option<Vec<usize>> {
        self.random_order.read().unwrap().clone()
    }

    /// Shuffle the queue: reorder `self.queue` so the current item comes first, followed by the
    /// rest in random order. Reordering the queue itself (rather than keeping a separate play
    /// order) means the queue view shows the order things will actually play in. The order
    /// before the first shuffle is kept so [Queue::set_shuffle] can restore it.
    fn generate_random_order(&self) {
        let mut random_order = self.random_order.write().unwrap();
        let mut q = self.queue.write().unwrap();
        let mut current = self.current_track.write().unwrap();

        let mut unshuffled = self.unshuffled.write().unwrap();
        if unshuffled.is_none() {
            *unshuffled = Some(q.clone());
        }

        let mut sources = self.sources.write().unwrap();
        sources.resize(q.len(), None);

        type Entry = (Playable, Option<Arc<str>>);
        let mut shuffled: Vec<Entry> = Vec::with_capacity(q.len());
        let mut rest: Vec<Entry> = Vec::with_capacity(q.len());
        for (i, entry) in q.drain(..).zip(sources.drain(..)).enumerate() {
            if Some(i) == *current {
                shuffled.push(entry);
            } else {
                rest.push(entry);
            }
        }
        rest.shuffle(&mut rand::rng());
        shuffled.extend(rest);

        (*q, *sources) = shuffled.into_iter().unzip();
        if current.is_some() {
            *current = Some(0);
        }
        *random_order = None;
    }

    /// Undo [Queue::generate_random_order]: put the queue back in its order from before
    /// shuffling. Items added while shuffled keep their place at the end, removed ones stay
    /// removed, and the current item keeps playing.
    fn restore_unshuffled_order(&self) {
        let mut random_order = self.random_order.write().unwrap();
        let mut q = self.queue.write().unwrap();
        let mut current = self.current_track.write().unwrap();
        *random_order = None;

        let Some(original) = self.unshuffled.write().unwrap().take() else {
            return;
        };

        // Match by URI, consuming positions so duplicate tracks map one-to-one.
        let mut positions: HashMap<String, VecDeque<usize>> = HashMap::new();
        for (i, track) in q.iter().enumerate() {
            positions.entry(track.uri()).or_default().push_back(i);
        }
        let mut taken = vec![false; q.len()];
        let mut order: Vec<usize> = Vec::with_capacity(q.len());
        for track in &original {
            if let Some(i) = positions.get_mut(&track.uri()).and_then(|p| p.pop_front()) {
                taken[i] = true;
                order.push(i);
            }
        }
        order.extend((0..q.len()).filter(|&i| !taken[i]));

        *current = current.and_then(|c| order.iter().position(|&i| i == c));
        let mut sources = self.sources.write().unwrap();
        sources.resize(q.len(), None);
        let mut slots: Vec<Option<(Playable, Option<Arc<str>>)>> =
            q.drain(..).zip(sources.drain(..)).map(Some).collect();
        (*q, *sources) = order.iter().filter_map(|&i| slots[i].take()).unzip();
    }

    /// Where each item was queued from, for saving the queue between sessions.
    pub fn sources(&self) -> Vec<Option<String>> {
        let len = self.queue.read().unwrap().len();
        let sources = self.sources.read().unwrap();
        (0..len)
            .map(|i| sources.get(i).cloned().flatten().map(|s| s.to_string()))
            .collect()
    }

    /// Where the item at `index` was queued from, if known.
    pub fn source(&self, index: usize) -> Option<Arc<str>> {
        self.sources.read().unwrap().get(index).cloned().flatten()
    }

    /// Up to `count` items that will play after the current one, in order, with their indices in
    /// `self.queue`. Without a current item, the queue from the start.
    pub fn upcoming(&self, count: usize) -> Vec<(usize, Playable)> {
        let q = self.queue.read().unwrap();
        let random_order = self.random_order.read().unwrap();
        let order: Vec<usize> = match random_order.as_ref() {
            Some(order) if order.len() == q.len() => order.clone(),
            _ => (0..q.len()).collect(),
        };
        let start = match *self.current_track.read().unwrap() {
            Some(current) => order
                .iter()
                .position(|&i| i == current)
                .map_or(0, |p| p + 1),
            None => 0,
        };
        order
            .iter()
            .skip(start)
            .take(count)
            .filter_map(|&i| q.get(i).map(|t| (i, t.clone())))
            .collect()
    }

    /// Set the current shuffle behavior.
    pub fn set_shuffle(&self, new: bool) {
        self.cfg.with_state_mut(|s| s.shuffle = new);
        if new {
            self.generate_random_order();
        } else {
            self.restore_unshuffled_order();
        }
    }

    /// Handle events that are specific to the queue.
    pub fn handle_event(&self, event: QueueEvent) {
        match event {
            QueueEvent::PreloadTrackRequest => {
                if let Some(next_index) = self.next_index() {
                    let track = self.queue.read().unwrap()[next_index].clone();
                    debug!("Preloading track {track} as requested by librespot");
                    self.spotify.preload(&track);
                }
            }
            QueueEvent::Autoplay { after, tracks } => self.add_autoplay(&after, tracks),
        }
    }

    /// Get the spotify session.
    pub fn get_spotify(&self) -> Spotify {
        self.spotify.clone()
    }
}

/// Id of the last notification shown by ttunes
#[cfg(all(feature = "notify", unix, not(target_os = "macos")))]
static LAST_NOTIFICATION_ID: std::sync::Mutex<Option<u32>> = std::sync::Mutex::new(None);

/// Insert `source` at `index` in `sources`, padding it first if it's fallen out of step with the
/// queue (so a mismatch can never panic).
fn insert_source(sources: &mut Vec<Option<Arc<str>>>, index: usize, source: Option<Arc<str>>) {
    if sources.len() < index {
        sources.resize(index, None);
    }
    sources.insert(index, source);
}

/// Send a notification using the desktops default notification method.
///
/// `summary_txt`: A short title for the notification.
/// `body_txt`: The actual content of the notification.
/// `cover_url`: A URL to an image to show in the notification.
///
/// reuses the id of the last notification shown if possible
#[cfg(feature = "notify")]
pub fn send_notification(summary_txt: &str, body_txt: &str, cover_url: Option<String>) {
    let mut n = Notification::new();
    n.appname("ttunes").summary(summary_txt).body(body_txt);

    // album cover image
    if let Some(u) = cover_url {
        let path = crate::utils::cache_path_for_url(u.to_string());
        if !path.exists()
            && crate::utils::local_path(&u).is_none()
            && let Err(e) = crate::utils::download(u, path.clone())
        {
            log::error!("Failed to download cover: {e}");
        }
        n.icon(path.to_str().unwrap());
    }

    // XDG desktop entry hints
    #[cfg(all(unix, not(target_os = "macos")))]
    n.urgency(notify_rust::Urgency::Low)
        .hint(notify_rust::Hint::Transient(true))
        .hint(notify_rust::Hint::DesktopEntry("ttunes".into()));

    // reuse the id of the previous notification
    // so we replace it and not create a new one
    #[cfg(all(unix, not(target_os = "macos")))]
    if let Some(notification_id) = *LAST_NOTIFICATION_ID.lock().unwrap() {
        n.id(notification_id);
    }

    match n.show() {
        Ok(handle) => {
            // only available for XDG
            #[cfg(all(unix, not(target_os = "macos")))]
            {
                info!("Created notification: {}", handle.id());
                *LAST_NOTIFICATION_ID.lock().unwrap() = Some(handle.id());
            }
            #[cfg(not(all(unix, not(target_os = "macos"))))]
            drop(handle);
        }
        Err(e) => log::error!("Failed to send notification cover: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, RwLock};

    use super::*;
    use crate::config::Config;
    use crate::events::EventManager;
    use crate::library::Library;
    use crate::model::track::Track;
    use crate::spotify::Spotify;

    fn make_track(id: u32) -> Playable {
        Playable::Track(Track {
            id: Some(format!("id_{id}")),
            uri: format!("spotify:track:id_{id}"),
            title: format!("Track {id}"),
            track_number: id,
            disc_number: 1,
            duration: 180_000,
            artists: vec!["Artist".to_string()],
            artist_ids: vec![],
            album: Some("Album".to_string()),
            album_id: None,
            album_artists: vec![],
            cover_url: None,
            url: String::new(),
            added_at: None,
            list_index: 0,
            is_local: false,
            // Must be Some(true) so Spotify::load() doesn't fire events.
            is_playable: Some(true),
        })
    }

    fn track_id(p: &Playable) -> &str {
        match p {
            Playable::Track(t) => t.id.as_deref().unwrap_or(""),
            Playable::Episode(_) => "",
        }
    }

    fn make_queue(tracks: Vec<Playable>, current: Option<usize>) -> Queue {
        let len = tracks.len();
        let cfg = Config::new_for_test();
        let ev = EventManager::new_for_test();
        let spotify = Spotify::new_for_test(cfg.clone(), ev.clone());
        let library = Library::new_for_test(ev, spotify.clone(), cfg.clone());
        Queue {
            queue: Arc::new(RwLock::new(tracks)),
            random_order: RwLock::new(None),
            unshuffled: RwLock::new(None),
            sources: RwLock::new(vec![None; len]),
            current_track: RwLock::new(current),
            sleep_timer: RwLock::new(None),
            loaded_at: RwLock::new(None),
            failed_in_a_row: RwLock::new(0),
            autoplay_after: RwLock::new(None),
            waiting_for_autoplay: RwLock::new(false),
            spotify,
            cfg,
            library,
        }
    }

    // --- loading ---

    #[test]
    fn test_play_pause_while_loading_keeps_the_track() {
        let q = make_queue(vec![make_track(0), make_track(1), make_track(2)], None);
        let _worker = q.spotify.connect_for_test();
        q.play(0, false, false);
        assert_eq!(q.spotify.loading(), Some(true));
        // Pressed while it loads: it won't start, and it's still the same track.
        q.toggleplayback();
        assert_eq!(q.spotify.loading(), Some(false));
        assert_eq!(q.get_current_index(), Some(0));
        q.toggleplayback();
        assert_eq!(q.spotify.loading(), Some(true));
        assert_eq!(q.get_current_index(), Some(0));
        // Once the player has word, it's loaded.
        q.spotify
            .update_status(PlayerEvent::Playing(std::time::SystemTime::now()));
        assert_eq!(q.spotify.loading(), None);
    }

    #[test]
    fn test_tracks_failing_one_after_another_stop_playback() {
        let q = make_queue((0..10).map(make_track).collect(), None);
        q.play(0, false, false);
        // Each "finishes" at once, as when Spotify refuses them: the first two go on to the next.
        q.next(false);
        q.next(false);
        assert_eq!(q.get_current_index(), Some(2));
        // The third in a row stops, instead of skipping through the queue.
        q.next(false);
        assert_eq!(q.get_current_index(), Some(2));
        // Skipping by hand still works.
        q.next(true);
        assert_eq!(q.get_current_index(), Some(3));
    }

    // --- autoplay ---

    fn autoplay_tracks() -> Vec<Playable> {
        vec![make_track(10), make_track(11)]
    }

    #[test]
    fn test_autoplay_adds_tracks_after_the_last_one() {
        let q = make_queue(vec![make_track(0), make_track(1)], Some(1));
        q.add_autoplay("spotify:track:id_1", autoplay_tracks());
        assert_eq!(ids(&q), ["id_0", "id_1", "id_10", "id_11"]);
        assert_eq!(source_of(&q, "id_10").as_deref(), Some(AUTOPLAY_SOURCE));
        // Still on the same track: nothing started playing.
        assert_eq!(q.get_current_index(), Some(1));
    }

    #[test]
    fn test_autoplay_is_dropped_when_no_longer_last() {
        let q = make_queue(vec![make_track(0), make_track(1)], Some(1));
        q.append(make_track(2));
        q.add_autoplay("spotify:track:id_1", autoplay_tracks());
        assert_eq!(ids(&q), ["id_0", "id_1", "id_2"]);
    }

    #[test]
    fn test_autoplay_plays_when_the_queue_ran_out() {
        let q = make_queue(vec![make_track(0), make_track(1)], Some(1));
        *q.waiting_for_autoplay.write().unwrap() = true;
        q.add_autoplay("spotify:track:id_1", autoplay_tracks());
        assert_eq!(q.get_current_index(), Some(2));
    }

    #[test]
    fn test_queued_tracks_go_ahead_of_autoplay() {
        let q = make_queue(vec![make_track(0), make_track(1)], Some(1));
        q.add_autoplay("spotify:track:id_1", autoplay_tracks());
        q.append(make_track(2));
        assert_eq!(ids(&q), ["id_0", "id_1", "id_2", "id_10", "id_11"]);
        assert_eq!(source_of(&q, "id_2"), None);
    }

    // --- sleep timer ---

    #[test]
    fn test_sleep_at_end_of_track_is_used_up_by_the_track_ending() {
        let q = make_queue(vec![make_track(0), make_track(1), make_track(2)], Some(0));
        q.set_sleep_timer(Some(SleepTimer::EndOfTrack));
        q.next(true);
        assert_eq!(q.sleep_timer(), Some(SleepTimer::EndOfTrack));
        q.next(false);
        assert_eq!(q.sleep_timer(), None);
        assert_eq!(q.get_current_index(), Some(2));
    }

    #[test]
    fn test_sleep_timer_pauses_only_when_due() {
        let q = make_queue(vec![make_track(0)], Some(0));
        let later = Instant::now() + std::time::Duration::from_secs(60);
        q.set_sleep_timer(Some(SleepTimer::At(later)));
        assert!(!q.sleep_if_due());
        assert_eq!(q.sleep_timer(), Some(SleepTimer::At(later)));

        q.set_sleep_timer(Some(SleepTimer::At(Instant::now())));
        assert!(q.sleep_if_due());
        assert_eq!(q.sleep_timer(), None);
        assert!(!q.sleep_if_due());
    }

    #[test]
    fn test_sleep_at_end_of_track_is_not_due_on_a_clock() {
        let q = make_queue(vec![make_track(0)], Some(0));
        q.set_sleep_timer(Some(SleepTimer::EndOfTrack));
        assert!(!q.sleep_if_due());
        assert_eq!(q.sleep_timer(), Some(SleepTimer::EndOfTrack));
    }

    // --- next_index / previous_index ---

    #[test]
    fn test_next_index_basic() {
        let q = make_queue(vec![make_track(0), make_track(1), make_track(2)], Some(0));
        assert_eq!(q.next_index(), Some(1));
    }

    #[test]
    fn test_next_index_at_end_returns_none() {
        let q = make_queue(vec![make_track(0), make_track(1), make_track(2)], Some(2));
        assert_eq!(q.next_index(), None);
    }

    #[test]
    fn test_next_index_no_current_returns_none() {
        let q = make_queue(vec![make_track(0), make_track(1)], None);
        assert_eq!(q.next_index(), None);
    }

    #[test]
    fn test_previous_index_basic() {
        let q = make_queue(vec![make_track(0), make_track(1), make_track(2)], Some(2));
        assert_eq!(q.previous_index(), Some(1));
    }

    #[test]
    fn test_previous_index_at_start_returns_none() {
        let q = make_queue(vec![make_track(0), make_track(1)], Some(0));
        assert_eq!(q.previous_index(), None);
    }

    #[test]
    fn test_previous_index_no_current_returns_none() {
        let q = make_queue(vec![make_track(0)], None);
        assert_eq!(q.previous_index(), None);
    }

    // --- next / previous with shuffle ---

    #[test]
    fn test_next_index_respects_random_order() {
        // Queue [0,1,2,3], current_track=2, random_order=[2,0,3,1]
        // Position of 2 in order = 0, next position = 1, order[1] = 0 → Some(0)
        let q = make_queue(
            vec![make_track(0), make_track(1), make_track(2), make_track(3)],
            Some(2),
        );
        *q.random_order.write().unwrap() = Some(vec![2, 0, 3, 1]);
        assert_eq!(q.next_index(), Some(0));
    }

    #[test]
    fn test_previous_index_respects_random_order() {
        // Queue [0,1,2,3], current_track=0, random_order=[2,0,3,1]
        // Position of 0 in order = 1, prev position = 0, order[0] = 2 → Some(2)
        let q = make_queue(
            vec![make_track(0), make_track(1), make_track(2), make_track(3)],
            Some(0),
        );
        *q.random_order.write().unwrap() = Some(vec![2, 0, 3, 1]);
        assert_eq!(q.previous_index(), Some(2));
    }

    #[test]
    fn test_next_index_at_end_of_shuffle_order_returns_none() {
        // Queue [0,1,2,3], current_track=1, random_order=[2,0,3,1]
        // Position of 1 in order = 3 (last), next = 4 → None
        let q = make_queue(
            vec![make_track(0), make_track(1), make_track(2), make_track(3)],
            Some(1),
        );
        *q.random_order.write().unwrap() = Some(vec![2, 0, 3, 1]);
        assert_eq!(q.next_index(), None);
    }

    // --- skip_index (count-prefixed next/previous) ---

    #[test]
    fn test_skip_index_forward_and_backward() {
        let q = make_queue(
            vec![make_track(0), make_track(1), make_track(2), make_track(3)],
            Some(1),
        );
        assert_eq!(q.skip_index(2), Some(3));
        assert_eq!(q.skip_index(-1), Some(0));
    }

    #[test]
    fn test_skip_index_clamps_to_queue_bounds() {
        let q = make_queue(vec![make_track(0), make_track(1), make_track(2)], Some(1));
        assert_eq!(q.skip_index(100), Some(2));
        assert_eq!(q.skip_index(-100), Some(0));
    }

    #[test]
    fn test_skip_index_no_current_returns_none() {
        let q = make_queue(vec![make_track(0), make_track(1)], None);
        assert_eq!(q.skip_index(1), None);
    }

    #[test]
    fn test_skip_index_respects_random_order() {
        // Queue [0,1,2,3], current_track=2, random_order=[2,0,3,1]
        // Position of 2 in order = 0, +2 = 2, order[2] = 3 → Some(3)
        let q = make_queue(
            vec![make_track(0), make_track(1), make_track(2), make_track(3)],
            Some(2),
        );
        *q.random_order.write().unwrap() = Some(vec![2, 0, 3, 1]);
        assert_eq!(q.skip_index(2), Some(3));
    }

    // --- len, get_current ---

    #[test]
    fn test_len() {
        let q = make_queue(vec![make_track(0), make_track(1), make_track(2)], None);
        assert_eq!(q.len(), 3);
    }

    #[test]
    fn test_get_current_returns_right_track() {
        let q = make_queue(vec![make_track(0), make_track(1), make_track(2)], Some(1));
        let current = q.get_current().expect("should have current");
        assert_eq!(track_id(&current), "id_1");
    }

    #[test]
    fn test_get_current_none_when_no_current() {
        let q = make_queue(vec![make_track(0)], None);
        assert!(q.get_current().is_none());
    }

    // --- append / append_next / insert_after_current ---

    #[test]
    fn test_append_increases_len() {
        let q = make_queue(vec![make_track(0)], None);
        q.append(make_track(1));
        assert_eq!(q.len(), 2);
    }

    #[test]
    fn test_append_adds_to_end() {
        let q = make_queue(vec![make_track(0), make_track(1)], None);
        q.append(make_track(2));
        let queue = q.queue.read().unwrap();
        assert_eq!(track_id(&queue[2]), "id_2");
    }

    #[test]
    fn test_append_next_inserts_after_current() {
        // [0, 1, 2], current=1 → append_next([3, 4]) → [0, 1, 3, 4, 2]
        let q = make_queue(vec![make_track(0), make_track(1), make_track(2)], Some(1));
        let first = q.append_next(&[make_track(3), make_track(4)]);
        assert_eq!(first, 2);
        let queue = q.queue.read().unwrap();
        let ids: Vec<&str> = queue.iter().map(track_id).collect();
        assert_eq!(ids, ["id_0", "id_1", "id_3", "id_4", "id_2"]);
    }

    #[test]
    fn test_append_next_no_current_appends_at_end() {
        let q = make_queue(vec![make_track(0), make_track(1)], None);
        let first = q.append_next(&[make_track(2)]);
        assert_eq!(first, 2);
        assert_eq!(q.len(), 3);
    }

    #[test]
    fn test_insert_after_current() {
        // [0, 1], current=0 → insert 99 → [0, 99, 1], current still 0, next is 99
        let q = make_queue(vec![make_track(0), make_track(1)], Some(0));
        q.insert_after_current(make_track(99));
        assert_eq!(q.len(), 3);
        assert_eq!(q.get_current_index(), Some(0));
        let queue = q.queue.read().unwrap();
        assert_eq!(track_id(&queue[1]), "id_99");
        assert_eq!(track_id(&queue[2]), "id_1");
    }

    #[test]
    fn test_insert_after_current_no_current_appends() {
        let q = make_queue(vec![make_track(0)], None);
        q.insert_after_current(make_track(1));
        assert_eq!(q.len(), 2);
    }

    // --- remove ---

    #[test]
    fn test_remove_item_after_current_keeps_index() {
        // [0, 1, 2], current=0, remove(2) → current stays 0
        let q = make_queue(vec![make_track(0), make_track(1), make_track(2)], Some(0));
        q.remove(2);
        assert_eq!(q.len(), 2);
        assert_eq!(q.get_current_index(), Some(0));
    }

    #[test]
    fn test_remove_item_before_current_adjusts_index() {
        // [0, 1, 2], current=2, remove(0) → current becomes 1
        let q = make_queue(vec![make_track(0), make_track(1), make_track(2)], Some(2));
        q.remove(0);
        assert_eq!(q.len(), 2);
        assert_eq!(q.get_current_index(), Some(1));
    }

    // --- shift ---

    #[test]
    fn test_shift_moves_item() {
        // [0, 1, 2], shift(2, 0) → [2, 0, 1]
        let q = make_queue(vec![make_track(0), make_track(1), make_track(2)], None);
        q.shift(2, 0);
        let queue = q.queue.read().unwrap();
        let ids: Vec<&str> = queue.iter().map(track_id).collect();
        assert_eq!(ids, ["id_2", "id_0", "id_1"]);
    }

    #[test]
    fn test_shift_updates_current_when_current_is_moved() {
        // [0, 1, 2], current=2, shift(2, 0) → current becomes 0
        let q = make_queue(vec![make_track(0), make_track(1), make_track(2)], Some(2));
        q.shift(2, 0);
        assert_eq!(q.get_current_index(), Some(0));
    }

    // --- shuffle ---

    #[test]
    fn test_shuffle_generates_random_order() {
        let q = make_queue(
            vec![make_track(0), make_track(1), make_track(2), make_track(3)],
            Some(1),
        );
        q.set_shuffle(true);
        // Shuffling reorders the queue itself: current track (id_1) first, every track once.
        let order = ids(&q);
        assert_eq!(order[0], "id_1");
        let mut sorted = order.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, ["id_0", "id_1", "id_2", "id_3"]);
    }

    #[test]
    fn test_shuffle_disable_clears_order() {
        let q = make_queue(vec![make_track(0), make_track(1)], Some(0));
        q.set_shuffle(true);
        q.set_shuffle(false);
        assert!(q.get_random_order().is_none());
        assert_eq!(ids(&q), ["id_0", "id_1"]);
    }

    #[test]
    fn test_append_in_shuffle_mode_maintains_valid_order() {
        // Appending to a shuffled queue adds the track at the end of the (shuffled) order.
        let q = make_queue(vec![make_track(0), make_track(1)], Some(0));
        q.set_shuffle(true);
        q.append(make_track(2));
        let order = ids(&q);
        assert_eq!(order.len(), 3);
        assert_eq!(order[2], "id_2");
    }

    // --- clear ---

    #[test]
    fn test_clear_empties_queue() {
        let q = make_queue(vec![make_track(0), make_track(1), make_track(2)], Some(1));
        q.clear();
        assert_eq!(q.len(), 0);
    }

    #[test]
    fn test_clear_when_shuffled() {
        let q = make_queue(vec![make_track(0), make_track(1), make_track(2)], Some(1));
        q.set_shuffle(true);
        q.clear();
        assert_eq!(q.len(), 0);
        // Shuffling reorders the queue itself, so there's no separate play order.
        assert_eq!(q.get_random_order(), None);
        assert!(q.unshuffled.read().unwrap().is_none());
    }

    // --- shuffle reorders the queue itself ---

    fn ids(q: &Queue) -> Vec<String> {
        q.queue
            .read()
            .unwrap()
            .iter()
            .map(|p| track_id(p).to_string())
            .collect()
    }

    fn make_tracks(n: u32) -> Vec<Playable> {
        (0..n).map(make_track).collect()
    }

    #[test]
    fn test_shuffle_puts_current_first_and_keeps_tracks() {
        let q = make_queue(make_tracks(20), Some(7));
        q.set_shuffle(true);

        assert_eq!(q.get_current_index(), Some(0));
        assert_eq!(track_id(&q.get_current().unwrap()), "id_7");
        assert_eq!(q.get_random_order(), None);

        let mut sorted = ids(&q);
        sorted.sort();
        let mut expected: Vec<String> = (0..20).map(|i| format!("id_{i}")).collect();
        expected.sort();
        assert_eq!(sorted, expected);
    }

    #[test]
    fn test_shuffle_plays_in_displayed_order() {
        let q = make_queue(make_tracks(10), Some(3));
        q.set_shuffle(true);
        // With the queue itself shuffled, playback order is simply the list order.
        assert_eq!(q.next_index(), Some(1));
        assert_eq!(q.previous_index(), None);
    }

    #[test]
    fn test_unshuffle_restores_original_order_and_current() {
        let q = make_queue(make_tracks(10), Some(3));
        q.set_shuffle(true);
        q.set_shuffle(false);

        let expected: Vec<String> = (0..10).map(|i| format!("id_{i}")).collect();
        assert_eq!(ids(&q), expected);
        assert_eq!(q.get_current_index(), Some(3));
        assert_eq!(track_id(&q.get_current().unwrap()), "id_3");
    }

    #[test]
    fn test_unshuffle_follows_current_track() {
        let q = make_queue(make_tracks(10), Some(3));
        q.set_shuffle(true);
        // Move on a few tracks while shuffled.
        *q.current_track.write().unwrap() = Some(4);
        let playing = track_id(&q.get_current().unwrap()).to_string();

        q.set_shuffle(false);
        assert_eq!(track_id(&q.get_current().unwrap()), playing);
    }

    #[test]
    fn test_unshuffle_keeps_added_and_drops_removed() {
        let q = make_queue(make_tracks(5), Some(0));
        q.set_shuffle(true);
        q.append(make_track(99));
        let removed_index = ids(&q).iter().position(|id| id == "id_2").unwrap();
        q.remove(removed_index);

        q.set_shuffle(false);
        assert_eq!(ids(&q), vec!["id_0", "id_1", "id_3", "id_4", "id_99"]);
    }

    #[test]
    fn test_unshuffle_handles_duplicate_tracks() {
        let q = make_queue(
            vec![make_track(1), make_track(2), make_track(1), make_track(3)],
            Some(0),
        );
        q.set_shuffle(true);
        q.set_shuffle(false);
        assert_eq!(ids(&q), vec!["id_1", "id_2", "id_1", "id_3"]);
    }

    #[test]
    fn test_remove_while_shuffled_keeps_remaining_order() {
        let q = make_queue(make_tracks(10), Some(0));
        q.set_shuffle(true);
        let mut before = ids(&q);
        q.remove(5);
        before.remove(5);
        assert_eq!(ids(&q), before);
    }

    // --- sources ("Next from: …") ---

    fn source_of(q: &Queue, id: &str) -> Option<String> {
        let index = ids(q).iter().position(|i| i == id).unwrap();
        q.source(index).map(|s| s.to_string())
    }

    #[test]
    fn test_append_next_from_records_source() {
        let q = make_queue(make_tracks(2), Some(0));
        q.append_next_from(&[make_track(10), make_track(11)], Some("Album"));
        assert_eq!(ids(&q), ["id_0", "id_10", "id_11", "id_1"]);
        assert_eq!(source_of(&q, "id_10").as_deref(), Some("Album"));
        assert_eq!(source_of(&q, "id_11").as_deref(), Some("Album"));
        assert_eq!(source_of(&q, "id_1"), None);
    }

    #[test]
    fn test_sources_follow_tracks_through_shuffle_and_removal() {
        let q = make_queue(make_tracks(1), Some(0));
        q.append_next_from(&make_tracks(20)[1..], Some("Playlist"));
        q.append(make_track(99));
        q.set_shuffle(true);
        assert_eq!(source_of(&q, "id_99"), None);
        assert_eq!(source_of(&q, "id_7").as_deref(), Some("Playlist"));

        let index = ids(&q).iter().position(|i| i == "id_3").unwrap();
        q.remove(index);
        q.set_shuffle(false);
        assert_eq!(source_of(&q, "id_99"), None);
        assert_eq!(source_of(&q, "id_12").as_deref(), Some("Playlist"));
        assert_eq!(q.sources.read().unwrap().len(), q.len());
    }

    #[test]
    fn test_upcoming_lists_items_after_current() {
        let q = make_queue(make_tracks(5), Some(1));
        let upcoming: Vec<usize> = q.upcoming(10).into_iter().map(|(i, _)| i).collect();
        assert_eq!(upcoming, [2, 3, 4]);
        assert_eq!(q.upcoming(2).len(), 2);

        let q = make_queue(make_tracks(3), None);
        assert_eq!(q.upcoming(10).len(), 3);
    }

    #[test]
    fn test_sources_for_saving_line_up_with_queue() {
        let q = make_queue(make_tracks(2), Some(0));
        q.append_next_from(&[make_track(5)], Some("Album"));
        assert_eq!(q.sources(), [None, Some("Album".to_string()), None]);
    }
}
