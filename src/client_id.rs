//! Where the client ID for Web API requests comes from. ttunes has no client ID of its own: each
//! user registers a Spotify app, and the first start asks for its ID once and saves it.
//!
//! The ID is taken from, in order: `client_id` in the configuration file, the saved `client_id`
//! file in the configuration directory, or the setup screen.

use std::fs;
use std::io::{self, IsTerminal};

use ratatui::Frame;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{
    self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers,
};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use crate::authentication::WEB_API_REDIRECT_URI;
use crate::config;

/// Name of the file in the configuration directory that holds the saved client ID.
const FILE_NAME: &str = "client_id";

const DASHBOARD_URL: &str = "https://developer.spotify.com/dashboard";

/// Longest input the setup screen accepts; IDs are 32 characters, this leaves room for typos.
const MAX_INPUT: usize = 64;

/// Spotify client IDs are 32 hexadecimal characters.
pub fn is_valid(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Pick the client ID from the configuration file's `client_id` or the saved file, in that order.
/// An invalid configured ID is an error, an invalid saved one is ignored (setup asks again).
fn resolve(configured: Option<&str>, saved: Option<&str>) -> Result<Option<String>, String> {
    if let Some(id) = configured.map(str::trim).filter(|id| !id.is_empty()) {
        return if is_valid(id) {
            Ok(Some(id.to_owned()))
        } else {
            Err(format!(
                "client_id \"{id}\" in the configuration file isn't a Spotify client ID \
                 (32 hexadecimal characters)"
            ))
        };
    }
    Ok(saved
        .map(str::trim)
        .filter(|id| is_valid(id))
        .map(str::to_owned))
}

/// The client ID to use, from the configuration, the saved file or the setup screen (which saves
/// it). `Ok(None)` if the user left the setup screen without entering one.
pub fn client_id(configured: Option<&str>) -> Result<Option<String>, String> {
    let path = config::config_path(FILE_NAME);
    let saved = fs::read_to_string(&path).ok();
    if let Some(id) = resolve(configured, saved.as_deref())? {
        return Ok(Some(id));
    }

    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(instructions());
    }
    match run_setup().map_err(|e| format!("Couldn't show the setup screen: {e}"))? {
        Some(id) => {
            fs::write(&path, format!("{id}\n"))
                .map_err(|e| format!("Couldn't save the client ID to {}: {e}", path.display()))?;
            Ok(Some(id))
        }
        None => {
            println!("{}", instructions());
            Ok(None)
        }
    }
}

/// The first three setup steps, shared by the setup screen and the printed instructions.
fn steps() -> [String; 3] {
    [
        format!("Create an app at {DASHBOARD_URL}"),
        format!("Add the redirect URI {WEB_API_REDIRECT_URI}"),
        "Choose the Web API".to_owned(),
    ]
}

/// The setup steps as printed to the terminal when there is no setup screen.
fn instructions() -> String {
    let mut text = String::from("ttunes needs a Spotify app of your own:\n");
    for (i, step) in steps().iter().enumerate() {
        text.push_str(&format!("  {}. {step}\n", i + 1));
    }
    text.push_str("  4. Start ttunes again and paste the app's client ID\n");
    text.push_str("Or set client_id in config.toml.");
    text
}

/// What a key press on the setup screen leads to.
#[derive(Debug, PartialEq)]
enum Step {
    Continue,
    Done(String),
    Quit,
}

/// The setup screen's state: the typed ID and the last error.
#[derive(Default)]
struct Setup {
    input: String,
    error: Option<String>,
}

impl Setup {
    fn key(&mut self, key: KeyEvent) -> Step {
        match key.code {
            KeyCode::Esc => Step::Quit,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => Step::Quit,
            KeyCode::Enter => {
                let id = self.input.trim();
                if is_valid(id) {
                    Step::Done(id.to_owned())
                } else {
                    self.error = Some("A client ID is 32 characters, 0-9 and a-f".to_owned());
                    Step::Continue
                }
            }
            KeyCode::Backspace => {
                self.input.pop();
                self.error = None;
                Step::Continue
            }
            KeyCode::Char(c) => {
                self.paste(&c.to_string());
                Step::Continue
            }
            _ => Step::Continue,
        }
    }

    fn paste(&mut self, text: &str) {
        for c in text.chars().filter(|c| !c.is_whitespace()) {
            if self.input.chars().count() < MAX_INPUT {
                self.input.push(c);
            }
        }
        self.error = None;
    }

    fn draw(&self, frame: &mut Frame) {
        let area = frame.area();
        let width = area.width.min(76);
        let margin = if width >= 50 { 2 } else { 0 };
        let area = Rect {
            x: area.x + (area.width - width) / 2 + margin,
            y: area.y + u16::from(area.height >= 16),
            width: width.saturating_sub(2 * margin),
            height: area.height.saturating_sub(u16::from(area.height >= 16)),
        };
        let [text_area, input_area, message_area] = Layout::vertical([
            Constraint::Min(0),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas(area);

        let bold = Style::default().add_modifier(Modifier::BOLD);
        let dim = Style::default().fg(Color::DarkGray);
        let mut lines = vec![
            Line::styled("Welcome to ttunes", bold),
            Line::from(""),
            Line::from("ttunes needs a Spotify app of your own:"),
        ];
        for (i, step) in steps().into_iter().enumerate() {
            lines.push(Line::from(format!("{}. {step}", i + 1)));
        }
        lines.push(Line::from("4. Paste the app's client ID below"));
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), text_area);

        let label = "Client ID: ";
        let room = (input_area.width as usize).saturating_sub(label.len() + 1);
        let skip = self.input.chars().count().saturating_sub(room);
        let shown: String = self.input.chars().skip(skip).collect();
        let cursor_x = input_area.x + (label.len() + shown.chars().count()) as u16;
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(label, bold),
                Span::raw(shown),
            ])),
            input_area,
        );
        if cursor_x < input_area.right() {
            frame.set_cursor_position((cursor_x, input_area.y));
        }

        let message = match &self.error {
            Some(error) => Line::styled(error.as_str(), Style::default().fg(Color::Red)),
            None => Line::styled("Enter to continue, Esc to quit", dim),
        };
        frame.render_widget(Paragraph::new(message), message_area);
    }
}

/// Leaves raw mode and the alternate screen when dropped, also on errors and panics.
struct RawScreen;

impl RawScreen {
    fn enter() -> io::Result<Self> {
        enable_raw_mode()?;
        let screen = Self;
        execute!(io::stdout(), EnterAlternateScreen, EnableBracketedPaste)?;
        Ok(screen)
    }
}

impl Drop for RawScreen {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), DisableBracketedPaste, LeaveAlternateScreen);
        let _ = disable_raw_mode();
    }
}

/// Show the setup screen until a valid ID is entered (`Some`) or the user quits (`None`).
fn run_setup() -> io::Result<Option<String>> {
    let _screen = RawScreen::enter()?;
    let mut terminal = ratatui::Terminal::new(CrosstermBackend::new(io::stdout()))?;
    let mut setup = Setup::default();
    loop {
        terminal.draw(|frame| setup.draw(frame))?;
        let step = match event::read()? {
            Event::Key(key) if key.kind != KeyEventKind::Release => setup.key(key),
            Event::Paste(text) => {
                setup.paste(&text);
                Step::Continue
            }
            _ => Step::Continue,
        };
        match step {
            Step::Continue => {}
            Step::Done(id) => return Ok(Some(id)),
            Step::Quit => return Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;

    const ID: &str = "0123456789abcdef0123456789ABCDEF";
    const OTHER_ID: &str = "fedcba9876543210fedcba9876543210";

    #[test]
    fn test_valid_ids() {
        assert!(is_valid(ID));
        assert!(!is_valid(""));
        assert!(!is_valid(&ID[1..]));
        assert!(!is_valid(&format!("{ID}0")));
        assert!(!is_valid("0123456789abcdef0123456789abcdeg"));
    }

    #[test]
    fn test_config_beats_saved_file() {
        assert_eq!(resolve(Some(ID), Some(OTHER_ID)), Ok(Some(ID.to_owned())));
        assert_eq!(resolve(None, Some(OTHER_ID)), Ok(Some(OTHER_ID.to_owned())));
        assert_eq!(
            resolve(Some("  "), Some(OTHER_ID)),
            Ok(Some(OTHER_ID.to_owned()))
        );
        assert_eq!(
            resolve(None, Some(&format!("{OTHER_ID}\n"))),
            Ok(Some(OTHER_ID.to_owned()))
        );
        assert_eq!(resolve(None, None), Ok(None));
    }

    #[test]
    fn test_bad_ids_rejected() {
        assert!(resolve(Some("not-an-id"), Some(OTHER_ID)).is_err());
        // A broken saved file means asking again.
        assert_eq!(resolve(None, Some("not-an-id")), Ok(None));
    }

    fn press(setup: &mut Setup, code: KeyCode) -> Step {
        setup.key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    #[test]
    fn test_setup_keys() {
        let mut setup = Setup::default();
        setup.paste(" 0123 ");
        assert_eq!(press(&mut setup, KeyCode::Enter), Step::Continue);
        assert!(setup.error.is_some());
        press(&mut setup, KeyCode::Backspace);
        assert_eq!(setup.input, "012");
        assert!(setup.error.is_none());

        let mut setup = Setup::default();
        for c in ID.chars() {
            press(&mut setup, KeyCode::Char(c));
        }
        assert_eq!(press(&mut setup, KeyCode::Enter), Step::Done(ID.to_owned()));
        assert_eq!(press(&mut setup, KeyCode::Esc), Step::Quit);
    }

    fn render(setup: &Setup, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| setup.draw(frame)).unwrap();
        let buffer = terminal.backend().buffer();
        let mut text = String::new();
        for y in 0..height {
            for x in 0..width {
                text.push_str(buffer[(x, y)].symbol());
            }
            text.push('\n');
        }
        text
    }

    #[test]
    fn test_setup_screen_renders() {
        let mut setup = Setup::default();
        setup.paste(ID);
        let text = render(&setup, 80, 24);
        for expected in [
            "Welcome to ttunes",
            DASHBOARD_URL,
            WEB_API_REDIRECT_URI,
            "Web API",
            ID,
            "Esc to quit",
        ] {
            assert!(text.contains(expected), "{expected:?} missing:\n{text}");
        }

        // Small windows keep the input line, showing the end of the ID.
        let text = render(&setup, 40, 10);
        assert!(text.contains("Client ID: "), "input missing:\n{text}");
        assert!(
            text.contains(&ID[ID.len() - 20..]),
            "input missing:\n{text}"
        );
        render(&setup, 10, 3);
    }
}
