//! The key list (`?`): every binding, in groups, saying what it does in words. Typing filters it,
//! Enter (or a click) runs the selected binding.

use ratatui::Frame;
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Rect;
use ratatui::widgets::{Block, BorderType, Borders, Clear};

use crate::command::{Command, MoveAmount, MoveMode};
use crate::tui::hints::{self, Action, Group};
use crate::tui::page::{Ctx, Outcome, Overlay};
use crate::tui::pointer;
use crate::tui::style::{SELECTION_MARKER, Styles, bold, fill, put, scroll_to, width};

/// A row of the list: a group's title, or an entry (by index).
#[derive(Clone, Copy, PartialEq)]
enum Row {
    Title(Group),
    Blank,
    Entry(usize),
}

pub struct Help {
    entries: Vec<Action>,
    filter: String,
    /// What's shown, for the filter.
    rows: Vec<Row>,
    /// The selected entry, as an index into `rows`.
    selected: usize,
    first: usize,
    list_area: Rect,
    area: Rect,
}

impl Help {
    /// The key list for the bindings in `bindings` (key name, commands).
    pub fn new(bindings: &[(String, Vec<Command>)]) -> Self {
        let entries = hints::actions(bindings);
        let mut help = Self {
            entries,
            filter: String::new(),
            rows: Vec::new(),
            selected: 0,
            first: 0,
            list_area: Rect::default(),
            area: Rect::default(),
        };
        help.apply_filter();
        help
    }

    /// Show the entries whose keys or words contain the filter, under their groups' titles.
    fn apply_filter(&mut self) {
        let query = self.filter.to_lowercase();
        self.rows.clear();
        let mut group = None;
        for (i, entry) in self.entries.iter().enumerate() {
            let text = format!("{} {}", entry.keys_label(), entry.label).to_lowercase();
            if !text.contains(&query) {
                continue;
            }
            if group != Some(entry.group) {
                if group.is_some() {
                    self.rows.push(Row::Blank);
                }
                self.rows.push(Row::Title(entry.group));
                group = Some(entry.group);
            }
            self.rows.push(Row::Entry(i));
        }
        self.first = 0;
        self.selected = self.next_entry(0, true).unwrap_or(0);
    }

    /// The first entry row from `from` on, going down (or up).
    fn next_entry(&self, from: usize, down: bool) -> Option<usize> {
        let is_entry = |i: &usize| matches!(self.rows.get(*i), Some(Row::Entry(_)));
        if down {
            (from..self.rows.len()).find(is_entry)
        } else {
            (0..=from.min(self.rows.len().saturating_sub(1)))
                .rev()
                .find(is_entry)
        }
    }

    fn step(&mut self, down: bool, by: usize) {
        for _ in 0..by {
            let next = if down {
                self.next_entry(self.selected + 1, true)
            } else {
                self.selected
                    .checked_sub(1)
                    .and_then(|i| self.next_entry(i, false))
            };
            match next {
                Some(next) => self.selected = next,
                None => break,
            }
        }
    }

    fn run_selected(&self) -> Outcome {
        match self.rows.get(self.selected) {
            Some(Row::Entry(i)) => Outcome::Run(self.entries[*i].commands.clone()),
            _ => Outcome::Close,
        }
    }
}

/// Width of the keys column.
const KEYS_WIDTH: u16 = 20;

impl Overlay for Help {
    fn draw(&mut self, frame: &mut Frame, screen: Rect, ctx: &Ctx) -> Rect {
        let styles = Styles(&ctx.palette);
        let w = screen
            .width
            .saturating_sub(4)
            .clamp(10, 78)
            .min(screen.width);
        let h = screen.height.saturating_sub(2).max(3).min(screen.height);
        let area = Rect::new(
            screen.x + screen.width.saturating_sub(w) / 2,
            screen.y + screen.height.saturating_sub(h) / 2,
            w,
            h,
        );
        self.area = area;
        frame.render_widget(Clear, area);
        let panel = styles.panel();
        let block = Block::new()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(styles.accent().bg(ctx.palette.statusbar_bg))
            .title(" Keys ")
            .title_style(bold(styles.accent().bg(ctx.palette.statusbar_bg)))
            .style(panel);
        let mut inner = block.inner(area);
        frame.render_widget(block, area);
        let room = usize::from(inner.width.saturating_sub(2));
        let buf = frame.buffer_mut();

        // The filter, then a tip.
        if inner.height > 2 {
            let field = Rect::new(inner.x, inner.y, inner.width, 1);
            pointer::text(field);
            let field_style = panel.bg(ctx.palette.highlight_inactive_bg);
            fill(buf, field, field_style);
            let (text, style) = if self.filter.is_empty() {
                (
                    "Type to find a key or what it does",
                    field_style.fg(ctx.palette.secondary),
                )
            } else {
                (self.filter.as_str(), field_style)
            };
            put(buf, field.x + 1, field.y, text, room, style);
            let typed = width(&self.filter).min(room) as u16;
            frame.set_cursor_position((field.x + 1 + typed, field.y));
            let hints_key = self
                .entries
                .iter()
                .find(|e| matches!(e.commands.as_slice(), [Command::Hints]));
            let tip = match hints_key {
                Some(entry) => format!(
                    "Enter runs a key · hold Ctrl+Shift or press {} to see keys on screen",
                    entry.keys_label()
                ),
                None => "Enter runs a key · hold Ctrl+Shift to see keys on screen".into(),
            };
            put(
                frame.buffer_mut(),
                inner.x + 1,
                inner.y + 1,
                &tip,
                room,
                panel.fg(ctx.palette.secondary),
            );
            inner.y += 3;
            inner.height = inner.height.saturating_sub(3);
        }
        self.list_area = inner;

        let buf = frame.buffer_mut();
        let rows = usize::from(inner.height);
        self.first = scroll_to(self.first, self.selected, rows);
        // Keep the titles above the selected row in view, up to the top for the first row.
        let mut top = self.selected;
        while top > 0 && !matches!(self.rows.get(top - 1), Some(Row::Entry(_))) {
            top -= 1;
        }
        if top < self.first && self.selected - top < rows {
            self.first = top;
        }
        if self.rows.is_empty() && rows > 0 {
            put(
                buf,
                inner.x + 1,
                inner.y,
                "No keys match",
                room,
                panel.fg(ctx.palette.secondary),
            );
        }
        for (i, row) in self.rows.iter().enumerate().skip(self.first).take(rows) {
            let y = inner.y + (i - self.first) as u16;
            let line = Rect::new(inner.x, y, inner.width, 1);
            match row {
                Row::Blank => {}
                Row::Title(group) => {
                    put(
                        buf,
                        inner.x + 1,
                        y,
                        group.title(),
                        room,
                        bold(styles.accent().bg(ctx.palette.statusbar_bg)),
                    );
                }
                Row::Entry(index) => {
                    let entry = &self.entries[*index];
                    let selected = i == self.selected;
                    let style = if selected {
                        styles.selected().bg(ctx.palette.highlight_inactive_bg)
                    } else {
                        panel
                    };
                    fill(buf, line, style);
                    pointer::clickable(line);
                    if selected {
                        put(
                            buf,
                            inner.x,
                            y,
                            SELECTION_MARKER,
                            1,
                            styles.marker().bg(ctx.palette.highlight_inactive_bg),
                        );
                    }
                    // Keys right-aligned in their column, then what they do.
                    let keys = entry.keys_label();
                    let keys_room = usize::from(KEYS_WIDTH.min(inner.width / 2));
                    let keys_width = width(&keys).min(keys_room) as u16;
                    put(
                        buf,
                        inner.x + 1 + keys_room as u16 - keys_width,
                        y,
                        &keys,
                        keys_room,
                        bold(style.fg(ctx.palette.title)),
                    );
                    let x = inner.x + keys_room as u16 + 4;
                    put(
                        buf,
                        x,
                        y,
                        &entry.label,
                        usize::from(inner.right().saturating_sub(x + 1)),
                        style,
                    );
                }
            }
        }
        area
    }

    fn command(&mut self, cmd: &Command, _ctx: &Ctx) -> Outcome {
        let page = usize::from(self.list_area.height.max(1));
        match cmd {
            Command::Back | Command::Help => return Outcome::Close,
            Command::Move(MoveMode::Up, amount) => self.step(false, steps(amount, page)),
            Command::Move(MoveMode::Down, amount) => self.step(true, steps(amount, page)),
            Command::Play | Command::Open(_) => return self.run_selected(),
            _ => return Outcome::Ignored,
        }
        Outcome::Consumed
    }

    fn captures_text(&self) -> bool {
        true
    }

    /// Typing edits the filter; other keys go on to the bindings (arrows, Enter, Esc).
    fn key(&mut self, key: KeyEvent, _ctx: &Ctx) -> Outcome {
        match key.code {
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.filter.push(c);
            }
            KeyCode::Backspace => {
                self.filter.pop();
            }
            // Esc clears the filter first, then closes.
            KeyCode::Esc if !self.filter.is_empty() => self.filter.clear(),
            _ => return Outcome::Ignored,
        }
        self.apply_filter();
        Outcome::Consumed
    }

    fn mouse(&mut self, event: MouseEvent, _ctx: &Ctx) -> Outcome {
        let inside = |area: Rect| area.contains((event.column, event.row).into());
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) if inside(self.list_area) => {
                let row = self.first + usize::from(event.row - self.list_area.y);
                if matches!(self.rows.get(row), Some(Row::Entry(_))) {
                    self.selected = row;
                    return self.run_selected();
                }
            }
            MouseEventKind::Down(_) if !inside(self.area) => return Outcome::Close,
            MouseEventKind::ScrollDown => self.step(true, 3),
            MouseEventKind::ScrollUp => self.step(false, 3),
            _ => {}
        }
        Outcome::Consumed
    }
}

/// How many entries a move of `amount` goes, with `page` rows showing.
fn steps(amount: &MoveAmount, page: usize) -> usize {
    match amount {
        MoveAmount::Integer(n) => n.unsigned_abs() as usize,
        MoveAmount::Float(scale) => ((page as f32 * scale) as usize).max(1),
        MoveAmount::Extreme => usize::MAX / 2,
    }
}
