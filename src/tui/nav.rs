//! Where things lead: the page an item opens, its context menu, the playlist picker. Kept in one
//! place so lists, cards and menus all open things the same way.

use crate::command::Command;
use crate::model::artist::Artist;
use crate::model::playable::Playable;
use crate::model::playlist::Playlist;
use crate::model::track::Track;
use crate::traits::{ListItem, OpenTarget};
use crate::tui::overlays::menu::{Entry, Menu};
use crate::tui::overlays::prompt::Prompt;
use crate::tui::page::{Ctx, Outcome, Overlay, Page};
use crate::tui::pages;

/// Where [queue_item] and [play_item] change the queue: one at a time, in order, on a thread of its
/// own, since an album's or playlist's tracks may have to be loaded first, which mustn't freeze
/// the interface.
static QUEUEING: crate::utils::InOrder = crate::utils::InOrder::new("queueing");

/// Queue `item`, after the current track if `next` (see [QUEUEING]).
pub fn queue_item(mut item: Box<dyn ListItem>, next: bool, ctx: &Ctx) {
    let queue = ctx.queue.clone();
    QUEUEING.run(move || {
        if next {
            item.play_next(&queue);
        } else {
            item.queue(&queue);
        }
        crate::tui::request_redraw();
    });
}

/// Replace the queue with `item` (an album, a playlist, an artist's top tracks…) and play it
/// (see [QUEUEING]).
pub fn play_item(mut item: Box<dyn ListItem>, ctx: &Ctx) {
    let queue = ctx.queue.clone();
    QUEUEING.run(move || {
        queue.clear();
        item.play(&queue);
        if let Some(target) = item.target() {
            pages::home::started(&target, &queue.queue.read().unwrap());
        }
        crate::tui::request_redraw();
    });
}

/// Whether the user may change `playlist`: their own, or a collaborative one.
pub fn can_edit(playlist: &Playlist, ctx: &Ctx) -> bool {
    playlist.collaborative || ctx.library.user_id.as_deref() == Some(playlist.owner_id.as_str())
}

/// Rename `playlist` to `name`. With no name, ask for one (starting from the current name);
/// submitting runs `rename` with it, so whoever shows the playlist sees the new name.
pub fn rename_playlist(playlist: &Playlist, name: &str, ctx: &Ctx) -> Outcome {
    if !can_edit(playlist, ctx) {
        return Outcome::Error("Only your own playlists can be renamed".into());
    }
    let name = name.trim();
    if name.is_empty() {
        let prompt = Prompt::new("Rename playlist", |name, _| {
            Outcome::Run(vec![Command::Rename(name.to_string())])
        })
        .with_text(playlist.name.clone());
        return Outcome::Overlay(Box::new(prompt));
    }
    match ctx.library.rename_playlist(&playlist.id, name) {
        Ok(()) => Outcome::Message(format!("Renamed to {name}")),
        Err(()) => Outcome::Error(format!("Couldn't rename {}", playlist.name)),
    }
}

/// The page for an album, playlist, artist, show, category or folder.
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
        OpenTarget::Folder(folder) => Box::new(pages::folder::FolderPage::new(folder)),
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

/// Pick a playlist to add `tracks` to (see [add_to_playlist]), unless they're local files, which
/// Spotify's playlists can't have added.
pub fn pick_playlist_for(tracks: Vec<Track>, ctx: &Ctx) -> Outcome {
    if tracks.iter().any(|track| track.is_local) {
        return Outcome::Error("Local files can't be added to Spotify playlists".into());
    }
    Outcome::Overlay(add_to_playlist(tracks, ctx))
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
    Box::new(Menu::new("Add to playlist", entries).filterable())
}
