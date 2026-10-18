//! The Search screen: a text field. Enter opens the results (see [search_results]); a Spotify
//! link or URI typed in opens that item directly. `:search term` runs a search from anywhere.
//!
//! The field has the keyboard while focused (shown with an accent underline); Esc leaves it so
//! keybindings work again, and Tab leaves it and switches screens as usual. Enter, `/`, a click or
//! coming back to the screen focus it.

use ratatui::Frame;
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::{Position, Rect};

use crate::command::{Command, InsertSource, JumpMode, TargetMode};
use crate::spotify::UriType;
use crate::spotify_url::SpotifyUrl;
use crate::tui::page::{Ctx, Outcome, Page};
use crate::tui::pages::search_results;
use crate::tui::style::{Styles, put, width};

const PLACEHOLDER: &str = "What do you want to listen to?";
const HINT: &str =
    "Enter searches songs, artists, albums, playlists and podcasts · Spotify links open directly";

pub struct SearchPage {
    text: String,
    /// Cursor position in characters.
    cursor: usize,
    focused: bool,
    /// Where the field was drawn last, for clicks.
    field: Rect,
}

pub fn new(_ctx: &Ctx) -> SearchPage {
    SearchPage {
        text: String::new(),
        cursor: 0,
        focused: true,
        field: Rect::default(),
    }
}

impl SearchPage {
    fn set_text(&mut self, text: &str) {
        self.text = text.to_string();
        self.cursor = self.text.chars().count();
    }

    /// Search for `query`: open its results, or the item a Spotify link points to.
    fn search(&mut self, query: &str, ctx: &Ctx) -> Outcome {
        let query = query.trim();
        if query.is_empty() {
            self.focused = true;
            return Outcome::Consumed;
        }
        self.focused = false;
        match parse_link(query) {
            // Opened like `:insert`, by the global actions.
            Some(url) => Outcome::Run(vec![Command::Insert(InsertSource::Input(url))]),
            None => Outcome::Open(Box::new(search_results::new(query, ctx))),
        }
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
        if area.height < 2 || area.width < 6 {
            return;
        }
        let styles = Styles(&ctx.palette);
        let x = area.x + 2;
        let room = area.width.saturating_sub(4);
        self.field = Rect::new(x, area.y, room, 2);
        let buf = frame.buffer_mut();

        // The text, scrolled so the cursor stays in view.
        let chars: Vec<char> = self.text.chars().collect();
        let visible = usize::from(room.saturating_sub(1));
        let skip = self.cursor.saturating_sub(visible);
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
            put(buf, x, area.y, &shown, usize::from(room), styles.primary());
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

        if area.height > 3 {
            let hint = if active {
                HINT.to_string()
            } else {
                "Press Enter or click to type a search".to_string()
            };
            put(
                buf,
                x,
                area.y + 3,
                &hint,
                usize::from(room),
                styles.secondary(),
            );
        }

        if active {
            let before: String = chars[skip..self.cursor].iter().collect();
            let cursor_x = (x + width(&before) as u16).min(x + room.saturating_sub(1));
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
            Command::Play | Command::Open(TargetMode::Selected) => {
                self.focused = true;
                Outcome::Consumed
            }
            _ => Outcome::Ignored,
        }
    }

    fn mouse(&mut self, event: MouseEvent, _ctx: &Ctx) -> Outcome {
        if let MouseEventKind::Down(MouseButton::Left) = event.kind {
            self.focused = self.field.contains(Position::new(event.column, event.row));
            if self.focused {
                let column = usize::from(event.column.saturating_sub(self.field.x));
                self.cursor = column.min(self.text.chars().count());
            }
            return Outcome::Consumed;
        }
        Outcome::Ignored
    }

    fn captures_text(&self) -> bool {
        self.focused
    }

    fn key(&mut self, key: KeyEvent, ctx: &Ctx) -> Outcome {
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
            _ if self.edit(key) => Outcome::Consumed,
            _ => Outcome::Ignored,
        }
    }

    fn on_show(&mut self, _ctx: &Ctx) {
        self.focused = true;
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
        let mut page = SearchPage {
            text: String::new(),
            cursor: 0,
            focused: true,
            field: Rect::default(),
        };
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
}
