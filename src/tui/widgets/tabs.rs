//! Tabs inside a page (the library's Playlists / Albums / Artists, an artist's Top tracks /
//! Albums, search results), drawn as plain words with the current one bold, in the accent and
//! underlined (design T1).

use ratatui::Frame;
use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use ratatui::style::Modifier;

use crate::command::{Command, MoveMode};
use crate::tui::header::{HeaderAction, PageHeader};
use crate::tui::page::{Ctx, Outcome, Page};
use crate::tui::pointer;
use crate::tui::style::{Styles, bold, put, width};

/// Draw tab labels at `area` (one row); returns where each was drawn, for clicks.
pub fn draw_tab_row(
    frame: &mut Frame,
    area: Rect,
    labels: &[String],
    current: usize,
    ctx: &Ctx,
) -> Vec<Rect> {
    let styles = Styles(&ctx.palette);
    let buf = frame.buffer_mut();
    let mut x = area.x + 2;
    let mut rects = Vec::new();
    for (i, label) in labels.iter().enumerate() {
        let style = if i == current {
            bold(styles.accent()).add_modifier(Modifier::UNDERLINED)
        } else {
            styles.secondary()
        };
        let room = usize::from(area.right().saturating_sub(x));
        let written = put(buf, x, area.y, label, room, style);
        let rect = Rect::new(x, area.y, written as u16, 1);
        pointer::clickable(rect);
        rects.push(rect);
        x += width(label) as u16 + 3;
    }
    rects
}

/// A page made of tabbed pages.
pub struct Tabbed {
    title: String,
    tabs: Vec<(String, Box<dyn Page>)>,
    current: usize,
    tab_rects: Vec<Rect>,
}

impl Tabbed {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            tabs: Vec::new(),
            current: 0,
            tab_rects: Vec::new(),
        }
    }

    pub fn tab(mut self, label: impl Into<String>, page: impl Page + 'static) -> Self {
        self.tabs.push((label.into(), Box::new(page)));
        self
    }

    pub fn add_tab(&mut self, label: impl Into<String>, page: Box<dyn Page>) {
        self.tabs.push((label.into(), page));
    }

    fn switch(&mut self, next: bool, ctx: &Ctx) {
        let len = self.tabs.len();
        if len == 0 {
            return;
        }
        self.current = if next {
            (self.current + 1).min(len - 1)
        } else {
            self.current.saturating_sub(1)
        };
        self.tabs[self.current].1.on_show(ctx);
    }
}

impl Page for Tabbed {
    fn title(&self) -> String {
        self.title.clone()
    }

    fn header(&self, ctx: &Ctx) -> Option<PageHeader> {
        self.tabs.get(self.current).and_then(|(_, p)| p.header(ctx))
    }

    fn header_action(&mut self, action: HeaderAction, ctx: &Ctx) -> Outcome {
        match self.tabs.get_mut(self.current) {
            Some((_, page)) => page.header_action(action, ctx),
            None => Outcome::Ignored,
        }
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect, ctx: &Ctx, focused: bool) {
        self.tab_rects.clear();
        if area.is_empty() {
            return;
        }
        // The tab row and a blank row under it; in very short areas only the tab row, and in a
        // single row only the page.
        let tab_rows = match area.height {
            1 => 0,
            2 => 1,
            _ => 2,
        };
        if tab_rows > 0 {
            let labels: Vec<String> = self.tabs.iter().map(|(l, _)| l.clone()).collect();
            self.tab_rects = draw_tab_row(
                frame,
                Rect::new(area.x, area.y, area.width, 1),
                &labels,
                self.current,
                ctx,
            );
        }
        if let Some((_, page)) = self.tabs.get_mut(self.current) {
            let inner = Rect::new(
                area.x,
                area.y + tab_rows,
                area.width,
                area.height - tab_rows,
            );
            page.draw(frame, inner, ctx, focused);
        }
    }

    fn command(&mut self, cmd: &Command, ctx: &Ctx) -> Outcome {
        let Some((_, page)) = self.tabs.get_mut(self.current) else {
            return Outcome::Ignored;
        };
        match cmd {
            Command::Tab(next) => {
                self.switch(*next, ctx);
                Outcome::Consumed
            }
            // Left/right switch tabs, unless the tab uses them itself (a grid of cards).
            Command::Move(mode @ (MoveMode::Left | MoveMode::Right), _)
                if !page.handles_horizontal_moves() =>
            {
                self.switch(matches!(mode, MoveMode::Right), ctx);
                Outcome::Consumed
            }
            _ => page.command(cmd, ctx),
        }
    }

    fn mouse(&mut self, event: MouseEvent, ctx: &Ctx) -> Outcome {
        if let MouseEventKind::Down(MouseButton::Left) = event.kind
            && let Some(i) = self
                .tab_rects
                .iter()
                .position(|r| r.contains((event.column, event.row).into()))
        {
            self.current = i;
            self.tabs[i].1.on_show(ctx);
            return Outcome::Consumed;
        }
        match self.tabs.get_mut(self.current) {
            Some((_, page)) => page.mouse(event, ctx),
            None => Outcome::Ignored,
        }
    }

    fn captures_text(&self) -> bool {
        self.tabs
            .get(self.current)
            .is_some_and(|(_, p)| p.captures_text())
    }

    fn key(&mut self, key: ratatui::crossterm::event::KeyEvent, ctx: &Ctx) -> Outcome {
        match self.tabs.get_mut(self.current) {
            Some((_, page)) => page.key(key, ctx),
            None => Outcome::Ignored,
        }
    }

    fn on_show(&mut self, ctx: &Ctx) {
        if let Some((_, page)) = self.tabs.get_mut(self.current) {
            page.on_show(ctx);
        }
    }
}
