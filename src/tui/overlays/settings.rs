//! The settings (`,`), over the page like the key list: which library tabs show, the Home card,
//! autoplay, and the keys, each changed in place. Changes are written to config.toml (keeping its
//! comments) and take effect at once.

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use ratatui::widgets::{Block, BorderType, Borders, Clear};
use strum::IntoEnumIterator;

use crate::command::{Command, MoveAmount, MoveMode};
use crate::config::{ConfigValues, LibraryTab};
use crate::tui::hints::{self, Action};
use crate::tui::keys::{self, Key};
use crate::tui::page::{Ctx, Outcome, Overlay};
use crate::tui::pointer;
use crate::tui::style::{SELECTION_MARKER, Styles, bold, fill, put, scroll_to, width};

/// A switch: its label, a word on what it does, and the setting it reads and writes.
#[derive(Clone, Copy)]
enum Switch {
    LibraryTab(usize),
    PickUp,
    Autoplay,
}

#[derive(Clone, Copy)]
enum Row {
    Title(&'static str),
    Blank,
    Switch(Switch),
    /// An action from [hints::actions], by index.
    Keys(usize),
}

pub struct Settings {
    rows: Vec<Row>,
    actions: Vec<Action>,
    selected: usize,
    first: usize,
    /// The action waiting for its new key, if any.
    rebinding: Option<usize>,
    /// Where the rows were drawn, and the whole panel, for the mouse.
    area: Rect,
    panel: Rect,
}

/// The library's tabs in their default order, with their names.
fn library_tabs() -> Vec<(LibraryTab, &'static str)> {
    LibraryTab::iter()
        .map(|tab| {
            let name = match tab {
                LibraryTab::Tracks => "Liked songs",
                LibraryTab::Albums => "Albums",
                LibraryTab::Artists => "Artists",
                LibraryTab::Playlists => "Playlists",
                LibraryTab::Podcasts => "Podcasts",
                LibraryTab::Browse => "Browse",
            };
            (tab, name)
        })
        .collect()
}

/// The library tabs shown.
fn shown_tabs(values: &ConfigValues) -> Vec<LibraryTab> {
    values
        .library_tabs
        .clone()
        .unwrap_or_else(|| LibraryTab::iter().collect())
}

impl Settings {
    pub fn new(bindings: &[(String, Vec<Command>)]) -> Self {
        let mut page = Self {
            rows: Vec::new(),
            actions: Vec::new(),
            selected: 0,
            first: 0,
            rebinding: None,
            area: Rect::default(),
            panel: Rect::default(),
        };
        page.load(bindings);
        page.selected = page.next_row(0, true).unwrap_or(0);
        page
    }

    /// Lay out the rows for the keys in `bindings`.
    fn load(&mut self, bindings: &[(String, Vec<Command>)]) {
        self.actions = hints::actions(bindings);
        let mut rows = vec![Row::Blank, Row::Title("Library tabs")];
        rows.extend((0..library_tabs().len()).map(|i| Row::Switch(Switch::LibraryTab(i))));
        rows.extend([
            Row::Blank,
            Row::Title("Home"),
            Row::Switch(Switch::PickUp),
            Row::Blank,
            Row::Title("Playback"),
            Row::Switch(Switch::Autoplay),
        ]);
        let mut group = None;
        for (i, action) in self.actions.iter().enumerate() {
            if group != Some(action.group) {
                rows.push(Row::Blank);
                rows.push(Row::Title(match action.group {
                    hints::Group::Playback => "Keys: playback",
                    hints::Group::Navigation => "Keys: getting around",
                    hints::Group::Library => "Keys: queue, library and playlists",
                    hints::Group::Other => "Keys: other",
                }));
                group = Some(action.group);
            }
            rows.push(Row::Keys(i));
        }
        self.rows = rows;
        self.selected = self.selected.min(self.rows.len().saturating_sub(1));
    }

    /// The first row that can be selected from `from` on, going down (or up).
    fn next_row(&self, from: usize, down: bool) -> Option<usize> {
        let selectable =
            |i: &usize| matches!(self.rows.get(*i), Some(Row::Switch(_) | Row::Keys(_)));
        if down {
            (from..self.rows.len()).find(selectable)
        } else {
            (0..=from.min(self.rows.len().saturating_sub(1)))
                .rev()
                .find(selectable)
        }
    }

    fn step(&mut self, down: bool, by: usize) {
        for _ in 0..by {
            let next = if down {
                self.next_row(self.selected + 1, true)
            } else {
                self.selected
                    .checked_sub(1)
                    .and_then(|i| self.next_row(i, false))
            };
            match next {
                Some(next) => self.selected = next,
                None => break,
            }
        }
    }

    fn switch_state(switch: Switch, values: &ConfigValues) -> bool {
        match switch {
            Switch::LibraryTab(i) => {
                let tab = library_tabs()[i].0;
                shown_tabs(values).contains(&tab)
            }
            Switch::PickUp => values.pick_up_where_you_left_off != Some(false),
            Switch::Autoplay => values.autoplay != Some(false),
        }
    }

    fn switch_label(switch: Switch) -> &'static str {
        match switch {
            Switch::LibraryTab(i) => library_tabs()[i].1,
            Switch::PickUp => "Pick up where you left off",
            Switch::Autoplay => "Autoplay similar songs when the queue ends",
        }
    }

    /// Turn `switch` on or off in the configuration file.
    fn toggle(&mut self, switch: Switch, ctx: &Ctx) -> Outcome {
        let on = Self::switch_state(switch, &ctx.config.values());
        let result = match switch {
            Switch::LibraryTab(i) => {
                let tab = library_tabs()[i].0;
                let shown = shown_tabs(&ctx.config.values());
                // In the default order, with the tab added or left out.
                let tabs: Vec<LibraryTab> = LibraryTab::iter()
                    .filter(|t| if *t == tab { !on } else { shown.contains(t) })
                    .collect();
                if tabs.is_empty() {
                    return Outcome::Error("The library needs at least one tab".into());
                }
                ctx.config.edit(|doc| {
                    let mut names = toml_edit::Array::new();
                    for tab in &tabs {
                        names.push(tab_name(*tab));
                    }
                    doc["library_tabs"] = toml_edit::value(names);
                })
            }
            Switch::PickUp => ctx.config.edit(|doc| {
                doc["pick_up_where_you_left_off"] = toml_edit::value(!on);
            }),
            Switch::Autoplay => ctx.config.edit(|doc| {
                doc["autoplay"] = toml_edit::value(!on);
            }),
        };
        self.saved(result, None, ctx)
    }

    /// Make `key` do what action `index` does, instead of its keys now.
    fn rebind(&mut self, index: usize, key: &Key, ctx: &Ctx) -> Outcome {
        let Some(name) = key.name() else {
            return Outcome::Error(
                "That key can't be bound: use at most one of Shift, Ctrl, Alt".into(),
            );
        };
        let action = &self.actions[index];
        let commands = action.commands_text();
        let old_keys = action.keys.clone();
        // What the key did before, to say so.
        let replaced = self
            .actions
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
        self.saved(result, Some(message), ctx)
    }

    /// Give action `index` its default keys back.
    fn reset(&mut self, index: usize, ctx: &Ctx) -> Outcome {
        let action = &self.actions[index];
        let commands = action.commands_text();
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
        self.saved(result, None, ctx)
    }

    /// After a change to the configuration file: show the keys as they are now and have the app
    /// use the new settings, or say what went wrong.
    fn saved(
        &mut self,
        result: Result<(), Box<dyn std::error::Error>>,
        message: Option<String>,
        ctx: &Ctx,
    ) -> Outcome {
        match result {
            Ok(()) => {
                self.load(&keys::bindings(&ctx.config).1);
                Outcome::SettingsChanged(message)
            }
            Err(e) => Outcome::Error(format!("Couldn't save the settings: {e}")),
        }
    }
}

/// The `[keybindings]` table of the configuration, made if it's missing.
fn keybindings_table(doc: &mut toml_edit::DocumentMut) -> &mut toml_edit::Table {
    if !doc.contains_table("keybindings") {
        doc["keybindings"] = toml_edit::table();
    }
    doc["keybindings"].as_table_mut().expect("just made")
}

/// The name of `tab` in `library_tabs`.
fn tab_name(tab: LibraryTab) -> &'static str {
    match tab {
        LibraryTab::Tracks => "tracks",
        LibraryTab::Albums => "albums",
        LibraryTab::Artists => "artists",
        LibraryTab::Playlists => "playlists",
        LibraryTab::Podcasts => "podcasts",
        LibraryTab::Browse => "browse",
    }
}

/// Width of the keys column.
const KEYS_WIDTH: u16 = 22;

impl Overlay for Settings {
    fn draw(&mut self, frame: &mut Frame, screen: Rect, ctx: &Ctx) -> Rect {
        let styles = Styles(&ctx.palette);
        let w = screen
            .width
            .saturating_sub(4)
            .clamp(10, 80)
            .min(screen.width);
        let h = screen.height.saturating_sub(2).max(3).min(screen.height);
        let panel = Rect::new(
            screen.x + screen.width.saturating_sub(w) / 2,
            screen.y + screen.height.saturating_sub(h) / 2,
            w,
            h,
        );
        self.panel = panel;
        frame.render_widget(Clear, panel);
        let block = Block::new()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(styles.accent().bg(ctx.palette.statusbar_bg))
            .title(" Settings ")
            .title_style(bold(styles.accent().bg(ctx.palette.statusbar_bg)))
            .style(styles.panel());
        let area = block.inner(panel);
        frame.render_widget(block, panel);
        self.area = area;
        let buf = frame.buffer_mut();
        let focused = true;
        let background = ctx.palette.statusbar_bg;
        let rows = usize::from(area.height.saturating_sub(1));
        self.first = scroll_to(self.first, self.selected, rows);
        // Keep the titles above the selected row in view, up to the top for the first row.
        let mut top = self.selected;
        while top > 0 && !matches!(self.rows.get(top - 1), Some(Row::Switch(_) | Row::Keys(_))) {
            top -= 1;
        }
        if top < self.first && self.selected - top < rows {
            self.first = top;
        }
        let room = usize::from(area.width.saturating_sub(4));
        let values = ctx.config.values().clone();

        // What the keys do here, on the first row.
        let hint = match self.rebinding {
            Some(_) => "Press the new key · Esc cancels",
            None => "Enter changes the selected setting · d resets a key · saved to config.toml",
        };
        put(
            buf,
            area.x + 2,
            area.y,
            hint,
            room,
            styles.secondary().bg(background),
        );
        let list = Rect::new(
            area.x,
            area.y + 1,
            area.width,
            area.height.saturating_sub(1),
        );

        for (i, row) in self.rows.iter().enumerate().skip(self.first).take(rows) {
            let y = list.y + (i - self.first) as u16;
            let line = Rect::new(list.x, y, list.width, 1);
            let selected = i == self.selected;
            let style = match (selected, focused) {
                (true, true) => styles.selected(),
                (true, false) => styles.selected_inactive(),
                _ => styles.panel(),
            };
            match row {
                Row::Blank => continue,
                Row::Title(title) => {
                    put(
                        buf,
                        area.x + 2,
                        y,
                        title,
                        room,
                        bold(styles.accent().bg(background)),
                    );
                    continue;
                }
                Row::Switch(_) | Row::Keys(_) => {
                    fill(buf, line, style);
                    pointer::clickable(line);
                    if selected && focused {
                        put(buf, area.x, y, SELECTION_MARKER, 1, styles.marker());
                    }
                }
            }
            match row {
                Row::Switch(switch) => {
                    let on = Self::switch_state(*switch, &values);
                    let (mark, mark_style) = if on {
                        ("[✓]", bold(style.fg(ctx.palette.title)))
                    } else {
                        ("[ ]", style.fg(ctx.palette.secondary))
                    };
                    put(buf, area.x + 2, y, mark, 3, mark_style);
                    put(buf, area.x + 6, y, Self::switch_label(*switch), room, style);
                }
                Row::Keys(index) => {
                    let action = &self.actions[*index];
                    let keys = if self.rebinding == Some(*index) {
                        "press a key…".to_string()
                    } else {
                        action.keys_label()
                    };
                    let keys_room = usize::from(KEYS_WIDTH.min(area.width / 2));
                    let keys_width = width(&keys).min(keys_room) as u16;
                    put(
                        buf,
                        area.x + 2 + keys_room as u16 - keys_width,
                        y,
                        &keys,
                        keys_room,
                        bold(style.fg(ctx.palette.title)),
                    );
                    let x = area.x + keys_room as u16 + 5;
                    put(
                        buf,
                        x,
                        y,
                        &action.label,
                        usize::from(area.right().saturating_sub(x + 1)),
                        style,
                    );
                }
                _ => {}
            }
        }
        panel
    }

    fn command(&mut self, cmd: &Command, ctx: &Ctx) -> Outcome {
        let page = usize::from(self.area.height.max(2) - 1);
        match cmd {
            Command::Back | Command::Settings => return Outcome::Close,
            Command::Move(MoveMode::Up, amount) => self.step(false, steps(amount, page)),
            Command::Move(MoveMode::Down, amount) => self.step(true, steps(amount, page)),
            Command::Play | Command::Open(_) => {
                return match self.rows.get(self.selected).copied() {
                    Some(Row::Switch(switch)) => self.toggle(switch, ctx),
                    Some(Row::Keys(index)) => {
                        self.rebinding = Some(index);
                        Outcome::Consumed
                    }
                    _ => Outcome::Consumed,
                };
            }
            Command::Delete => {
                if let Some(Row::Keys(index)) = self.rows.get(self.selected).copied() {
                    return self.reset(index, ctx);
                }
            }
            _ => return Outcome::Ignored,
        }
        Outcome::Consumed
    }

    /// While waiting for a new key, every key comes here.
    fn captures_text(&self) -> bool {
        self.rebinding.is_some()
    }

    fn key(&mut self, event: KeyEvent, ctx: &Ctx) -> Outcome {
        let Some(index) = self.rebinding.take() else {
            return Outcome::Ignored;
        };
        if event.code == KeyCode::Esc {
            return Outcome::Consumed;
        }
        match Key::from_event(&event) {
            Some(key) => self.rebind(index, &key, ctx),
            None => Outcome::Consumed,
        }
    }

    fn mouse(&mut self, event: MouseEvent, ctx: &Ctx) -> Outcome {
        let list_y = self.area.y + 1;
        let inside = |area: Rect| area.contains((event.column, event.row).into());
        match event.kind {
            MouseEventKind::Down(_) if !inside(self.panel) => return Outcome::Close,
            MouseEventKind::Down(MouseButton::Left) if inside(self.area) && event.row >= list_y => {
                let row = self.first + usize::from(event.row - list_y);
                if matches!(self.rows.get(row), Some(Row::Switch(_) | Row::Keys(_))) {
                    self.selected = row;
                    return self.command(&Command::Play, ctx);
                }
            }
            MouseEventKind::ScrollDown => self.step(true, 3),
            MouseEventKind::ScrollUp => self.step(false, 3),
            _ => {}
        }
        Outcome::Consumed
    }
}

/// How many rows a move of `amount` goes, with `page` rows showing.
fn steps(amount: &MoveAmount, page: usize) -> usize {
    match amount {
        MoveAmount::Integer(n) => n.unsigned_abs() as usize,
        MoveAmount::Float(scale) => ((page as f32 * scale) as usize).max(1),
        MoveAmount::Extreme => usize::MAX / 2,
    }
}
