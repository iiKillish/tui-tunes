//! A menu over the page: a titled list of entries, picked with Enter or a click. The building
//! block for context menus, confirmations and pickers.

use ratatui::Frame;
use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use ratatui::widgets::{Block, BorderType, Borders, Clear};

use crate::command::{Command, MoveAmount, MoveMode};
use crate::tui::page::{Ctx, Outcome, Overlay};
use crate::tui::style::{SELECTION_MARKER, Styles, bold, fill, put, scroll_to, width};

/// What picking an entry does. Returning [Outcome::Consumed] closes the menu.
pub type Action = Box<dyn FnMut(&Ctx) -> Outcome>;

pub struct Entry {
    pub label: String,
    pub action: Action,
}

impl Entry {
    pub fn new(label: impl Into<String>, action: impl FnMut(&Ctx) -> Outcome + 'static) -> Self {
        Self {
            label: label.into(),
            action: Box::new(action),
        }
    }
}

pub struct Menu {
    title: String,
    entries: Vec<Entry>,
    selected: usize,
    first: usize,
    /// Where the entries were drawn last, for clicks.
    list_area: Rect,
    area: Rect,
}

impl Menu {
    pub fn new(title: impl Into<String>, entries: Vec<Entry>) -> Self {
        Self {
            title: title.into(),
            entries,
            selected: 0,
            first: 0,
            list_area: Rect::default(),
            area: Rect::default(),
        }
    }

    /// A Yes/No question; `on_yes` runs on Yes.
    pub fn confirm(question: impl Into<String>, on_yes: impl FnMut(&Ctx) -> Outcome + 'static) -> Self {
        Self::new(
            question,
            vec![
                Entry::new("Yes", on_yes),
                Entry::new("No", |_| Outcome::Close),
            ],
        )
    }

    fn pick(&mut self, ctx: &Ctx) -> Outcome {
        match self.entries.get_mut(self.selected) {
            Some(entry) => match (entry.action)(ctx) {
                Outcome::Consumed | Outcome::Ignored => Outcome::Close,
                other => other,
            },
            None => Outcome::Close,
        }
    }
}

impl Overlay for Menu {
    fn draw(&mut self, frame: &mut Frame, screen: Rect, ctx: &Ctx) -> Rect {
        let styles = Styles(&ctx.palette);
        let content_width = self
            .entries
            .iter()
            .map(|e| width(&e.label))
            .chain(std::iter::once(width(&self.title)))
            .max()
            .unwrap_or(10);
        let w = (content_width as u16 + 6).clamp(24, screen.width.saturating_sub(4).max(10));
        let h = (self.entries.len() as u16 + 2).min(screen.height.saturating_sub(4).max(3));
        let area = Rect::new(
            screen.x + (screen.width.saturating_sub(w)) / 2,
            screen.y + (screen.height.saturating_sub(h)) / 3,
            w,
            h,
        );
        self.area = area;
        frame.render_widget(Clear, area);
        let block = Block::new()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(styles.accent().bg(ctx.palette.statusbar_bg))
            .title(format!(" {} ", self.title))
            .title_style(bold(styles.accent().bg(ctx.palette.statusbar_bg)))
            .style(styles.panel());
        let inner = block.inner(area);
        frame.render_widget(block, area);
        self.list_area = inner;

        let rows = usize::from(inner.height);
        self.first = scroll_to(self.first, self.selected, rows);
        let buf = frame.buffer_mut();
        for (row, (i, entry)) in self.entries.iter().enumerate().skip(self.first).take(rows).enumerate() {
            let y = inner.y + row as u16;
            let style = if i == self.selected {
                styles.selected().bg(ctx.palette.highlight_inactive_bg)
            } else {
                styles.panel()
            };
            fill(buf, Rect::new(inner.x, y, inner.width, 1), style);
            if i == self.selected {
                put(buf, inner.x, y, SELECTION_MARKER, 1, styles.marker().bg(ctx.palette.highlight_inactive_bg));
            }
            put(buf, inner.x + 1, y, &entry.label, usize::from(inner.width.saturating_sub(2)), style);
        }
        area
    }

    fn command(&mut self, cmd: &Command, ctx: &Ctx) -> Outcome {
        let last = self.entries.len().saturating_sub(1);
        match cmd {
            Command::Back => return Outcome::Close,
            Command::Move(MoveMode::Up, amount) => {
                self.selected = match amount {
                    MoveAmount::Extreme => 0,
                    _ => self.selected.saturating_sub(1),
                };
            }
            Command::Move(MoveMode::Down, amount) => {
                self.selected = match amount {
                    MoveAmount::Extreme => last,
                    _ => (self.selected + 1).min(last),
                };
            }
            Command::Play | Command::Open(_) => return self.pick(ctx),
            _ => return Outcome::Ignored,
        }
        Outcome::Consumed
    }

    fn mouse(&mut self, event: MouseEvent, ctx: &Ctx) -> Outcome {
        let inside = |area: Rect| area.contains((event.column, event.row).into());
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) if inside(self.list_area) => {
                self.selected = self.first + usize::from(event.row - self.list_area.y);
                if self.selected < self.entries.len() {
                    return self.pick(ctx);
                }
            }
            MouseEventKind::Down(_) if !inside(self.area) => return Outcome::Close,
            MouseEventKind::ScrollDown => {
                self.selected = (self.selected + 1).min(self.entries.len().saturating_sub(1));
            }
            MouseEventKind::ScrollUp => self.selected = self.selected.saturating_sub(1),
            _ => {}
        }
        Outcome::Consumed
    }
}
