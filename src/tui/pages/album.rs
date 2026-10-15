use std::sync::{Arc, RwLock};

use ratatui::Frame;
use ratatui::crossterm::event::MouseEvent;
use ratatui::layout::Rect;

use crate::command::Command;
use crate::model::album::Album;
use crate::model::track::Track;
use crate::tui::header::{HeaderAction, PageHeader};
use crate::tui::page::{Ctx, Outcome, Page};
use crate::tui::style::songs_and_length;
use crate::tui::widgets::list::ItemList;

/// An album: its tracks, numbered, under a header.
pub struct AlbumPage {
    album: Album,
    list: ItemList<Track>,
}

impl AlbumPage {
    pub fn new(album: Album, ctx: &Ctx) -> Self {
        let mut album = album;
        album.load_all_tracks(ctx.spotify.clone());
        let tracks = album.tracks.clone().unwrap_or_default();
        let list = ItemList::new(Arc::new(RwLock::new(tracks)))
            .numbered()
            .without_thumbnails()
            .with_source(&album.title);
        Self { album, list }
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
        self.list.draw(frame, area, ctx, focused);
    }

    fn command(&mut self, cmd: &Command, ctx: &Ctx) -> Outcome {
        self.list.command(cmd, ctx)
    }

    fn mouse(&mut self, event: MouseEvent, ctx: &Ctx) -> Outcome {
        self.list.mouse(event, ctx)
    }
}
