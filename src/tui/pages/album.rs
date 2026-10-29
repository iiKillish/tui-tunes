use std::sync::{Arc, Mutex, RwLock};

use ratatui::Frame;
use ratatui::crossterm::event::MouseEvent;
use ratatui::layout::Rect;

use crate::command::Command;
use crate::model::album::Album;
use crate::model::track::Track;
use crate::traits::OpenTarget;
use crate::tui::header::{HeaderAction, PageHeader};
use crate::tui::page::{Ctx, Outcome, Page};
use crate::tui::style::{Styles, put, songs_and_length};
use crate::tui::widgets::list::ItemList;

/// An album: its tracks, numbered, under a header.
/// Tracks not already known are loaded in the background, with "Loading…" shown until they're
/// there.
pub struct AlbumPage {
    album: Album,
    list: ItemList<Track>,
    /// The album with all its tracks once it's been loaded in the background, until it's shown.
    loaded: Arc<Mutex<Option<Album>>>,
}

impl AlbumPage {
    pub fn new(mut album: Album, ctx: &Ctx) -> Self {
        if album.id.is_none() {
            // Nothing to load it with.
            album.tracks.get_or_insert_default();
        }
        let loaded = Arc::new(Mutex::new(None));
        if album.id.is_some() && !album.has_all_tracks() {
            let mut loading = album.without_tracks();
            let had_tracks = album.tracks.as_ref().is_some_and(|t| !t.is_empty());
            let spotify = ctx.spotify.clone();
            let loaded = loaded.clone();
            std::thread::spawn(move || {
                loading.load_all_tracks(spotify);
                // If none could be loaded, keep the ones there were.
                let tracks = loading.tracks.get_or_insert_default();
                if !tracks.is_empty() || !had_tracks {
                    *loaded.lock().unwrap() = Some(loading);
                    crate::tui::request_redraw();
                }
            });
        }
        Self {
            list: Self::list(&album),
            album,
            loaded,
        }
    }

    fn list(album: &Album) -> ItemList<Track> {
        let tracks = album.tracks.clone().unwrap_or_default();
        ItemList::new(Arc::new(RwLock::new(tracks)))
            .numbered()
            .without_thumbnails()
            .without_album()
            .with_source(&album.title)
            .played_from(OpenTarget::Album(album.without_tracks()))
    }

    /// Show the album loaded in the background if it's arrived since the last look.
    fn take_loaded(&mut self) {
        let Some(album) = self.loaded.lock().unwrap().take() else {
            return;
        };
        let selected = self.list.selected();
        self.album = album;
        self.list = Self::list(&self.album);
        self.list.select(selected);
        // The header was drawn before the album arrived.
        crate::tui::request_redraw();
    }
}

impl Page for AlbumPage {
    fn title(&self) -> String {
        self.album.title.clone()
    }

    fn header(&self, ctx: &Ctx) -> Option<PageHeader> {
        let mut details = vec![self.album.artists.join(", ")];
        if !self.album.year.is_empty() {
            details.push(self.album.year.clone());
        }
        if let Some(tracks) = &self.album.tracks {
            details.push(songs_and_length(tracks.iter().map(|t| t.duration)));
        }
        Some(PageHeader {
            kind: "Album".into(),
            title: self.album.title.clone(),
            details,
            cover_url: self.album.cover_url.clone(),
            playable: true,
            liked: Some(ctx.library.is_saved_album(&self.album)),
        })
    }

    fn header_action(&mut self, action: HeaderAction, ctx: &Ctx) -> Outcome {
        match action {
            HeaderAction::Play | HeaderAction::Shuffle => {
                ctx.queue.set_shuffle(action == HeaderAction::Shuffle);
                self.list.select(0);
                self.list.command(&Command::Play, ctx)
            }
            HeaderAction::Like => {
                if ctx.library.is_saved_album(&self.album) {
                    ctx.library.unsave_album(&self.album);
                } else {
                    ctx.library.save_album(&self.album);
                }
                Outcome::Consumed
            }
        }
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect, ctx: &Ctx, focused: bool) {
        self.take_loaded();
        if self.album.tracks.is_none() {
            put(
                frame.buffer_mut(),
                area.x + 2,
                area.y,
                "Loading…",
                usize::from(area.width.saturating_sub(2)),
                Styles(&ctx.palette).secondary(),
            );
            return;
        }
        self.list.draw(frame, area, ctx, focused);
    }

    fn command(&mut self, cmd: &Command, ctx: &Ctx) -> Outcome {
        self.take_loaded();
        self.list.command(cmd, ctx)
    }

    fn mouse(&mut self, event: MouseEvent, ctx: &Ctx) -> Outcome {
        self.list.mouse(event, ctx)
    }
}
