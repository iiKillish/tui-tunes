use std::cmp::Ordering;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, RwLock};

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

/// Events that are specific to the [Queue].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QueueEvent {
    /// Request the player to 'preload' a track, basically making sure that
    /// transitions between tracks can be uninterrupted.
    PreloadTrackRequest,
}

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
    spotify: Spotify,
    cfg: Arc<Config>,
    library: Arc<Library>,
}

impl Queue {
    pub fn new(spotify: Spotify, cfg: Arc<Config>, library: Arc<Library>) -> Self {
        let queue_state = cfg.state().queuestate.clone();
        let sources = vec![None; queue_state.queue.len()];

        Self {
            queue: Arc::new(RwLock::new(queue_state.queue)),
            spotify: spotify.clone(),
            current_track: RwLock::new(queue_state.current_track),
            random_order: RwLock::new(queue_state.random_order),
            unshuffled: RwLock::new(None),
            sources: RwLock::new(sources),
            cfg,
            library,
        }
    }

    /// The index of the next item in `self.queue` that should be played. None
    /// if at the end of the queue.
    pub fn next_index(&self) -> Option<usize> {
        match *self.current_track.read().unwrap() {
            Some(mut index) => {
                let random_order = self.random_order.read().unwrap();
                if let Some(order) = random_order.as_ref() {
                    index = order.iter().position(|&i| i == index).unwrap();
                }

                let mut next_index = index + 1;
                if next_index < self.queue.read().unwrap().len() {
                    if let Some(order) = random_order.as_ref() {
                        next_index = order[next_index];
                    }

                    Some(next_index)
                } else {
                    None
                }
            }
            None => None,
        }
    }

    /// The index of the previous item in `self.queue` that should be played.
    /// None if at the start of the queue.
    pub fn previous_index(&self) -> Option<usize> {
        match *self.current_track.read().unwrap() {
            Some(mut index) => {
                let random_order = self.random_order.read().unwrap();
                if let Some(order) = random_order.as_ref() {
                    index = order.iter().position(|&i| i == index).unwrap();
                }

                if index > 0 {
                    let mut next_index = index - 1;
                    if let Some(order) = random_order.as_ref() {
                        next_index = order[next_index];
                    }

                    Some(next_index)
                } else {
                    None
                }
            }
            None => None,
        }
    }

    /// The currently playing item from `self.queue`.
    pub fn get_current(&self) -> Option<Playable> {
        self.get_current_index()
            .map(|index| self.queue.read().unwrap()[index].clone())
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
        q.push(track);
        let mut sources = self.sources.write().unwrap();
        let len = q.len();
        insert_source(&mut sources, len - 1, None);
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
    pub fn play(&self, mut index: usize, reshuffle: bool, shuffle_index: bool) {
        let queue_length = self.queue.read().unwrap().len();
        // The length of the queue must be bigger than 0 or gen_range panics!
        if queue_length > 0 && shuffle_index && self.get_shuffle() {
            let mut rng = rand::rng();
            index = rng.random_range(0..queue_length);
        }

        if let Some(track) = &self.queue.read().unwrap().get(index) {
            self.spotify.load(track, true, 0);
            let mut current = self.current_track.write().unwrap();
            current.replace(index);
            self.spotify.update_track();

            #[cfg(feature = "notify")]
            if self.cfg.values().notify.unwrap_or(false) {
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
    }

    /// Toggle the playback. If playback is currently stopped, this will either
    /// play the next song if one is available, or restart from the start.
    pub fn toggleplayback(&self) {
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
        let q = self.queue.read().unwrap();
        let current = *self.current_track.read().unwrap();
        let repeat = self.cfg.state().repeat;

        if repeat == RepeatSetting::RepeatTrack && !manual {
            if let Some(index) = current
                && q[index].is_playable()
            {
                self.play(index, false, false);
            }
        } else if let Some(index) = self.next_index() {
            self.play(index, false, false);
            if repeat == RepeatSetting::RepeatTrack && manual {
                self.set_repeat(RepeatSetting::RepeatPlaylist);
            }
        } else if repeat == RepeatSetting::RepeatPlaylist
            && !q.is_empty()
            && q.iter().any(|track| track.is_playable())
        {
            let random_order = self.random_order.read().unwrap();
            self.play(
                random_order.as_ref().map(|o| o[0]).unwrap_or(0),
                false,
                false,
            );
        } else {
            self.spotify.stop();
        }
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
        }
    }

    /// Get the spotify session.
    pub fn get_spotify(&self) -> Spotify {
        self.spotify.clone()
    }
}

/// Send a notification using the desktops default notification method.
///
/// `summary_txt`: A short title for the notification.
/// `body_txt`: The actual content of the notification.
/// `cover_url`: A URL to an image to show in the notification.
/// `notification_id`: Unique id for a notification, that can be used to operate
/// on a previous notification (for example to close it).
#[cfg(feature = "notify")]
/// Insert `source` at `index` in `sources`, padding it first if it's fallen out of step with the
/// queue (so a mismatch can never panic).
fn insert_source(sources: &mut Vec<Option<Arc<str>>>, index: usize, source: Option<Arc<str>>) {
    if sources.len() < index {
        sources.resize(index, None);
    }
    sources.insert(index, source);
}

pub fn send_notification(summary_txt: &str, body_txt: &str, cover_url: Option<String>) {
    let mut n = Notification::new();
    n.appname("ncspot").summary(summary_txt).body(body_txt);

    // album cover image
    if let Some(u) = cover_url {
        let path = crate::utils::cache_path_for_url(u.to_string());
        if !path.exists()
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
        .hint(notify_rust::Hint::DesktopEntry("ncspot".into()));

    match n.show() {
        Ok(handle) => {
            // only available for XDG
            #[cfg(all(unix, not(target_os = "macos")))]
            info!("Created notification: {}", handle.id());
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
            spotify,
            cfg,
            library,
        }
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
}
