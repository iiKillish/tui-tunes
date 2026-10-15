//! The home screen, like Spotify's: rows of cards for what you've been listening to.
//!
//! * Jump back in: albums, playlists and artists played from recently.
//! * Recently played: tracks.
//! * Your top tracks and Your top artists, of the last four weeks.
//!
//! See [CardGrid] for how the cards work.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::{Duration, Instant};

use cursive::view::{View, ViewWrapper};
use cursive::{Cursive, Printer};
use log::error;

use crate::command::Command;
use crate::commands::CommandResult;
use crate::library::Library;
use crate::queue::Queue;
use crate::spotify::Spotify;
use crate::spotify_api::{PlayContext, RecentPlay};
use crate::traits::ViewExt;
use crate::ui::cards::{Card, CardGrid, CardLayout, Section};

/// Cards per row at most.
const MAX_CARDS: usize = 20;
/// The home screen is reloaded when shown again after this long.
const REFRESH_AFTER: Duration = Duration::from_secs(10 * 60);

/// Load the home screen's sections from Spotify. Slow (several requests): run in the background.
fn load(spotify: &Spotify, library: &Library) -> Vec<Section> {
    let api = &spotify.api;
    let plays = api.recently_played().unwrap_or_else(|()| {
        error!("couldn't load recently played tracks");
        Vec::new()
    });

    // Jump back in: what recent tracks were played from, or their album when that's unknown or
    // can't be loaded (Spotify doesn't let apps load its own mixes, like Daily Mix playlists).
    let mut loaded: HashMap<(PlayContext, String), Option<Card>> = HashMap::new();
    let mut shown = HashSet::new();
    let mut jump_back_in = Vec::new();
    for play in &plays {
        if jump_back_in.len() >= MAX_CARDS {
            break;
        }
        let context_card = play.context.as_ref().and_then(|(kind, id)| {
            loaded
                .entry((*kind, id.clone()))
                .or_insert_with(|| load_context(api, library, play, *kind, id))
                .clone()
                .map(|card| ((*kind, id.clone()), card))
        });
        let card = context_card.or_else(|| {
            let id = play.album.id.clone()?;
            Some(((PlayContext::Album, id), Card::album(play.album.clone())))
        });
        if let Some((key, card)) = card
            && shown.insert(key)
        {
            jump_back_in.push(card);
        }
    }

    let mut seen = HashSet::new();
    let recent_tracks = plays
        .into_iter()
        .filter(|play| seen.insert(play.track.id.clone()))
        .take(MAX_CARDS)
        .map(|play| Card::track(play.track))
        .collect();

    let top_tracks = api
        .top_tracks(MAX_CARDS as u32)
        .unwrap_or_default()
        .into_iter()
        .map(Card::track)
        .collect();
    let top_artists = api
        .top_artists(MAX_CARDS as u32)
        .unwrap_or_default()
        .into_iter()
        .map(|(artist, _)| Card::artist(artist))
        .collect();

    [
        ("Jump back in", jump_back_in),
        ("Recently played", recent_tracks),
        ("Your top tracks", top_tracks),
        ("Your top artists", top_artists),
    ]
    .into_iter()
    .filter(|(_, cards)| !cards.is_empty())
    .map(|(title, cards)| Section {
        title: title.into(),
        cards,
    })
    .collect()
}

/// A card for the album, playlist or artist `play` was played from, if it can be loaded.
fn load_context(
    api: &crate::spotify_api::WebApi,
    library: &Library,
    play: &RecentPlay,
    kind: PlayContext,
    id: &str,
) -> Option<Card> {
    match kind {
        PlayContext::Album if play.album.id.as_deref() == Some(id) => {
            Some(Card::album(play.album.clone()))
        }
        PlayContext::Album => api.album(id).ok().map(|a| Card::album((&a).into())),
        PlayContext::Playlist => library
            .playlists
            .read()
            .unwrap()
            .iter()
            .find(|p| p.id == id)
            .cloned()
            .or_else(|| api.playlist(id).ok().map(|p| (&p).into()))
            .map(Card::playlist),
        PlayContext::Artist => api.artist(id).ok().map(|a| Card::artist((&a).into())),
    }
}

pub struct HomeView {
    grid: CardGrid,
    library: Arc<Library>,
    spotify: Spotify,
    sections: Arc<RwLock<Option<Vec<Section>>>>,
    loaded_at: RwLock<Option<Instant>>,
}

impl HomeView {
    pub fn new(queue: Arc<Queue>, library: Arc<Library>) -> Self {
        let sections: Arc<RwLock<Option<Vec<Section>>>> = Arc::new(RwLock::new(None));
        let source = {
            let sections = sections.clone();
            Box::new(move || sections.read().unwrap().clone())
        };
        Self {
            spotify: queue.get_spotify(),
            grid: CardGrid::new(queue, library.clone(), CardLayout::Rows, source)
                .empty_message("Nothing here yet: play some music first"),
            library,
            sections,
            loaded_at: RwLock::new(None),
        }
    }

    /// Load the sections in the background if they haven't been, or are getting old.
    fn refresh_if_needed(&self) {
        let mut loaded_at = self.loaded_at.write().unwrap();
        if loaded_at.is_some_and(|at| at.elapsed() < REFRESH_AFTER) {
            return;
        }
        *loaded_at = Some(Instant::now());
        let sections = self.sections.clone();
        let library = self.library.clone();
        let spotify = self.spotify.clone();
        thread::spawn(move || {
            let loaded = load(&spotify, &library);
            *sections.write().unwrap() = Some(loaded);
            library.trigger_redraw();
        });
    }
}

impl ViewWrapper for HomeView {
    cursive::wrap_impl!(self.grid: CardGrid);

    fn wrap_draw(&self, printer: &Printer<'_, '_>) {
        self.refresh_if_needed();
        self.grid.draw(printer);
    }
}

impl ViewExt for HomeView {
    fn title(&self) -> String {
        "Home".into()
    }

    fn title_sub(&self) -> String {
        match &self.library.display_name {
            Some(name) => format!("Hi, {name}"),
            None => String::new(),
        }
    }

    fn on_command(&mut self, s: &mut Cursive, cmd: &Command) -> Result<CommandResult, String> {
        match self.grid.on_command(s, cmd)? {
            // At the edge of a row: nothing to switch to here.
            CommandResult::Ignored if matches!(cmd, Command::Move(..)) => {
                Ok(CommandResult::Consumed(None))
            }
            result => Ok(result),
        }
    }
}
