//! The Library screen: tabs in the order of the `library_tabs` setting. Playlists (with Liked
//! Songs first), albums and artists as grids of covers like Spotify's library; saved tracks,
//! podcasts and the browse categories as lists.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use strum::IntoEnumIterator;

use crate::config::LibraryTab;
use crate::library::Library;
use crate::model::category::Category;
use crate::model::show::Show;
use crate::model::track::Track;
use crate::tui::page::Ctx;
use crate::tui::pages::loading::LoadingList;
use crate::tui::widgets::cards::{Card, CardGrid, CardLayout, Section};
use crate::tui::widgets::list::ItemList;
use crate::tui::widgets::tabs::Tabbed;

pub fn new(ctx: &Ctx) -> Tabbed {
    let library = &ctx.library;
    let tabs = library
        .cfg
        .values()
        .library_tabs
        .clone()
        .unwrap_or_else(|| LibraryTab::iter().collect());
    let mut tabbed = Tabbed::new("Library");
    for tab in tabs {
        match tab {
            LibraryTab::Playlists => tabbed.add_tab(
                "Playlists",
                Box::new(grid("Playlists", library, |library| {
                    let playlists = library.playlists.read().unwrap();
                    if playlists.is_empty() && is_loading(library) {
                        return None;
                    }
                    let liked = library.tracks.read().unwrap().len();
                    Some(
                        std::iter::once(Card::liked_songs(liked))
                            .chain(playlists.iter().cloned().map(Card::playlist))
                            .collect(),
                    )
                })),
            ),
            LibraryTab::Albums => tabbed.add_tab(
                "Albums",
                Box::new(grid("Albums", library, |library| {
                    let albums = library.albums.read().unwrap();
                    (!albums.is_empty() || !is_loading(library))
                        .then(|| albums.iter().cloned().map(Card::album).collect())
                })),
            ),
            LibraryTab::Artists => tabbed.add_tab(
                "Artists",
                Box::new(grid("Artists", library, |library| {
                    let artists = library.artists.read().unwrap();
                    (!artists.is_empty() || !is_loading(library))
                        .then(|| artists.iter().cloned().map(Card::artist).collect())
                })),
            ),
            LibraryTab::Tracks => {
                let list = ItemList::new(library.tracks.clone()).with_source("Liked Songs");
                let library = library.clone();
                tabbed.add_tab(
                    "Tracks",
                    Box::new(
                        LoadingList::<Track>::new("Tracks", list, move || is_loading(&library))
                            .empty_message("No liked songs yet"),
                    ),
                );
            }
            LibraryTab::Podcasts => {
                let list = ItemList::new(library.shows.clone());
                let library = library.clone();
                tabbed.add_tab(
                    "Podcasts",
                    Box::new(
                        LoadingList::<Show>::new("Podcasts", list, move || is_loading(&library))
                            .empty_message("No saved podcasts yet"),
                    ),
                );
            }
            LibraryTab::Browse => tabbed.add_tab("Browse", Box::new(browse())),
        }
    }
    tabbed
}

/// Whether the library is still loading (only the first time, before there's a cache).
fn is_loading(library: &Library) -> bool {
    !*library.is_done.read().unwrap()
}

/// A grid of the cards `cards` makes from the library (`None` while loading), kept up to date as
/// the library changes.
fn grid(
    title: &str,
    library: &Arc<Library>,
    cards: impl Fn(&Library) -> Option<Vec<Card>> + 'static,
) -> CardGrid {
    let library = library.clone();
    let source = Box::new(move || cards(&library).map(|cards| vec![Section::new("", cards)]));
    CardGrid::new(title, CardLayout::Grid, source)
}

/// Spotify's browse categories, loaded the first time the tab is shown.
fn browse() -> LoadingList<Category> {
    let list = ItemList::new(Arc::default());
    let content = list.content().clone();
    let pagination = list.pagination().clone();
    let loading = Arc::new(AtomicBool::new(true));
    let still_loading = loading.clone();
    LoadingList::new("Browse", list, move || still_loading.load(Ordering::Relaxed))
        .empty_message("No categories")
        .load_when_shown(move |ctx| {
            let api = ctx.spotify.api.clone();
            let events = ctx.events.clone();
            std::thread::spawn(move || {
                let categories = api.categories();
                let items = categories.items.read().unwrap().clone();
                let loaded = items.len();
                content.write().unwrap().extend(items);
                if !categories.at_end() {
                    let total = categories.total as usize;
                    pagination.set(
                        loaded,
                        total,
                        Box::new(move |content| {
                            if let Some(items) = categories.next() {
                                content.write().unwrap().extend(items);
                            }
                        }),
                    );
                }
                loading.store(false, Ordering::Relaxed);
                events.trigger();
            });
        })
}
