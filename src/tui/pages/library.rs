//! The Library screen: playlists (with Liked Songs), albums, artists, saved tracks and podcasts.
//! Lists for now; cover grids come next.

use crate::model::track::Track;
use crate::tui::page::Ctx;
use crate::tui::pages::ListPage;
use crate::tui::widgets::list::ItemList;
use crate::tui::widgets::tabs::Tabbed;

pub fn new(ctx: &Ctx) -> Tabbed {
    let library = &ctx.library;
    Tabbed::new("Library")
        .tab(
            "Playlists",
            ListPage::new("Playlists", ItemList::new(library.playlists.clone())),
        )
        .tab(
            "Albums",
            ListPage::new("Albums", ItemList::new(library.albums.clone())),
        )
        .tab(
            "Artists",
            ListPage::new("Artists", ItemList::new(library.artists.clone())),
        )
        .tab(
            "Liked Songs",
            ListPage::<Track>::new(
                "Liked Songs",
                ItemList::new(library.tracks.clone()).with_source("Liked Songs"),
            ),
        )
        .tab(
            "Podcasts",
            ListPage::new("Podcasts", ItemList::new(library.shows.clone())),
        )
}
