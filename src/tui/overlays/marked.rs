//! The menu for several marked songs (`o`, or a right click on one of them): what can be done to
//! all of them at once. Its entries run the same commands as the keys, which act on the marked
//! songs while there are any.

use crate::command::Command;
use crate::model::track::Track;
use crate::traits::ListItem;
use crate::tui::marks::songs;
use crate::tui::overlays::menu::{Entry, Menu};
use crate::tui::page::{Ctx, Outcome, Overlay};

/// The menu for `items`, the marked songs: `in_queue` for Up next, where they're queued already,
/// and with an entry `remove` for removing them, where they can be.
pub fn menu(
    items: &[Box<dyn ListItem>],
    in_queue: bool,
    remove: Option<&str>,
    ctx: &Ctx,
) -> Box<dyn Overlay> {
    let run = |command: Command| move |_: &Ctx| Outcome::Run(vec![command.clone()]);
    let mut entries = Vec::new();
    if !in_queue {
        entries.push(Entry::new("Play", run(Command::Play)));
        entries.push(Entry::new("Play next", run(Command::PlayNext)));
        entries.push(Entry::new("Add to queue", run(Command::Queue)));
    }
    // Local files aren't on Spotify, to be liked or added to playlists.
    let tracks: Vec<Track> = items
        .iter()
        .filter_map(|item| item.track())
        .filter(|track| !track.is_local)
        .collect();
    if !tracks.is_empty() {
        let saved = tracks
            .iter()
            .all(|track| ctx.library.is_saved_track_id(track.id.as_deref()));
        let label = if saved {
            "Remove from Liked Songs"
        } else {
            "Save to Liked Songs"
        };
        entries.push(Entry::new(label, run(Command::Save)));
        entries.push(Entry::new("Add to playlist…", run(Command::Add)));
    }
    if let Some(label) = remove {
        entries.push(Entry::new(label, run(Command::Delete)));
    }
    Box::new(Menu::new(songs(items.len()), entries))
}
