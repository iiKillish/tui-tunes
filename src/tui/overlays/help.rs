//! The key list (`?`): every binding and every command, in sections (Playback, Getting around,
//! …, Commands) shown one at a time, saying what they do in words. ←/→ or Tab go from section to
//! section; typing finds keys and commands in all of them; Enter (or a click) runs the selected
//! one, or puts a command that needs more on the command line. Ctrl+E changes the selected key
//! and Ctrl+R gives it its default keys back, saved to config.toml.

use ratatui::Frame;
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::{Block, BorderType, Borders, Clear};

use crate::command::{self, ALIASES, COMMANDS, Command, CommandInfo, MoveAmount, MoveMode};
use crate::tui::hints::{self, Action, Group};
use crate::tui::keys::{self, Key};
use crate::tui::page::{Ctx, Outcome, Overlay};
use crate::tui::pointer;
use crate::tui::style::{SELECTION_MARKER, Styles, bold, fill, put, scroll_to, width};

/// The sections: the keys of each group, then the commands.
const SECTIONS: usize = Group::ALL.len() + 1;

fn section_title(section: usize) -> &'static str {
    Group::ALL
        .get(section)
        .map_or("Commands", |g| g.short_title())
}

/// A row of the list.
#[derive(Clone, Copy, PartialEq)]
enum Row {
    Title(&'static str),
    Blank,
    /// A key (by index into the actions).
    Key(usize),
    /// A command (by index into [COMMANDS]).
    Command(usize),
}

impl Row {
    fn is_entry(self) -> bool {
        matches!(self, Self::Key(_) | Self::Command(_))
    }
}

pub struct Help {
    entries: Vec<Action>,
    /// The group of each command in [COMMANDS].
    command_groups: Vec<Group>,
    filter: String,
    /// The section showing (without a filter), or the selected row's (with one).
    section: usize,
    /// What's shown, with the section of each row.
    rows: Vec<(usize, Row)>,
    /// The selected entry, as an index into `rows`.
    selected: usize,
    first: usize,
    list_area: Rect,
    tab_areas: Vec<Rect>,
    area: Rect,
    /// The key (by index into the actions) waiting for its new key, if any.
    rebinding: Option<usize>,
}

impl Help {
    /// The key list for the bindings in `bindings` (key name, commands).
    pub fn new(bindings: &[(String, Vec<Command>)]) -> Self {
        let mut help = Self {
            entries: hints::actions(bindings),
            command_groups: COMMANDS.iter().map(command_group).collect(),
            filter: String::new(),
            section: 0,
            rows: Vec::new(),
            selected: 0,
            first: 0,
            list_area: Rect::default(),
            tab_areas: Vec::new(),
            area: Rect::default(),
            rebinding: None,
        };
        help.apply_filter();
        help
    }

    /// The rows of `section`, with those that `matches` only.
    fn section_rows(&self, section: usize, matches: impl Fn(&str) -> bool) -> Vec<Row> {
        if let Some(&group) = Group::ALL.get(section) {
            return (self.entries.iter().enumerate())
                .filter(|(_, e)| e.group == group)
                .filter(|(_, e)| matches(&format!("{} {}", e.keys_label(), e.label)))
                .map(|(i, _)| Row::Key(i))
                .collect();
        }
        // The commands, under the titles of their groups.
        let mut rows = Vec::new();
        for group in Group::ALL {
            let commands: Vec<Row> = (COMMANDS.iter().enumerate())
                .filter(|(i, _)| self.command_groups[*i] == group)
                .filter(|(_, c)| matches(&command_text(c)))
                .map(|(i, _)| Row::Command(i))
                .collect();
            if !commands.is_empty() {
                if !rows.is_empty() {
                    rows.push(Row::Blank);
                }
                rows.push(Row::Title(group.title()));
                rows.extend(commands);
            }
        }
        rows
    }

    /// Show the current section, or with a filter, what matches it in every section, under the
    /// sections' titles.
    fn apply_filter(&mut self) {
        let query = self.filter.to_lowercase();
        self.rows.clear();
        if query.is_empty() {
            let rows = self.section_rows(self.section, |_| true);
            self.rows = rows.into_iter().map(|row| (self.section, row)).collect();
        } else {
            for section in 0..SECTIONS {
                let rows = self.section_rows(section, |text| text.to_lowercase().contains(&query));
                // Only the entries: the commands' group titles are in the way here.
                let entries: Vec<Row> = rows.into_iter().filter(|r| r.is_entry()).collect();
                if entries.is_empty() {
                    continue;
                }
                if !self.rows.is_empty() {
                    self.rows.push((section, Row::Blank));
                }
                self.rows
                    .push((section, Row::Title(section_title(section))));
                self.rows
                    .extend(entries.into_iter().map(|row| (section, row)));
            }
        }
        self.first = 0;
        self.select(self.next_entry(0, true).unwrap_or(0));
    }

    fn select(&mut self, row: usize) {
        self.selected = row;
        if !self.filter.is_empty()
            && let Some((section, _)) = self.rows.get(row)
        {
            self.section = *section;
        }
    }

    /// Show the next (or previous) section; with a filter, select its first match.
    fn switch_section(&mut self, next: bool) {
        if self.filter.is_empty() {
            self.section = if next {
                (self.section + 1) % SECTIONS
            } else {
                (self.section + SECTIONS - 1) % SECTIONS
            };
            self.apply_filter();
            return;
        }
        let sections: Vec<usize> = (0..SECTIONS)
            .filter(|s| self.rows.iter().any(|(section, _)| section == s))
            .collect();
        let Some(at) = sections.iter().position(|s| *s == self.section) else {
            return;
        };
        let len = sections.len();
        let to = sections[if next {
            (at + 1) % len
        } else {
            (at + len - 1) % len
        }];
        self.show_section(to);
    }

    /// Go to `section`: show it, or with a filter, select its first match.
    fn show_section(&mut self, section: usize) {
        if self.filter.is_empty() {
            self.section = section;
            self.apply_filter();
        } else if let Some(row) = self
            .rows
            .iter()
            .position(|(s, row)| *s == section && row.is_entry())
        {
            self.select(row);
            // With its title.
            self.first = row.saturating_sub(1);
        }
    }

    /// The first entry row from `from` on, going down (or up).
    fn next_entry(&self, from: usize, down: bool) -> Option<usize> {
        let is_entry = |i: &usize| self.rows.get(*i).is_some_and(|(_, row)| row.is_entry());
        if down {
            (from..self.rows.len()).find(is_entry)
        } else {
            (0..=from.min(self.rows.len().saturating_sub(1)))
                .rev()
                .find(is_entry)
        }
    }

    fn step(&mut self, down: bool, by: usize) {
        let mut selected = self.selected;
        for _ in 0..by {
            let next = if down {
                self.next_entry(selected + 1, true)
            } else {
                selected
                    .checked_sub(1)
                    .and_then(|i| self.next_entry(i, false))
            };
            match next {
                Some(next) => selected = next,
                None => break,
            }
        }
        self.select(selected);
    }

    fn run_selected(&self) -> Outcome {
        match self.rows.get(self.selected) {
            Some((_, Row::Key(i))) => Outcome::Run(self.entries[*i].commands.clone()),
            // Commands that need more go on the command line, to type the rest.
            Some((_, Row::Command(i))) => {
                let info = &COMMANDS[*i];
                match command::parse(info.name) {
                    Ok(commands) if !info.args.starts_with('<') => Outcome::Run(commands),
                    _ => Outcome::CommandLine(format!("{} ", info.name)),
                }
            }
            _ => Outcome::Close,
        }
    }

    /// The key selected, by index into the actions.
    fn selected_key(&self) -> Option<usize> {
        match self.rows.get(self.selected) {
            Some((_, Row::Key(i))) => Some(*i),
            _ => None,
        }
    }

    /// Make `key` do what action `index` does, instead of its keys now.
    fn rebind(&mut self, index: usize, key: &Key, ctx: &Ctx) -> Outcome {
        let Some(name) = key.name() else {
            return Outcome::Error(
                "That key can't be bound: use at most one of Shift, Ctrl, Alt".into(),
            );
        };
        let action = &self.entries[index];
        let commands = action.commands_text();
        let old_keys = action.keys.clone();
        // What the key did before, to say so.
        let replaced = self
            .entries
            .iter()
            .find(|a| a.keys.contains(&name) && a.commands_text() != commands)
            .map(|a| a.label.clone());
        let result = ctx.config.edit(|doc| {
            let table = keybindings_table(doc);
            for old in old_keys.iter().filter(|k| **k != name) {
                table[old.as_str()] = toml_edit::value("noop");
            }
            table[name.as_str()] = toml_edit::value(commands.as_str());
        });
        let label = hints::key_label(&name);
        let message = match replaced {
            Some(before) => format!("{label} now does this, instead of \"{before}\""),
            None => format!("{label} now does this"),
        };
        self.saved(result, Some(message), &commands, ctx)
    }

    /// Give action `index` its default keys back.
    fn reset(&mut self, index: usize, ctx: &Ctx) -> Outcome {
        let commands = self.entries[index].commands_text();
        let defaults: Vec<String> = keys::default_keybindings()
            .into_iter()
            .filter(|(_, c)| {
                c.iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("; ")
                    == commands
            })
            .map(|(name, _)| name)
            .collect();
        let result = ctx.config.edit(|doc| {
            let table = keybindings_table(doc);
            let ours: Vec<String> = table
                .iter()
                .filter(|(name, value)| {
                    value.as_str() == Some(commands.as_str())
                        || (defaults.iter().any(|d| d == name) && value.as_str() == Some("noop"))
                })
                .map(|(name, _)| name.to_string())
                .collect();
            for name in ours {
                table.remove(&name);
            }
        });
        self.saved(
            result,
            Some("Back to its default keys".into()),
            &commands,
            ctx,
        )
    }

    /// After the keys changed in the configuration file: list them as they are now, with the
    /// same one selected, and have the app use them; or say what went wrong.
    fn saved(
        &mut self,
        result: Result<(), Box<dyn std::error::Error>>,
        message: Option<String>,
        commands: &str,
        ctx: &Ctx,
    ) -> Outcome {
        if let Err(e) = result {
            return Outcome::Error(format!("Couldn't save the keys: {e}"));
        }
        self.entries = hints::actions(&keys::bindings(&ctx.config).1);
        let (section, first) = (self.section, self.first);
        self.apply_filter();
        self.section = section;
        if let Some(row) = self.rows.iter().position(
            |(_, row)| matches!(row, Row::Key(i) if self.entries[*i].commands_text() == commands),
        ) {
            self.select(row);
            self.first = first;
        }
        Outcome::SettingsChanged(message)
    }

    /// The selected entry's number and how many there are.
    fn position(&self) -> (usize, usize) {
        let entries = || self.rows.iter().filter(|(_, row)| row.is_entry());
        let before = self.rows[..self.selected.min(self.rows.len())]
            .iter()
            .filter(|(_, row)| row.is_entry())
            .count();
        (before + 1, entries().count())
    }
}

/// What a command is found by: its name, aliases, arguments and what it does.
fn command_text(info: &CommandInfo) -> String {
    let aliases: Vec<&str> = ALIASES
        .iter()
        .filter(|(_, name)| *name == info.name)
        .map(|(alias, _)| *alias)
        .collect();
    format!(
        ":{} {} {} {}",
        info.name,
        aliases.join(" "),
        info.args,
        info.about
    )
}

/// The group a command goes in, as its keys do: from what it gives with its first value, or with
/// something typed.
fn command_group(info: &CommandInfo) -> Group {
    let tries = [
        info.name.to_string(),
        format!("{} {}", info.name, info.values.first().unwrap_or(&"1")),
        format!("{} 1", info.name),
    ];
    tries
        .iter()
        .find_map(|line| command::parse(line).ok())
        .map_or(Group::Other, |commands| hints::group(&commands))
}

/// Width of the keys column.
const KEYS_WIDTH: u16 = 20;
/// Rows above the list: the filter, the sections, a tip and a blank row.
const TOP_ROWS: u16 = 4;

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
        let border = styles.accent().bg(ctx.palette.statusbar_bg);
        let dim = panel.fg(ctx.palette.secondary);
        let (number, total) = self.position();
        let mut block = Block::new()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(border)
            .title(" Keys and commands ")
            .title_style(bold(border))
            .title_bottom(Line::styled(
                match (self.rebinding, self.selected_key()) {
                    (Some(_), _) => " Press the new key · Esc cancels ",
                    (None, Some(_)) => " ←→ sections · Enter runs · ^E change key · ^R reset ",
                    (None, None) => " ←→ sections · ↑↓ choose · Enter runs · Esc closes ",
                },
                dim,
            ))
            .style(panel);
        if total > 0 {
            block = block
                .title_bottom(Line::styled(format!(" {number}/{total} "), dim).right_aligned());
        }
        let mut inner = block.inner(area);
        frame.render_widget(block, area);
        let room = usize::from(inner.width.saturating_sub(2));
        self.tab_areas.clear();

        // The filter, the sections and a tip.
        if inner.height > TOP_ROWS {
            let buf = frame.buffer_mut();
            let field = Rect::new(inner.x, inner.y, inner.width, 1);
            pointer::text(field);
            let field_style = panel.bg(ctx.palette.highlight_inactive_bg);
            fill(buf, field, field_style);
            let (text, style) = if self.filter.is_empty() {
                (
                    "Type to find a key or command",
                    field_style.fg(ctx.palette.secondary),
                )
            } else {
                (self.filter.as_str(), field_style)
            };
            put(buf, field.x + 1, field.y, text, room, style);
            let typed = width(&self.filter).min(room) as u16;
            frame.set_cursor_position((field.x + 1 + typed, field.y));

            // The sections, the current one lit; with a filter, how many match in each.
            let buf = frame.buffer_mut();
            let mut x = inner.x + 1;
            let y = inner.y + 1;
            for section in 0..SECTIONS {
                let matches = self
                    .rows
                    .iter()
                    .filter(|(s, r)| *s == section && r.is_entry())
                    .count();
                let label = if self.filter.is_empty() {
                    format!(" {} ", section_title(section))
                } else {
                    format!(" {} {matches} ", section_title(section))
                };
                let style = if section == self.section {
                    styles.button()
                } else if !self.filter.is_empty() && matches == 0 {
                    dim
                } else {
                    panel
                };
                let left = usize::from(inner.right().saturating_sub(x + 1));
                let written = put(buf, x, y, &label, left, style) as u16;
                let tab = Rect::new(x, y, written, 1);
                pointer::clickable(tab);
                self.tab_areas.push(tab);
                x += written + 1;
            }

            let hints_key = self
                .entries
                .iter()
                .find(|e| matches!(e.commands.as_slice(), [Command::Hints]));
            let tip = match hints_key {
                Some(entry) => format!(
                    "Hold Ctrl+Shift or press {} to see keys on screen · : types a command",
                    entry.keys_label()
                ),
                None => "Hold Ctrl+Shift to see keys on screen · : types a command".into(),
            };
            put(buf, inner.x + 1, inner.y + 2, &tip, room, dim);
            inner.y += TOP_ROWS;
            inner.height -= TOP_ROWS;
        }
        self.list_area = inner;

        let buf = frame.buffer_mut();
        let rows = usize::from(inner.height);
        self.first = scroll_to(self.first, self.selected, rows);
        // Keep the titles above the selected row in view, up to the top for the first row.
        let mut top = self.selected;
        while top > 0 && !self.rows.get(top - 1).is_some_and(|(_, r)| r.is_entry()) {
            top -= 1;
        }
        if top < self.first && self.selected - top < rows {
            self.first = top;
        }
        if self.rows.is_empty() && rows > 0 {
            put(buf, inner.x + 1, inner.y, "Nothing matches", room, dim);
        }
        // The commands' names (and arguments) take as much room as the longest showing.
        let name_room = (self.rows.iter().skip(self.first).take(rows))
            .filter_map(|(_, row)| match row {
                Row::Command(i) => Some(width(&command_name(&COMMANDS[*i]))),
                _ => None,
            })
            .max()
            .unwrap_or(0)
            .min(usize::from(inner.width / 2));
        for (i, (_, row)) in self.rows.iter().enumerate().skip(self.first).take(rows) {
            let y = inner.y + (i - self.first) as u16;
            let line = Rect::new(inner.x, y, inner.width, 1);
            let selected = i == self.selected;
            let style = if selected {
                styles.selected().bg(ctx.palette.highlight_inactive_bg)
            } else {
                panel
            };
            if row.is_entry() {
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
            }
            let about_room = |x: u16| usize::from(inner.right().saturating_sub(x + 1));
            match row {
                Row::Blank => {}
                Row::Title(title) => {
                    put(buf, inner.x + 1, y, title, room, bold(border));
                }
                Row::Key(index) => {
                    let entry = &self.entries[*index];
                    // Keys right-aligned in their column, then what they do.
                    let keys = if self.rebinding == Some(*index) {
                        "press a key…".to_string()
                    } else {
                        entry.keys_label()
                    };
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
                    put(buf, x, y, &entry.label, about_room(x), style);
                }
                Row::Command(index) => {
                    // `:name args`, then what it does.
                    let info = &COMMANDS[*index];
                    let x = inner.x + 2;
                    let name = format!(":{}", info.name);
                    let used = put(
                        buf,
                        x,
                        y,
                        &name,
                        name_room,
                        bold(style.fg(ctx.palette.title)),
                    );
                    if !info.args.is_empty() && used + 1 < name_room {
                        put(
                            buf,
                            x + used as u16 + 1,
                            y,
                            info.args,
                            name_room - used - 1,
                            style.fg(ctx.palette.secondary),
                        );
                    }
                    let x = x + name_room as u16 + 3;
                    put(buf, x, y, info.about, about_room(x), style);
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
            Command::Move(MoveMode::Left, _) => self.switch_section(false),
            Command::Move(MoveMode::Right, _) => self.switch_section(true),
            Command::Tab(next) | Command::Screen(next) => self.switch_section(*next),
            Command::Play | Command::Open(_) => return self.run_selected(),
            _ => return Outcome::Ignored,
        }
        Outcome::Consumed
    }

    fn captures_text(&self) -> bool {
        true
    }

    /// Typing edits the filter, Tab goes to the next section, Ctrl+E and Ctrl+R change the
    /// selected key; other keys go on to the bindings (arrows, Enter, Esc).
    fn key(&mut self, key: KeyEvent, ctx: &Ctx) -> Outcome {
        // Every key, while waiting for a new one.
        if let Some(index) = self.rebinding.take() {
            if key.code == KeyCode::Esc {
                return Outcome::Consumed;
            }
            return match Key::from_event(&key) {
                Some(new) => self.rebind(index, &new, ctx),
                None => Outcome::Consumed,
            };
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('e') if ctrl => {
                if let Some(index) = self.selected_key() {
                    self.rebinding = Some(index);
                }
                return Outcome::Consumed;
            }
            KeyCode::Char('r') if ctrl => {
                return match self.selected_key() {
                    Some(index) => self.reset(index, ctx),
                    None => Outcome::Consumed,
                };
            }
            KeyCode::Tab => {
                self.switch_section(true);
                return Outcome::Consumed;
            }
            KeyCode::BackTab => {
                self.switch_section(false);
                return Outcome::Consumed;
            }
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
        // A section's tab (not `if let` in a match guard, which older Rust doesn't have).
        if matches!(event.kind, MouseEventKind::Down(MouseButton::Left))
            && let Some(section) = self.tab_areas.iter().position(|t| inside(*t))
        {
            self.show_section(section);
            return Outcome::Consumed;
        }
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) if inside(self.list_area) => {
                let row = self.first + usize::from(event.row - self.list_area.y);
                if self.rows.get(row).is_some_and(|(_, r)| r.is_entry()) {
                    self.select(row);
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

/// The `[keybindings]` table of the configuration, made if it's missing.
fn keybindings_table(doc: &mut toml_edit::DocumentMut) -> &mut toml_edit::Table {
    if !doc.contains_table("keybindings") {
        doc["keybindings"] = toml_edit::table();
    }
    doc["keybindings"].as_table_mut().expect("just made")
}

/// A command's name and arguments as shown.
fn command_name(info: &CommandInfo) -> String {
    if info.args.is_empty() {
        format!(":{}", info.name)
    } else {
        format!(":{} {}", info.name, info.args)
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
