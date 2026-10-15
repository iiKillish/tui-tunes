//! Pages: the top-level screens and the pages opened on top of them.

pub mod album;
pub mod artist;
pub mod category;
pub mod home;
pub mod library;
pub mod liked;
pub mod loading;
pub mod lyrics;
pub mod playlist;
pub mod queue;
pub mod search;
pub mod search_results;
pub mod show;

use ratatui::Frame;
use ratatui::crossterm::event::MouseEvent;
use ratatui::layout::Rect;

use crate::command::Command;
use crate::traits::ListItem;
use crate::tui::header::PageHeader;
use crate::tui::page::{Ctx, Outcome, Page};
use crate::tui::widgets::list::ItemList;

/// A page that's a titled list, optionally with a header. The base for simple pages.
pub struct ListPage<I: ListItem + Clone> {
    pub title: String,
    pub list: ItemList<I>,
    pub header: Option<PageHeader>,
}

impl<I: ListItem + Clone> ListPage<I> {
    pub fn new(title: impl Into<String>, list: ItemList<I>) -> Self {
        Self {
            title: title.into(),
            list,
            header: None,
        }
    }

    pub fn with_header(mut self, header: PageHeader) -> Self {
        self.header = Some(header);
        self
    }
}

impl<I: ListItem + Clone> Page for ListPage<I> {
    fn title(&self) -> String {
        self.title.clone()
    }

    fn header(&self, _ctx: &Ctx) -> Option<PageHeader> {
        self.header.clone()
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

/// A page that only shows a message (for screens that aren't built yet).
pub struct MessagePage {
    pub title: String,
    pub message: String,
}

impl Page for MessagePage {
    fn title(&self) -> String {
        self.title.clone()
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect, ctx: &Ctx, _focused: bool) {
        let styles = crate::tui::style::Styles(&ctx.palette);
        crate::tui::style::put(
            frame.buffer_mut(),
            area.x + 2,
            area.y + 1,
            &self.message,
            usize::from(area.width.saturating_sub(4)),
            styles.secondary(),
        );
    }

    fn command(&mut self, _cmd: &Command, _ctx: &Ctx) -> Outcome {
        Outcome::Ignored
    }
}
