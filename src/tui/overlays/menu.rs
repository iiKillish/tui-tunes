//! A menu over the page: a titled list of entries, picked with Enter or a click. The building
//! block for context menus, confirmations and pickers. A filterable menu narrows its entries to
//! the ones matching what's typed.

use ratatui::Frame;
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Rect;
use ratatui::widgets::{Block, BorderType, Borders, Clear};

use crate::command::{Command, MoveAmount, MoveMode};
use crate::tui::page::{Ctx, Outcome, Overlay};
use crate::tui::pointer;
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
    /// What's been typed, for a filterable menu; `None` if the menu can't be filtered.
    filter: Option<String>,
    /// The entries shown, as indices into `entries`: those matching the filter.
    visible: Vec<usize>,
    /// Index into `visible`.
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
            visible: (0..entries.len()).collect(),
            entries,
            filter: None,
            selected: 0,
            first: 0,
            list_area: Rect::default(),
            area: Rect::default(),
        }
    }

    /// A Yes/No question; `on_yes` runs on Yes.
    pub fn confirm(
        question: impl Into<String>,
        on_yes: impl FnMut(&Ctx) -> Outcome + 'static,
    ) -> Self {
        Self::new(
            question,
            vec![
                Entry::new("Yes", on_yes),
                Entry::new("No", |_| Outcome::Close),
            ],
        )
    }

    /// Let typing filter the entries, for menus too long to scroll through.
    pub fn filterable(mut self) -> Self {
        self.filter = Some(String::new());
        self
    }

    /// Show only the entries whose label contains the filter, ignoring case.
    fn apply_filter(&mut self) {
        let query = self.filter.as_deref().unwrap_or_default().to_lowercase();
        self.visible = (0..self.entries.len())
            .filter(|&i| self.entries[i].label.to_lowercase().contains(&query))
            .collect();
        self.selected = 0;
        self.first = 0;
    }

    fn pick(&mut self, ctx: &Ctx) -> Outcome {
        let entry = self.visible.get(self.selected).copied();
        match entry.and_then(|i| self.entries.get_mut(i)) {
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
        // A margin around it where there's room; never larger than the screen.
        let w = (content_width as u16 + 6)
            .max(24)
            .min(screen.width.saturating_sub(4).max(10))
            .min(screen.width);
        let filter_rows = u16::from(self.filter.is_some());
        let h = (self.entries.len() as u16 + 2 + filter_rows)
            .min(screen.height.saturating_sub(4).max(3))
            .min(screen.height);
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
        let mut inner = block.inner(area);
        frame.render_widget(block, area);
        if let Some(filter) = &self.filter
            && inner.height > 1
        {
            let field = Rect::new(inner.x, inner.y, inner.width, 1);
            pointer::text(field);
            let field_style = styles.panel().bg(ctx.palette.highlight_inactive_bg);
            fill(frame.buffer_mut(), field, field_style);
            let room = usize::from(field.width.saturating_sub(2));
            let (text, style) = if filter.is_empty() {
                ("Type to filter", field_style.fg(ctx.palette.secondary))
            } else {
                (filter.as_str(), field_style)
            };
            put(frame.buffer_mut(), field.x + 1, field.y, text, room, style);
            let typed = width(filter).min(room) as u16;
            frame.set_cursor_position((field.x + 1 + typed, field.y));
            inner.y += 1;
            inner.height -= 1;
        }
        self.list_area = inner;

        let rows = usize::from(inner.height);
        self.first = scroll_to(self.first, self.selected, rows);
        let buf = frame.buffer_mut();
        if self.visible.is_empty() && rows > 0 {
            put(
                buf,
                inner.x + 1,
                inner.y,
                "No matches",
                usize::from(inner.width.saturating_sub(2)),
                styles.panel().fg(ctx.palette.secondary),
            );
        }
        for (row, (i, entry)) in self
            .visible
            .iter()
            .map(|&index| &self.entries[index])
            .enumerate()
            .skip(self.first)
            .take(rows)
            .enumerate()
        {
            let y = inner.y + row as u16;
            let style = if i == self.selected {
                styles.selected().bg(ctx.palette.highlight_inactive_bg)
            } else {
                styles.panel()
            };
            fill(buf, Rect::new(inner.x, y, inner.width, 1), style);
            pointer::clickable(Rect::new(inner.x, y, inner.width, 1));
            if i == self.selected {
                put(
                    buf,
                    inner.x,
                    y,
                    SELECTION_MARKER,
                    1,
                    styles.marker().bg(ctx.palette.highlight_inactive_bg),
                );
            }
            put(
                buf,
                inner.x + 1,
                y,
                &entry.label,
                usize::from(inner.width.saturating_sub(2)),
                style,
            );
        }
        area
    }

    fn command(&mut self, cmd: &Command, ctx: &Ctx) -> Outcome {
        let last = self.visible.len().saturating_sub(1);
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

    fn captures_text(&self) -> bool {
        self.filter.is_some()
    }

    /// Typing edits the filter; other keys go on to the key bindings (arrows, Enter, Esc).
    fn key(&mut self, key: KeyEvent, _ctx: &Ctx) -> Outcome {
        let Some(filter) = self.filter.as_mut() else {
            return Outcome::Ignored;
        };
        match key.code {
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => filter.push(c),
            KeyCode::Backspace => {
                filter.pop();
            }
            // Esc clears the filter first, then closes the menu.
            KeyCode::Esc if !filter.is_empty() => filter.clear(),
            _ => return Outcome::Ignored,
        }
        self.apply_filter();
        Outcome::Consumed
    }

    fn mouse(&mut self, event: MouseEvent, ctx: &Ctx) -> Outcome {
        let inside = |area: Rect| area.contains((event.column, event.row).into());
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) if inside(self.list_area) => {
                self.selected = self.first + usize::from(event.row - self.list_area.y);
                if self.selected < self.visible.len() {
                    return self.pick(ctx);
                }
            }
            MouseEventKind::Down(_) if !inside(self.area) => return Outcome::Close,
            MouseEventKind::ScrollDown => {
                self.selected = (self.selected + 1).min(self.visible.len().saturating_sub(1));
            }
            MouseEventKind::ScrollUp => self.selected = self.selected.saturating_sub(1),
            _ => {}
        }
        Outcome::Consumed
    }
}
