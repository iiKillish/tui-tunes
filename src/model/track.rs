use std::fmt;

use crate::config;
use crate::utils::ms_to_hms;
use chrono::{DateTime, Utc};
use rspotify::model::Id;
use rspotify::model::album::FullAlbum;
use rspotify::model::track::{FullTrack, SavedTrack, SimplifiedTrack};

use crate::library::Library;
use crate::model::album::Album;
use crate::model::artist::Artist;
use crate::model::playable::Playable;
use crate::queue::Queue;
use crate::traits::ListItem;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Track {
    pub id: Option<String>,
    pub uri: String,
    pub title: String,
    pub track_number: u32,
    pub disc_number: i32,
    pub duration: u32,
    pub artists: Vec<String>,
    pub artist_ids: Vec<String>,
    pub album: Option<String>,
    pub album_id: Option<String>,
    pub album_artists: Vec<String>,
    pub cover_url: Option<String>,
    pub url: String,
    pub added_at: Option<DateTime<Utc>>,
    pub list_index: usize,
    pub is_local: bool,
    pub is_playable: Option<bool>,
}

impl Track {
    pub fn from_simplified_track(track: &SimplifiedTrack, album: &FullAlbum) -> Self {
        let artists = track
            .artists
            .iter()
            .map(|artist| artist.name.clone())
            .collect::<Vec<String>>();
        let artist_ids = track
            .artists
            .iter()
            .filter_map(|a| a.id.as_ref().map(|id| id.id().to_string()))
            .collect::<Vec<String>>();
        let album_artists = album
            .artists
            .iter()
            .map(|artist| artist.name.clone())
            .collect::<Vec<String>>();

        Self {
            id: track.id.as_ref().map(|id| id.id().to_string()),
            uri: track.id.as_ref().map(|id| id.uri()).unwrap_or_default(),
            title: track.name.clone(),
            track_number: track.track_number,
            disc_number: track.disc_number,
            duration: track.duration.num_milliseconds() as u32,
            artists,
            artist_ids,
            album: Some(album.name.clone()),
            album_id: Some(album.id.id().to_string()),
            album_artists,
            cover_url: album.images.first().map(|img| img.url.clone()),
            url: track.id.as_ref().map(|id| id.url()).unwrap_or_default(),
            added_at: None,
            list_index: 0,
            is_local: track.is_local,
            is_playable: track.is_playable,
        }
    }

    pub fn duration_str(&self) -> String {
        ms_to_hms(self.duration)
    }
}

/// A track loaded through the playback session (see [crate::model::playlist::Playlist::from_session]).
impl From<&librespot_metadata::Track> for Track {
    fn from(track: &librespot_metadata::Track) -> Self {
        let id = track.id.to_id().ok();
        let base62 = |uri: &librespot_core::SpotifyUri| uri.to_id().unwrap_or_default();
        let album = &track.album;
        let cover = album
            .covers
            .iter()
            .chain(album.cover_group.iter())
            .max_by_key(|image| image.width)
            .and_then(|image| image.id.to_base16().ok());
        Self {
            uri: track.id.to_uri().unwrap_or_default(),
            url: id
                .as_ref()
                .map(|id| format!("https://open.spotify.com/track/{id}"))
                .unwrap_or_default(),
            id,
            title: track.name.clone(),
            track_number: track.number.max(0) as u32,
            disc_number: track.disc_number,
            duration: track.duration.max(0) as u32,
            artists: track.artists.iter().map(|a| a.name.clone()).collect(),
            artist_ids: track.artists.iter().map(|a| base62(&a.id)).collect(),
            album: Some(album.name.clone()),
            album_id: Some(base62(&album.id)),
            album_artists: album.artists.iter().map(|a| a.name.clone()).collect(),
            cover_url: cover.map(|hex| format!("https://i.scdn.co/image/{hex}")),
            added_at: None,
            list_index: 0,
            is_local: false,
            is_playable: Some(true),
        }
    }
}

impl From<&SimplifiedTrack> for Track {
    fn from(track: &SimplifiedTrack) -> Self {
        let artists = track
            .artists
            .iter()
            .map(|artist| artist.name.clone())
            .collect::<Vec<String>>();
        let artist_ids = track
            .artists
            .iter()
            .filter_map(|a| a.id.as_ref().map(|a| a.id().to_string()))
            .collect::<Vec<String>>();

        Self {
            id: track.id.as_ref().map(|id| id.id().to_string()),
            uri: track.id.as_ref().map(|id| id.uri()).unwrap_or_default(),
            title: track.name.clone(),
            track_number: track.track_number,
            disc_number: track.disc_number,
            duration: track.duration.num_milliseconds() as u32,
            artists,
            artist_ids,
            album: None,
            album_id: None,
            album_artists: Vec::new(),
            cover_url: None,
            url: track.id.as_ref().map(|id| id.url()).unwrap_or_default(),
            added_at: None,
            list_index: 0,
            is_local: track.is_local,
            is_playable: track.is_playable,
        }
    }
}

impl From<&FullTrack> for Track {
    fn from(track: &FullTrack) -> Self {
        let artists = track
            .artists
            .iter()
            .map(|artist| artist.name.clone())
            .collect::<Vec<String>>();
        let artist_ids = track
            .artists
            .iter()
            .filter_map(|a| a.id.as_ref().map(|a| a.id().to_string()))
            .collect::<Vec<String>>();
        let album_artists = track
            .album
            .artists
            .iter()
            .map(|artist| artist.name.clone())
            .collect::<Vec<String>>();

        Self {
            id: track.id.as_ref().map(|id| id.id().to_string()),
            uri: track.id.as_ref().map(|id| id.uri()).unwrap_or_default(),
            title: track.name.clone(),
            track_number: track.track_number,
            disc_number: track.disc_number,
            duration: track.duration.num_milliseconds() as u32,
            artists,
            artist_ids,
            album: Some(track.album.name.clone()),
            album_id: track.album.id.as_ref().map(|a| a.id().to_string()),
            album_artists,
            cover_url: track.album.images.first().map(|img| img.url.clone()),
            url: track.id.as_ref().map(|id| id.url()).unwrap_or_default(),
            added_at: None,
            list_index: 0,
            is_local: track.is_local,
            is_playable: track.is_playable,
        }
    }
}

impl From<&SavedTrack> for Track {
    fn from(st: &SavedTrack) -> Self {
        let mut track: Self = (&st.track).into();
        track.added_at = Some(st.added_at);
        track
    }
}

impl fmt::Display for Track {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{} - {}", self.artists.join(", "), self.title)
    }
}

impl ListItem for Track {
    fn thumbnail_url(&self) -> Option<String> {
        self.cover_url.clone()
    }

    fn is_playing(&self, queue: &Queue) -> bool {
        queue.is_current_id(self.id.as_deref())
    }

    fn display_left(&self, library: &Library) -> String {
        match custom_format(library, |f| &f.left) {
            Some(left) => Playable::format(&Playable::Track(self.clone()), &left, library),
            None => format!("{self}"),
        }
    }

    fn display_center(&self, library: &Library) -> String {
        match custom_format(library, |f| &f.center) {
            Some(center) => Playable::format(&Playable::Track(self.clone()), &center, library),
            None => self.album.clone().unwrap_or_default(),
        }
    }

    fn display_right(&self, library: &Library) -> String {
        if let Some(right) = custom_format(library, |f| &f.right) {
            return Playable::format(&Playable::Track(self.clone()), &right, library);
        }
        let saved = if library.is_saved_track_id(self.id.as_deref()) {
            if library.cfg.values().use_nerdfont.unwrap_or(false) {
                "\u{f012c}"
            } else {
                "✓"
            }
        } else {
            ""
        };
        format!("{} {}", saved, self.duration_str())
    }

    fn play(&mut self, queue: &Queue) {
        let index = queue.append_next(&[Playable::Track(self.clone())]);
        queue.play(index, true, false);
    }

    fn play_next(&mut self, queue: &Queue) {
        queue.insert_after_current(Playable::Track(self.clone()));
    }

    fn queue(&mut self, queue: &Queue) {
        queue.append(Playable::Track(self.clone()));
    }

    fn save(&mut self, library: &Library) {
        library.save_tracks(&[self]);
    }

    fn share_url(&self) -> Option<String> {
        self.id
            .clone()
            .map(|id| format!("https://open.spotify.com/track/{id}"))
    }

    fn album(&self) -> Option<Album> {
        let id = self.album_id.clone().filter(|id| !id.is_empty())?;
        Some(Album::stub(
            id,
            self.album.clone().unwrap_or_default(),
            self.album_artists.clone(),
            self.cover_url.clone(),
        ))
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

    fn track(&self) -> Option<Track> {
        Some(self.clone())
    }

    fn track_ref(&self) -> Option<&Track> {
        Some(self)
    }

    #[inline]
    fn is_playable(&self) -> bool {
        self.is_playable == Some(true)
    }

    fn as_listitem(&self) -> Box<dyn ListItem> {
        Box::new(self.clone())
    }
}

/// The user's `track_format` for one part of a track's row, if they set one other than the
/// default. Track rows are drawn often, so the usual case copies nothing.
fn custom_format(
    library: &Library,
    part: fn(&config::TrackFormat) -> &Option<String>,
) -> Option<String> {
    let values = library.cfg.values();
    let custom = part(values.track_format.as_ref()?).as_ref()?;
    (Some(custom) != part(&config::TrackFormat::default()).as_ref()).then(|| custom.clone())
}
