use std::cmp::Ordering;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, RwLock};
use std::time::Instant;

use log::{debug, info, warn};
#[cfg(feature = "notify")]
use notify_rust::Notification;

use rand::prelude::*;
use strum_macros::Display;

use crate::config::Config;
use crate::library::Library;
use crate::model::playable::Playable;
use crate::model::track::Track;
use crate::spotify::PlayerEvent;
use crate::spotify::Spotify;
use crate::traits::{ListItem, OpenTarget};

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
    /// `manual` if asked for with [Queue::autoplay_now].
    Autoplay {
        after: String,
        tracks: Vec<Playable>,
        manual: bool,
    },
    /// The tracks of the radio seeded with `seed`; see [Queue::request_radio].
    Radio {
        seed: Box<RadioSeed>,
        tracks: Vec<Playable>,
    },
}

/// What a radio is made from, as with "Go to radio" in Spotify's apps: a song, album or artist.
#[derive(Clone, Debug)]
pub struct RadioSeed {
    /// The Spotify URI of the song, album or artist.
    pub uri: String,
    /// Its name, which names the radio.
    pub name: String,
    /// The song, which plays first, when it's a song's radio.
    pub track: Option<Track>,
}

impl RadioSeed {
    /// The radio for `item`, if it can have one: a song on Spotify (not a local file), an album
    /// or an artist. Not podcasts, playlists or anything else.
    pub fn of(item: &dyn ListItem) -> Option<Self> {
        match item.target() {
            Some(OpenTarget::Album(album)) => Some(Self {
                uri: format!("spotify:album:{}", album.id?),
                name: album.title,
                track: None,
            }),
            Some(OpenTarget::Artist(artist)) => Some(Self {
                uri: format!("spotify:artist:{}", artist.id?),
                name: artist.name,
                track: None,
            }),
            Some(_) => None,
            None => {
                let track = item
                    .track()
                    .filter(|track| !track.is_local && track.uri.starts_with("spotify:track:"))?;
                Some(Self {
                    uri: track.uri.clone(),
                    name: track.title.clone(),
                    track: Some(track),
                })
            }
        }
    }

    /// The radio's name, its tracks' source ("Next from: …").
    pub fn label(&self) -> String {
        format!("{} Radio", self.name)
    }
}

/// How many recently listened to tracks autoplay is told to leave out.
const AUTOPLAY_RECENT: usize = 50;

/// The last `limit` distinct track URIs listened to, oldest first and ending with `current`:
/// those `started` this session (oldest first), then from the listening `history` (oldest first)
/// before it.
fn recent_tracks(
    started: &[String],
    history: &[String],
    current: &str,
    limit: usize,
) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut recent: Vec<String> = std::iter::once(current)
        .chain(started.iter().rev().map(String::as_str))
        .chain(history.iter().rev().map(String::as_str))
        .filter(|uri| seen.insert(*uri))
        .take(limit)
        .map(String::from)
        .collect();
    recent.reverse();
    recent
}

/// The source ("Next from: …") of tracks added by autoplay.
pub const AUTOPLAY_SOURCE: &str = "Autoplay";

/// The queue determines the playback order of [Playable] items, and is also used to control
/// playback itself.
///
/// It's used from more than one thread (the UI's and MPRIS's), so its locks are only ever taken
/// in this order, each released before one earlier in it is taken: `queue`, `sources`,
/// `current_track`, `unshuffled`. The others are only held on their own. A guard is never held
/// across a call to another method that locks: that could take a lock twice, and a second read
/// of a `RwLock` waits behind a waiting writer, so even that can deadlock.
pub struct Queue {
    /// The internal data, which doesn't change with shuffle or repeat. This is
    /// the raw data only.
    pub queue: Arc<RwLock<Vec<Playable>>>,
    /// The queue as it was before shuffling, so turning shuffle off can restore that order.
    unshuffled: RwLock<Option<Vec<Playable>>>,
    /// Where each item in `self.queue` was queued from (an album or playlist name), shown as
    /// "Next from: …" in the queue panel. Always as long as `self.queue`; `None` for items
    /// queued individually or restored from a previous session.
    sources: RwLock<Vec<Option<Arc<str>>>>,
    current_track: RwLock<Option<usize>>,
    sleep_timer: RwLock<Option<SleepTimer>>,
    /// When the current track was loaded, and how many tracks in a row "finished" moments after
    /// loading, as ones that fail to play do; see [Queue::failing].
    loaded_at: RwLock<Option<Instant>>,
    failed_in_a_row: RwLock<u32>,
    /// How many tracks in a row were skipped as unplayable; see [Queue::skip_unplayable].
    unplayable_in_a_row: RwLock<usize>,
    /// The URI of the track autoplay is being asked to follow, until its answer arrives (used or
    /// not), so it isn't asked twice at once.
    autoplay_after: RwLock<Option<String>>,
    /// The queue ran out while autoplay was still looking: play what it finds.
    waiting_for_autoplay: RwLock<bool>,
    /// The URIs of the tracks started this session, oldest first, skipped ones too, so autoplay
    /// can leave them out; see [recent_tracks].
    started: RwLock<Vec<String>>,
    /// The URI of the radio last asked for, until it starts: an older one arriving late doesn't.
    radio_pending: RwLock<Option<String>>,
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

        // The play order older versions saved for shuffle (`random_order`) isn't used: shuffling
        // reorders the queue itself now, and that order fell out of step as the queue changed.
        // The saved queue plays in the order it's in.
        let current_track = queue_state
            .current_track
            .filter(|&i| i < queue_state.queue.len());

        Self {
            queue: Arc::new(RwLock::new(queue_state.queue)),
            spotify: spotify.clone(),
            current_track: RwLock::new(current_track),
            sleep_timer: RwLock::new(None),
            loaded_at: RwLock::new(None),
            failed_in_a_row: RwLock::new(0),
            unplayable_in_a_row: RwLock::new(0),
            autoplay_after: RwLock::new(None),
            waiting_for_autoplay: RwLock::new(false),
            started: RwLock::new(Vec::new()),
            radio_pending: RwLock::new(None),
            unshuffled: RwLock::new(None),
            sources: RwLock::new(sources),
            cfg,
            library,
        }
    }

    /// The index of the next item in `self.queue` that should be played. None
    /// if at the end of the queue. Shuffling reorders the queue itself, so the
    /// play order is the queue's order.
    pub fn next_index(&self) -> Option<usize> {
        let len = self.queue.read().unwrap().len();
        let next = self.get_current_index()? + 1;
        (next < len).then_some(next)
    }

    /// The index of the previous item in `self.queue` that should be played.
    /// None if at the start of the queue.
    pub fn previous_index(&self) -> Option<usize> {
        self.get_current_index()?.checked_sub(1)
    }

    /// The `self.queue` index `offset` items from the current one in play
    /// order (negative = backward), clamped to the queue bounds. None if
    /// nothing is playing or the queue is empty.
    fn skip_index(&self, offset: isize) -> Option<usize> {
        let len = self.queue.read().unwrap().len();
        let pos = self.get_current_index()?;
        (len > 0).then(|| (pos as isize + offset).clamp(0, len as isize - 1) as usize)
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
        let index = self.get_current_index()?;
        self.queue.read().unwrap().get(index).cloned()
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
        let mut q = self.queue.write().unwrap();
        let mut sources = self.sources.write().unwrap();
        let current = *self.current_track.read().unwrap();
        match current {
            Some(index) if index < q.len() => {
                q.insert(index + 1, track);
                insert_source(&mut sources, index + 1, None);
            }
            _ => {
                // As `append`.
                let index = self.autoplay_start(&q, &sources);
                q.insert(index, track);
                insert_source(&mut sources, index, None);
            }
        }
    }

    /// Add `track` to the end of the queue.
    pub fn append(&self, track: Playable) {
        let mut q = self.queue.write().unwrap();
        let mut sources = self.sources.write().unwrap();
        // Ahead of what autoplay added, as in Spotify's apps.
        let index = self.autoplay_start(&q, &sources);
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
        let mut sources = self.sources.write().unwrap();
        let first = match *self.current_track.read().unwrap() {
            Some(index) => (index + 1).min(q.len()),
            None => q.len(),
        };

        let source: Option<Arc<str>> = source.map(Arc::from);
        for (i, track) in (first..).zip(tracks.iter()) {
            q.insert(i, track.clone());
            insert_source(&mut sources, i, source.clone());
        }

        first
    }

    /// Remove the item at `index`. This doesn't take into account shuffle
    /// status, and will literally remove the item at `index` in `self.queue`.
    pub fn remove(&self, index: usize) {
        let removing_current = self.get_current_index() == Some(index);
        if removing_current {
            // Noted while it's still the current track, as far as it was listened to: what's
            // played next isn't noted again, as there's no current track until then.
            self.record_listened();
        }
        let len = {
            let mut q = self.queue.write().unwrap();
            let mut sources = self.sources.write().unwrap();
            let mut current = self.current_track.write().unwrap();
            if index >= q.len() {
                info!("nothing to remove at {index}");
                return;
            }
            q.remove(index);
            if index < sources.len() {
                sources.remove(index);
            }
            // The current track's index follows it, or there's none if it was removed.
            *current = match *current {
                Some(c) => match c.cmp(&index) {
                    Ordering::Less => Some(c),
                    Ordering::Equal => None,
                    Ordering::Greater => Some(c - 1),
                },
                None => None,
            };
            q.len()
        };

        if removing_current {
            if index < len {
                // What came after it is now where it was.
                self.play(index, false, false);
            } else if len > 0 && self.get_repeat() == RepeatSetting::RepeatPlaylist {
                // It was the last: back to the start.
                self.play(0, false, false);
            } else {
                self.stop();
            }
        } else if len == 0 {
            self.stop();
        }

        // Shuffling reorders the queue itself, so a removal leaves the remaining (already
        // shuffled) order intact rather than reshuffling everything.

        // What was up next may be gone: autoplay follows the current track instead.
        self.autoplay_if_last();
    }

    /// Clear all the items from the queue and stop playback.
    pub fn clear(&self) {
        self.stop();

        let mut q = self.queue.write().unwrap();
        let mut sources = self.sources.write().unwrap();
        q.clear();
        sources.clear();
        *self.unshuffled.write().unwrap() = None;
    }

    /// Remove everything after the current item, which keeps playing, and let autoplay pick what
    /// follows it. Clears the whole queue if nothing is playing.
    pub fn clear_upcoming(&self) {
        let Some(current) = *self.current_track.read().unwrap() else {
            return self.clear();
        };
        {
            let mut q = self.queue.write().unwrap();
            let mut sources = self.sources.write().unwrap();
            q.truncate(current + 1);
            sources.truncate(current + 1);
        }
        // Unless it's being asked for this track already: that answer is used.
        self.request_autoplay(current);
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
    fn load(&self, index: usize, reshuffle: bool, shuffle_index: bool, start_playing: bool) {
        self.record_listened();
        *self.unplayable_in_a_row.write().unwrap() = 0;
        self.load_unrecorded(index, reshuffle, shuffle_index, start_playing);
    }

    /// [Queue::load], without noting the current track in the listening history.
    fn load_unrecorded(
        &self,
        mut index: usize,
        reshuffle: bool,
        shuffle_index: bool,
        start_playing: bool,
    ) {
        *self.loaded_at.write().unwrap() = Some(Instant::now());
        let queue_length = self.queue.read().unwrap().len();
        // The length of the queue must be bigger than 0 or gen_range panics!
        if queue_length > 0 && shuffle_index && self.get_shuffle() {
            let mut rng = rand::rng();
            index = rng.random_range(0..queue_length);
        }

        let track = self.queue.read().unwrap().get(index).cloned();
        if let Some(track) = &track {
            // Before loading, which has MPRIS announce the current track.
            self.current_track.write().unwrap().replace(index);
            self.spotify.load(track, start_playing, 0);
            if let Playable::Track(track) = track {
                let mut started = self.started.write().unwrap();
                started.push(track.uri.clone());
                // Only the latest are sent to autoplay.
                if started.len() > 2 * AUTOPLAY_RECENT {
                    started.drain(..AUTOPLAY_RECENT);
                }
            }
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
        // Shuffling moves the current track to the start.
        if let Some(index) = self.get_current_index() {
            self.request_autoplay(index);
        }
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
        let current = self.get_current_index();
        // What's needed of the queue, taken before loading, which locks it again.
        let (current_playable, current_uri, any_playable) = {
            let q = self.queue.read().unwrap();
            let current = current.and_then(|i| q.get(i));
            (
                current.is_some_and(Playable::is_playable),
                current.map(Playable::uri),
                q.iter().any(Playable::is_playable),
            )
        };
        let repeat = self.cfg.state().repeat;
        // A track that ended with the sleep timer set to its end loads the next one paused.
        let start_playing = manual || !self.take_sleep_timer(SleepTimer::EndOfTrack);

        if repeat == RepeatSetting::RepeatTrack && !manual {
            if let Some(index) = current
                && current_playable
            {
                self.load(index, false, false, start_playing);
            }
        } else if let Some(index) = self.next_index() {
            self.load(index, false, false, start_playing);
            if repeat == RepeatSetting::RepeatTrack && manual {
                self.set_repeat(RepeatSetting::RepeatPlaylist);
            }
        } else if repeat == RepeatSetting::RepeatPlaylist && any_playable {
            self.load(0, false, false, start_playing);
        } else {
            self.run_out(current, current_uri, manual);
        }
    }

    /// The queue ran out after the track at `current` (with URI `current_uri`): stop. Autoplay
    /// may still be looking for what comes next: it plays it when it's found. Unless `manual`, if
    /// it wasn't asked yet (or found nothing last time), it's asked now.
    fn run_out(&self, current: Option<usize>, current_uri: Option<String>, manual: bool) {
        if !manual && let Some(index) = current {
            self.request_autoplay(index);
        }
        *self.waiting_for_autoplay.write().unwrap() =
            !manual && current_uri.is_some() && *self.autoplay_after.read().unwrap() == current_uri;
        self.spotify.stop();
    }

    /// Go on from the current track, with URI `uri`, which can't be played (greyed out in
    /// Spotify, say for being unavailable in this country): to the next, starting it if
    /// `start_playing`, whatever the repeat setting. It isn't noted in the listening history, nor
    /// counted as a failure to play (see [Queue::failing]). Once every track in the queue was
    /// skipped in a row, it stops.
    pub fn skip_unplayable(&self, uri: &str, start_playing: bool) {
        // Another track was picked meanwhile.
        if !self.is_current_uri(uri) {
            return;
        }
        // No word of it will come from the player.
        self.spotify.cancel_loading();
        let skipped = {
            let mut skipped = self.unplayable_in_a_row.write().unwrap();
            *skipped += 1;
            *skipped
        };
        let len = self.len();
        let next = self.next_index().or_else(|| {
            (self.get_repeat() == RepeatSetting::RepeatPlaylist && skipped < len).then_some(0)
        });
        match next {
            Some(index) => self.load_unrecorded(index, false, false, start_playing),
            None => self.run_out(self.get_current_index(), Some(uri.to_string()), false),
        }
    }

    /// Whether autoplay is on: the `autoplay` setting, and only without repeat.
    fn autoplay_enabled(&self) -> bool {
        self.cfg.values().autoplay.unwrap_or(true) && self.cfg.state().repeat == RepeatSetting::None
    }

    /// Ask autoplay for tracks to follow the current one if it's the last to play, as when it
    /// started: after the queue or the settings change.
    pub fn autoplay_if_last(&self) {
        if let Some(index) = self.get_current_index() {
            self.request_autoplay(index);
        }
    }

    /// When the track at `index` is the last one to play, ask autoplay for tracks to follow it,
    /// based on its album and the tracks listened to lately (see [recent_tracks]). Not asked
    /// again while it's being asked for this track.
    fn request_autoplay(&self, index: usize) {
        if !self.autoplay_enabled() || self.next_index().is_some() {
            return;
        }
        // Podcasts and local files have no autoplay.
        let Some(Playable::Track(track)) = self.queue.read().unwrap().get(index).cloned() else {
            return;
        };
        if track.is_local {
            return;
        }
        {
            let mut after = self.autoplay_after.write().unwrap();
            if after.as_deref() == Some(track.uri.as_str()) {
                return;
            }
            *after = Some(track.uri.clone());
        }
        self.ask_autoplay(&track, false);
    }

    /// Ask autoplay for tracks to follow the last song in the queue, now: whatever the setting,
    /// and even if it was asked already. Plays them if nothing's playing. The tracks arrive as
    /// [QueueEvent::Autoplay]; the error says why it can't be asked.
    pub fn autoplay_now(&self) -> Result<(), &'static str> {
        let (last, len) = {
            let q = self.queue.read().unwrap();
            (q.last().cloned(), q.len())
        };
        let track = match last {
            Some(Playable::Track(track)) if !track.is_local => track,
            Some(Playable::Track(_)) => return Err("Autoplay can't follow a local file"),
            Some(Playable::Episode(_)) => return Err("Autoplay can't follow a podcast"),
            None => return Err("Play a song first: autoplay follows it"),
        };
        // So it isn't asked again for the same track when it starts.
        if self.get_current_index() == Some(len - 1) {
            *self.autoplay_after.write().unwrap() = Some(track.uri.clone());
        }
        self.ask_autoplay(&track, true);
        Ok(())
    }

    /// The tracks listened to lately, ending with `current`, so autoplay doesn't repeat them: the
    /// ones started this session, skipped or not and wherever they are in the queue (it may have
    /// been cleared, shuffled or repeated), then the listening history from before.
    fn recent(&self, current: &str) -> Vec<String> {
        let history: Vec<String> = if cfg!(test) {
            Vec::new()
        } else {
            crate::stats::load()
                .into_iter()
                .map(|play| play.uri)
                .collect()
        };
        recent_tracks(
            &self.started.read().unwrap(),
            &history,
            current,
            AUTOPLAY_RECENT,
        )
    }

    fn ask_autoplay(&self, track: &Track, manual: bool) {
        let recent = self.recent(&track.uri);
        // Seeded with its album, or the track itself when that isn't known.
        let context = match &track.album_id {
            Some(album_id) => format!("spotify:album:{album_id}"),
            None => track.uri.clone(),
        };
        debug!("Asking autoplay for tracks after {} ({context})", track.uri);
        self.spotify
            .request_autoplay(context, recent, track.uri.clone(), manual);
    }

    /// Add `tracks` from autoplay after the track `after`. Asked for by itself, only if `after`
    /// is still the last one to play, and it plays the first if playback stopped at the end of
    /// the queue meanwhile. Asked for by hand (`manual`), after the end of the queue, and it
    /// plays the first if nothing's playing. Returns a message for the latter.
    fn add_autoplay(
        &self,
        after: &str,
        tracks: Vec<Playable>,
        manual: bool,
    ) -> Option<Result<String, String>> {
        // Answered, whether what it found is used or not: if `after` is the last to play again
        // (say the tracks it added are removed), it's asked again.
        {
            let mut asked = self.autoplay_after.write().unwrap();
            if asked.as_deref() == Some(after) {
                *asked = None;
            }
        }
        if tracks.is_empty() {
            warn!("Autoplay found nothing to follow {after}");
        }

        if manual {
            let last = self.queue.read().unwrap().last().map(Playable::uri);
            if last.as_deref() != Some(after) {
                // The end of the queue changed meanwhile.
                return None;
            }
            // The queue ran out at `after`, the last track, while this was asked.
            let waiting = std::mem::take(&mut *self.waiting_for_autoplay.write().unwrap());
            if tracks.is_empty() {
                return Some(Err("Autoplay found nothing: try again in a bit".into()));
            }
            let count = tracks.len();
            let stopped = waiting
                || self.get_current_index().is_none()
                || (self.spotify.get_current_status() == PlayerEvent::Stopped
                    && self.spotify.loading().is_none());
            let first = self.append_autoplay(tracks);
            if stopped {
                self.play(first, false, false);
            }
            return Some(Ok(format!("Autoplay added {count} songs")));
        }

        let still_last = self.next_index().is_none()
            && self.get_current().map(|t| t.uri()).as_deref() == Some(after);
        if !still_last {
            // Whatever's waiting is waiting for another track's answer.
            return None;
        }
        let waiting = std::mem::take(&mut *self.waiting_for_autoplay.write().unwrap());
        if !self.autoplay_enabled() || tracks.is_empty() {
            return None;
        }
        let first = self.append_autoplay(tracks);
        if waiting {
            self.play(first, false, false);
        }
        None
    }

    /// Ask for the radio seeded with `seed`, leaving out the tracks listened to lately and the
    /// seed song, which is put first itself. It replaces the queue when its tracks arrive, as
    /// [QueueEvent::Radio]; see [Queue::start_radio].
    pub fn request_radio(&self, seed: RadioSeed) {
        let recent = match &seed.track {
            Some(track) => self.recent(&track.uri),
            None => {
                let mut recent = self.recent("");
                recent.retain(|uri| !uri.is_empty());
                recent
            }
        };
        debug!("Asking for the radio of {}", seed.uri);
        *self.radio_pending.write().unwrap() = Some(seed.uri.clone());
        self.spotify.request_radio(seed, recent);
    }

    /// Replace the queue with the radio seeded with `seed` and play it: the seed song first, if
    /// it's a song's radio, then `tracks`, all from "<name> Radio". Autoplay continues it when
    /// they run out. If nothing came back the queue is left alone, with an error. A radio that
    /// isn't the last one asked for is dropped.
    fn start_radio(
        &self,
        seed: RadioSeed,
        mut tracks: Vec<Playable>,
    ) -> Option<Result<String, String>> {
        {
            let mut pending = self.radio_pending.write().unwrap();
            if pending.as_deref() != Some(seed.uri.as_str()) {
                return None;
            }
            *pending = None;
        }
        let mut seen = std::collections::HashSet::from([seed.uri.clone()]);
        tracks.retain(|track| seen.insert(track.uri()));
        if tracks.is_empty() {
            return Some(Err(format!(
                "Couldn't find a radio for {}: try again in a bit",
                seed.name
            )));
        }
        let label: Arc<str> = Arc::from(seed.label());
        let tracks: Vec<Playable> = seed
            .track
            .map(Playable::Track)
            .into_iter()
            .chain(tracks)
            .collect();
        self.clear();
        {
            let mut q = self.queue.write().unwrap();
            *self.sources.write().unwrap() = vec![Some(label); tracks.len()];
            *q = tracks;
        }
        *self.autoplay_after.write().unwrap() = None;
        *self.waiting_for_autoplay.write().unwrap() = false;
        self.play(0, false, false);
        None
    }

    /// Add `tracks` to the end of the queue, from autoplay. Returns the index of the first.
    fn append_autoplay(&self, tracks: Vec<Playable>) -> usize {
        let mut q = self.queue.write().unwrap();
        let mut sources = self.sources.write().unwrap();
        let first = q.len();
        for track in tracks {
            q.push(track);
            let len = q.len();
            insert_source(&mut sources, len - 1, Some(Arc::from(AUTOPLAY_SOURCE)));
        }
        first
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
        let len = self.len();
        let current = self.get_current_index();
        let repeat = self.cfg.state().repeat;

        if let Some(index) = self.previous_index() {
            self.play(index, false, false);
        } else if repeat == RepeatSetting::RepeatPlaylist && len > 0 {
            self.play(len - 1, false, false);
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
        // Autoplay is off with repeat on: back on, it follows the current track.
        self.autoplay_if_last();
    }

    /// Get the current shuffle behavior.
    pub fn get_shuffle(&self) -> bool {
        self.cfg.state().shuffle
    }

    /// Shuffle the queue: reorder `self.queue` so the current item comes first, followed by the
    /// rest in random order. Reordering the queue itself (rather than keeping a separate play
    /// order) means the queue view shows the order things will actually play in. The order
    /// before the first shuffle is kept so [Queue::set_shuffle] can restore it.
    fn generate_random_order(&self) {
        let mut q = self.queue.write().unwrap();
        let mut sources = self.sources.write().unwrap();
        let mut current = self.current_track.write().unwrap();
        {
            let mut unshuffled = self.unshuffled.write().unwrap();
            if unshuffled.is_none() {
                *unshuffled = Some(q.clone());
            }
        }
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
    }

    /// Undo [Queue::generate_random_order]: put the queue back in its order from before
    /// shuffling. Items added while shuffled keep their place at the end, removed ones stay
    /// removed, and the current item keeps playing.
    fn restore_unshuffled_order(&self) {
        let mut q = self.queue.write().unwrap();
        let mut sources = self.sources.write().unwrap();
        let mut current = self.current_track.write().unwrap();

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
        sources.resize(q.len(), None);
        let mut slots: Vec<Option<(Playable, Option<Arc<str>>)>> =
            q.drain(..).zip(sources.drain(..)).map(Some).collect();
        (*q, *sources) = order.iter().filter_map(|&i| slots[i].take()).unzip();
    }

    /// Where each item was queued from, for saving the queue between sessions.
    ///
    /// Doesn't lock `self.queue`, which `sources` is always as long as, so it can be called while
    /// holding that.
    pub fn sources(&self) -> Vec<Option<String>> {
        self.sources
            .read()
            .unwrap()
            .iter()
            .map(|s| s.as_deref().map(String::from))
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
        let start = self.current_track.read().unwrap().map_or(0, |i| i + 1);
        q.iter()
            .enumerate()
            .skip(start)
            .take(count)
            .map(|(i, t)| (i, t.clone()))
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

    /// Handle events that are specific to the queue. Returns a message to show, if any.
    pub fn handle_event(&self, event: QueueEvent) -> Option<Result<String, String>> {
        match event {
            QueueEvent::PreloadTrackRequest => {
                if let Some(track) = self
                    .next_index()
                    .and_then(|i| self.queue.read().unwrap().get(i).cloned())
                {
                    debug!("Preloading track {track} as requested by librespot");
                    self.spotify.preload(&track);
                }
                None
            }
            QueueEvent::Autoplay {
                after,
                tracks,
                manual,
            } => self.add_autoplay(&after, tracks, manual),
            QueueEvent::Radio { seed, tracks } => self.start_radio(*seed, tracks),
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
            unshuffled: RwLock::new(None),
            sources: RwLock::new(vec![None; len]),
            current_track: RwLock::new(current),
            sleep_timer: RwLock::new(None),
            loaded_at: RwLock::new(None),
            failed_in_a_row: RwLock::new(0),
            unplayable_in_a_row: RwLock::new(0),
            autoplay_after: RwLock::new(None),
            waiting_for_autoplay: RwLock::new(false),
            started: RwLock::new(Vec::new()),
            radio_pending: RwLock::new(None),
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
    fn test_the_end_of_the_track_before_is_ignored_while_the_next_loads() {
        let q = make_queue(make_tracks(3), Some(0));
        let _worker = q.spotify.connect_for_test();
        q.spotify
            .update_status(PlayerEvent::Playing(std::time::SystemTime::now()));
        assert!(!q.spotify.is_stale(&PlayerEvent::FinishedTrack));
        // Picked in the last moments of the one playing, which then says it finished.
        q.play(1, false, false);
        assert!(q.spotify.is_stale(&PlayerEvent::FinishedTrack));
        // Once the one picked plays, its end is its end.
        q.spotify
            .update_status(PlayerEvent::Playing(std::time::SystemTime::now()));
        assert!(!q.spotify.is_stale(&PlayerEvent::FinishedTrack));
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

    // --- unplayable tracks ---

    fn make_unplayable_track(id: u32) -> Playable {
        let Playable::Track(mut track) = make_track(id) else {
            unreachable!()
        };
        track.is_playable = Some(false);
        Playable::Track(track)
    }

    /// Load the track at `index` as by hand and, if it can't be played, skip it as the player has
    /// it.
    fn play_and_skip_unplayable(q: &Queue, index: usize) {
        q.play(index, false, false);
        while let Some(current) = q.get_current().filter(|t| !t.is_playable()) {
            let before = q.get_current_index();
            q.skip_unplayable(&current.uri(), true);
            if q.get_current_index() == before {
                break;
            }
        }
    }

    #[test]
    fn test_unplayable_tracks_are_skipped_without_being_noted_or_stopping_playback() {
        let q = make_queue(
            vec![
                make_track(0),
                make_unplayable_track(1),
                make_unplayable_track(2),
                make_unplayable_track(3),
                make_track(4),
            ],
            Some(0),
        );
        listened_for(&q, 60);
        // Picked by hand moments after loading, so a failure would count as quick.
        *q.loaded_at.write().unwrap() = Some(Instant::now());
        play_and_skip_unplayable(&q, 1);
        // Three in a row aren't taken for Spotify refusing songs.
        assert_eq!(q.get_current_index(), Some(4));
        assert_eq!(*q.failed_in_a_row.read().unwrap(), 0);
        // Only the track listened to is noted.
        let noted: Vec<String> = recorded().into_iter().map(|(uri, _)| uri).collect();
        assert_eq!(noted, ["spotify:track:id_0"]);
    }

    #[test]
    fn test_unplayable_track_skip_for_a_track_no_longer_current_is_ignored() {
        let q = make_queue(
            vec![make_track(0), make_unplayable_track(1), make_track(2)],
            None,
        );
        q.play(1, false, false);
        q.play(0, false, false);
        q.skip_unplayable("spotify:track:id_1", true);
        assert_eq!(q.get_current_index(), Some(0));
    }

    #[test]
    fn test_unplayable_tracks_at_the_end_stop() {
        let q = make_queue(vec![make_track(0), make_unplayable_track(1)], Some(0));
        q.spotify.update_status(PlayerEvent::Stopped);
        play_and_skip_unplayable(&q, 1);
        assert_eq!(q.get_current_index(), Some(1));
        // The player's status isn't touched: stopped, it stays stopped, so play plays.
        assert_eq!(q.spotify.get_current_status(), PlayerEvent::Stopped);
        assert!(recorded().is_empty());
    }

    #[test]
    fn test_a_queue_of_only_unplayable_tracks_on_repeat_doesnt_skip_forever() {
        let q = make_queue(
            vec![make_unplayable_track(0), make_unplayable_track(1)],
            None,
        );
        q.set_repeat(RepeatSetting::RepeatPlaylist);
        let mut skips = 0;
        q.play(0, false, false);
        while let Some(current) = q.get_current() {
            let index = q.get_current_index();
            q.skip_unplayable(&current.uri(), true);
            skips += 1;
            if q.get_current_index() == index || skips > 10 {
                break;
            }
        }
        assert!(skips <= 3);
    }

    #[test]
    fn test_unplayable_tracks_wrap_around_on_repeat() {
        let q = make_queue(vec![make_track(0), make_unplayable_track(1)], Some(0));
        q.set_repeat(RepeatSetting::RepeatPlaylist);
        play_and_skip_unplayable(&q, 1);
        assert_eq!(q.get_current_index(), Some(0));
    }

    // --- autoplay ---

    fn autoplay_tracks() -> Vec<Playable> {
        vec![make_track(10), make_track(11)]
    }

    #[test]
    fn test_autoplay_adds_tracks_after_the_last_one() {
        let q = make_queue(vec![make_track(0), make_track(1)], Some(1));
        q.add_autoplay("spotify:track:id_1", autoplay_tracks(), false);
        assert_eq!(ids(&q), ["id_0", "id_1", "id_10", "id_11"]);
        assert_eq!(source_of(&q, "id_10").as_deref(), Some(AUTOPLAY_SOURCE));
        // Still on the same track: nothing started playing.
        assert_eq!(q.get_current_index(), Some(1));
    }

    #[test]
    fn test_autoplay_is_dropped_when_no_longer_last() {
        let q = make_queue(vec![make_track(0), make_track(1)], Some(1));
        q.append(make_track(2));
        q.add_autoplay("spotify:track:id_1", autoplay_tracks(), false);
        assert_eq!(ids(&q), ["id_0", "id_1", "id_2"]);
    }

    #[test]
    fn test_autoplay_plays_when_the_queue_ran_out() {
        let q = make_queue(vec![make_track(0), make_track(1)], Some(1));
        *q.waiting_for_autoplay.write().unwrap() = true;
        q.add_autoplay("spotify:track:id_1", autoplay_tracks(), false);
        assert_eq!(q.get_current_index(), Some(2));
    }

    #[test]
    fn test_queued_tracks_go_ahead_of_autoplay() {
        let q = make_queue(vec![make_track(0), make_track(1)], Some(1));
        q.add_autoplay("spotify:track:id_1", autoplay_tracks(), false);
        q.append(make_track(2));
        assert_eq!(ids(&q), ["id_0", "id_1", "id_2", "id_10", "id_11"]);
        assert_eq!(source_of(&q, "id_2"), None);
    }

    #[test]
    fn test_clear_upcoming_keeps_the_current_track_and_lets_autoplay_follow_it() {
        let q = make_queue(make_tracks(4), Some(1));
        *q.autoplay_after.write().unwrap() = Some("spotify:track:id_1".into());
        q.clear_upcoming();
        assert_eq!(ids(&q), ["id_0", "id_1"]);
        assert_eq!(q.get_current_index(), Some(1));
        // What autoplay finds for the current track is taken again.
        q.add_autoplay("spotify:track:id_1", autoplay_tracks(), false);
        assert_eq!(ids(&q), ["id_0", "id_1", "id_10", "id_11"]);
    }

    #[test]
    fn test_autoplay_is_asked_again_when_what_it_found_went_unused() {
        let q = make_queue(make_tracks(3), Some(1));
        *q.autoplay_after.write().unwrap() = Some("spotify:track:id_1".into());
        q.add_autoplay("spotify:track:id_1", autoplay_tracks(), false);
        assert_eq!(*q.autoplay_after.read().unwrap(), None);
    }

    #[test]
    fn test_autoplay_that_found_nothing_is_asked_again_when_the_queue_runs_out() {
        let q = make_queue(make_tracks(2), Some(1));
        *q.autoplay_after.write().unwrap() = Some("spotify:track:id_1".into());
        q.add_autoplay("spotify:track:id_1", Vec::new(), false);
        assert_eq!(*q.autoplay_after.read().unwrap(), None);
        q.next(false);
        assert_eq!(
            q.autoplay_after.read().unwrap().as_deref(),
            Some("spotify:track:id_1")
        );
        // What it finds this time plays.
        q.add_autoplay("spotify:track:id_1", autoplay_tracks(), false);
        assert_eq!(q.get_current_index(), Some(2));
    }

    #[test]
    fn test_autoplay_is_asked_again_when_what_it_added_is_removed() {
        let q = make_queue(make_tracks(2), Some(1));
        q.autoplay_if_last();
        assert_eq!(
            q.autoplay_after.read().unwrap().as_deref(),
            Some("spotify:track:id_1")
        );
        q.add_autoplay("spotify:track:id_1", autoplay_tracks(), false);
        assert_eq!(*q.autoplay_after.read().unwrap(), None);
        q.remove(3);
        q.remove(2);
        // Asked again, so the queue doesn't run out with nothing coming.
        assert_eq!(
            q.autoplay_after.read().unwrap().as_deref(),
            Some("spotify:track:id_1")
        );
    }

    #[test]
    fn test_manual_autoplay_that_found_nothing_or_was_dropped_is_asked_again() {
        let q = make_queue(make_tracks(2), Some(1));
        assert!(q.autoplay_now().is_ok());
        q.add_autoplay("spotify:track:id_1", Vec::new(), true);
        assert_eq!(*q.autoplay_after.read().unwrap(), None);

        assert!(q.autoplay_now().is_ok());
        // The queue changed meanwhile, then went back.
        q.append(make_track(2));
        q.add_autoplay("spotify:track:id_1", autoplay_tracks(), true);
        q.remove(2);
        assert_eq!(
            q.autoplay_after.read().unwrap().as_deref(),
            Some("spotify:track:id_1")
        );
    }

    #[test]
    fn test_manual_autoplay_plays_when_the_queue_ran_out_waiting_for_it() {
        let q = make_queue(make_tracks(2), Some(1));
        q.spotify
            .update_status(PlayerEvent::Playing(std::time::SystemTime::now()));
        assert!(q.autoplay_now().is_ok());
        // The track ends before the answer arrives (the player hasn't said it's stopped yet).
        q.next(false);
        assert!(*q.waiting_for_autoplay.read().unwrap());
        q.add_autoplay("spotify:track:id_1", autoplay_tracks(), true);
        assert_eq!(q.get_current_index(), Some(2));
    }

    #[test]
    fn test_autoplay_for_another_track_leaves_the_queue_waiting() {
        let q = make_queue(make_tracks(2), Some(1));
        q.next(false);
        assert!(*q.waiting_for_autoplay.read().unwrap());
        // An answer for a track played before arrives first.
        q.add_autoplay("spotify:track:id_0", autoplay_tracks(), false);
        assert_eq!(ids(&q), ["id_0", "id_1"]);
        q.add_autoplay("spotify:track:id_1", autoplay_tracks(), false);
        assert_eq!(q.get_current_index(), Some(2));
    }

    #[test]
    fn test_removing_whats_up_next_lets_autoplay_follow_the_current_track() {
        let q = make_queue(make_tracks(3), Some(1));
        q.remove(2);
        assert_eq!(
            q.autoplay_after.read().unwrap().as_deref(),
            Some("spotify:track:id_1")
        );
    }

    #[test]
    fn test_manual_autoplay_follows_the_end_of_the_queue_without_interrupting() {
        let q = make_queue(make_tracks(3), Some(0));
        q.spotify
            .update_status(PlayerEvent::Playing(std::time::SystemTime::now()));
        assert!(q.autoplay_now().is_ok());
        let message = q.add_autoplay("spotify:track:id_2", autoplay_tracks(), true);
        assert_eq!(message, Some(Ok("Autoplay added 2 songs".into())));
        assert_eq!(ids(&q), ["id_0", "id_1", "id_2", "id_10", "id_11"]);
        assert_eq!(source_of(&q, "id_10").as_deref(), Some(AUTOPLAY_SOURCE));
        assert_eq!(q.get_current_index(), Some(0));
    }

    #[test]
    fn test_manual_autoplay_works_with_the_setting_off_and_plays_when_stopped() {
        let q = make_queue(make_tracks(2), None);
        q.cfg
            .edit(|doc| doc["autoplay"] = toml_edit::value(false))
            .unwrap();
        assert!(!q.autoplay_enabled());
        q.add_autoplay("spotify:track:id_1", autoplay_tracks(), true);
        assert_eq!(q.get_current_index(), Some(2));
    }

    #[test]
    fn test_manual_autoplay_says_when_it_found_nothing_or_cant_ask() {
        let q = make_queue(make_tracks(2), None);
        assert!(matches!(
            q.add_autoplay("spotify:track:id_1", Vec::new(), true),
            Some(Err(_))
        ));
        assert_eq!(ids(&q), ["id_0", "id_1"]);
        assert!(make_queue(Vec::new(), None).autoplay_now().is_err());
    }

    // --- radio ---

    fn song_seed(id: u32) -> RadioSeed {
        let Playable::Track(track) = make_track(id) else {
            unreachable!()
        };
        RadioSeed {
            uri: track.uri.clone(),
            name: track.title.clone(),
            track: Some(track),
        }
    }

    fn album_seed() -> RadioSeed {
        RadioSeed {
            uri: "spotify:album:album".into(),
            name: "Album".into(),
            track: None,
        }
    }

    fn radio(q: &Queue, seed: RadioSeed, tracks: Vec<Playable>) -> Option<Result<String, String>> {
        q.request_radio(seed.clone());
        q.handle_event(QueueEvent::Radio {
            seed: Box::new(seed),
            tracks,
        })
    }

    #[test]
    fn test_song_radio_replaces_the_queue_and_plays_the_song_first() {
        let q = make_queue(make_tracks(3), Some(1));
        // The song coming back among the radio's tracks isn't played twice.
        let tracks = vec![make_track(10), make_track(7), make_track(11)];
        assert_eq!(radio(&q, song_seed(7), tracks), None);
        assert_eq!(ids(&q), ["id_7", "id_10", "id_11"]);
        assert_eq!(q.get_current_index(), Some(0));
        for id in ["id_7", "id_10", "id_11"] {
            assert_eq!(source_of(&q, id).as_deref(), Some("Track 7 Radio"));
        }
    }

    #[test]
    fn test_album_radio_plays_its_tracks() {
        let q = make_queue(make_tracks(3), None);
        assert_eq!(radio(&q, album_seed(), autoplay_tracks()), None);
        assert_eq!(ids(&q), ["id_10", "id_11"]);
        assert_eq!(q.get_current_index(), Some(0));
        assert_eq!(source_of(&q, "id_11").as_deref(), Some("Album Radio"));
    }

    #[test]
    fn test_radio_that_found_nothing_leaves_the_queue_alone() {
        let q = make_queue(make_tracks(3), Some(1));
        // Only the song itself is nothing new.
        let message = radio(&q, song_seed(7), vec![make_track(7)]);
        assert!(matches!(message, Some(Err(e)) if e.contains("Track 7")));
        assert_eq!(ids(&q), ["id_0", "id_1", "id_2"]);
        assert_eq!(q.get_current_index(), Some(1));
    }

    #[test]
    fn test_only_the_last_radio_asked_for_starts() {
        let q = make_queue(make_tracks(3), Some(1));
        q.request_radio(album_seed());
        q.request_radio(song_seed(7));
        let late = q.handle_event(QueueEvent::Radio {
            seed: Box::new(album_seed()),
            tracks: autoplay_tracks(),
        });
        assert_eq!(late, None);
        assert_eq!(ids(&q), ["id_0", "id_1", "id_2"]);
        q.handle_event(QueueEvent::Radio {
            seed: Box::new(song_seed(7)),
            tracks: autoplay_tracks(),
        });
        assert_eq!(ids(&q), ["id_7", "id_10", "id_11"]);
    }

    #[test]
    fn test_autoplay_continues_a_radio_that_ran_out() {
        let q = make_queue(Vec::new(), None);
        radio(&q, song_seed(7), vec![make_track(10)]);
        q.next(true);
        assert_eq!(q.get_current_index(), Some(1));
        assert_eq!(
            q.autoplay_after.read().unwrap().as_deref(),
            Some("spotify:track:id_10")
        );
        q.add_autoplay("spotify:track:id_10", vec![make_track(20)], false);
        assert_eq!(ids(&q), ["id_7", "id_10", "id_20"]);
        assert_eq!(source_of(&q, "id_20").as_deref(), Some(AUTOPLAY_SOURCE));
    }

    fn uris(ids: &[u32]) -> Vec<String> {
        ids.iter()
            .map(|id| format!("spotify:track:id_{id}"))
            .collect()
    }

    #[test]
    fn test_autoplay_leaves_out_tracks_started_this_session_wherever_they_were() {
        // A playlist played through, repeated back to the start, and cleared after it.
        let started = uris(&[0, 1, 2, 3, 0]);
        let recent = recent_tracks(&started, &[], "spotify:track:id_0", 50);
        assert_eq!(recent, uris(&[1, 2, 3, 0]));
    }

    #[test]
    fn test_autoplay_leaves_out_the_history_before_this_session_up_to_the_limit() {
        let recent = recent_tracks(
            &uris(&[5, 6]),
            &uris(&[1, 2, 3, 6]),
            "spotify:track:id_6",
            4,
        );
        assert_eq!(recent, uris(&[2, 3, 5, 6]));
    }

    #[test]
    fn test_autoplay_leaves_out_the_current_track_even_if_not_started_this_session() {
        let recent = recent_tracks(&[], &uris(&[1]), "spotify:track:id_7", 50);
        assert_eq!(recent, uris(&[1, 7]));
    }

    #[test]
    fn test_clear_upcoming_with_nothing_playing_clears_everything() {
        let q = make_queue(make_tracks(3), None);
        q.clear_upcoming();
        assert!(ids(&q).is_empty());
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

    #[test]
    fn test_shuffle_order_saved_by_older_versions_is_ignored() {
        let cfg = Config::new_for_test();
        let tracks = make_tracks(3);
        cfg.with_state_mut(|s| {
            s.queuestate.queue.clone_from(&tracks);
            s.queuestate.current_track = Some(1);
            // Out of step with the queue, as it got: one index too many, and repeated.
            s.queuestate.random_order = Some(vec![1, 1, 0, 2, 3]);
        });
        let ev = EventManager::new_for_test();
        let spotify = Spotify::new_for_test(cfg.clone(), ev.clone());
        let library = Library::new_for_test(ev, spotify.clone(), cfg.clone());
        let q = Queue::new(spotify, cfg, library);
        assert_eq!(q.next_index(), Some(2));
        assert_eq!(q.previous_index(), Some(0));
        q.remove(0);
        q.append_next(&[make_track(7)]);
        assert_eq!(q.next_index(), Some(1));
        assert_eq!(q.handle_event(QueueEvent::PreloadTrackRequest), None);
    }

    #[test]
    fn test_saved_current_track_past_the_end_is_dropped() {
        let cfg = Config::new_for_test();
        cfg.with_state_mut(|s| {
            s.queuestate.queue = make_tracks(2);
            s.queuestate.current_track = Some(5);
        });
        let ev = EventManager::new_for_test();
        let spotify = Spotify::new_for_test(cfg.clone(), ev.clone());
        let library = Library::new_for_test(ev, spotify.clone(), cfg.clone());
        let q = Queue::new(spotify, cfg, library);
        assert_eq!(q.get_current_index(), None);
        assert!(q.get_current().is_none());
    }

    #[test]
    fn test_queue_used_from_two_threads_at_once_doesnt_deadlock() {
        // As MPRIS does from its own thread while the UI's goes on.
        let q = Arc::new(make_queue(make_tracks(30), Some(0)));
        let other = std::thread::spawn({
            let q = q.clone();
            move || {
                for i in 0..300 {
                    q.set_shuffle(i % 2 == 0);
                    let _ = q.next_index();
                    let _ = q.get_current();
                    let _ = q.upcoming(5);
                    q.append(make_track(100 + i));
                }
            }
        });
        for i in 0..300 {
            q.next(true);
            q.previous();
            q.append_next(&[make_track(1000 + i)]);
            let _ = q.handle_event(QueueEvent::PreloadTrackRequest);
        }
        other.join().unwrap();
        assert_eq!(q.sources.read().unwrap().len(), q.len());
    }

    #[test]
    fn test_get_current_past_the_end_is_none() {
        let q = make_queue(make_tracks(2), Some(2));
        assert!(q.get_current().is_none());
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

    /// What would have been noted in the listening history so far.
    fn recorded() -> Vec<(String, std::time::Duration)> {
        crate::stats::RECORDED.with_borrow(Clone::clone)
    }

    /// Have the player say it's paused `secs` into the current track.
    fn listened_for(q: &Queue, secs: u64) {
        q.spotify
            .update_status(PlayerEvent::Paused(std::time::Duration::from_secs(secs)));
    }

    #[test]
    fn test_removing_the_only_track_while_it_plays_stops() {
        let q = make_queue(make_tracks(1), Some(0));
        listened_for(&q, 60);
        q.remove(0);
        assert_eq!(q.len(), 0);
        assert_eq!(q.get_current_index(), None);
        assert_eq!(
            recorded(),
            [(
                "spotify:track:id_0".into(),
                std::time::Duration::from_secs(60)
            )]
        );
    }

    #[test]
    fn test_removing_the_last_track_while_it_plays_stops() {
        let q = make_queue(make_tracks(3), Some(2));
        listened_for(&q, 60);
        q.remove(2);
        assert_eq!(ids(&q), ["id_0", "id_1"]);
        assert_eq!(q.get_current_index(), None);
        assert_eq!(recorded().len(), 1);
        assert_eq!(recorded()[0].0, "spotify:track:id_2");
    }

    #[test]
    fn test_removing_the_last_track_while_it_plays_with_repeat_goes_back_to_the_start() {
        let q = make_queue(make_tracks(3), Some(2));
        q.set_repeat(RepeatSetting::RepeatPlaylist);
        listened_for(&q, 60);
        q.remove(2);
        assert_eq!(q.get_current_index(), Some(0));
        assert_eq!(recorded().len(), 1);
        assert_eq!(recorded()[0].0, "spotify:track:id_2");
    }

    #[test]
    fn test_removing_the_playing_track_plays_the_next_and_notes_the_removed_one() {
        let q = make_queue(make_tracks(3), Some(1));
        listened_for(&q, 60);
        q.remove(1);
        assert_eq!(ids(&q), ["id_0", "id_2"]);
        assert_eq!(q.get_current_index(), Some(1));
        // The removed track is noted, not the one that took its place.
        assert_eq!(
            recorded(),
            [(
                "spotify:track:id_1".into(),
                std::time::Duration::from_secs(60)
            )]
        );
    }

    #[test]
    fn test_removing_past_the_end_does_nothing() {
        let q = make_queue(make_tracks(2), Some(0));
        q.remove(5);
        assert_eq!(q.len(), 2);
        assert_eq!(q.get_current_index(), Some(0));
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
