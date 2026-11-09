//! The Search screen: a text field. Enter opens the results (see [search_results]); a Spotify
//! link or URI typed in opens that item directly. `:search term` runs a search from anywhere.
//!
//! The field has the keyboard while focused (shown with an accent underline); Esc leaves it so
//! keybindings work again, and Tab leaves it and switches screens as usual. Enter, `/`, a click or
//! coming back to the screen focus it. While there's text, a ✕ at its end clears it.
//!
//! With the field empty, "Recent searches" lists the last [MAX_RECENT] searches, newest first,
//! like Spotify's apps: Enter or a click searches one again, ✕ (shown on the row under the mouse
//! and the selected one) or the delete key removes one, and Clear all (the clear key) removes them
//! all. ↓ moves from the field into the list and ↑ from its top back. A search is noted when its
//! results open (Enter, `/`, `:search`), so the same search in other words is noted once, at the
//! top; Spotify links aren't searches and aren't noted. They're kept in [RECENT_FILE].

use std::path::{Path, PathBuf};

use log::error;
use ratatui::Frame;
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::{Position, Rect};
use serde::{Deserialize, Serialize};
use unicode_width::UnicodeWidthChar;

use crate::command::{Command, InsertSource, JumpMode, MoveAmount, MoveMode, TargetMode};
use crate::spotify::UriType;
use crate::spotify_url::SpotifyUrl;
use crate::tui::hints;
use crate::tui::page::{Ctx, Outcome, Page};
use crate::tui::pages::search_results;
use crate::tui::pointer;
use crate::tui::style::{SELECTION_MARKER, Styles, bold, fill, put, scroll_to, width};
use crate::utils::InOrder;

const PLACEHOLDER: &str = "What do you want to listen to?";
const HINT: &str =
    "Enter searches songs, artists, albums, playlists and podcasts · Spotify links open directly";
const RECENT_TITLE: &str = "Recent searches";
const CLEAR_ALL: &str = " Clear all ";
const REMOVE: &str = "✕";
/// Recent searches, in the data directory.
const RECENT_FILE: &str = "recent_searches.json";
/// Recent searches kept at most.
const MAX_RECENT: usize = 10;
/// Narrowest field with a ✕ at its end to clear it.
const MIN_WIDTH_FOR_CLEAR: u16 = 12;

/// Saves of [RECENT_FILE], in the order they were made, off the interface's thread.
static SAVES: InOrder = InOrder::new("recent searches");

/// What [RECENT_FILE] holds. An object rather than a list, so more can be added later.
#[derive(Default, Serialize, Deserialize)]
struct SavedSearches {
    #[serde(default)]
    searches: Vec<String>,
}

/// The searches made, newest first, and where they're kept (nowhere in tests).
struct RecentSearches {
    queries: Vec<String>,
    path: Option<PathBuf>,
}

impl RecentSearches {
    /// What's kept at `path`, to be kept there; none if there's nothing or it can't be read.
    fn load(path: Option<PathBuf>) -> Self {
        let queries = path.as_deref().map(read_recent).unwrap_or_default();
        Self { queries, path }
    }

    /// The ones kept in the data directory, or none in tests, which mustn't touch it.
    fn load_default() -> Self {
        Self::load((!cfg!(test)).then(|| crate::config::data_path(RECENT_FILE)))
    }

    fn len(&self) -> usize {
        self.queries.len()
    }

    fn is_empty(&self) -> bool {
        self.queries.is_empty()
    }

    /// Note a search for `query`: at the top, in place of the same search in other case,
    /// [MAX_RECENT] at most.
    fn add(&mut self, query: &str) {
        let query = query.trim();
        if query.is_empty() {
            return;
        }
        let key = query.to_lowercase();
        self.queries.retain(|q| q.to_lowercase() != key);
        self.queries.insert(0, query.to_string());
        self.queries.truncate(MAX_RECENT);
        self.save();
    }

    fn remove(&mut self, index: usize) {
        if index < self.queries.len() {
            self.queries.remove(index);
            self.save();
        }
    }

    fn clear(&mut self) {
        if !self.queries.is_empty() {
            self.queries.clear();
            self.save();
        }
    }

    fn save(&self) {
        let Some(path) = self.path.clone() else {
            return;
        };
        let queries = self.queries.clone();
        SAVES.run(move || {
            if let Err(e) = write_recent(&path, &queries) {
                error!("Couldn't save recent searches in {}: {e}", path.display());
            }
        });
    }
}

/// The searches kept at `path`, newest first; none if there's no file or it can't be read.
fn read_recent(path: &Path) -> Vec<String> {
    let saved: SavedSearches = std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();
    let mut queries: Vec<String> = saved
        .searches
        .into_iter()
        .filter(|q| !q.trim().is_empty())
        .collect();
    queries.truncate(MAX_RECENT);
    queries
}

/// Keep `queries` at `path`, replacing what was there in one go.
fn write_recent(path: &Path, queries: &[String]) -> std::io::Result<()> {
    let saved = SavedSearches {
        searches: queries.to_vec(),
    };
    let json = serde_json::to_vec(&saved)?;
    crate::serialization::write_atomically(path, &json, true)
}

/// Where a recent search was drawn, for clicks.
struct RecentRow {
    index: usize,
    row: Rect,
    remove: Rect,
}

pub struct SearchPage {
    text: String,
    /// Cursor position in characters.
    cursor: usize,
    /// The first character shown, when the text is wider than the field.
    scroll: usize,
    focused: bool,
    /// Where the field was drawn last, for clicks.
    field: Rect,
    /// The ✕ at the end of the field, while there's text to clear.
    clear_text: Option<Rect>,
    recent: RecentSearches,
    /// The recent search with the keyboard, when the list has it rather than the field.
    selected: Option<usize>,
    /// The first recent search shown, when they don't all fit.
    first: usize,
    /// The recent search under the mouse, which shows its ✕.
    hovered: Option<usize>,
    /// Where the recent searches and Clear all were drawn last.
    rows: Vec<RecentRow>,
    clear_all: Option<Rect>,
}

pub fn new(_ctx: &Ctx) -> SearchPage {
    with_recent(RecentSearches::load_default())
}

fn with_recent(recent: RecentSearches) -> SearchPage {
    SearchPage {
        text: String::new(),
        cursor: 0,
        scroll: 0,
        focused: true,
        field: Rect::default(),
        clear_text: None,
        recent,
        selected: None,
        first: 0,
        hovered: None,
        rows: Vec::new(),
        clear_all: None,
    }
}

impl SearchPage {
    fn set_text(&mut self, text: &str) {
        self.text = text.to_string();
        self.cursor = self.text.chars().count();
    }

    /// Whether the recent searches are shown: with the field empty, when there are any.
    fn showing_recent(&self) -> bool {
        self.text.is_empty() && !self.recent.is_empty()
    }

    /// Give the field the keyboard, from the list.
    fn focus_field(&mut self) {
        self.selected = None;
        self.focused = true;
    }

    /// Search for `query`: open its results, or the item a Spotify link points to.
    fn search(&mut self, query: &str, ctx: &Ctx) -> Outcome {
        let query = query.trim();
        if query.is_empty() {
            self.focus_field();
            return Outcome::Consumed;
        }
        self.focused = false;
        self.selected = None;
        match parse_link(query) {
            // Opened like `:insert`, by the global actions.
            Some(url) => Outcome::Run(vec![Command::Insert(InsertSource::Input(url))]),
            None => {
                self.recent.add(query);
                Outcome::Open(Box::new(search_results::new(query, ctx)))
            }
        }
    }

    /// Search for the recent search at `index` again.
    fn search_recent(&mut self, index: usize, ctx: &Ctx) -> Outcome {
        let Some(query) = self.recent.queries.get(index).cloned() else {
            return Outcome::Consumed;
        };
        self.set_text(&query);
        self.search(&query, ctx)
    }

    /// Remove the recent search at `index`, keeping the selection on the one after it (or before,
    /// if it was the last); back to the field once there are none.
    fn remove_recent(&mut self, index: usize) {
        self.recent.remove(index);
        self.hovered = None;
        if self.recent.is_empty() {
            if self.selected.is_some() {
                self.focus_field();
            }
        } else if let Some(selected) = self.selected {
            self.selected = Some(selected.min(self.recent.len() - 1));
        }
    }

    fn clear_recent(&mut self) {
        self.recent.clear();
        self.hovered = None;
        if self.selected.is_some() {
            self.focus_field();
        }
    }

    /// Move the selection in the recent searches: down from the field into them, up from the top
    /// one back to the field.
    fn move_selection(&mut self, mode: &MoveMode, amount: &MoveAmount) -> bool {
        if !self.showing_recent() {
            return false;
        }
        let last = self.recent.len() - 1;
        let steps = match amount {
            MoveAmount::Integer(n) => n.unsigned_abs().max(1) as usize,
            MoveAmount::Float(_) | MoveAmount::Extreme => usize::MAX,
        };
        match (mode, self.selected) {
            (MoveMode::Down, None) => {
                self.focused = false;
                self.selected = Some(0);
            }
            (MoveMode::Down, Some(i)) => self.selected = Some(i.saturating_add(steps).min(last)),
            (MoveMode::Up, Some(0)) => self.focus_field(),
            (MoveMode::Up, Some(i)) => self.selected = Some(i.saturating_sub(steps)),
            _ => return false,
        }
        true
    }

    /// A key while the recent searches have the keyboard. Keys it leaves go to the keybindings
    /// (the delete and clear keys among them).
    fn list_key(&mut self, key: KeyEvent, index: usize, ctx: &Ctx) -> Outcome {
        match key.code {
            KeyCode::Up => {
                self.move_selection(&MoveMode::Up, &MoveAmount::Integer(1));
            }
            KeyCode::Down => {
                self.move_selection(&MoveMode::Down, &MoveAmount::Integer(1));
            }
            KeyCode::Home => self.selected = Some(0),
            KeyCode::End => self.selected = Some(self.recent.len().saturating_sub(1)),
            KeyCode::Enter => return self.search_recent(index, ctx),
            KeyCode::Delete => self.remove_recent(index),
            KeyCode::Esc => self.focus_field(),
            KeyCode::Tab | KeyCode::BackTab => {
                self.selected = None;
                return Outcome::Ignored;
            }
            _ => return Outcome::Ignored,
        }
        Outcome::Consumed
    }

    fn edit(&mut self, key: KeyEvent) -> bool {
        let mut chars: Vec<char> = self.text.chars().collect();
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('u') if ctrl => {
                chars.drain(..self.cursor);
                self.cursor = 0;
            }
            KeyCode::Char(c) if !ctrl && !key.modifiers.contains(KeyModifiers::ALT) => {
                chars.insert(self.cursor, c);
                self.cursor += 1;
            }
            KeyCode::Backspace if self.cursor > 0 => {
                self.cursor -= 1;
                chars.remove(self.cursor);
            }
            KeyCode::Delete if self.cursor < chars.len() => {
                chars.remove(self.cursor);
            }
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => self.cursor = (self.cursor + 1).min(chars.len()),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = chars.len(),
            KeyCode::Backspace | KeyCode::Delete => {}
            _ => return false,
        }
        self.text = chars.into_iter().collect();
        true
    }

    /// Draw "Recent searches" from row `y` down to the bottom of `area`, at `x`, `room` columns
    /// wide. Its keys are labelled unless `typing` (the field has them).
    fn draw_recent(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        (x, y, room): (u16, u16, u16),
        typing: bool,
        ctx: &Ctx,
    ) {
        let styles = Styles(&ctx.palette);
        let buf = frame.buffer_mut();
        let room_width = usize::from(room);

        // The heading, with Clear all at the right when both fit.
        let button_width = width(CLEAR_ALL) as u16;
        let title_width = width(RECENT_TITLE) as u16;
        let mut heading_width = room_width;
        if room >= title_width.min(8) + 1 + button_width {
            let bx = x + room - button_width;
            put(buf, bx, y, CLEAR_ALL, usize::MAX, styles.primary());
            let rect = Rect::new(bx, y, button_width, 1);
            pointer::clickable(rect);
            if !typing {
                hints::add(rect, Command::Clear);
            }
            self.clear_all = Some(rect);
            heading_width = usize::from(bx.saturating_sub(x + 1));
        }
        put(
            buf,
            x,
            y,
            RECENT_TITLE,
            heading_width,
            bold(styles.accent()),
        );

        // The searches, one a row, scrolled to keep the selected one in view.
        let top = y + 1;
        let visible = usize::from(area.bottom().saturating_sub(top));
        if visible == 0 {
            return;
        }
        let len = self.recent.len();
        if let Some(selected) = self.selected {
            self.first = scroll_to(self.first, selected, visible);
        }
        self.first = self.first.min(len.saturating_sub(visible));
        // A ✕ at the end when there's room for it beside some text.
        let remove_width = if room >= 6 { width(REMOVE) as u16 } else { 0 };
        let text_width = usize::from(room.saturating_sub(remove_width + 1).max(1));
        for (row, index) in (self.first..len).take(visible).enumerate() {
            let ry = top + row as u16;
            let line = Rect::new(x - 1, ry, room + 1, 1);
            let remove = Rect::new(x + room - remove_width, ry, remove_width, 1);
            let selected = self.selected == Some(index);
            let style = if selected {
                styles.selected()
            } else {
                styles.primary()
            };
            if selected {
                fill(buf, line, style);
                put(buf, x - 1, ry, SELECTION_MARKER, 1, styles.marker());
            }
            pointer::clickable(line);
            put(buf, x, ry, &self.recent.queries[index], text_width, style);
            if remove_width > 0 && (selected || self.hovered == Some(index)) {
                let remove_style = if selected {
                    styles.selected_inactive()
                } else {
                    styles.secondary()
                };
                put(buf, remove.x, ry, REMOVE, usize::MAX, remove_style);
                if selected {
                    hints::add(remove, Command::Delete);
                }
            }
            if selected {
                let text = Rect::new(line.x, ry, line.width.saturating_sub(remove_width + 1), 1);
                hints::add_row(text, vec![Command::Play]);
            }
            self.rows.push(RecentRow {
                index,
                row: line,
                remove,
            });
        }
    }
}

/// Columns `chars` take on screen.
fn columns(chars: &[char]) -> usize {
    chars.iter().map(|c| c.width().unwrap_or(0)).sum()
}

/// The Spotify item `query` links to, if it's a `spotify:` URI or an open.spotify.com link.
fn parse_link(query: &str) -> Option<SpotifyUrl> {
    if let Ok(uri_type) = query.parse::<UriType>() {
        // spotify:album:ID, or spotify:user:NAME:playlist:ID
        let id = query.rsplit(':').next().filter(|id| !id.is_empty())?;
        return Some(SpotifyUrl::new(id, uri_type));
    }
    SpotifyUrl::from_url(query)
}

impl Page for SearchPage {
    fn title(&self) -> String {
        "Search".into()
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect, ctx: &Ctx, focused: bool) {
        self.rows.clear();
        self.clear_all = None;
        self.clear_text = None;
        if !self.showing_recent() {
            self.selected = None;
            self.hovered = None;
        }
        if let Some(selected) = self.selected {
            self.selected = Some(selected.min(self.recent.len().saturating_sub(1)));
        }
        if area.height < 2 || area.width < 6 {
            return;
        }
        let styles = Styles(&ctx.palette);
        let x = area.x + 2;
        let room = area.width.saturating_sub(4);
        self.field = Rect::new(x, area.y, room, 2);
        pointer::text(self.field);

        // A ✕ at the end of the field clears it, while there's text.
        let mut text_room = room;
        if !self.text.is_empty() && room >= MIN_WIDTH_FOR_CLEAR {
            let w = width(REMOVE) as u16;
            let rect = Rect::new(x + room - w, area.y, w, 1);
            put(
                frame.buffer_mut(),
                rect.x,
                rect.y,
                REMOVE,
                usize::MAX,
                styles.secondary(),
            );
            pointer::clickable(rect);
            self.clear_text = Some(rect);
            text_room = room - w - 1;
        }
        let buf = frame.buffer_mut();

        // The text, scrolled so the cursor stays in view, with a column for it after the text.
        let chars: Vec<char> = self.text.chars().collect();
        self.cursor = self.cursor.min(chars.len());
        let visible = usize::from(text_room.saturating_sub(1));
        self.scroll = self.scroll.min(self.cursor);
        while columns(&chars[self.scroll..self.cursor]) > visible {
            self.scroll += 1;
        }
        let skip = self.scroll;
        if chars.is_empty() {
            put(
                buf,
                x,
                area.y,
                PLACEHOLDER,
                usize::from(room),
                styles.secondary(),
            );
        } else {
            let shown: String = chars[skip..].iter().collect();
            put(
                buf,
                x,
                area.y,
                &shown,
                usize::from(text_room),
                styles.primary(),
            );
        }

        // An underline instead of a box (V1): in the accent while the field has the keyboard.
        let active = self.focused && focused;
        let line_style = if active {
            styles.accent()
        } else {
            styles.secondary()
        };
        put(
            buf,
            x,
            area.y + 1,
            &"─".repeat(usize::from(room)),
            usize::from(room),
            line_style,
        );

        // The hint, then the recent searches; in short windows the hint makes way for them.
        let recent = self.showing_recent();
        let (hint_row, recent_row) = if recent && area.height < 8 {
            (None, 3)
        } else {
            (Some(3), 5)
        };
        if let Some(row) = hint_row
            && area.height > row
        {
            let hint = if active {
                HINT
            } else if self.selected.is_some() {
                "↑ from the top goes back to the search field"
            } else {
                "Press Enter or click to type a search"
            };
            put(
                buf,
                x,
                area.y + row,
                hint,
                usize::from(room),
                styles.secondary(),
            );
        }
        if recent && area.height > recent_row + 1 {
            self.draw_recent(frame, area, (x, area.y + recent_row, room), active, ctx);
        }

        if active {
            let before = columns(&chars[skip..self.cursor]) as u16;
            let cursor_x = (x + before).min(x + text_room.saturating_sub(1));
            frame.set_cursor_position((cursor_x, area.y));
        }
    }

    fn command(&mut self, cmd: &Command, ctx: &Ctx) -> Outcome {
        match cmd {
            Command::Search(term) => {
                self.set_text(term);
                self.search(term, ctx)
            }
            // `/` opens the app's search prompt: what's typed there is searched for.
            Command::Jump(JumpMode::Query(query)) => {
                if !query.is_empty() {
                    self.set_text(query);
                }
                self.search(query, ctx)
            }
            Command::Play | Command::Open(TargetMode::Selected) => match self.selected {
                Some(index) => self.search_recent(index, ctx),
                None => {
                    self.focus_field();
                    Outcome::Consumed
                }
            },
            Command::Move(mode, amount) if self.move_selection(mode, amount) => Outcome::Consumed,
            Command::Delete if self.showing_recent() => {
                if let Some(index) = self.selected {
                    self.remove_recent(index);
                }
                Outcome::Consumed
            }
            // Clear all, rather than the queue, while the recent searches are shown.
            Command::Clear if self.showing_recent() => {
                self.clear_recent();
                Outcome::Consumed
            }
            _ => Outcome::Ignored,
        }
    }

    fn mouse(&mut self, event: MouseEvent, ctx: &Ctx) -> Outcome {
        let at = Position::new(event.column, event.row);
        let row_at = |rows: &[RecentRow]| rows.iter().find(|r| r.row.contains(at)).map(|r| r.index);
        match event.kind {
            MouseEventKind::Moved => {
                self.hovered = row_at(&self.rows);
                Outcome::Ignored
            }
            MouseEventKind::ScrollDown if !self.rows.is_empty() => {
                let last_first = self.recent.len().saturating_sub(self.rows.len());
                self.first = (self.first + 1).min(last_first);
                if let Some(selected) = self.selected {
                    self.selected = Some(selected.max(self.first));
                }
                Outcome::Consumed
            }
            MouseEventKind::ScrollUp if !self.rows.is_empty() => {
                self.first = self.first.saturating_sub(1);
                if let Some(selected) = self.selected {
                    self.selected = Some(selected.min(self.first + self.rows.len() - 1));
                }
                Outcome::Consumed
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if self.clear_text.is_some_and(|r| r.contains(at)) {
                    self.set_text("");
                    self.focus_field();
                    return Outcome::Consumed;
                }
                if self.clear_all.is_some_and(|r| r.contains(at)) {
                    self.clear_recent();
                    return Outcome::Consumed;
                }
                if let Some(row) = self.rows.iter().find(|r| r.row.contains(at)) {
                    let index = row.index;
                    if row.remove.contains(at) {
                        self.remove_recent(index);
                        return Outcome::Consumed;
                    }
                    return self.search_recent(index, ctx);
                }
                self.selected = None;
                self.focused = self.field.contains(at);
                if self.focused {
                    // Before the character clicked, counting columns from the first one shown.
                    let column = usize::from(event.column.saturating_sub(self.field.x));
                    let chars: Vec<char> = self.text.chars().collect();
                    let mut used = 0;
                    self.cursor = chars.len();
                    for (i, c) in chars.iter().enumerate().skip(self.scroll) {
                        used += c.width().unwrap_or(0);
                        if used > column {
                            self.cursor = i;
                            break;
                        }
                    }
                }
                Outcome::Consumed
            }
            _ => Outcome::Ignored,
        }
    }

    fn captures_text(&self) -> bool {
        // The recent searches take the arrows, Enter and Delete too (see [SearchPage::list_key]).
        self.focused || self.selected.is_some()
    }

    fn key(&mut self, key: KeyEvent, ctx: &Ctx) -> Outcome {
        if let Some(index) = self.selected {
            return self.list_key(key, index, ctx);
        }
        match key.code {
            KeyCode::Enter => {
                let query = self.text.clone();
                self.search(&query, ctx)
            }
            KeyCode::Esc => {
                self.focused = false;
                Outcome::Consumed
            }
            // Leave the field and let the key bindings have Tab, so it switches screens in one
            // press like everywhere else.
            KeyCode::Tab | KeyCode::BackTab => {
                self.focused = false;
                Outcome::Ignored
            }
            KeyCode::Down if self.move_selection(&MoveMode::Down, &MoveAmount::Integer(1)) => {
                Outcome::Consumed
            }
            _ if self.edit(key) => Outcome::Consumed,
            _ => Outcome::Ignored,
        }
    }

    fn on_show(&mut self, _ctx: &Ctx) {
        self.focus_field();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_link() {
        let uri = parse_link("spotify:album:abc123").unwrap();
        assert_eq!(uri.uri_type, UriType::Album);
        assert_eq!(uri.id, "abc123");
        let playlist = parse_link("spotify:user:someone:playlist:xyz").unwrap();
        assert_eq!(playlist.uri_type, UriType::Playlist);
        assert_eq!(playlist.id, "xyz");
        let url = parse_link("https://open.spotify.com/track/4uLU6hMCjMI75M1A2tKUQC").unwrap();
        assert_eq!(url.uri_type, UriType::Track);
        assert_eq!(url.id, "4uLU6hMCjMI75M1A2tKUQC");
        assert!(parse_link("daft punk").is_none());
        assert!(parse_link("spotify:track:").is_none());
    }

    #[test]
    fn test_edit() {
        let mut page = with_recent(RecentSearches::load(None));
        let key = |code| KeyEvent::new(code, KeyModifiers::NONE);
        for c in "abd".chars() {
            assert!(page.edit(key(KeyCode::Char(c))));
        }
        page.edit(key(KeyCode::Left));
        page.edit(key(KeyCode::Char('c')));
        assert_eq!(page.text, "abcd");
        page.edit(key(KeyCode::Backspace));
        assert_eq!(page.text, "abd");
        page.edit(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        assert_eq!(page.text, "d");
        assert!(!page.edit(key(KeyCode::Up)));
    }

    fn ctx() -> Ctx {
        crate::tui::tests::app().ctx.clone()
    }

    /// A page with `queries` as its recent searches (newest first), kept nowhere.
    fn page_with(queries: &[&str]) -> SearchPage {
        let mut recent = RecentSearches::load(None);
        recent.queries = queries.iter().map(|q| q.to_string()).collect();
        with_recent(recent)
    }

    fn screen_text(terminal: &ratatui::Terminal<ratatui::backend::TestBackend>) -> String {
        let buffer = terminal.backend().buffer();
        let mut text = String::new();
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                text.push_str(buffer[(x, y)].symbol());
            }
            text.push('\n');
        }
        text
    }

    fn draw(
        page: &mut SearchPage,
        terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>,
        ctx: &Ctx,
    ) -> String {
        terminal
            .draw(|frame| page.draw(frame, frame.area(), ctx, true))
            .unwrap();
        screen_text(terminal)
    }

    fn terminal(w: u16, h: u16) -> ratatui::Terminal<ratatui::backend::TestBackend> {
        ratatui::Terminal::new(ratatui::backend::TestBackend::new(w, h)).unwrap()
    }

    /// Where `text` was drawn: the column and row of its first character.
    fn find(screen: &str, text: &str) -> Option<(u16, u16)> {
        screen.lines().enumerate().find_map(|(row, line)| {
            let byte = line.find(text)?;
            Some((line[..byte].chars().count() as u16, row as u16))
        })
    }

    /// Row `row` of `screen`.
    fn line(screen: &str, row: u16) -> &str {
        screen.lines().nth(usize::from(row)).unwrap_or_default()
    }

    fn mouse(kind: MouseEventKind, (column, row): (u16, u16)) -> MouseEvent {
        MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    fn click(at: (u16, u16)) -> MouseEvent {
        mouse(MouseEventKind::Down(MouseButton::Left), at)
    }

    fn press(page: &mut SearchPage, code: KeyCode, ctx: &Ctx) -> Outcome {
        page.key(KeyEvent::new(code, KeyModifiers::NONE), ctx)
    }

    #[test]
    fn test_empty_field_shows_recent_searches() {
        let ctx = ctx();
        let mut terminal = terminal(60, 16);
        let mut page = page_with(&[]);
        let screen = draw(&mut page, &mut terminal, &ctx);
        assert!(!screen.contains(RECENT_TITLE), "{screen}");

        let mut page = page_with(&["radiohead", "daft punk"]);
        let screen = draw(&mut page, &mut terminal, &ctx);
        assert!(screen.contains(PLACEHOLDER), "{screen}");
        assert!(screen.contains(RECENT_TITLE), "{screen}");
        assert!(screen.contains("Clear all"), "{screen}");
        let newest = find(&screen, "radiohead").unwrap();
        let older = find(&screen, "daft punk").unwrap();
        assert!(newest.1 < older.1, "newest first:\n{screen}");

        // Typing hides them.
        page.edit(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
        let screen = draw(&mut page, &mut terminal, &ctx);
        assert!(!screen.contains(RECENT_TITLE), "{screen}");
    }

    #[test]
    fn test_searches_are_noted_once_newest_first() {
        let ctx = ctx();
        let mut page = page_with(&[]);
        assert!(matches!(
            page.command(&Command::Search("Daft Punk".into()), &ctx),
            Outcome::Open(_)
        ));
        // Typed, with Enter: noted when the results open, not as it's typed.
        page.set_text("");
        page.focus_field();
        for c in " radiohead ".chars() {
            press(&mut page, KeyCode::Char(c), &ctx);
            assert_eq!(page.recent.queries, ["Daft Punk"]);
        }
        assert!(matches!(
            press(&mut page, KeyCode::Enter, &ctx),
            Outcome::Open(_)
        ));
        assert_eq!(page.recent.queries, ["radiohead", "Daft Punk"]);
        // The same search in other case moves to the top, as typed this time.
        page.command(&Command::Jump(JumpMode::Query("daft punk".into())), &ctx);
        assert_eq!(page.recent.queries, ["daft punk", "radiohead"]);
        // Links and empty searches aren't searches.
        page.command(&Command::Search("spotify:album:abc123".into()), &ctx);
        page.command(&Command::Search("   ".into()), &ctx);
        assert_eq!(page.recent.queries, ["daft punk", "radiohead"]);
        // Only the last ones are kept.
        for n in 0..MAX_RECENT + 5 {
            page.command(&Command::Search(format!("search {n}")), &ctx);
        }
        assert_eq!(page.recent.len(), MAX_RECENT);
        assert_eq!(page.recent.queries[0], format!("search {}", MAX_RECENT + 4));
    }

    #[test]
    fn test_clicking_a_recent_search_searches_it() {
        let ctx = ctx();
        let mut terminal = terminal(60, 16);
        let mut page = page_with(&["radiohead", "daft punk"]);
        let screen = draw(&mut page, &mut terminal, &ctx);
        let row = find(&screen, "daft punk").unwrap();
        assert!(matches!(page.mouse(click(row), &ctx), Outcome::Open(_)));
        assert_eq!(page.text, "daft punk");
        assert_eq!(page.recent.queries, ["daft punk", "radiohead"]);

        // The ✕ in the field clears it, which brings them back.
        let screen = draw(&mut page, &mut terminal, &ctx);
        assert!(!screen.contains(RECENT_TITLE), "{screen}");
        let clear = page.clear_text.expect("a ✕ to clear the field");
        page.mouse(click((clear.x, clear.y)), &ctx);
        assert!(page.text.is_empty() && page.focused);
        let screen = draw(&mut page, &mut terminal, &ctx);
        assert!(screen.contains(RECENT_TITLE), "{screen}");
    }

    #[test]
    fn test_arrows_move_between_the_field_and_recent_searches() {
        let ctx = ctx();
        let mut terminal = terminal(60, 16);
        let mut page = page_with(&["radiohead", "daft punk"]);
        draw(&mut page, &mut terminal, &ctx);
        press(&mut page, KeyCode::Down, &ctx);
        assert_eq!(page.selected, Some(0));
        assert!(!page.focused && page.captures_text());
        let screen = draw(&mut page, &mut terminal, &ctx);
        let (_, row) = find(&screen, "radiohead").unwrap();
        assert!(line(&screen, row).contains(SELECTION_MARKER), "{screen}");
        assert!(line(&screen, row).contains(REMOVE), "{screen}");
        // Down stops at the last; the keybindings' moves do the same.
        press(&mut page, KeyCode::Down, &ctx);
        press(&mut page, KeyCode::Down, &ctx);
        assert_eq!(page.selected, Some(1));
        page.command(&Command::Move(MoveMode::Up, MoveAmount::Integer(1)), &ctx);
        assert_eq!(page.selected, Some(0));
        // Up from the top goes back to the field.
        press(&mut page, KeyCode::Up, &ctx);
        assert_eq!(page.selected, None);
        assert!(page.focused);
        // Enter on one searches it.
        press(&mut page, KeyCode::Down, &ctx);
        press(&mut page, KeyCode::Down, &ctx);
        assert!(matches!(
            press(&mut page, KeyCode::Enter, &ctx),
            Outcome::Open(_)
        ));
        assert_eq!(page.text, "daft punk");
        assert_eq!(page.recent.queries, ["daft punk", "radiohead"]);
        // With text in the field, Down isn't for the list.
        page.on_show(&ctx);
        press(&mut page, KeyCode::Down, &ctx);
        assert_eq!(page.selected, None);
    }

    #[test]
    fn test_removing_and_clearing_recent_searches() {
        let ctx = ctx();
        let mut terminal = terminal(60, 16);
        let mut page = page_with(&["one", "two", "three", "four"]);
        draw(&mut page, &mut terminal, &ctx);

        // The delete key on the selected one, which moves to the next.
        press(&mut page, KeyCode::Down, &ctx);
        press(&mut page, KeyCode::Down, &ctx);
        press(&mut page, KeyCode::Delete, &ctx);
        assert_eq!(page.recent.queries, ["one", "three", "four"]);
        assert_eq!(page.selected, Some(1));
        // Keys the list leaves go to the keybindings: `d` deletes.
        assert!(press(&mut page, KeyCode::Char('d'), &ctx).is_ignored());
        page.command(&Command::Delete, &ctx);
        assert_eq!(page.recent.queries, ["one", "four"]);

        // The ✕ shows on the row under the mouse, and a click on it removes that one.
        page.focus_field();
        let screen = draw(&mut page, &mut terminal, &ctx);
        let (_, row) = find(&screen, "four").unwrap();
        assert!(!line(&screen, row).contains(REMOVE), "{screen}");
        page.mouse(mouse(MouseEventKind::Moved, (5, row)), &ctx);
        let screen = draw(&mut page, &mut terminal, &ctx);
        let remove = find(&screen, REMOVE).unwrap();
        assert_eq!(remove.1, row, "{screen}");
        page.mouse(click(remove), &ctx);
        assert_eq!(page.recent.queries, ["one"]);
        assert_eq!(page.text, "", "removing doesn't search");

        // Clear all, with a click or the clear key.
        let screen = draw(&mut page, &mut terminal, &ctx);
        page.mouse(click(find(&screen, "Clear all").unwrap()), &ctx);
        assert!(page.recent.is_empty());
        let screen = draw(&mut page, &mut terminal, &ctx);
        assert!(!screen.contains(RECENT_TITLE), "{screen}");
        let mut page = page_with(&["one", "two"]);
        assert!(matches!(
            page.command(&Command::Clear, &ctx),
            Outcome::Consumed
        ));
        assert!(page.recent.is_empty());
        // Without recent searches, the clear key is the queue's again.
        assert!(page.command(&Command::Clear, &ctx).is_ignored());
    }

    #[test]
    fn test_recent_searches_are_kept_across_restarts() {
        let dir = std::env::temp_dir().join(format!("ttunes-recent-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(RECENT_FILE);

        // Nothing saved yet, or something unreadable: none.
        assert!(RecentSearches::load(Some(path.clone())).is_empty());
        std::fs::write(&path, "not json").unwrap();
        assert!(RecentSearches::load(Some(path.clone())).is_empty());

        let mut recent = RecentSearches::load(Some(path.clone()));
        recent.add("daft punk");
        recent.add("日本語の歌");
        recent.add("radiohead");
        recent.remove(2);
        // Saved in the background, in order: wait for the saves so far.
        let (done, saved) = std::sync::mpsc::channel();
        SAVES.run(move || done.send(()).unwrap());
        saved.recv().unwrap();
        let loaded = RecentSearches::load(Some(path.clone()));
        assert_eq!(loaded.queries, ["radiohead", "日本語の歌"]);

        write_recent(&path, &[]).unwrap();
        assert!(read_recent(&path).is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn test_tiny_windows_and_wide_text_dont_panic() {
        let ctx = ctx();
        let long = "日本語の歌を探しています".repeat(4);
        for (w, h) in (0..24).flat_map(|w| (0..12).map(move |h| (w, h))) {
            let mut terminal = terminal(w, h);
            for selected in [None, Some(0), Some(2)] {
                let mut page = page_with(&[&long, "two", "三", "four"]);
                page.selected = selected;
                page.focused = selected.is_none();
                page.hovered = Some(1);
                let screen = draw(&mut page, &mut terminal, &ctx);
                if w >= 20 && h >= 7 {
                    assert!(screen.contains(RECENT_TITLE), "{w}x{h}:\n{screen}");
                }
                for event in [
                    mouse(MouseEventKind::ScrollDown, (3, h.saturating_sub(1))),
                    mouse(MouseEventKind::ScrollUp, (3, h.saturating_sub(1))),
                    click((w.saturating_sub(3), h.saturating_sub(1))),
                    click((w / 2, h / 2)),
                ] {
                    page.mouse(event, &ctx);
                    draw(&mut page, &mut terminal, &ctx);
                }
            }
        }
        // Wide text is cut to fit, before the ✕.
        let mut terminal = terminal(30, 12);
        let mut page = page_with(&[&long]);
        let screen = draw(&mut page, &mut terminal, &ctx);
        let (_, row) = find(&screen, "日").unwrap();
        page.mouse(mouse(MouseEventKind::Moved, (3, row)), &ctx);
        let screen = draw(&mut page, &mut terminal, &ctx);
        let shown = line(&screen, row);
        assert!(shown.contains('…') && shown.contains(REMOVE), "{screen}");
    }

    #[test]
    fn test_recent_searches_through_the_app() {
        use crate::command::ScreenId;
        use crate::tui::app::App;

        let mut app = crate::tui::tests::app();
        let mut terminal = terminal(80, 50);
        let key = |app: &mut App, code| app.key(KeyEvent::new(code, KeyModifiers::NONE));
        let mut screen = |app: &mut App| {
            terminal.draw(|frame| app.draw(frame)).unwrap();
            screen_text(&terminal)
        };
        app.run(&Command::Search("first search".into()));
        app.run(&Command::Search("second search".into()));
        app.run(&Command::Back);
        assert_eq!(app.current_screen(), ScreenId::Search);
        // Back on Search, the field has the last search; emptied, the recent ones show.
        app.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        let text = screen(&mut app);
        let second = find(&text, "second search").expect(&text);
        let first = find(&text, "first search").expect(&text);
        assert!(second.1 < first.1, "{text}");
        // ↓ into the list, then `d` (bound to delete) removes the selected one.
        key(&mut app, KeyCode::Down);
        key(&mut app, KeyCode::Char('d'));
        let text = screen(&mut app);
        assert!(!text.contains("second search"), "{text}");
        assert!(text.contains("first search"), "{text}");
        // `c` clears them all, rather than the queue.
        key(&mut app, KeyCode::Esc);
        key(&mut app, KeyCode::Esc);
        key(&mut app, KeyCode::Char('c'));
        let text = screen(&mut app);
        assert!(!text.contains(RECENT_TITLE), "{text}");
        assert!(!text.contains("Clear the queue"), "{text}");
    }

    #[test]
    fn test_wide_text_scrolls_and_clicks_by_column() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let ctx = crate::tui::tests::app().ctx.clone();
        let mut page = new(&ctx);
        // Ten characters two columns wide each, in a field ten columns wide.
        for c in "日本語の歌を探しています".chars() {
            page.edit(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
        let mut terminal = Terminal::new(TestBackend::new(14, 4)).unwrap();
        let mut draw = |page: &mut SearchPage| {
            terminal
                .draw(|frame| page.draw(frame, frame.area(), &ctx, true))
                .unwrap();
            let buffer = terminal.backend().buffer().clone();
            // Wide characters leave a blank in the cell after them.
            let row: String = (0..14).map(|x| buffer[(x, 0)].symbol()).collect();
            let row = row.replace(" ", "");
            let cursor = terminal.get_cursor_position().unwrap();
            (row, cursor.x)
        };
        // What was typed last shows, with the cursor after it.
        let (row, cursor) = draw(&mut page);
        assert!(row.contains("います"), "{row:?}");
        assert_eq!(cursor, 2 + 8, "{row:?}");
        // A click on the second character shown puts the cursor before it.
        let first_shown = page.scroll;
        page.mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 2 + 3,
                row: 0,
                modifiers: KeyModifiers::NONE,
            },
            &ctx,
        );
        assert_eq!(page.cursor, first_shown + 1);
        let (_, cursor) = draw(&mut page);
        assert_eq!(cursor, 2 + 2);
        // Home scrolls back to the start.
        page.edit(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));
        let (row, cursor) = draw(&mut page);
        assert!(row.contains("日本語"), "{row:?}");
        assert_eq!(cursor, 2);
    }
}
