//! Where things lead: the page an item opens, its context menu, the playlist picker. Kept in one
//! place so lists, cards and menus all open things the same way.

use crate::command::{Command, GotoMode};
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

/// Queue `items` (marked songs) in their order, after the current track if `next` (see
/// [QUEUEING]).
pub fn queue_items(mut items: Vec<Box<dyn ListItem>>, next: bool, ctx: &Ctx) {
    let queue = ctx.queue.clone();
    QUEUEING.run(move || {
        if next {
            // Each goes straight after the current track: the last one first.
            for item in items.iter_mut().rev() {
                item.play_next(&queue);
            }
        } else {
            for item in &mut items {
                item.queue(&queue);
            }
        }
        crate::tui::request_redraw();
    });
}

/// Replace the queue with `items` (marked songs) and play them from the first (see
/// [QUEUEING]).
pub fn play_items(mut items: Vec<Box<dyn ListItem>>, ctx: &Ctx) {
    let queue = ctx.queue.clone();
    QUEUEING.run(move || {
        queue.clear();
        for item in &mut items {
            item.queue(&queue);
        }
        if queue.len() > 0 {
            queue.play(0, true, false);
        }
        crate::tui::request_redraw();
    });
}

/// Save `tracks` (marked songs) to Liked Songs, or remove them if they're all there already, on
/// a thread of its own. Local files are left out: they aren't on Spotify.
pub fn save_tracks(tracks: Vec<Track>, ctx: &Ctx) -> Outcome {
    let tracks: Vec<Track> = tracks
        .into_iter()
        .filter(|track| !track.is_local && track.id.is_some())
        .collect();
    if tracks.is_empty() {
        return Outcome::Error("Local files can't be liked".into());
    }
    let saved = |track: &Track| ctx.library.is_saved_track_id(track.id.as_deref());
    let remove = tracks.iter().all(saved);
    let songs = crate::tui::marks::songs(tracks.len());
    let library = ctx.library.clone();
    std::thread::spawn(move || {
        let tracks: Vec<&Track> = tracks.iter().collect();
        if remove {
            library.unsave_tracks(&tracks);
        } else {
            library.save_tracks(&tracks);
        }
        library.trigger_redraw();
    });
    Outcome::Message(if remove {
        format!("Removed {songs} from Liked Songs")
    } else {
        format!("Saved {songs} to Liked Songs")
    })
}

/// Pick a playlist to add `tracks` (marked songs) to, leaving out local files.
pub fn add_tracks(tracks: Vec<Track>, ctx: &Ctx) -> Outcome {
    let tracks: Vec<Track> = tracks.into_iter().filter(|t| !t.is_local).collect();
    if tracks.is_empty() {
        return Outcome::Error("Local files can't be added to Spotify playlists".into());
    }
    pick_playlist_for(tracks, ctx)
}

/// The menu for marked songs (`o`, or a right click on one of them): `in_queue` for Up next,
/// where they're queued already, and with an entry `remove` saying how to remove them, where
/// they can be.
pub fn marked_menu(
    items: &[Box<dyn ListItem>],
    in_queue: bool,
    remove: Option<&str>,
    ctx: &Ctx,
) -> Box<dyn Overlay> {
    crate::tui::overlays::marked::menu(items, in_queue, remove, ctx)
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

/// Whether [item_command] handles `cmd`.
pub fn is_item_command(cmd: &Command) -> bool {
    #[cfg(feature = "share_clipboard")]
    if matches!(cmd, Command::Share(crate::command::TargetMode::Selected)) {
        return true;
    }
    matches!(
        cmd,
        Command::Save | Command::Add | Command::Goto(_) | Command::Radio
    )
}

/// What a list does with its selected `item` for `cmd`: save it (`s`), add it to a playlist,
/// go to its album or artist (`a`, `A`), start its radio or copy its link (`x`). [Outcome::Ignored] for other
/// commands.
pub fn item_command(item: &mut dyn ListItem, cmd: &Command, ctx: &Ctx) -> Outcome {
    match cmd {
        Command::Save => item.save(&ctx.library),
        Command::Add => {
            if let Some(track) = item.track() {
                return pick_playlist_for(vec![track], ctx);
            }
        }
        Command::Goto(GotoMode::Album) => {
            if let Some(album) = item.album() {
                return Outcome::Open(open_target(OpenTarget::Album(album), ctx));
            }
        }
        Command::Goto(GotoMode::Artist) => {
            if let Some(artists) = item.artists() {
                return go_to_artists(artists, ctx);
            }
        }
        Command::Radio => return crate::tui::actions::radio(item, ctx),
        #[cfg(feature = "share_clipboard")]
        Command::Share(crate::command::TargetMode::Selected) => {
            if let Some(url) = item.share_url() {
                let _ = crate::sharing::write_share(url);
                return Outcome::Message("Link copied".into());
            }
        }
        _ => return Outcome::Ignored,
    }
    Outcome::Consumed
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
