use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::Write;
use std::iter::Iterator;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::thread;

use log::{debug, error, info};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::config::Config;
use crate::config::{self, CACHE_VERSION};
use crate::events::EventManager;
use crate::model::album::Album;
use crate::model::artist::Artist;
use crate::model::playable::Playable;
use crate::model::playlist::Playlist;
use crate::model::show::Show;
use crate::model::track::Track;
use crate::spotify::Spotify;

/// Cached tracks database filename.
const CACHE_TRACKS: &str = "tracks.db";

/// Cached albums database filename.
const CACHE_ALBUMS: &str = "albums.db";

/// Cached artists database filename.
const CACHE_ARTISTS: &str = "artists.db";

/// Cached playlists database filename.
const CACHE_PLAYLISTS: &str = "playlists.db";

/// The user library with all their saved tracks, albums, playlists... High level interface to the
/// Spotify API used to manage items in the user library.
#[derive(Clone)]
pub struct Library {
    pub tracks: Arc<RwLock<Vec<Track>>>,
    pub albums: Arc<RwLock<Vec<Album>>>,
    pub artists: Arc<RwLock<Vec<Artist>>>,
    pub playlists: Arc<RwLock<Vec<Playlist>>>,
    pub shows: Arc<RwLock<Vec<Show>>>,
    pub is_done: Arc<RwLock<bool>>,
    pub user_id: Option<String>,
    /// Tracks being saved (true) or removed (false) by [Self::toggle_saved_track_in_background],
    /// shown in their new state until Spotify has been told.
    pending_saves: Arc<RwLock<HashMap<String, bool>>>,
    /// IDs of [Self::tracks] and [Self::playlists], kept up to date with them by
    /// [Self::index_tracks] and [Self::index_playlists].
    saved_ids: Arc<RwLock<SavedIds>>,
    /// A hash of what [Self::save_cache] last wrote to each file, so it isn't written again
    /// unchanged.
    saved_caches: Arc<Mutex<HashMap<PathBuf, u64>>>,
    ev: EventManager,
    spotify: Spotify,
    pub cfg: Arc<Config>,
}

impl Library {
    /// Create an empty library for use in tests. No cache is loaded and no API calls are made.
    #[cfg(test)]
    pub fn new_for_test(ev: EventManager, spotify: Spotify, cfg: Arc<Config>) -> Arc<Self> {
        Arc::new(Self {
            tracks: Arc::new(RwLock::new(Vec::new())),
            albums: Arc::new(RwLock::new(Vec::new())),
            artists: Arc::new(RwLock::new(Vec::new())),
            playlists: Arc::new(RwLock::new(Vec::new())),
            shows: Arc::new(RwLock::new(Vec::new())),
            is_done: Arc::new(RwLock::new(false)),
            user_id: None,
            pending_saves: Arc::default(),
            saved_ids: Arc::default(),
            saved_caches: Arc::default(),
            ev,
            spotify,
            cfg,
        })
    }

    pub fn new(ev: EventManager, spotify: Spotify, cfg: Arc<Config>) -> Self {
        // The session's username is the user's ID: asking Spotify (/me) could wait out a rate
        // limit before anything is shown.
        let user_id = spotify.api.user();

        let library = Self {
            tracks: Arc::new(RwLock::new(Vec::new())),
            albums: Arc::new(RwLock::new(Vec::new())),
            artists: Arc::new(RwLock::new(Vec::new())),
            playlists: Arc::new(RwLock::new(Vec::new())),
            shows: Arc::new(RwLock::new(Vec::new())),
            is_done: Arc::new(RwLock::new(false)),
            user_id,
            pending_saves: Arc::default(),
            saved_ids: Arc::default(),
            saved_caches: Arc::default(),
            ev,
            spotify,
            cfg,
        };

        library.update_library(false);
        library
    }

    /// Load cached items from the file at `cache_path` into the given `store`.
    fn load_cache<T: DeserializeOwned>(&self, cache_path: &Path, store: &mut Vec<T>) {
        let saved_cache_version = self.cfg.state().cache_version;
        if saved_cache_version < CACHE_VERSION {
            debug!(
                "Cache version for {cache_path:?} has changed from {saved_cache_version} to {CACHE_VERSION}, ignoring cache"
            );
            return;
        }

        if let Ok(contents) = std::fs::read_to_string(cache_path) {
            debug!("loading cache from {}", cache_path.display());
            // Parse from in-memory string instead of directly from the file because it's faster.
            let parsed = serde_json::from_str::<Vec<_>>(&contents);
            match parsed {
                Ok(cache) => {
                    debug!(
                        "cache from {} loaded ({} items)",
                        cache_path.display(),
                        cache.len()
                    );
                    store.clear();
                    store.extend(cache);

                    // force refresh of UI (if visible)
                    self.trigger_redraw();
                }
                Err(e) => {
                    error!("can't parse cache: {e}");
                }
            }
        }
    }

    /// Save the items from `store` in the file at `cache_path`, unless that's what it holds
    /// already. Written to a file beside it first, then moved over it, so a crash or a full disk
    /// can't leave half a cache.
    fn save_cache<T: Serialize>(&self, cache_path: &Path, store: &[T]) {
        let json = match serde_json::to_vec(store) {
            Ok(json) => json,
            Err(e) => {
                error!("could not write cache {}: {e}", cache_path.display());
                return;
            }
        };
        let mut hasher = DefaultHasher::new();
        json.hash(&mut hasher);
        let hash = hasher.finish();
        let mut saved = self.saved_caches.lock().unwrap();
        if saved.get(cache_path) == Some(&hash) {
            return;
        }
        let temp = cache_path.with_extension("db.tmp");
        let written = std::fs::File::create(&temp)
            .and_then(|mut file| file.write_all(&json))
            .and_then(|()| std::fs::rename(&temp, cache_path));
        match written {
            Ok(()) => {
                saved.insert(cache_path.to_path_buf(), hash);
            }
            Err(e) => {
                error!("could not write cache {}: {e}", cache_path.display());
                let _ = std::fs::remove_file(&temp);
            }
        }
    }

    /// Check whether the `remote` [Playlist] is newer than its locally saved version. Returns
    /// `true` if it is or if a local version isn't found.
    fn needs_download(&self, remote: &Playlist) -> bool {
        self.playlists
            .read()
            .unwrap()
            .iter()
            .find(|local| local.id == remote.id)
            .map(|local| local.snapshot_id != remote.snapshot_id)
            .unwrap_or(true)
    }

    /// Append `updated` to the local playlists or update the local version if it exists. Return the
    /// index of the appended/updated playlist.
    fn append_or_update(&self, updated: Playlist) -> usize {
        let mut store = self.playlists.write().unwrap();
        for (index, local) in store.iter_mut().enumerate() {
            if local.id == updated.id {
                *local = updated;
                return index;
            }
        }
        store.push(updated);
        let index = store.len() - 1;
        drop(store);
        self.index_playlists();
        index
    }

    /// Delete the playlist with the given `id` if it exists.
    pub fn delete_playlist(&self, id: &str) {
        if !*self.is_done.read().unwrap() {
            return;
        }

        let position = self
            .playlists
            .read()
            .unwrap()
            .iter()
            .position(|i| i.id == id);

        if let Some(position) = position
            && self.spotify.api.delete_playlist(id).is_ok()
        {
            self.playlists.write().unwrap().remove(position);
            self.index_playlists();
            self.save_cache(
                &config::cache_path(CACHE_PLAYLISTS),
                &self.playlists.read().unwrap(),
            );
        }
    }

    /// Set the playlist with `id` to contain only `tracks`. If the playlist already contains
    /// tracks, they will be removed. Update the cache to match the new state.
    pub fn overwrite_playlist(&self, id: &str, tracks: &[Playable]) {
        debug!("saving {} tracks to list {}", tracks.len(), id);
        self.spotify.api.overwrite_playlist(id, tracks);

        self.fetch_playlists();
        self.save_cache(
            &config::cache_path(CACHE_PLAYLISTS),
            &self.playlists.read().unwrap(),
        );
    }

    /// Create a playlist with the given `name` and add `tracks` to it.
    pub fn save_playlist(&self, name: &str, tracks: &[Playable]) {
        debug!("saving {} tracks to new list {}", tracks.len(), name);
        match self.spotify.api.create_playlist(name, None, None) {
            Ok(id) => self.overwrite_playlist(&id, tracks),
            Err(_) => error!("could not create new playlist.."),
        }
    }

    /// Update the local library and its cache on disk. `force` refetches unchanged lists too.
    pub fn update_library(&self, force: bool) {
        *self.is_done.write().unwrap() = false;

        let library = self.clone();
        thread::spawn(move || {
            let t_tracks = {
                let library = library.clone();
                thread::spawn(move || {
                    library.load_cache(
                        &config::cache_path(CACHE_TRACKS),
                        library.tracks.write().unwrap().as_mut(),
                    );
                    library.index_tracks();
                    if library.fetch_tracks(force) {
                        library.save_cache(
                            &config::cache_path(CACHE_TRACKS),
                            &library.tracks.read().unwrap(),
                        );
                    }
                })
            };

            let t_albums = {
                let library = library.clone();
                thread::spawn(move || {
                    library.load_cache(
                        &config::cache_path(CACHE_ALBUMS),
                        library.albums.write().unwrap().as_mut(),
                    );
                    if library.fetch_albums(force) {
                        library.save_cache(
                            &config::cache_path(CACHE_ALBUMS),
                            &library.albums.read().unwrap(),
                        );
                    }
                })
            };

            let t_artists = {
                let library = library.clone();
                thread::spawn(move || {
                    library.load_cache(
                        &config::cache_path(CACHE_ARTISTS),
                        library.artists.write().unwrap().as_mut(),
                    );
                    library.fetch_artists(force);
                })
            };

            let t_playlists = {
                let library = library.clone();
                thread::spawn(move || {
                    library.load_cache(
                        &config::cache_path(CACHE_PLAYLISTS),
                        library.playlists.write().unwrap().as_mut(),
                    );
                    library.index_playlists();
                    library.fetch_playlists();
                    library.save_cache(
                        &config::cache_path(CACHE_PLAYLISTS),
                        &library.playlists.read().unwrap(),
                    );
                })
            };

            let t_shows = {
                let library = library.clone();
                thread::spawn(move || {
                    library.fetch_shows();
                })
            };

            t_tracks.join().unwrap();
            t_artists.join().unwrap();

            library.populate_artists();
            library.save_cache(
                &config::cache_path(CACHE_ARTISTS),
                &library.artists.read().unwrap(),
            );

            t_albums.join().unwrap();
            t_playlists.join().unwrap();
            t_shows.join().unwrap();

            let mut is_done = library.is_done.write().unwrap();
            *is_done = true;

            library.ev.trigger();
        });
    }

    /// Fetch the shows from the web API and save them to the local library.
    fn fetch_shows(&self) {
        debug!("loading shows");

        let mut saved_shows: Vec<Show> = Vec::new();
        let mut shows_result = self.spotify.api.get_saved_shows(0).ok();

        while let Some(shows) = shows_result {
            saved_shows.extend(shows.items.iter().map(|show| (&show.show).into()));

            // load next batch if necessary
            shows_result = match shows.next {
                Some(_) => {
                    debug!("requesting shows again..");
                    self.spotify
                        .api
                        .get_saved_shows(shows.offset + shows.items.len() as u32)
                        .ok()
                }
                None => None,
            }
        }

        *self.shows.write().unwrap() = saved_shows;
    }

    /// Fetch the playlists from the web API and save them to the local library. This synchronizes
    /// the local version with the remote, pruning removed playlists in the process.
    fn fetch_playlists(&self) {
        debug!("loading playlists");
        let mut stale_lists = self.playlists.read().unwrap().clone();
        let mut list_order = Vec::new();

        let lists_page = self.spotify.api.current_user_playlist();
        let mut lists_batch = Some(lists_page.items.read().unwrap().clone());
        while let Some(lists) = lists_batch {
            for (index, remote) in lists.iter().enumerate() {
                list_order.push(remote.id.clone());

                // remove from stale playlists so we won't prune it later on
                if let Some(index) = stale_lists.iter().position(|x| x.id == remote.id) {
                    stale_lists.remove(index);
                }

                if self.needs_download(remote) {
                    info!("updating playlist {} (index: {})", remote.name, index);
                    let mut playlist: Playlist = remote.clone();
                    playlist.tracks = None;
                    playlist.load_tracks(&self.spotify);
                    self.append_or_update(playlist);
                    // trigger redraw
                    self.trigger_redraw();
                } else if let Some(cached) = self
                    .playlists
                    .write()
                    .unwrap()
                    .iter_mut()
                    .find(|p| p.id == remote.id)
                {
                    // Unchanged, but caches from before covers were kept lack the cover.
                    cached.cover_url.clone_from(&remote.cover_url);
                }
            }
            lists_batch = lists_page.next();
        }

        // remove stale playlists
        for stale in stale_lists {
            let index = self
                .playlists
                .read()
                .unwrap()
                .iter()
                .position(|x| x.id == stale.id);
            if let Some(index) = index {
                debug!("removing stale list: {:?}", stale.name);
                self.playlists.write().unwrap().remove(index);
            }
        }
        self.index_playlists();

        // sort by remote order
        self.playlists.write().unwrap().sort_by(|a, b| {
            let a_index = list_order.iter().position(|x| x == &a.id);
            let b_index = list_order.iter().position(|x| x == &b.id);
            a_index.cmp(&b_index)
        });

        // trigger redraw
        self.trigger_redraw();
    }

    /// Fetch the followed artists from the web API and save them to the local library. Stops
    /// after the first page if it looks unchanged from the cache, unless `force`.
    fn fetch_artists(&self, force: bool) {
        let mut artists: Vec<Artist> = Vec::new();
        let mut last: Option<&str> = None;
        let mut i = 0u32;

        loop {
            let page = self.spotify.api.current_user_followed_artists(last);
            debug!("artists page: {i}");
            i += 1;
            if page.is_err() {
                error!("Failed to fetch artists.");
                return;
            }
            let page = page.unwrap();
            let items: Vec<Artist> = page.items.iter().map(|fa| fa.into()).collect();

            // Spotify offers no cheaper way to tell whether they changed.
            let total = page
                .total
                .or_else(|| page.next.is_none().then_some(items.len() as u32));
            if !force
                && last.is_none()
                && followed_unchanged(total, &items, &self.artists.read().unwrap())
            {
                debug!("followed artists unchanged");
                return;
            }

            artists.extend(items);

            if page.next.is_some() {
                last = artists.last().unwrap().id.as_deref();
            } else {
                break;
            }
        }

        let mut store = self.artists.write().unwrap();

        for mut artist in artists {
            let pos = store.iter().position(|a| a.id == artist.id);
            if let Some(i) = pos {
                store[i].is_followed = true;
                continue;
            }

            artist.is_followed = true;

            store.push(artist);
        }
    }

    /// Add the artist with `id` and `name` to the user library, but don't sync with the API.
    /// This does not add if there is already an artist with `id`.
    fn insert_artist(&self, id: String, name: String) {
        let mut artists = self.artists.write().unwrap();

        if !artists
            .iter()
            .any(|a| a.id.as_ref().is_some_and(|value| *value == id))
        {
            let mut artist = Artist::new(id.to_string(), name.to_string());
            artist.tracks = Some(Vec::new());
            artists.push(artist);
        }
    }

    /// Fetch the albums from the web API and store them in the local library. Returns `false` if
    /// the cache doesn't need saving.
    fn fetch_albums(&self, force: bool) -> bool {
        let mut albums: Vec<Album> = Vec::new();
        let mut i = 0u32;

        loop {
            let page = self
                .spotify
                .api
                .current_user_saved_albums(albums.len() as u32);
            debug!("albums page: {i}");

            i += 1;

            if page.is_err() {
                error!("Failed to fetch albums.");
                return true;
            }

            let page = page.unwrap();
            let items: Vec<Album> = page.items.iter().map(|a| a.into()).collect();

            if !force
                && page.offset == 0
                && albums_unchanged(page.total, &items, &self.albums.read().unwrap())
            {
                return sort_albums(&mut self.albums.write().unwrap());
            }

            albums.extend(items);

            if page.next.is_none() {
                break;
            }
        }

        sort_albums(&mut albums);

        *self.albums.write().unwrap() = albums;
        true
    }

    /// Fetch the tracks from the web API and save them in the local library. Returns `false` if
    /// the cache doesn't need saving.
    fn fetch_tracks(&self, force: bool) -> bool {
        let mut tracks = Vec::new();
        let mut i = 0u32;

        loop {
            let page = self
                .spotify
                .api
                .current_user_saved_tracks(tracks.len() as u32);

            debug!("tracks page: {i}");
            i += 1;

            if page.is_err() {
                error!("Failed to fetch tracks.");
                return true;
            }
            let page = page.unwrap();
            let items: Vec<Track> = page.items.iter().map(|t| t.into()).collect();

            if !force
                && page.offset == 0
                && tracks_unchanged(page.total, &items, &self.tracks.read().unwrap())
            {
                return false;
            }

            tracks.extend(items);

            if page.next.is_none() {
                break;
            }
        }

        *self.tracks.write().unwrap() = tracks;
        self.index_tracks();
        true
    }

    fn populate_artists(&self) {
        // Remove old unfollowed artists
        {
            let mut artists = self.artists.write().unwrap();
            *artists = artists.iter().filter(|a| a.is_followed).cloned().collect();
        }

        // Add artists that aren't followed but have saved tracks
        {
            let tracks = self.tracks.read().unwrap();
            let mut track_artists: Vec<(&String, &String)> = tracks
                .iter()
                .flat_map(|t| t.artist_ids.iter().zip(t.artists.iter()))
                .collect();
            track_artists.dedup_by(|a, b| a.0 == b.0);

            for (id, name) in track_artists.iter() {
                self.insert_artist(id.to_string(), name.to_string());
            }
        }

        let mut artists = self.artists.write().unwrap();
        let mut lookup: HashMap<String, Option<usize>> = HashMap::new();

        // Make sure only saved tracks are played when playing artists
        for artist in artists.iter_mut() {
            artist.tracks = Some(Vec::new());
        }

        artists.sort_unstable_by(|a, b| {
            let a_cmp = a.name.strip_prefix("The ").unwrap_or(&a.name);
            let b_cmp = b.name.strip_prefix("The ").unwrap_or(&b.name);

            a_cmp.partial_cmp(b_cmp).unwrap()
        });

        // Add saved tracks to artists
        {
            let tracks = self.tracks.read().unwrap();
            for track in tracks.iter() {
                for artist_id in &track.artist_ids {
                    let index = if let Some(i) = lookup.get(artist_id).cloned() {
                        i
                    } else {
                        let i = artists
                            .iter()
                            .position(|a| &a.id.clone().unwrap_or_default() == artist_id);
                        lookup.insert(artist_id.clone(), i);
                        i
                    };

                    if let Some(i) = index {
                        let artist = artists.get_mut(i).unwrap();
                        if artist.tracks.is_none() {
                            artist.tracks = Some(Vec::new());
                        }

                        if let Some(tracks) = artist.tracks.as_mut() {
                            if tracks.iter().any(|t| t.id == track.id) {
                                continue;
                            }

                            tracks.push(track.clone());
                        }
                    }
                }
            }
        }
    }

    /// If there is a local version of the playlist, update it and rewrite the cache.
    /// Rename the playlist with `id` on Spotify and here.
    pub fn rename_playlist(&self, id: &str, name: &str) -> Result<(), ()> {
        self.spotify.api.rename_playlist(id, name)?;
        if let Some(playlist) = self
            .playlists
            .write()
            .unwrap()
            .iter_mut()
            .find(|p| p.id == id)
        {
            playlist.name = name.to_string();
        }
        self.save_cache(
            &config::cache_path(CACHE_PLAYLISTS),
            &self.playlists.read().unwrap(),
        );
        Ok(())
    }

    pub fn playlist_update(&self, updated: &Playlist) {
        {
            let mut playlists = self.playlists.write().unwrap();
            if let Some(playlist) = playlists.iter_mut().find(|p| p.id == updated.id) {
                *playlist = updated.clone();
            }
        }

        self.save_cache(
            &config::cache_path(CACHE_PLAYLISTS),
            &self.playlists.read().unwrap(),
        );
    }

    /// Check whether `track` is saved in the user's library.
    pub fn is_saved_track(&self, track: &Playable) -> bool {
        self.is_saved_track_id(track.id_ref())
    }

    /// Whether the track with `id` is saved; see [Self::is_saved_track].
    pub fn is_saved_track_id(&self, id: Option<&str>) -> bool {
        if !*self.is_done.read().unwrap() {
            return false;
        }
        if let Some(id) = id
            && let Some(&saved) = self.pending_saves.read().unwrap().get(id)
        {
            return saved;
        }

        // Not `self.tracks`: Liked Songs holds it while drawing, and taking it again would wait
        // behind any change to it that's waiting for that.
        id.is_some_and(|id| self.saved_ids.read().unwrap().tracks.contains(id))
    }

    /// Bring the IDs [Self::is_saved_track] looks up in line with [Self::tracks].
    pub(crate) fn index_tracks(&self) {
        let ids = self
            .tracks
            .read()
            .unwrap()
            .iter()
            .filter_map(|t| t.id.clone())
            .collect();
        self.saved_ids.write().unwrap().tracks = ids;
    }

    /// Bring the IDs [Self::is_saved_playlist] looks up in line with [Self::playlists].
    pub(crate) fn index_playlists(&self) {
        let ids = self
            .playlists
            .read()
            .unwrap()
            .iter()
            .map(|p| p.id.clone())
            .collect();
        self.saved_ids.write().unwrap().playlists = ids;
    }

    /// Save `track` if it isn't saved, else remove it, without waiting for Spotify: the new state
    /// shows straight away (see [Self::is_saved_track]) while the request runs on another thread.
    pub fn toggle_saved_track_in_background(&self, track: &Track) {
        let Some(id) = track.id.clone() else {
            return;
        };
        if !*self.is_done.read().unwrap() {
            return;
        }
        let save = !self.is_saved_track_id(Some(&id));
        self.pending_saves.write().unwrap().insert(id.clone(), save);
        self.trigger_redraw();

        let library = self.clone();
        let track = track.clone();
        thread::spawn(move || {
            if save {
                library.save_tracks(&[&track]);
            } else {
                library.unsave_tracks(&[&track]);
            }
            library.pending_saves.write().unwrap().remove(&id);
            library.trigger_redraw();
        });
    }

    /// Save `tracks` to the user's library.
    pub fn save_tracks(&self, tracks: &[&Track]) {
        // Local files aren't on Spotify to be liked.
        let tracks: Vec<&Track> = tracks.iter().copied().filter(|t| !t.is_local).collect();
        if tracks.is_empty() || !*self.is_done.read().unwrap() {
            return;
        }

        let save_tracks_result = self
            .spotify
            .api
            .current_user_saved_tracks_add(tracks.iter().filter_map(|t| t.id.as_deref()).collect());

        if save_tracks_result.is_err() {
            return;
        }

        let mut added = Vec::new();
        {
            let mut store = self.tracks.write().unwrap();
            for track in &tracks {
                if store.iter().any(|t| t.id == track.id) {
                    continue;
                }

                store.insert(added.len(), (*track).clone());
                added.push(*track);
            }
        }
        self.index_tracks();

        // Just the artists of these tracks change. The artists cache isn't saved: what it holds
        // about saved tracks is worked out from the tracks again on every start.
        add_artist_tracks(&mut self.artists.write().unwrap(), &added);

        self.save_cache(
            &config::cache_path(CACHE_TRACKS),
            &self.tracks.read().unwrap(),
        );
    }

    /// Remove `tracks` from the user's library.
    pub fn unsave_tracks(&self, tracks: &[&Track]) {
        if !*self.is_done.read().unwrap() {
            return;
        }

        if self
            .spotify
            .api
            .current_user_saved_tracks_delete(
                tracks.iter().filter_map(|t| t.id.as_deref()).collect(),
            )
            .is_err()
        {
            return;
        }

        self.tracks
            .write()
            .unwrap()
            .retain(|t| !tracks.iter().any(|tt| t.id == tt.id));
        self.index_tracks();

        // As in [Self::save_tracks].
        remove_artist_tracks(&mut self.artists.write().unwrap(), tracks);

        self.save_cache(
            &config::cache_path(CACHE_TRACKS),
            &self.tracks.read().unwrap(),
        );
    }

    /// Check whether `album` is saved to the user's library.
    pub fn is_saved_album(&self, album: &Album) -> bool {
        if !*self.is_done.read().unwrap() {
            return false;
        }

        let albums = self.albums.read().unwrap();
        albums.iter().any(|a| a.id == album.id)
    }

    /// Save `album` to the user's library.
    pub fn save_album(&self, album: &Album) {
        if !*self.is_done.read().unwrap() {
            return;
        }

        if let Some(ref album_id) = album.id
            && self
                .spotify
                .api
                .current_user_saved_albums_add(vec![album_id.as_str()])
                .is_err()
        {
            return;
        }

        {
            let mut store = self.albums.write().unwrap();
            if !store.iter().any(|a| a.id == album.id) {
                store.insert(0, album.clone());
                sort_albums(&mut store);
            }
        }

        self.save_cache(
            &config::cache_path(CACHE_ALBUMS),
            &self.albums.read().unwrap(),
        );
    }

    /// Remove `album` from the user's library.
    pub fn unsave_album(&self, album: &Album) {
        if !*self.is_done.read().unwrap() {
            return;
        }

        if let Some(ref album_id) = album.id
            && self
                .spotify
                .api
                .current_user_saved_albums_delete(vec![album_id.as_str()])
                .is_err()
        {
            return;
        }

        {
            let mut store = self.albums.write().unwrap();
            *store = store.iter().filter(|a| a.id != album.id).cloned().collect();
        }

        self.save_cache(
            &config::cache_path(CACHE_ALBUMS),
            &self.albums.read().unwrap(),
        );
    }

    /// Check whether the user follows `artist`.
    pub fn is_followed_artist(&self, artist: &Artist) -> bool {
        if !*self.is_done.read().unwrap() {
            return false;
        }

        let artists = self.artists.read().unwrap();
        artists.iter().any(|a| a.id == artist.id && a.is_followed)
    }

    /// Follow `artist` as the logged in user.
    pub fn follow_artist(&self, artist: &Artist) {
        if !*self.is_done.read().unwrap() {
            return;
        }

        if let Some(ref artist_id) = artist.id
            && self
                .spotify
                .api
                .user_follow_artists(vec![artist_id.as_str()])
                .is_err()
        {
            return;
        }

        {
            let mut store = self.artists.write().unwrap();
            if let Some(i) = store.iter().position(|a| a.id == artist.id) {
                store[i].is_followed = true;
            } else {
                let mut artist = artist.clone();
                artist.is_followed = true;
                store.push(artist);
            }
        }

        self.populate_artists();

        self.save_cache(
            &config::cache_path(CACHE_ARTISTS),
            &self.artists.read().unwrap(),
        );
    }

    /// Unfollow `artist` as the logged in user.
    pub fn unfollow_artist(&self, artist: &Artist) {
        if !*self.is_done.read().unwrap() {
            return;
        }

        if let Some(ref artist_id) = artist.id
            && self
                .spotify
                .api
                .user_unfollow_artists(vec![artist_id.as_str()])
                .is_err()
        {
            return;
        }

        {
            let mut store = self.artists.write().unwrap();
            if let Some(i) = store.iter().position(|a| a.id == artist.id) {
                store[i].is_followed = false;
            }
        }

        self.populate_artists();

        self.save_cache(
            &config::cache_path(CACHE_ARTISTS),
            &self.artists.read().unwrap(),
        );
    }

    /// Check whether `playlist` is saved in the user's library.
    pub fn is_saved_playlist(&self, playlist: &Playlist) -> bool {
        if !*self.is_done.read().unwrap() {
            return false;
        }

        self.saved_ids
            .read()
            .unwrap()
            .playlists
            .contains(&playlist.id)
    }

    /// Check whether `playlist` is in the library but not created by the library's owner.
    pub fn is_followed_playlist(&self, playlist: &Playlist) -> bool {
        self.user_id
            .as_ref()
            .map(|id| id != &playlist.owner_id)
            .unwrap_or(false)
    }

    /// Add `playlist` to the user's library by following it as the logged in user.
    pub fn follow_playlist(&self, mut playlist: Playlist) {
        if !*self.is_done.read().unwrap() {
            return;
        }

        let follow_playlist_result = self.spotify.api.user_playlist_follow_playlist(&playlist.id);

        if follow_playlist_result.is_err() {
            return;
        }

        playlist.load_tracks(&self.spotify);

        {
            let mut store = self.playlists.write().unwrap();
            if !store.iter().any(|p| p.id == playlist.id) {
                store.insert(0, playlist);
            }
        }
        self.index_playlists();

        self.save_cache(
            &config::cache_path(CACHE_PLAYLISTS),
            &self.playlists.read().unwrap(),
        );
    }

    /// Check whether `show` is already in the user's library.
    pub fn is_saved_show(&self, show: &Show) -> bool {
        if !*self.is_done.read().unwrap() {
            return false;
        }

        let shows = self.shows.read().unwrap();
        shows.iter().any(|s| s.id == show.id)
    }

    /// Save the `show` to the user's library.
    pub fn save_show(&self, show: &Show) {
        if !*self.is_done.read().unwrap() {
            return;
        }

        if self.spotify.api.save_shows(&[show.id.as_str()]).is_ok() {
            {
                let mut store = self.shows.write().unwrap();
                if !store.iter().any(|s| s.id == show.id) {
                    store.insert(0, show.clone());
                }
            }
        }
    }

    /// Force redraw the user interface.
    pub fn trigger_redraw(&self) {
        self.ev.trigger();
    }
}

/// How [Library::populate_artists] orders artists: by name, ignoring a leading "The ".
fn artist_sort_name(artist: &Artist) -> &str {
    artist.name.strip_prefix("The ").unwrap_or(&artist.name)
}

/// [Library::populate_artists] after `added` were saved (newest first, before the tracks saved
/// earlier), without going through every saved track: they're put first in their artists' saved
/// tracks, and artists not in `artists` yet are added in order.
fn add_artist_tracks(artists: &mut Vec<Artist>, added: &[&Track]) {
    for track in added.iter().rev() {
        for (id, name) in track.artist_ids.iter().zip(&track.artists) {
            let artist = match artists.iter().position(|a| a.id.as_ref() == Some(id)) {
                Some(index) => &mut artists[index],
                None => {
                    let artist = Artist::new(id.clone(), name.clone());
                    let index = artists.partition_point(|a| {
                        artist_sort_name(a)
                            .partial_cmp(artist_sort_name(&artist))
                            .is_some_and(|order| order.is_le())
                    });
                    artists.insert(index, artist);
                    &mut artists[index]
                }
            };
            let saved = artist.tracks.get_or_insert_default();
            if !saved.iter().any(|t| t.id == track.id) {
                saved.insert(0, (*track).clone());
            }
        }
    }
}

/// [Library::populate_artists] after `removed` were unsaved: they're taken out of their artists'
/// saved tracks, and artists left with none that aren't followed are taken out of `artists`.
fn remove_artist_tracks(artists: &mut Vec<Artist>, removed: &[&Track]) {
    for artist in artists.iter_mut() {
        if let Some(saved) = &mut artist.tracks {
            saved.retain(|t| !removed.iter().any(|r| r.id == t.id));
        }
    }
    artists.retain(|a| a.is_followed || a.tracks.as_ref().is_some_and(|t| !t.is_empty()));
}

/// See [Library::saved_ids].
#[derive(Default)]
struct SavedIds {
    tracks: HashSet<String>,
    playlists: HashSet<String>,
}

/// New saves are listed first, so any change shows in the first page or the total.
fn tracks_unchanged(total: u32, first_page: &[Track], cached: &[Track]) -> bool {
    total as usize == cached.len()
        && first_page
            .iter()
            .zip(cached)
            .all(|(new, old)| new.id == old.id)
}

/// Whether the followed artists in the cache look the same as Spotify's: as many (`total`, if
/// known) and including those on the first page.
fn followed_unchanged(total: Option<u32>, first_page: &[Artist], cached: &[Artist]) -> bool {
    let followed: HashSet<&str> = cached
        .iter()
        .filter(|a| a.is_followed)
        .filter_map(|a| a.id.as_deref())
        .collect();
    total.is_some_and(|total| total as usize == followed.len())
        && first_page
            .iter()
            .all(|new| new.id.as_deref().is_some_and(|id| followed.contains(id)))
}

/// Albums saved in ttunes have no `added_at`, so they force a refetch.
fn albums_unchanged(total: u32, first_page: &[Album], cached: &[Album]) -> bool {
    total as usize == cached.len()
        && cached.iter().all(|a| a.added_at.is_some())
        && first_page
            .iter()
            .all(|new| cached.iter().any(|old| old.id == new.id))
}

/// Returns `true` if the order changed.
fn sort_albums(albums: &mut [Album]) -> bool {
    if albums.is_sorted_by_key(album_sort_key) {
        return false;
    }
    albums.sort_unstable_by_key(album_sort_key);
    true
}

fn album_sort_key(album: &Album) -> String {
    let album_artist = album.artists[0]
        .strip_prefix("The ")
        .unwrap_or(&album.artists[0]);
    let album_title = album.title.strip_prefix("The ").unwrap_or(&album.title);
    format!(
        "{}{}{}",
        album_artist.to_lowercase(),
        album.year,
        album_title.to_lowercase()
    )
}

#[cfg(test)]
mod tests {
    use super::{
        add_artist_tracks, albums_unchanged, followed_unchanged, remove_artist_tracks, sort_albums,
        tracks_unchanged,
    };
    use crate::model::album::Album;
    use crate::model::artist::Artist;
    use crate::model::track::Track;

    fn tracks(ids: &[&str]) -> Vec<Track> {
        ids.iter()
            .map(|id| Track {
                id: Some(id.to_string()),
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
                list_index: 0,
                is_local: false,
                is_playable: None,
            })
            .collect()
    }

    fn album(id: &str, artist: &str, title: &str, added_at: Option<&str>) -> Album {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "title": title,
            "artists": [artist],
            "artist_ids": [],
            "year": "2000",
            "added_at": added_at,
        }))
        .unwrap()
    }

    fn albums(ids: &[&str]) -> Vec<Album> {
        ids.iter()
            .map(|id| album(id, "artist", "title", Some("2026-01-01T00:00:00Z")))
            .collect()
    }

    #[test]
    fn tracks_with_same_total_and_first_page_are_unchanged() {
        assert!(tracks_unchanged(
            3,
            &tracks(&["c", "b"]),
            &tracks(&["c", "b", "a"])
        ));
    }

    #[test]
    fn track_removal_changes_the_total() {
        assert!(!tracks_unchanged(
            2,
            &tracks(&["c", "b"]),
            &tracks(&["c", "b", "a"])
        ));
    }

    #[test]
    fn track_addition_shows_in_the_first_page() {
        assert!(!tracks_unchanged(
            3,
            &tracks(&["d", "c"]),
            &tracks(&["c", "b", "a"])
        ));
    }

    #[test]
    fn track_saved_again_moves_to_the_top() {
        assert!(!tracks_unchanged(
            3,
            &tracks(&["a", "c"]),
            &tracks(&["c", "b", "a"])
        ));
    }

    #[test]
    fn albums_cached_in_another_order_are_unchanged() {
        assert!(albums_unchanged(
            3,
            &albums(&["c", "b"]),
            &albums(&["a", "b", "c"])
        ));
    }

    #[test]
    fn album_removal_changes_the_total() {
        assert!(!albums_unchanged(
            2,
            &albums(&["c", "b"]),
            &albums(&["a", "b", "c"])
        ));
    }

    #[test]
    fn album_addition_shows_in_the_first_page() {
        assert!(!albums_unchanged(
            3,
            &albums(&["d", "c"]),
            &albums(&["a", "b", "c"])
        ));
    }

    #[test]
    fn album_saved_in_ttunes_is_refreshed() {
        let mut cached = albums(&["a", "b", "c"]);
        cached[0].added_at = None;
        assert!(!albums_unchanged(3, &albums(&["c", "b"]), &cached));
    }

    #[test]
    fn albums_sort_ignores_case_and_leading_the() {
        let mut list = vec![
            album("1", "The Beatles", "Abbey Road", None),
            album("2", "abba", "Arrival", None),
            album("3", "Cream", "Disraeli Gears", None),
        ];
        assert!(sort_albums(&mut list));
        let ids: Vec<_> = list.iter().map(|a| a.id.clone().unwrap()).collect();
        assert_eq!(ids, ["2", "1", "3"]);
        assert!(!sort_albums(&mut list));
    }

    fn followed(ids: &[&str]) -> Vec<Artist> {
        ids.iter()
            .map(|id| Artist {
                is_followed: true,
                ..Artist::new(id.to_string(), id.to_string())
            })
            .collect()
    }

    #[test]
    fn followed_artists_unchanged_when_count_and_first_page_match() {
        let mut cached = followed(&["a", "b", "c"]);
        // Artists with saved tracks but not followed don't count.
        cached.push(Artist::new("d".into(), "d".into()));
        assert!(followed_unchanged(Some(3), &followed(&["b", "a"]), &cached));
        assert!(!followed_unchanged(
            Some(4),
            &followed(&["b", "a"]),
            &cached
        ));
        assert!(!followed_unchanged(
            Some(3),
            &followed(&["b", "x"]),
            &cached
        ));
        assert!(!followed_unchanged(None, &followed(&["b", "a"]), &cached));
    }

    #[test]
    fn liking_and_unliking_updates_just_those_artists() {
        let mut followed = Artist::new("f".into(), "The Followed".into());
        followed.is_followed = true;
        followed.tracks = Some(Vec::new());
        let mut artists = vec![followed];
        let mut song = tracks(&["s1"]).remove(0);
        song.artist_ids = vec!["f".into(), "n".into()];
        song.artists = vec!["The Followed".into(), "Abba".into()];
        add_artist_tracks(&mut artists, &[&song]);
        let names: Vec<_> = artists.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["Abba", "The Followed"]);
        assert!(
            artists
                .iter()
                .all(|a| a.tracks.as_ref().unwrap().len() == 1)
        );
        // Liked again: not twice.
        add_artist_tracks(&mut artists, &[&song]);
        assert_eq!(artists[0].tracks.as_ref().unwrap().len(), 1);

        remove_artist_tracks(&mut artists, &[&song]);
        let names: Vec<_> = artists.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["The Followed"]);
        assert!(artists[0].tracks.as_ref().unwrap().is_empty());
    }
}
