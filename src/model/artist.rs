use std::collections::HashMap;
use std::fmt;
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::thread;

use rspotify::model::Id;
use rspotify::model::artist::{FullArtist, SimplifiedArtist};

use crate::library::Library;
use crate::model::playable::Playable;
use crate::model::track::Track;
use crate::queue::Queue;
use crate::spotify::Spotify;
use crate::traits::{OpenTarget};
use crate::traits::{ListItem};

/// Artists' pictures by artist ID, for cards. Filled whenever an artist comes from the Web API
/// with its pictures (search results, followed and top artists); others are looked up one at a
/// time in the background once [init_image_lookups] has run. `None` means looked up, no picture.
static IMAGES: OnceLock<RwLock<HashMap<String, Option<String>>>> = OnceLock::new();
static LOOKUPS: OnceLock<Mutex<Sender<String>>> = OnceLock::new();

fn images() -> &'static RwLock<HashMap<String, Option<String>>> {
    IMAGES.get_or_init(Default::default)
}

/// Start looking up pictures of artists that come without one, calling `on_found` after each.
pub fn init_image_lookups(spotify: Spotify, on_found: impl Fn() + Send + 'static) {
    let (tx, rx) = channel::<String>();
    if LOOKUPS.set(Mutex::new(tx)).is_err() {
        return;
    }
    thread::spawn(move || {
        for id in rx {
            let image = spotify
                .api
                .artist(&id)
                .ok()
                .and_then(|a| a.images.first().map(|i| i.url.clone()));
            images().write().unwrap().insert(id, image);
            on_found();
        }
    });
}

#[derive(Clone, Deserialize, Serialize)]
pub struct Artist {
    pub id: Option<String>,
    pub name: String,
    pub url: Option<String>,
    pub tracks: Option<Vec<Track>>,
    pub is_followed: bool,
}

impl Artist {
    /// URL of the artist's picture, if it's known already.
    pub fn known_image_url(&self) -> Option<String> {
        images()
            .read()
            .unwrap()
            .get(self.id.as_ref()?)
            .cloned()
            .flatten()
    }

    /// URL of the artist's picture, if known. Starts a lookup the first time it's unknown.
    pub fn image_url(&self) -> Option<String> {
        let id = self.id.as_ref()?;
        if let Some(image) = images().read().unwrap().get(id) {
            return image.clone();
        }
        if let Some(lookups) = LOOKUPS.get() {
            images().write().unwrap().insert(id.clone(), None);
            let _ = lookups.lock().unwrap().send(id.clone());
        }
        None
    }

    pub fn new(id: String, name: String) -> Self {
        Self {
            id: Some(id),
            name,
            url: None,
            tracks: None,
            is_followed: false,
        }
    }

    fn load_top_tracks(&mut self, spotify: Spotify) {
        if let Some(artist_id) = &self.id
            && self.tracks.is_none()
        {
            self.tracks = spotify.api.artist_top_tracks(artist_id).ok();
        }
    }
}

impl From<&SimplifiedArtist> for Artist {
    fn from(sa: &SimplifiedArtist) -> Self {
        Self {
            id: sa.id.as_ref().map(|id| id.id().to_string()),
            name: sa.name.clone(),
            url: sa.id.as_ref().map(|id| id.url()),
            tracks: None,
            is_followed: false,
        }
    }
}

impl From<&FullArtist> for Artist {
    fn from(fa: &FullArtist) -> Self {
        let id = fa.id.id().to_string();
        images()
            .write()
            .unwrap()
            .insert(id, fa.images.first().map(|i| i.url.clone()));
        Self {
            id: Some(fa.id.id().to_string()),
            name: fa.name.clone(),
            url: Some(fa.id.url()),
            tracks: None,
            is_followed: false,
        }
    }
}

impl fmt::Display for Artist {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.name)
    }
}

impl fmt::Debug for Artist {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{} ({:?})", self.name, self.id)
    }
}

impl ListItem for Artist {
    fn target(&self) -> Option<OpenTarget> {
        Some(OpenTarget::Artist(self.clone()))
    }

    fn thumbnail_url(&self) -> Option<String> {
        self.known_image_url()
    }

    fn is_playing(&self, queue: &Queue) -> bool {
        if let Some(tracks) = &self.tracks {
            let playing: Vec<String> = queue
                .queue
                .read()
                .unwrap()
                .iter()
                .filter_map(|t| t.id())
                .collect();
            let ids: Vec<String> = tracks.iter().filter_map(|t| t.id.clone()).collect();
            !ids.is_empty() && playing == ids
        } else {
            false
        }
    }

    fn display_left(&self, _library: &Library) -> String {
        format!("{self}")
    }

    fn display_right(&self, library: &Library) -> String {
        let followed = if library.is_followed_artist(self) {
            if library.cfg.values().use_nerdfont.unwrap_or(false) {
                "\u{f012c} "
            } else {
                "✓ "
            }
        } else {
            ""
        };

        let tracks = if let Some(tracks) = self.tracks.as_ref() {
            format!("{:>3} saved tracks", tracks.len())
        } else {
            "".into()
        };

        format!("{followed}{tracks}")
    }

    fn play(&mut self, queue: &Queue) {
        self.load_top_tracks(queue.get_spotify());

        if let Some(tracks) = self.tracks.as_ref() {
            let tracks: Vec<Playable> = tracks
                .iter()
                .map(|track| Playable::Track(track.clone()))
                .collect();
            let index = queue.append_next(&tracks);
            queue.play(index, true, true);
        }
    }

    fn play_next(&mut self, queue: &Queue) {
        self.load_top_tracks(queue.get_spotify());

        if let Some(tracks) = self.tracks.as_ref() {
            for t in tracks.iter().rev() {
                queue.insert_after_current(Playable::Track(t.clone()));
            }
        }
    }

    fn queue(&mut self, queue: &Queue) {
        self.load_top_tracks(queue.get_spotify());

        if let Some(tracks) = &self.tracks {
            for t in tracks {
                queue.append(Playable::Track(t.clone()));
            }
        }
    }

    fn toggle_saved(&mut self, library: &Library) {
        if library.is_followed_artist(self) {
            library.unfollow_artist(self);
        } else {
            library.follow_artist(self);
        }
    }

    fn save(&mut self, library: &Library) {
        library.follow_artist(self);
    }

    fn unsave(&mut self, library: &Library) {
        library.unfollow_artist(self);
    }

    fn share_url(&self) -> Option<String> {
        self.id
            .clone()
            .map(|id| format!("https://open.spotify.com/artist/{id}"))
    }

    #[inline]
    fn is_saved(&self, library: &Library) -> Option<bool> {
        Some(library.is_followed_artist(self))
    }

    #[inline]
    fn is_playable(&self) -> bool {
        true
    }

    fn as_listitem(&self) -> Box<dyn ListItem> {
        Box::new(self.clone())
    }
}
