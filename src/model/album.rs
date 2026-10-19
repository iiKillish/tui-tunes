use rspotify::model::Id;
use std::fmt;

use chrono::{DateTime, Utc};
use log::debug;
use rspotify::model::album::{FullAlbum, SavedAlbum, SimplifiedAlbum};

use crate::library::Library;
use crate::model::artist::Artist;
use crate::model::playable::Playable;
use crate::model::track::Track;
use crate::queue::Queue;
use crate::spotify::Spotify;
use crate::traits::ListItem;
use crate::traits::OpenTarget;

#[derive(Clone, Deserialize, Serialize)]
pub struct Album {
    pub id: Option<String>,
    pub title: String,
    pub artists: Vec<String>,
    pub artist_ids: Vec<String>,
    pub year: String,
    pub cover_url: Option<String>,
    pub url: Option<String>,
    pub tracks: Option<Vec<Track>>,
    pub added_at: Option<DateTime<Utc>>,
    total_tracks: Option<usize>,
}

impl Album {
    /// A copy without the tracks, which are slow to clone (e.g. for showing it as a card).
    pub fn without_tracks(&self) -> Self {
        Self {
            id: self.id.clone(),
            title: self.title.clone(),
            artists: self.artists.clone(),
            artist_ids: self.artist_ids.clone(),
            year: self.year.clone(),
            cover_url: self.cover_url.clone(),
            url: self.url.clone(),
            tracks: None,
            added_at: self.added_at,
            total_tracks: self.total_tracks,
        }
    }

    /// An album known only from one of its tracks; [Self::load_all_tracks] fills in the rest.
    pub fn stub(
        id: String,
        title: String,
        artists: Vec<String>,
        cover_url: Option<String>,
    ) -> Self {
        Self {
            url: Some(format!("https://open.spotify.com/album/{id}")),
            id: Some(id),
            title,
            artists,
            artist_ids: Vec::new(),
            year: String::new(),
            cover_url,
            tracks: None,
            added_at: None,
            total_tracks: None,
        }
    }

    /// Whether all the album's tracks are here, so [Self::load_all_tracks] has nothing to do.
    pub fn has_all_tracks(&self) -> bool {
        self.tracks.is_some() && self.tracks.as_ref().map(|t| t.len()) == self.total_tracks
    }

    /// Load the album's tracks from Spotify, and its details if it's a [Self::stub].
    pub fn load_all_tracks(&mut self, spotify: Spotify) {
        if self.has_all_tracks() {
            return;
        }

        if let Some(album_id) = self.id.clone() {
            let album_id = &album_id;
            let mut collected_tracks = Vec::new();
            if let Ok(full_album) = spotify.api.album(album_id) {
                if self.year.is_empty() {
                    let full = Self::from(&full_album);
                    self.title = full.title;
                    self.artists = full.artists;
                    self.artist_ids = full.artist_ids;
                    self.year = full.year;
                    self.cover_url = self.cover_url.take().or(full.cover_url);
                }
                let mut tracks_result = Some(full_album.tracks.clone());
                while let Some(ref tracks) = tracks_result {
                    for t in &tracks.items {
                        collected_tracks.push(Track::from_simplified_track(t, &full_album));
                    }

                    debug!("got {} tracks", tracks.items.len());

                    // load next batch if necessary
                    tracks_result = match tracks.next {
                        Some(_) => {
                            debug!("requesting tracks again..");
                            spotify
                                .api
                                .album_tracks(
                                    album_id,
                                    50,
                                    tracks.offset + tracks.items.len() as u32,
                                )
                                .ok()
                        }
                        None => None,
                    }
                }
            }

            self.total_tracks = Some(collected_tracks.len());
            self.tracks = Some(collected_tracks);
        }
    }
}

impl From<&SimplifiedAlbum> for Album {
    fn from(sa: &SimplifiedAlbum) -> Self {
        Self {
            id: sa.id.as_ref().map(|id| id.id().to_string()),
            title: sa.name.clone(),
            artists: sa.artists.iter().map(|sa| sa.name.clone()).collect(),
            artist_ids: sa
                .artists
                .iter()
                .filter_map(|a| a.id.as_ref().map(|id| id.id().to_string()))
                .collect(),
            year: sa
                .release_date
                .clone()
                .unwrap_or_default()
                .split('-')
                .next()
                .unwrap()
                .into(),
            cover_url: sa.images.first().map(|i| i.url.clone()),
            url: sa.id.as_ref().map(|id| id.url()),
            tracks: None,
            added_at: None,
            total_tracks: None,
        }
    }
}

impl From<&FullAlbum> for Album {
    fn from(fa: &FullAlbum) -> Self {
        let tracks = Some(
            fa.tracks
                .items
                .iter()
                .map(|st| Track::from_simplified_track(st, fa))
                .collect(),
        );

        Self {
            id: Some(fa.id.id().to_string()),
            title: fa.name.clone(),
            artists: fa.artists.iter().map(|sa| sa.name.clone()).collect(),
            artist_ids: fa
                .artists
                .iter()
                .filter_map(|a| a.id.as_ref().map(|id| id.id().to_string()))
                .collect(),
            year: fa.release_date.split('-').next().unwrap().into(),
            cover_url: fa.images.first().map(|i| i.url.clone()),
            url: Some(fa.id.uri()),
            tracks,
            added_at: None,
            total_tracks: Some(fa.tracks.total as usize),
        }
    }
}

impl From<&SavedAlbum> for Album {
    fn from(sa: &SavedAlbum) -> Self {
        let mut album: Self = (&sa.album).into();
        album.added_at = Some(sa.added_at);
        album
    }
}

impl fmt::Display for Album {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{} - {}", self.artists.join(", "), self.title)
    }
}

impl fmt::Debug for Album {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "({} - {} ({:?}))",
            self.artists.join(", "),
            self.title,
            self.id
        )
    }
}

impl ListItem for Album {
    fn target(&self) -> Option<OpenTarget> {
        Some(OpenTarget::Album(self.clone()))
    }

    fn thumbnail_url(&self) -> Option<String> {
        self.cover_url.clone()
    }

    fn is_playing(&self, queue: &Queue) -> bool {
        self.tracks
            .as_ref()
            .is_some_and(|tracks| queue.consists_of(tracks.iter().filter_map(|t| t.id.as_deref())))
    }

    fn display_left(&self, _library: &Library) -> String {
        format!("{self}")
    }

    fn display_right(&self, library: &Library) -> String {
        let saved = if library.is_saved_album(self) {
            if library.cfg.values().use_nerdfont.unwrap_or(false) {
                "\u{f012c} "
            } else {
                "✓ "
            }
        } else {
            ""
        };
        format!("{}{}", saved, self.year)
    }

    fn play(&mut self, queue: &Queue) {
        self.load_all_tracks(queue.get_spotify());

        if let Some(tracks) = self.tracks.as_ref() {
            let tracks: Vec<Playable> = tracks
                .iter()
                .map(|track| Playable::Track(track.clone()))
                .collect();
            let index = queue.append_next_from(&tracks, Some(&self.title));
            queue.play(index, true, true);
        }
    }

    fn play_next(&mut self, queue: &Queue) {
        self.load_all_tracks(queue.get_spotify());

        if let Some(tracks) = self.tracks.as_ref() {
            for t in tracks.iter().rev() {
                queue.insert_after_current(Playable::Track(t.clone()));
            }
        }
    }

    fn queue(&mut self, queue: &Queue) {
        self.load_all_tracks(queue.get_spotify());

        if let Some(tracks) = self.tracks.as_ref() {
            for t in tracks {
                queue.append(Playable::Track(t.clone()));
            }
        }
    }

    fn save(&mut self, library: &Library) {
        library.save_album(self);
    }

    fn share_url(&self) -> Option<String> {
        self.id
            .clone()
            .map(|id| format!("https://open.spotify.com/album/{id}"))
    }

    fn artists(&self) -> Option<Vec<Artist>> {
        Some(
            self.artist_ids
                .iter()
                .zip(self.artists.iter())
                .map(|(id, name)| Artist::new(id.clone(), name.clone()))
                .collect(),
        )
    }

    #[inline]
    fn is_playable(&self) -> bool {
        true
    }

    fn as_listitem(&self) -> Box<dyn ListItem> {
        Box::new(self.clone())
    }
}
