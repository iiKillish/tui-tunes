//! The menu for a track or episode.

use crate::model::playable::Playable;
use crate::traits::{ListItem, OpenTarget};
use crate::tui::nav;
use crate::tui::overlays::menu::{Entry, Menu};
use crate::tui::page::{Ctx, Outcome, Overlay};

pub fn context_menu(item: Box<dyn ListItem>, ctx: &Ctx) -> Box<dyn Overlay> {
    let title = item.display_left(&ctx.library);
    let item: std::sync::Arc<dyn ListItem> = std::sync::Arc::from(item);
    let mut entries = Vec::new();

    let i = item.clone();
    entries.push(Entry::new("Play", move |ctx| {
        i.as_listitem().play(&ctx.queue);
        Outcome::Consumed
    }));
    let i = item.clone();
    entries.push(Entry::new("Play next", move |ctx| {
        nav::queue_item(i.as_listitem(), true, ctx);
        Outcome::Message("Playing next".into())
    }));
    let i = item.clone();
    entries.push(Entry::new("Add to queue", move |ctx| {
        nav::queue_item(i.as_listitem(), false, ctx);
        Outcome::Message("Added to the queue".into())
    }));
    if let Some(track) = item.track() {
        let saved = ctx.library.is_saved_track(&Playable::Track(track.clone()));
        let t = track.clone();
        entries.push(Entry::new(
            if saved {
                "Remove from Liked Songs"
            } else {
                "Save to Liked Songs"
            },
            move |ctx| {
                ctx.library.toggle_saved_track_in_background(&t);
                Outcome::Consumed
            },
        ));
        let t = track.clone();
        entries.push(Entry::new("Add to playlist…", move |ctx| {
            Outcome::Replace(nav::add_to_playlist(vec![t.clone()], ctx))
        }));
    }
    if let Some(album) = item.album() {
        entries.push(Entry::new("Go to album", move |ctx| {
            Outcome::Open(nav::open_target(OpenTarget::Album(album.clone()), ctx))
        }));
    }
    if let Some(artists) = item.artists().filter(|a| !a.is_empty()) {
        let label = if artists.len() == 1 {
            "Go to artist"
        } else {
            "Go to artists…"
        };
        entries.push(Entry::new(label, move |ctx| {
            match nav::go_to_artists(artists.clone(), ctx) {
                Outcome::Overlay(menu) => Outcome::Replace(menu),
                other => other,
            }
        }));
    }
    #[cfg(feature = "share_clipboard")]
    if let Some(url) = item.share_url() {
        entries.push(Entry::new("Copy link", move |_| {
            let _ = crate::sharing::write_share(url.clone());
            Outcome::Message("Link copied".into())
        }));
    }
    Box::new(Menu::new(title, entries))
}
