//! The menu for a track or episode.

use crate::model::playable::Playable;
use crate::queue::RadioSeed;
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
    // Local files aren't on Spotify, to be liked or added to playlists.
    if let Some(track) = item.track().filter(|track| !track.is_local) {
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
    if RadioSeed::of(item.as_ref()).is_some() {
        let i = item.clone();
        entries.push(Entry::new("Go to radio", move |ctx| {
            crate::tui::actions::radio(i.as_ref(), ctx)
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

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;
    use crate::model::album::Album;
    use crate::model::artist::Artist;
    use crate::model::episode::Episode;
    use crate::model::track::Track;

    /// The text of `item`'s menu, drawn.
    fn menu_text(item: Box<dyn ListItem>) -> String {
        let app = crate::tui::tests::app();
        let mut menu = context_menu(item, &app.ctx);
        let mut terminal = Terminal::new(TestBackend::new(60, 30)).unwrap();
        terminal
            .draw(|frame| {
                menu.draw(frame, frame.area(), &app.ctx);
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        buffer.content().iter().map(|cell| cell.symbol()).collect()
    }

    fn track(is_local: bool) -> Track {
        serde_json::from_value(serde_json::json!({
            "id": "song",
            "uri": "spotify:track:song",
            "title": "Song",
            "track_number": 1,
            "disc_number": 1,
            "duration": 180_000,
            "artists": ["Artist"],
            "artist_ids": ["artist"],
            "album": "Album",
            "album_id": "album",
            "album_artists": ["Artist"],
            "cover_url": null,
            "url": "",
            "added_at": null,
            "list_index": 0,
            "is_local": is_local,
            "is_playable": true,
        }))
        .unwrap()
    }

    #[test]
    fn songs_albums_and_artists_have_a_radio() {
        let album: Album = serde_json::from_value(serde_json::json!({
            "id": "album",
            "title": "Album",
            "artists": ["Artist"],
            "artist_ids": ["artist"],
            "year": "1999",
            "cover_url": null,
            "url": null,
            "tracks": null,
            "added_at": null,
        }))
        .unwrap();
        let artist = Artist {
            id: Some("artist".into()),
            name: "Artist".into(),
            url: None,
            tracks: None,
            is_followed: false,
        };
        for item in [
            track(false).as_listitem(),
            album.as_listitem(),
            artist.as_listitem(),
        ] {
            assert!(menu_text(item).contains("Go to radio"));
        }
    }

    #[test]
    fn podcasts_and_local_files_have_no_radio() {
        let episode = Episode {
            id: "episode".into(),
            uri: "spotify:episode:episode".into(),
            duration: 1_000,
            name: "Episode".into(),
            description: String::new(),
            release_date: String::new(),
            cover_url: None,
            added_at: None,
            list_index: 0,
        };
        let text = menu_text(episode.as_listitem());
        assert!(text.contains("Play next"), "{text}");
        assert!(!text.contains("Go to radio"));
        assert!(!menu_text(track(true).as_listitem()).contains("Go to radio"));
    }
}
