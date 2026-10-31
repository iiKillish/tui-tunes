use ratatui::Frame;
use ratatui::crossterm::event::MouseEvent;
use ratatui::layout::Rect;

use crate::command::Command;
use crate::model::track::Track;
use crate::tui::header::{HeaderAction, PageHeader};
use crate::tui::page::{Ctx, Outcome, Page};
use crate::tui::style::songs_and_length;
use crate::tui::widgets::list::ItemList;

/// The user's saved tracks, as a playlist-like page.
pub struct LikedPage {
    list: ItemList<Track>,
}

impl LikedPage {
    pub fn new(ctx: &Ctx) -> Self {
        Self {
            list: ItemList::new(ctx.library.tracks.clone())
                .numbered()
                .with_source("Liked Songs"),
        }
    }
}

impl Page for LikedPage {
    fn title(&self) -> String {
        "Liked Songs".into()
    }

    fn header(&self, ctx: &Ctx) -> Option<PageHeader> {
        let tracks = ctx.library.tracks.read().unwrap();
        Some(PageHeader {
            kind: "Playlist".into(),
            title: "Liked Songs".into(),
            details: vec![songs_and_length(tracks.iter().map(|t| t.duration))],
            cover_url: None,
            playable: true,
            liked: None,
        })
    }

    fn header_action(&mut self, action: HeaderAction, ctx: &Ctx) -> Outcome {
        ctx.queue.set_shuffle(action == HeaderAction::Shuffle);
        self.list.play_all(ctx);
        Outcome::Consumed
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
