//! Where things lead: the page an item opens, its context menu, the playlist picker. Kept in one
//! place so lists, cards and menus all open things the same way.

use crate::model::artist::Artist;
use crate::model::playable::Playable;
use crate::model::track::Track;
use crate::traits::{ListItem, OpenTarget};
use crate::tui::overlays::menu::{Entry, Menu};
use crate::tui::page::{Ctx, Outcome, Overlay, Page};
use crate::tui::pages;

/// The page for an album, playlist, artist, show or category.
pub fn open_target(target: OpenTarget, ctx: &Ctx) -> Box<dyn Page> {
    match target {
        OpenTarget::Album(album) => Box::new(pages::album::AlbumPage::new(album, ctx)),
        OpenTarget::Playlist(playlist) => {
            Box::new(pages::playlist::PlaylistPage::new(playlist, ctx))
        }
        OpenTarget::Artist(artist) => Box::new(pages::artist::ArtistPage::new(artist, ctx)),
        OpenTarget::Show(show) => Box::new(pages::show::ShowPage::new_show(show, ctx)),
        OpenTarget::Category(category) => {
            Box::new(pages::category::CategoryPage::new_category(category, ctx))
        }
    }
}

/// Go to an artist's page; with several artists, ask which.
pub fn go_to_artists(artists: Vec<Artist>, ctx: &Ctx) -> Outcome {
    match artists.len() {
        0 => Outcome::Consumed,
        1 => Outcome::Open(open_target(
            OpenTarget::Artist(artists.into_iter().next().unwrap()),
            ctx,
        )),
        _ => Outcome::Overlay(Box::new(Menu::new(
            "Go to artist",
            artists
                .into_iter()
                .map(|artist| {
                    let label = artist.name.clone();
                    Entry::new(label, move |ctx| {
                        Outcome::Open(open_target(OpenTarget::Artist(artist.clone()), ctx))
                    })
                })
                .collect(),
        ))),
    }
}

/// The menu for a track or episode (`o`, or a right click).
pub fn context_menu(item: Box<dyn ListItem>, ctx: &Ctx) -> Box<dyn Overlay> {
    crate::tui::overlays::context::context_menu(item, ctx)
}

/// Pick one of the user's playlists (their own or collaborative) to add `tracks` to.
pub fn add_to_playlist(tracks: Vec<Track>, ctx: &Ctx) -> Box<dyn Overlay> {
    let user = ctx.library.user_id.clone().unwrap_or_default();
    let playlists: Vec<_> = ctx
        .library
        .playlists
        .read()
        .unwrap()
        .iter()
        .filter(|p| p.owner_id == user || p.collaborative)
        .cloned()
        .collect();
    let entries = playlists
        .into_iter()
        .map(|playlist| {
            let tracks = tracks.clone();
            Entry::new(playlist.name.clone(), move |ctx| {
                let mut playlist = playlist.clone();
                let playables: Vec<Playable> =
                    tracks.iter().cloned().map(Playable::Track).collect();
                playlist.append_tracks(&playables, &ctx.spotify, &ctx.library);
                Outcome::Message(format!("Added to {}", playlist.name))
            })
        })
        .collect();
    Box::new(Menu::new("Add to playlist", entries))
}
