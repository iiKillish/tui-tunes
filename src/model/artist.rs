use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::mpsc::{RecvTimeoutError, Sender, channel};
use std::sync::{Mutex, OnceLock, RwLock};
use std::thread;
use std::time::Duration;

use log::error;

use rspotify::model::Id;
use rspotify::model::artist::{FullArtist, SimplifiedArtist};

use crate::library::Library;
use crate::model::playable::Playable;
use crate::model::track::Track;
use crate::queue::Queue;
use crate::spotify::Spotify;
use crate::traits::ListItem;
use crate::traits::OpenTarget;

/// Where the pictures looked up are kept between runs, in the cache directory.
const PICTURES_FILE: &str = "artist_pictures.json";
/// How long the lookup thread waits for more artists to look up together.
const GATHER_LOOKUPS: Duration = Duration::from_millis(200);
/// How often pictures that came with artists from the Web API are saved, if there are new ones.
const SAVE_EVERY: Duration = Duration::from_secs(60);

/// Artists' pictures by artist ID, for cards. Filled whenever an artist comes from the Web API
/// with its pictures (search results, followed and top artists); others are looked up in the
/// background once [init_image_lookups] has run, which also loads the ones from earlier runs.
#[derive(Default)]
struct Pictures {
    /// Picture URLs by artist ID; `None` means looked up, no picture.
    known: HashMap<String, Option<String>>,
    /// Artists being looked up.
    pending: HashSet<String>,
    /// Whether `known` has changed since it was saved.
    unsaved: bool,
}

impl Pictures {
    fn insert(&mut self, id: String, picture: Option<String>) {
        self.pending.remove(&id);
        if self.known.get(&id) != Some(&picture) {
            self.known.insert(id, picture);
            self.unsaved = true;
        }
    }

    /// Add pictures saved by an earlier run, for artists not known yet.
    fn add_saved(&mut self, saved: HashMap<String, Option<String>>) {
        for (id, picture) in saved {
            self.pending.remove(&id);
            self.known.entry(id).or_insert(picture);
        }
    }
}

static IMAGES: OnceLock<RwLock<Pictures>> = OnceLock::new();
static LOOKUPS: OnceLock<Mutex<Sender<String>>> = OnceLock::new();

fn images() -> &'static RwLock<Pictures> {
    IMAGES.get_or_init(Default::default)
}

/// The pictures saved by earlier runs.
fn load_pictures() -> HashMap<String, Option<String>> {
    std::fs::read_to_string(crate::config::cache_path(PICTURES_FILE))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// Save the pictures known, if they changed since they were last saved.
fn save_pictures() {
    let json = {
        let mut pictures = images().write().unwrap();
        if !pictures.unsaved {
            return;
        }
        pictures.unsaved = false;
        serde_json::to_string(&pictures.known)
    };
    let saved = json.map_err(|e| e.to_string()).and_then(|json| {
        std::fs::write(crate::config::cache_path(PICTURES_FILE), json).map_err(|e| e.to_string())
    });
    if let Err(e) = saved {
        error!("Couldn't save artist pictures: {e}");
    }
}

/// Start looking up pictures of artists that come without one, calling `on_found` as they're
/// found. Pictures found are saved, so each artist is only looked up once.
pub fn init_image_lookups(spotify: Spotify, on_found: impl Fn() + Send + 'static) {
    let (tx, rx) = channel::<String>();
    if LOOKUPS.set(Mutex::new(tx)).is_err() {
        return;
    }
    thread::spawn(move || {
        let saved = load_pictures();
        images().write().unwrap().add_saved(saved);
        on_found();
        loop {
            let first = match rx.recv_timeout(SAVE_EVERY) {
                Ok(id) => id,
                Err(RecvTimeoutError::Timeout) => {
                    save_pictures();
                    continue;
                }
                Err(RecvTimeoutError::Disconnected) => break,
            };
            // A page of cards asks for its artists all at once: look them up together.
            thread::sleep(GATHER_LOOKUPS);
            let ids: Vec<String> = std::iter::once(first)
                .chain(rx.try_iter())
                .filter(|id| images().read().unwrap().pending.contains(id))
                .collect();
            if ids.is_empty() {
                continue;
            }
            // Through the session first, which costs no Web API requests; the Web API for
            // the ones it has no picture of.
            let pictures = spotify.api.session_artist_pictures(&ids);
            let mut missing = Vec::new();
            {
                let mut images = images().write().unwrap();
                for (id, picture) in ids.into_iter().zip(pictures) {
                    match picture {
                        Some(url) => images.insert(id, Some(url)),
                        None => missing.push(id),
                    }
                }
            }
            on_found();
            for id in missing {
                let picture = spotify
                    .api
                    .artist(&id)
                    .ok()
                    .and_then(|a| a.images.first().map(|i| i.url.clone()));
                images().write().unwrap().insert(id, picture);
                on_found();
            }
            save_pictures();
        }
        save_pictures();
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
            .known
            .get(self.id.as_ref()?)
            .cloned()
            .flatten()
    }

    /// URL of the artist's picture, if known. Starts a lookup the first time it's unknown.
    pub fn image_url(&self) -> Option<String> {
        let id = self.id.as_ref()?;
        {
            let images = images().read().unwrap();
            if let Some(image) = images.known.get(id) {
                return image.clone();
            }
            if images.pending.contains(id) {
                return None;
            }
        }
        if let Some(lookups) = LOOKUPS.get()
            && images().write().unwrap().pending.insert(id.clone())
        {
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
        self.tracks
            .as_ref()
            .is_some_and(|tracks| queue.consists_of(tracks.iter().filter_map(|t| t.id.as_deref())))
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

    fn save(&mut self, library: &Library) {
        library.follow_artist(self);
    }

    fn share_url(&self) -> Option<String> {
        self.id
            .clone()
            .map(|id| format!("https://open.spotify.com/artist/{id}"))
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

    #[test]
    fn test_pictures_saved_earlier_fill_in_pending_but_not_newer() {
        let mut pictures = Pictures::default();
        pictures.pending.insert("a".into());
        pictures.insert("b".into(), Some("new".into()));
        pictures.add_saved(HashMap::from([
            ("a".to_string(), None),
            ("b".to_string(), Some("old".into())),
        ]));
        assert!(pictures.pending.is_empty());
        assert_eq!(pictures.known.get("a"), Some(&None));
        assert_eq!(pictures.known.get("b"), Some(&Some("new".into())));
    }

    #[test]
    fn test_pictures_only_unsaved_when_changed() {
        let mut pictures = Pictures::default();
        pictures.add_saved(HashMap::from([("a".to_string(), Some("url".into()))]));
        assert!(!pictures.unsaved);
        pictures.insert("a".into(), Some("url".into()));
        assert!(!pictures.unsaved);
        pictures.insert("a".into(), None);
        assert!(pictures.unsaved);
    }
}
