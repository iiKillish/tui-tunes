//! The app: screens and the pages opened on them, the player, the tab row, the command line and
//! overlays; turning keys and clicks into commands and drawing it all.
//!
//! Layout (design B): the player as a hero at the top (a left column on wide windows, a bar at
//! the bottom of small ones), then the tab row (Home, Search, Library, Up next, Lyrics), a title
//! row with a back button when a page is open on top of a screen, the page's header if it has
//! one, and the page. The command line and messages take the bottom row(s).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use log::info;
use ratatui::Frame;
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::{Position, Rect};
use ratatui::style::Modifier;

use crate::command::{self, Command, JumpMode, PanelAction, ScreenId};
use crate::tui::actions;
use crate::tui::header::{self, HEADER_ROWS, HeaderAction};
use crate::tui::images;
use crate::tui::keys::{self, Key, MAX_COUNT, accumulate_count};
use crate::tui::page::{Ctx, Outcome, Overlay, Page};
use crate::tui::pages;
use crate::tui::pointer;
use crate::tui::style::{Styles, bold, fill, put, truncate, width};
use crate::tui::widgets::player::{BAR_ROWS, HERO_MIN_ROWS, Player};
use crate::tui::widgets::tabs::draw_tab_row;

/// How long a message stays in the status line.
const MESSAGE_TIME: Duration = Duration::from_secs(5);
/// Fewest rows to keep for the page under the player.
const MIN_PAGE_ROWS: u16 = 10;
/// Fewest rows to keep for a page below its header; with less room the header isn't shown.
const MIN_ROWS_BELOW_HEADER: u16 = 6;

struct Screen {
    id: ScreenId,
    label: &'static str,
    root: Box<dyn Page>,
    /// Pages opened on top of the root, last on top.
    stack: Vec<Box<dyn Page>>,
}

impl Screen {
    fn top(&mut self) -> &mut Box<dyn Page> {
        self.stack.last_mut().unwrap_or(&mut self.root)
    }

    fn top_ref(&self) -> &dyn Page {
        self.stack.last().map_or(self.root.as_ref(), |p| p.as_ref())
    }
}

/// The text being typed after `:` (a command) or `/` (a search in the page).
struct CommandLine {
    prefix: char,
    text: String,
    /// Cursor position in characters.
    cursor: usize,
}

/// Where things were drawn last, for the mouse.
#[derive(Default)]
struct Regions {
    player: Rect,
    tabs: Vec<Rect>,
    back: Option<Rect>,
    header_buttons: Vec<(Rect, HeaderAction)>,
    page: Rect,
}

pub struct App {
    pub ctx: Ctx,
    screens: Vec<Screen>,
    current: usize,
    /// The screen shown before the current one, for toggling back (e.g. lyrics).
    previous: usize,
    overlays: Vec<Box<dyn Overlay>>,
    overlay_areas: Vec<Rect>,
    player: Player,
    bindings: HashMap<Key, Vec<Command>>,
    pub binding_list: Vec<(String, Vec<Command>)>,
    /// Count prefix being typed (like the 10 of `10j`).
    count: usize,
    cmdline: Option<CommandLine>,
    message: Option<(Result<String, String>, Instant)>,
    regions: Regions,
    pub quit: bool,
    /// Ask the terminal to repaint everything on the next draw.
    pub clear: bool,
}

impl App {
    pub fn new(ctx: Ctx) -> Self {
        let (bindings, binding_list) = keys::bindings(&ctx.config);
        let screens = vec![
            Screen {
                id: ScreenId::Home,
                label: "Home",
                root: Box::new(pages::home::new(&ctx)),
                stack: Vec::new(),
            },
            Screen {
                id: ScreenId::Search,
                label: "Search",
                root: Box::new(pages::search::new(&ctx)),
                stack: Vec::new(),
            },
            Screen {
                id: ScreenId::Library,
                label: "Library",
                root: Box::new(pages::library::new(&ctx)),
                stack: Vec::new(),
            },
            Screen {
                id: ScreenId::Queue,
                label: "Up next",
                root: Box::new(pages::queue::new(&ctx)),
                stack: Vec::new(),
            },
            Screen {
                id: ScreenId::Lyrics,
                label: "Lyrics",
                root: Box::new(pages::lyrics::new(&ctx)),
                stack: Vec::new(),
            },
        ];
        let initial = ctx
            .config
            .values()
            .initial_screen
            .as_deref()
            .and_then(|id| id.parse().ok())
            .unwrap_or(ScreenId::Home);
        let current = screens.iter().position(|s| s.id == initial).unwrap_or(0);
        let mut app = Self {
            ctx,
            screens,
            current,
            previous: current,
            overlays: Vec::new(),
            overlay_areas: Vec::new(),
            player: Player::new(),
            bindings,
            binding_list,
            count: 0,
            cmdline: None,
            message: None,
            regions: Regions::default(),
            quit: false,
            clear: false,
        };
        let ctx = app.ctx.clone();
        app.screens[app.current].top().on_show(&ctx);
        app
    }

    /// Reload keybindings and colors after the configuration changed.
    pub fn reload(&mut self) {
        let (bindings, list) = keys::bindings(&self.ctx.config);
        self.bindings = bindings;
        self.binding_list = list;
        self.ctx.palette = self.ctx.config.build_theme();
    }

    fn screen(&mut self) -> &mut Screen {
        &mut self.screens[self.current]
    }

    pub fn set_screen(&mut self, id: ScreenId) {
        let Some(index) = self.screens.iter().position(|s| s.id == id) else {
            return;
        };
        if index != self.current {
            self.previous = self.current;
            self.current = index;
        }
        let ctx = self.ctx.clone();
        self.screen().top().on_show(&ctx);
    }

    fn back(&mut self) {
        let ctx = self.ctx.clone();
        let screen = self.screen();
        if screen.stack.pop().is_some() {
            screen.top().on_show(&ctx);
        }
    }

    pub fn set_message(&mut self, message: Result<String, String>) {
        self.message = Some((message, Instant::now()));
    }

    /// Open `page` on top of the current screen, as if an item had been opened.
    #[cfg(test)]
    pub fn open_page(&mut self, page: Box<dyn crate::tui::page::Page>) {
        self.apply(Outcome::Open(page), false);
    }

    /// The screen showing.
    #[cfg(test)]
    pub fn current_screen(&self) -> ScreenId {
        self.screens[self.current].id
    }

    /// Replace the root page of the screen `id`.
    #[cfg(test)]
    pub fn set_root(&mut self, id: ScreenId, page: Box<dyn crate::tui::page::Page>) {
        if let Some(screen) = self.screens.iter_mut().find(|s| s.id == id) {
            screen.root = page;
            screen.stack.clear();
        }
    }

    /// Act on what a page or overlay returned. `from_overlay` closes the overlay where the
    /// outcome means it's done.
    fn apply(&mut self, outcome: Outcome, from_overlay: bool) {
        let ctx = self.ctx.clone();
        match outcome {
            Outcome::Ignored | Outcome::Consumed => {}
            Outcome::Open(mut page) => {
                if from_overlay {
                    self.overlays.pop();
                }
                page.on_show(&ctx);
                self.screen().stack.push(page);
            }
            Outcome::Overlay(overlay) => self.overlays.push(overlay),
            Outcome::Close => {
                self.overlays.pop();
            }
            Outcome::Replace(overlay) => {
                self.overlays.pop();
                self.overlays.push(overlay);
            }
            Outcome::Message(text) => {
                if from_overlay {
                    self.overlays.pop();
                }
                self.set_message(Ok(text));
            }
            Outcome::Error(text) => {
                if from_overlay {
                    self.overlays.pop();
                }
                self.set_message(Err(text));
            }
            Outcome::Run(commands) => {
                if from_overlay {
                    self.overlays.pop();
                }
                for cmd in commands {
                    self.run(&cmd);
                }
            }
        }
    }

    /// Run a command: the top overlay first, else the page, then the app, then global actions.
    pub fn run(&mut self, cmd: &Command) {
        let ctx = self.ctx.clone();
        if let Some(overlay) = self.overlays.last_mut() {
            let outcome = overlay.command(cmd, &ctx);
            if !outcome.is_ignored() {
                self.apply(outcome, true);
                return;
            }
        } else {
            let outcome = self.screen().top().command(cmd, &ctx);
            if !outcome.is_ignored() {
                self.apply(outcome, false);
                return;
            }
        }

        match cmd {
            Command::Focus(id) => {
                if *id == ScreenId::Search && self.screens[self.current].id == ScreenId::Search {
                    // Focusing search again starts a new search.
                    self.screen().stack.clear();
                }
                self.set_screen(*id);
            }
            Command::Back => self.back(),
            Command::Panel(PanelAction::Lyrics) => {
                if self.screens[self.current].id == ScreenId::Lyrics {
                    let previous = self.screens[self.previous].id;
                    self.set_screen(previous);
                } else {
                    self.set_screen(ScreenId::Lyrics);
                }
            }
            Command::Panel(PanelAction::Queue) => {
                self.set_screen(ScreenId::Queue);
            }
            // Pages without tabs of their own have nothing to switch.
            Command::Tab(_) => {}
            Command::Screen(next) => {
                let len = self.screens.len();
                let index = if *next {
                    (self.current + 1) % len
                } else {
                    (self.current + len - 1) % len
                };
                let id = self.screens[index].id;
                self.set_screen(id);
            }
            Command::Search(_) => {
                self.set_screen(ScreenId::Search);
                self.screen().stack.clear();
                let outcome = self.screen().root.command(cmd, &ctx);
                self.apply(outcome, false);
            }
            Command::Help => {
                let overlay = actions::help(self);
                self.overlays.push(overlay);
            }
            Command::Redraw => self.clear = true,
            Command::ReloadConfig => match ctx.config.reload() {
                Ok(()) => self.reload(),
                Err(_) => self.set_message(Err(
                    "Failed to reload the configuration: fix the errors in config.toml and try again".into(),
                )),
            },
            Command::Quit => {
                actions::save_state(&ctx);
                self.quit = true;
            }
            _ => {
                let outcome = actions::run(cmd, &ctx);
                match outcome {
                    Outcome::Ignored => self.set_message(Err(format!(
                        "The command \"{}\" isn't available here",
                        cmd.basename()
                    ))),
                    outcome => self.apply(outcome, false),
                }
            }
        }
    }

    /// Run the commands bound to a key, `count` times where that makes sense.
    fn run_binding(&mut self, commands: Vec<Command>) {
        let count = match std::mem::take(&mut self.count) {
            0 => 1,
            n => n,
        };
        let eligible = commands.iter().all(Command::accepts_count);
        if eligible
            && count > 1
            && let [command] = commands.as_slice()
        {
            // One scaled command instead of many (seeking, volume, skipping).
            if let Some(scaled) = command.scaled(count) {
                self.run(&scaled);
                return;
            }
            match command {
                Command::Next => return self.ctx.queue.skip(count as isize),
                Command::Previous => return self.ctx.queue.skip(-(count as isize)),
                _ => {}
            }
        }
        let repeats = if eligible { count } else { 1 };
        for _ in 0..repeats {
            for command in &commands {
                self.run(command);
            }
        }
    }

    fn command_key(&self) -> char {
        self.ctx
            .config
            .values()
            .command_key
            .unwrap_or(crate::config::DEFAULT_COMMAND_KEY)
    }

    pub fn key(&mut self, event: KeyEvent) {
        let ctx = self.ctx.clone();
        if self.cmdline.is_some() {
            self.cmdline_key(event);
            return;
        }
        // Text fields (a search box, a playlist name) get keys first.
        if let Some(overlay) = self.overlays.last_mut() {
            if overlay.captures_text() {
                let outcome = overlay.key(event, &ctx);
                if !outcome.is_ignored() {
                    self.apply(outcome, true);
                    return;
                }
            }
        } else if self.screens[self.current].top_ref().captures_text() {
            let outcome = self.screen().top().key(event, &ctx);
            if !outcome.is_ignored() {
                self.apply(outcome, false);
                return;
            }
        }

        let Some(key) = Key::from_event(&event) else {
            return;
        };
        if key.modifiers == KeyModifiers::NONE
            && let KeyCode::Char(c) = key.code
        {
            if c == self.command_key() || c == '/' {
                self.count = 0;
                self.cmdline = Some(CommandLine {
                    prefix: c,
                    text: String::new(),
                    cursor: 0,
                });
                return;
            }
            if let Some(digit) = c.to_digit(10)
                && !self.bindings.contains_key(&key)
            {
                self.count = accumulate_count(self.count, digit, MAX_COUNT);
                return;
            }
        }
        match self.bindings.get(&key).cloned() {
            Some(commands) => self.run_binding(commands),
            None => self.count = 0,
        }
    }

    fn cmdline_key(&mut self, event: KeyEvent) {
        let Some(line) = self.cmdline.as_mut() else {
            return;
        };
        let chars: Vec<char> = line.text.chars().collect();
        match event.code {
            KeyCode::Esc => self.cmdline = None,
            KeyCode::Enter => {
                let line = self.cmdline.take().unwrap();
                self.submit(line.prefix, &line.text);
            }
            KeyCode::Backspace if chars.is_empty() => self.cmdline = None,
            KeyCode::Backspace if line.cursor > 0 => {
                line.text = chars[..line.cursor - 1]
                    .iter()
                    .chain(&chars[line.cursor..])
                    .collect();
                line.cursor -= 1;
            }
            KeyCode::Delete if line.cursor < chars.len() => {
                line.text = chars[..line.cursor]
                    .iter()
                    .chain(&chars[line.cursor + 1..])
                    .collect();
            }
            KeyCode::Left => line.cursor = line.cursor.saturating_sub(1),
            KeyCode::Right => line.cursor = (line.cursor + 1).min(chars.len()),
            KeyCode::Home => line.cursor = 0,
            KeyCode::End => line.cursor = chars.len(),
            KeyCode::Tab if line.prefix != '/' => {
                line.text = word_completion(&line.text, command::ALL_COMMANDS);
                line.cursor = line.text.chars().count();
            }
            KeyCode::Char(c) if !event.modifiers.contains(KeyModifiers::CONTROL) => {
                line.text = chars[..line.cursor]
                    .iter()
                    .chain(std::iter::once(&c))
                    .chain(&chars[line.cursor..])
                    .collect();
                line.cursor += 1;
            }
            _ => {}
        }
    }

    fn submit(&mut self, prefix: char, text: &str) {
        if prefix == '/' {
            self.run(&Command::Jump(JumpMode::Query(text.to_string())));
            return;
        }
        match command::parse(text) {
            Ok(commands) => {
                for cmd in commands {
                    info!("command line: {cmd}");
                    self.run(&cmd);
                }
            }
            Err(e) => self.set_message(Err(e.to_string())),
        }
    }

    pub fn mouse(&mut self, event: MouseEvent) {
        let ctx = self.ctx.clone();
        let at = Position::new(event.column, event.row);
        if let Some(overlay) = self.overlays.last_mut() {
            let outcome = overlay.mouse(event, &ctx);
            self.apply(outcome, true);
            return;
        }
        let click = matches!(event.kind, MouseEventKind::Down(MouseButton::Left));
        if self.regions.player.contains(at) || self.player.dragging() {
            let outcome = self.player.mouse(event, &ctx);
            self.apply(outcome, false);
            return;
        }
        if click {
            if let Some(i) = self.regions.tabs.iter().position(|r| r.contains(at)) {
                let id = self.screens[i].id;
                if i == self.current {
                    // Clicking the current tab goes back to the screen itself.
                    self.screen().stack.clear();
                }
                self.set_screen(id);
                return;
            }
            if self.regions.back.is_some_and(|r| r.contains(at)) {
                self.back();
                return;
            }
            if let Some((_, action)) = self
                .regions
                .header_buttons
                .iter()
                .find(|(r, _)| r.contains(at))
                .copied()
            {
                let outcome = self.screen().top().header_action(action, &ctx);
                self.apply(outcome, false);
                return;
            }
        }
        if self.regions.page.contains(at) {
            let outcome = self.screen().top().mouse(event, &ctx);
            self.apply(outcome, false);
        }
    }

    /// Areas for the player and the rest, for a terminal of `size`: the hero at the top (or a
    /// left column when the window is wide), else a bar at the bottom.
    fn split(&self, area: Rect) -> (Rect, Rect, bool) {
        let wide = area.width >= 110 && area.width >= area.height * 3;
        if wide && area.height >= HERO_MIN_ROWS {
            let w = (area.width / 3).clamp(36, 60);
            return (
                Rect::new(area.x, area.y, w, area.height),
                Rect::new(area.x + w + 1, area.y, area.width - w - 1, area.height),
                true,
            );
        }
        let hero_rows = (area.height * 2 / 5).clamp(HERO_MIN_ROWS, 28);
        if area.height >= hero_rows + MIN_PAGE_ROWS {
            return (
                Rect::new(area.x, area.y, area.width, hero_rows),
                Rect::new(
                    area.x,
                    area.y + hero_rows,
                    area.width,
                    area.height - hero_rows,
                ),
                true,
            );
        }
        let bar = BAR_ROWS.min(area.height / 3);
        (
            Rect::new(area.x, area.bottom() - bar, area.width, bar),
            Rect::new(area.x, area.y, area.width, area.height - bar),
            false,
        )
    }

    pub fn draw(&mut self, frame: &mut Frame) {
        images::begin_frame();
        let ctx = self.ctx.clone();
        let styles = Styles(&ctx.palette);
        let full = frame.area();
        fill(frame.buffer_mut(), full, styles.primary());

        // Bottom rows: the command line and the last message.
        let message = self
            .message
            .as_ref()
            .filter(|(_, at)| at.elapsed() < MESSAGE_TIME)
            .map(|(m, _)| m.clone());
        let bottom_rows = u16::from(self.cmdline.is_some()) + u16::from(message.is_some());
        let main = Rect::new(
            full.x,
            full.y,
            full.width,
            full.height.saturating_sub(bottom_rows),
        );

        let (player_area, content, _hero) = self.split(main);
        self.regions = Regions::default();
        pointer::clear();
        let on_lyrics_or_cover = false;
        self.player
            .draw(frame, player_area, &ctx, self.count, !on_lyrics_or_cover);
        self.regions.player = player_area;

        // Tab row (design T1).
        let mut y = content.y;
        if content.height > 2 {
            let labels: Vec<String> = self.screens.iter().map(|s| s.label.to_string()).collect();
            self.regions.tabs = draw_tab_row(
                frame,
                Rect::new(content.x, y, content.width, 1),
                &labels,
                self.current,
                &ctx,
            );
            y += 2;
        }

        // Title row with the back button, for pages opened on top of a screen.
        let screen = &self.screens[self.current];
        let page_title = screen.top_ref().title();
        let header = screen.top_ref().header(&ctx);
        let has_stack = !screen.stack.is_empty();
        let parent = if screen.stack.len() > 1 {
            screen.stack[screen.stack.len() - 2].title()
        } else {
            screen.root.title()
        };
        let remaining = content.bottom().saturating_sub(y);
        let show_header = header.is_some() && remaining > HEADER_ROWS + MIN_ROWS_BELOW_HEADER;
        if has_stack || (!show_header && remaining > 4) {
            let buf = frame.buffer_mut();
            let mut x = content.x + 2;
            if has_stack {
                let back = format!("‹ {parent}");
                let written = put(
                    buf,
                    x,
                    y,
                    &back,
                    usize::from(content.width / 2),
                    styles.secondary(),
                );
                let back = Rect::new(x, y, written as u16, 1);
                pointer::clickable(back);
                self.regions.back = Some(back);
                x += written as u16 + 3;
            }
            if !show_header && has_stack {
                put(
                    buf,
                    x,
                    y,
                    &page_title,
                    usize::from(content.right().saturating_sub(x + 2)),
                    bold(styles.accent()),
                );
            }
            let subtitle = screen.top_ref().subtitle();
            if !subtitle.is_empty() && !show_header {
                let sx = content.right().saturating_sub(width(&subtitle) as u16 + 2);
                if sx > x + 4 {
                    put(buf, sx, y, &subtitle, usize::MAX, styles.secondary());
                }
            }
            if has_stack {
                y += 1;
            }
        }
        if show_header && let Some(header) = &header {
            let area = Rect::new(content.x, y, content.width, HEADER_ROWS);
            self.regions.header_buttons = header::draw(frame, area, header, &ctx.palette);
            for (button, _) in &self.regions.header_buttons {
                pointer::clickable(*button);
            }
            y += HEADER_ROWS;
        }

        let page_area = Rect::new(
            content.x,
            y,
            content.width,
            content.bottom().saturating_sub(y),
        );
        self.regions.page = page_area;
        let focused = self.overlays.is_empty();
        self.screens[self.current]
            .top()
            .draw(frame, page_area, &ctx, focused);

        // Command line and message.
        let mut row = main.bottom();
        if let Some(message) = message {
            let style = match &message {
                Ok(_) => styles.secondary(),
                Err(_) => styles.error(),
            };
            let text = match &message {
                Ok(text) => text.clone(),
                Err(text) => format!("Error: {text}"),
            };
            let line = Rect::new(full.x, row, full.width, 1);
            fill(frame.buffer_mut(), line, style);
            put(
                frame.buffer_mut(),
                full.x + 1,
                row,
                &text,
                usize::from(full.width - 1),
                style,
            );
            row += 1;
        }
        if let Some(line) = &self.cmdline {
            let style = ratatui::style::Style::new()
                .fg(ctx.palette.cmdline)
                .bg(ctx.palette.cmdline_bg);
            let area = Rect::new(full.x, row, full.width, 1);
            fill(frame.buffer_mut(), area, style);
            let text = format!("{}{}", line.prefix, line.text);
            put(
                frame.buffer_mut(),
                full.x,
                row,
                &truncate(&text, usize::from(full.width)),
                usize::MAX,
                style,
            );
            let cursor_x = full.x
                + 1
                + line
                    .text
                    .chars()
                    .take(line.cursor)
                    .collect::<String>()
                    .chars()
                    .count() as u16;
            frame.set_cursor_position((cursor_x.min(full.right().saturating_sub(1)), row));
            if let Some(cell) = frame.buffer_mut().cell_mut((cursor_x, row)) {
                cell.modifier.insert(Modifier::REVERSED);
            }
        }

        // Overlays, top last. An overlay takes the clicks, so what's under it isn't clickable.
        self.overlay_areas.clear();
        for overlay in &mut self.overlays {
            pointer::clear();
            let area = overlay.draw(frame, full, &ctx);
            self.overlay_areas.push(area);
        }
    }

    /// Where overlays were drawn, so images under them are left out.
    pub fn overlay_areas(&self) -> &[Rect] {
        &self.overlay_areas
    }
}

/// Complete `input` with the words from `dictionary` that start with it: the word if there's
/// one, else their common prefix, else `input` unchanged.
fn word_completion(input: &str, dictionary: &[&str]) -> String {
    let matching: Vec<&&str> = dictionary.iter().filter(|w| w.starts_with(input)).collect();
    let Some(first) = matching.first() else {
        return input.into();
    };
    let mut prefix: Vec<char> = first.chars().collect();
    for word in matching {
        let common = prefix
            .iter()
            .zip(word.chars())
            .take_while(|(a, b)| **a == *b)
            .count();
        prefix.truncate(common);
    }
    prefix.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORDS: &[&str] = &["test1", "test2", "tset"];

    #[test]
    fn test_word_completion() {
        assert_eq!(word_completion("ts", WORDS), "tset");
        assert_eq!(word_completion("Luna", WORDS), "Luna");
        assert_eq!(word_completion("te", WORDS), "test");
        assert_eq!(word_completion("t", WORDS), "t");
    }
}
