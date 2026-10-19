use chrono::{DateTime, Utc};
use rspotify::model::PlayableItem;

use crate::library::Library;
use crate::model::album::Album;
use crate::model::artist::Artist;
use crate::model::episode::Episode;
use crate::model::track::Track;
use crate::queue::Queue;
use crate::traits::ListItem;
use crate::utils::ms_to_hms;
use std::fmt;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type")]
pub enum Playable {
    Track(Track),
    Episode(Episode),
}

impl Playable {
    pub fn format(playable: &Self, formatting: &str, library: &Library) -> String {
        // The artists with an ID, as [ListItem::artists] lists them.
        let artists = || -> Vec<&str> {
            match playable {
                Self::Track(track) => track
                    .artists
                    .iter()
                    .take(track.artist_ids.len())
                    .map(String::as_str)
                    .collect(),
                Self::Episode(_) => Vec::new(),
            }
        };
        let mut text = formatting.to_string();
        // Only work out what the format uses ("%artists" before "%artist", which it contains).
        if text.contains("%artists") {
            text = text.replace("%artists", &artists().join(", "));
        }
        if text.contains("%artist") {
            text = text.replace("%artist", artists().first().copied().unwrap_or_default());
        }
        if text.contains("%title") {
            let title = match playable {
                Self::Episode(episode) => &episode.name,
                Self::Track(track) => &track.title,
            };
            text = text.replace("%title", title);
        }
        if text.contains("%album") {
            let album = match playable {
                Self::Track(track) => track.album.as_deref().unwrap_or_default(),
                Self::Episode(_) => "",
            };
            text = text.replace("%album", album);
        }
        if text.contains("%saved") {
            let saved = if library.is_saved_track(playable) {
                if library.cfg.values().use_nerdfont.unwrap_or_default() {
                    "\u{f012c}"
                } else {
                    "✓"
                }
            } else {
                ""
            };
            text = text.replace("%saved", saved);
        }
        if text.contains("%duration") {
            text = text.replace("%duration", &playable.duration_str());
        }
        text
    }

    /// The ID, without copying it.
    pub fn id_ref(&self) -> Option<&str> {
        match self {
            Self::Track(track) => track.id.as_deref(),
            Self::Episode(episode) => Some(&episode.id),
        }
    }

    pub fn id(&self) -> Option<String> {
        match self {
            Self::Track(track) => track.id.clone(),
            Self::Episode(episode) => Some(episode.id.clone()),
        }
    }

    pub fn uri(&self) -> String {
        match self {
            Self::Track(track) => track.uri.clone(),
            Self::Episode(episode) => episode.uri.clone(),
        }
    }

    pub fn cover_url(&self) -> Option<String> {
        match self {
            Self::Track(track) => track.cover_url.clone(),
            Self::Episode(episode) => episode.cover_url.clone(),
        }
    }

    pub fn duration(&self) -> u32 {
        match self {
            Self::Track(track) => track.duration,
            Self::Episode(episode) => episode.duration,
        }
    }

    pub fn list_index(&self) -> usize {
        match self {
            Self::Track(track) => track.list_index,
            Self::Episode(episode) => episode.list_index,
        }
    }

    pub fn set_list_index(&mut self, index: usize) {
        match self {
            Self::Track(track) => track.list_index = index,
            Self::Episode(episode) => episode.list_index = index,
        }
    }

    pub fn set_added_at(&mut self, added_at: Option<DateTime<Utc>>) {
        match self {
            Self::Track(track) => track.added_at = added_at,
            Self::Episode(episode) => episode.added_at = added_at,
        }
    }

    pub fn duration_str(&self) -> String {
        ms_to_hms(self.duration())
    }

    /// The track or episode, for [ListItem] calls without the copy [Self::as_listitem] makes.
    fn inner(&self) -> &dyn ListItem {
        match self {
            Self::Track(track) => track,
            Self::Episode(episode) => episode,
        }
    }

    fn inner_mut(&mut self) -> &mut dyn ListItem {
        match self {
            Self::Track(track) => track,
            Self::Episode(episode) => episode,
        }
    }

    pub fn as_listitem(&self) -> Box<dyn ListItem> {
        match self {
            Self::Track(track) => track.as_listitem(),
            Self::Episode(episode) => episode.as_listitem(),
        }
    }
}

impl From<&PlayableItem> for Playable {
    fn from(item: &PlayableItem) -> Self {
        match item {
            PlayableItem::Episode(episode) => Self::Episode(episode.into()),
            PlayableItem::Track(track) => Self::Track(track.into()),
            PlayableItem::Unknown(value) => panic!("Unknown playable item {value:?}"),
        }
    }
}

impl From<&Playable> for Option<rspotify::prelude::PlayableId<'_>> {
    fn from(p: &Playable) -> Self {
        match p {
            Playable::Track(t) => {
                t.id.clone()
                    .and_then(|id| rspotify::model::TrackId::from_id(id).ok())
                    .map(rspotify::prelude::PlayableId::Track)
            }
            Playable::Episode(e) => rspotify::model::EpisodeId::from_id(e.id.clone())
                .map(rspotify::prelude::PlayableId::Episode)
                .ok(),
        }
    }
}

impl fmt::Display for Playable {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Track(track) => track.fmt(f),
            Self::Episode(episode) => episode.fmt(f),
        }
    }
}

impl ListItem for Playable {
    fn thumbnail_url(&self) -> Option<String> {
        self.cover_url()
    }

    fn is_playable(&self) -> bool {
        self.inner().is_playable()
    }

    fn is_playing(&self, queue: &Queue) -> bool {
        self.inner().is_playing(queue)
    }

    fn display_left(&self, library: &Library) -> String {
        self.inner().display_left(library)
    }

    fn display_center(&self, library: &Library) -> String {
        self.inner().display_center(library)
    }

    fn display_right(&self, library: &Library) -> String {
        self.inner().display_right(library)
    }

    fn play(&mut self, queue: &Queue) {
        self.inner_mut().play(queue)
    }

    fn play_next(&mut self, queue: &Queue) {
        self.inner_mut().play_next(queue)
    }

    fn queue(&mut self, queue: &Queue) {
        self.inner_mut().queue(queue)
    }

    fn save(&mut self, library: &Library) {
        self.inner_mut().save(library)
    }

    fn share_url(&self) -> Option<String> {
        self.inner().share_url()
    }

    fn album(&self, queue: &Queue) -> Option<Album> {
        self.inner().album(queue)
    }

    fn artists(&self) -> Option<Vec<Artist>> {
        self.inner().artists()
    }

    fn track(&self) -> Option<Track> {
        self.inner().track()
    }

    fn track_ref(&self) -> Option<&Track> {
        match self {
            Self::Track(track) => Some(track),
            Self::Episode(_) => None,
        }
    }

    fn as_listitem(&self) -> Box<dyn ListItem> {
        self.as_listitem()
    }
}
