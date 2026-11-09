use std::sync::Arc;
use std::{cmp::Ordering, iter::Iterator};

use log::{debug, warn};
use rspotify::model::Id;
use rspotify::model::playlist::{FullPlaylist, SimplifiedPlaylist};

use crate::model::playable::Playable;
use crate::model::track::Track;
use crate::queue::Queue;
use crate::spotify::Spotify;
use crate::spotify_api::WebApi;
use crate::traits::ListItem;
use crate::traits::OpenTarget;
use crate::utils::InOrder;
use crate::{command::SortDirection, command::SortKey, library::Library};

/// Whether the playlist with `id` is one Spotify makes (Daily Mix, Discover Weekly, artist radios,
/// editorial playlists), which the Web API refuses to give apps. Their IDs all start alike.
pub fn is_made_by_spotify(id: &str) -> bool {
    // 37i9dQZF…: Daily Mix, radios and editorial playlists; 37i9dQZEVX…: Discover Weekly and
    // Release Radar.
    id.starts_with("37i9dQZ")
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Playlist {
    pub id: String,
    pub name: String,
    pub owner_id: String,
    pub owner_name: Option<String>,
    pub snapshot_id: String,
    pub num_tracks: usize,
    pub tracks: Option<Vec<Playable>>,
    pub collaborative: bool,
    /// Cover image; missing from caches written before it was added.
    #[serde(default)]
    pub cover_url: Option<String>,
}

impl Playlist {
    /// A copy without the tracks, which are slow to clone (e.g. for showing it as a card).
    pub fn without_tracks(&self) -> Self {
        Self {
            id: self.id.clone(),
            name: self.name.clone(),
            owner_id: self.owner_id.clone(),
            owner_name: self.owner_name.clone(),
            snapshot_id: self.snapshot_id.clone(),
            num_tracks: self.num_tracks,
            tracks: None,
            collaborative: self.collaborative,
            cover_url: self.cover_url.clone(),
        }
    }

    /// Load the tracks, unless they're here already. Returns whether they all loaded: if some
    /// failed, [Self::tracks] holds those that did.
    pub fn load_tracks(&mut self, spotify: &Spotify) -> bool {
        if self.tracks.is_some() {
            return true;
        }

        // The Web API has none of the tracks of playlists Spotify makes: ask the session (also
        // when the Web API comes back empty, for ones that aren't recognized as Spotify's).
        let (mut tracks, mut complete) = if is_made_by_spotify(&self.id) {
            (Vec::new(), false)
        } else {
            self.get_all_tracks(spotify)
        };
        if tracks.is_empty()
            && let Some(playlist) = spotify.api.session_playlist(&self.id, true)
        {
            tracks = playlist.tracks.unwrap_or_default();
            complete = true;
        }
        self.tracks = Some(tracks);
        complete
    }

    /// A playlist loaded through the session (see [crate::spotify_api::WebApi::session_playlist]),
    /// with its `tracks` if they were loaded. Only Spotify's playlists need this, so Spotify is
    /// the owner. Its revision stands in for the snapshot ID.
    pub fn from_session(
        id: &str,
        playlist: &librespot_metadata::Playlist,
        tracks: Option<&[librespot_metadata::Track]>,
    ) -> Self {
        let attributes = &playlist.attributes;
        let cover_url = attributes
            .picture_sizes
            .iter()
            .find(|size| size.target_name == "large")
            .or_else(|| attributes.picture_sizes.first())
            .map(|size| size.url.clone())
            .or_else(|| {
                (!attributes.picture.is_empty()).then(|| {
                    let hex: String = attributes
                        .picture
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect();
                    format!("https://i.scdn.co/image/{hex}")
                })
            });
        let tracks: Option<Vec<Playable>> = tracks.map(|tracks| {
            tracks
                .iter()
                .enumerate()
                .map(|(index, track)| {
                    let mut playable = Playable::Track(Track::from(track));
                    playable.set_list_index(index);
                    playable
                })
                .collect()
        });
        Self {
            id: id.to_string(),
            name: playlist.name().to_string(),
            owner_id: "spotify".into(),
            owner_name: Some("Spotify".into()),
            snapshot_id: playlist
                .revision
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect(),
            num_tracks: playlist.contents.items.len(),
            tracks,
            collaborative: false,
            cover_url,
        }
    }

    /// All the tracks from the Web API, and whether they all loaded.
    fn get_all_tracks(&self, spotify: &Spotify) -> (Vec<Playable>, bool) {
        let mut tracks_result = spotify.api.user_playlist_tracks(&self.id);
        let complete = tracks_result.fetch_all();
        let tracks = std::mem::take(&mut *tracks_result.items.write().unwrap());
        (tracks, complete)
    }

    /// Remove the track at `index`: here at once, on Spotify in the background (see [sync]).
    pub fn delete_track(
        &mut self,
        index: usize,
        spotify: &Spotify,
        library: &Arc<Library>,
    ) -> bool {
        let Some(playable) = self.tracks.as_ref().and_then(|t| t.get(index)).cloned() else {
            return false;
        };
        debug!("deleting track: {index} {playable:?}");

        if playable.track().map(|t| t.is_local) == Some(true) {
            warn!("track is a local file, can't delete");
            return false;
        }

        if let Some(tracks) = &mut self.tracks {
            tracks.remove(index);
            // The tracks after it move up one place (in the playlist's own order, which a sorted
            // list doesn't show).
            for track in tracks.iter_mut() {
                let position = track.list_index();
                if position > playable.list_index() {
                    track.set_list_index(position - 1);
                }
            }
        }
        let id = self.id.clone();
        self.sync(spotify, library, "remove the song", move |api| {
            api.delete_tracks(&id, None, &[playable]).map(|_| ())
        });
        true
    }

    /// Move the track at `from` to `to` (indices in [Self::tracks]): here at once, on Spotify in
    /// the background (see [sync]). The tracks must be in the playlist's own order (not sorted).
    pub fn move_track(
        &mut self,
        from: usize,
        to: usize,
        spotify: &Spotify,
        library: &Arc<Library>,
    ) -> bool {
        let Some(tracks) = &mut self.tracks else {
            return false;
        };
        let Some((from, to)) = move_positions(tracks, from, to) else {
            return false;
        };
        let id = self.id.clone();
        self.sync(spotify, library, "move the song", move |api| {
            api.move_track(&id, None, from, to).map(|_| ())
        });
        true
    }

    /// Make a change on Spotify that's already been made here, then save the playlist in the
    /// library. Changes are made one at a time, in order, on a thread of their own, so the UI
    /// doesn't wait for Spotify and each change applies to the playlist as the one before left
    /// it (positions refer to the playlist as it is, not to a snapshot). A failed change is
    /// reported in the UI; the playlist is then out of step until it's opened again.
    fn sync(
        &self,
        spotify: &Spotify,
        library: &Arc<Library>,
        what: &'static str,
        change: impl FnOnce(&WebApi) -> Result<(), ()> + Send + 'static,
    ) {
        static EDITS: InOrder = InOrder::new("playlist edits");
        let api = spotify.api.clone();
        let library = library.clone();
        let playlist = self.clone();
        EDITS.run(move || {
            if change(&api).is_ok() {
                library.playlist_update(&playlist);
            } else {
                crate::tui::report_error(format!("Couldn't {what} on Spotify"));
            }
        });
    }

    pub fn append_tracks(&mut self, new_tracks: &[Playable], spotify: &Spotify, library: &Library) {
        let mut has_modified = false;

        if spotify
            .api
            .append_tracks(&self.id, new_tracks, None)
            .is_ok()
            && let Some(tracks) = &mut self.tracks
        {
            tracks.append(&mut new_tracks.to_vec());
            has_modified = true;
        }

        if has_modified {
            library.playlist_update(self);
        }
    }

    pub fn sort(&mut self, key: &SortKey, direction: &SortDirection) {
        fn compare_artists(a: &[String], b: &[String]) -> Ordering {
            let sanitize_artists_name = |x: &[String]| -> Vec<String> {
                x.iter()
                    .map(|x| {
                        x.to_lowercase()
                            .split(' ')
                            .skip_while(|x| x == &"the")
                            .collect()
                    })
                    .collect()
            };

            let a = sanitize_artists_name(a);
            let b = sanitize_artists_name(b);

            a.cmp(&b)
        }

        fn compare_album(a: &Track, b: &Track) -> Ordering {
            a.album
                .as_ref()
                .map(|x| x.to_lowercase())
                .cmp(&b.album.as_ref().map(|x| x.to_lowercase()))
                .then_with(|| a.disc_number.cmp(&b.disc_number))
                .then_with(|| a.track_number.cmp(&b.track_number))
        }

        if let SortKey::Original = key {
            if let Some(tracks) = self.tracks.as_mut() {
                tracks.sort_by_key(Playable::list_index);
                if let SortDirection::Descending = direction {
                    tracks.reverse();
                }
            }
            return;
        }
        if let Some(c) = self.tracks.as_mut() {
            c.sort_by(|a, b| match (a.track(), b.track()) {
                (Some(a), Some(b)) => {
                    let (a, b) = match *direction {
                        SortDirection::Ascending => (a, b),
                        SortDirection::Descending => (b, a),
                    };
                    match *key {
                        SortKey::Title => a.title.to_lowercase().cmp(&b.title.to_lowercase()),
                        SortKey::Duration => a.duration.cmp(&b.duration),
                        SortKey::Album => compare_album(&a, &b),
                        SortKey::Added => a.added_at.cmp(&b.added_at),
                        SortKey::Artist => compare_artists(&a.artists, &b.artists)
                            .then_with(|| compare_album(&a, &b)),
                        SortKey::Original => a.list_index.cmp(&b.list_index),
                    }
                }
                _ => std::cmp::Ordering::Equal,
            })
        }
    }
}

/// Move the track at `from` in `tracks` to `to`, along with its [Playable::list_index] and those
/// of the tracks it passes. Returns the positions it moved from and to on Spotify, which differ
/// from the indices here when the playlist has items on Spotify that aren't listed (ones Spotify
/// no longer knows). `None` if either index is out of range.
fn move_positions(tracks: &mut Vec<Playable>, from: usize, to: usize) -> Option<(usize, usize)> {
    if from >= tracks.len() || to >= tracks.len() {
        return None;
    }
    let old = tracks[from].list_index();
    let new = tracks[to].list_index();
    let track = tracks.remove(from);
    tracks.insert(to, track);
    for track in tracks.iter_mut() {
        let position = track.list_index();
        if position == old {
            track.set_list_index(new);
        } else if old < new && position > old && position <= new {
            track.set_list_index(position - 1);
        } else if new < old && position >= new && position < old {
            track.set_list_index(position + 1);
        }
    }
    Some((old, new))
}

impl From<&SimplifiedPlaylist> for Playlist {
    fn from(list: &SimplifiedPlaylist) -> Self {
        Self {
            id: list.id.id().to_string(),
            name: list.name.clone(),
            owner_id: list.owner.id.id().to_string(),
            owner_name: list.owner.display_name.clone(),
            snapshot_id: list.snapshot_id.clone(),
            num_tracks: list.items.total as usize,
            tracks: None,
            collaborative: list.collaborative,
            cover_url: list.images.first().map(|i| i.url.clone()),
        }
    }
}

impl From<&FullPlaylist> for Playlist {
    fn from(list: &FullPlaylist) -> Self {
        Self {
            id: list.id.id().to_string(),
            name: list.name.clone(),
            owner_id: list.owner.id.id().to_string(),
            owner_name: list.owner.display_name.clone(),
            snapshot_id: list.snapshot_id.clone(),
            num_tracks: list.items.total as usize,
            tracks: None,
            collaborative: list.collaborative,
            cover_url: list.images.first().map(|i| i.url.clone()),
        }
    }
}

impl ListItem for Playlist {
    fn target(&self) -> Option<OpenTarget> {
        Some(OpenTarget::Playlist(self.clone()))
    }

    fn thumbnail_url(&self) -> Option<String> {
        self.cover_url.clone()
    }

    fn is_playing(&self, queue: &Queue) -> bool {
        self.tracks
            .as_ref()
            .is_some_and(|tracks| queue.consists_of(tracks.iter().filter_map(Playable::id_ref)))
    }

    fn display_left(&self, library: &Library) -> String {
        let hide_owners = library.cfg.values().hide_display_names.unwrap_or(false);
        match (self.owner_name.as_ref(), hide_owners) {
            (Some(owner), false) => format!("{} • {}", self.name, owner),
            _ => self.name.clone(),
        }
    }

    fn display_right(&self, library: &Library) -> String {
        let saved = if library.is_saved_playlist(self) {
            if library.cfg.values().use_nerdfont.unwrap_or(false) {
                "\u{f012c} "
            } else {
                "✓ "
            }
        } else {
            ""
        };

        let num_tracks = self
            .tracks
            .as_ref()
            .map(|t| t.len())
            .unwrap_or(self.num_tracks);

        format!("{saved}{num_tracks:>4} tracks")
    }

    fn play(&mut self, queue: &Queue) {
        self.load_tracks(&queue.get_spotify());

        if let Some(tracks) = &self.tracks {
            let index = queue.append_next_from(tracks, Some(&self.name));
            queue.play(index, true, true);
        }
    }

    fn play_next(&mut self, queue: &Queue) {
        self.load_tracks(&queue.get_spotify());

        if let Some(tracks) = self.tracks.as_ref() {
            for track in tracks.iter().rev() {
                queue.insert_after_current(track.clone());
            }
        }
    }

    fn queue(&mut self, queue: &Queue) {
        self.load_tracks(&queue.get_spotify());

        if let Some(tracks) = self.tracks.as_ref() {
            for track in tracks.iter() {
                queue.append(track.clone());
            }
        }
    }

    fn save(&mut self, library: &Library) {
        library.follow_playlist(self.clone());
    }

    fn share_url(&self) -> Option<String> {
        Some(format!(
            "https://open.spotify.com/user/{}/playlist/{}",
            self.owner_id, self.id
        ))
    }

    #[inline]
    fn is_playable(&self) -> bool {
        true
    }

    fn as_listitem(&self) -> Box<dyn ListItem> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tracks_at(positions: &[usize]) -> Vec<Playable> {
        positions
            .iter()
            .map(|&position| {
                Playable::Track(Track {
                    id: Some(position.to_string()),
                    uri: String::new(),
                    title: String::new(),
                    track_number: 0,
                    disc_number: 0,
                    duration: 0,
                    artists: Vec::new(),
                    artist_ids: Vec::new(),
                    album: None,
                    album_id: None,
                    album_artists: Vec::new(),
                    cover_url: None,
                    url: String::new(),
                    added_at: None,
                    list_index: position,
                    is_local: false,
                    is_playable: None,
                })
            })
            .collect()
    }

    fn positions(tracks: &[Playable]) -> Vec<(String, usize)> {
        tracks
            .iter()
            .map(|t| (t.id().unwrap(), t.list_index()))
            .collect()
    }

    #[test]
    fn moves_use_spotify_positions_past_unlisted_items() {
        // Spotify's item at 1 isn't listed (Spotify no longer knows the song).
        let mut tracks = tracks_at(&[0, 2, 3, 4]);
        // Down: the song at 2 goes where the one at 4 was.
        assert_eq!(move_positions(&mut tracks, 1, 3), Some((2, 4)));
        let expected = [("0", 0), ("3", 2), ("4", 3), ("2", 4)].map(|(id, p)| (id.into(), p));
        assert_eq!(positions(&tracks), expected);
        // And back up.
        assert_eq!(move_positions(&mut tracks, 3, 1), Some((4, 2)));
        let expected = [("0", 0), ("2", 2), ("3", 3), ("4", 4)].map(|(id, p)| (id.into(), p));
        assert_eq!(positions(&tracks), expected);
        assert_eq!(move_positions(&mut tracks, 0, 4), None);
    }

    #[test]
    fn test_playlists_made_by_spotify() {
        // A Daily Mix, Discover Weekly and an artist radio.
        assert!(is_made_by_spotify("37i9dQZF1E37pQSz62cUyl"));
        assert!(is_made_by_spotify("37i9dQZEVXcJZyENOWUFo7"));
        assert!(is_made_by_spotify("37i9dQZF1E4lx2yZrJ95uQ"));
        assert!(!is_made_by_spotify("0DNDN3ahYfw9RS9XyOr3BN"));
    }
}
