//! The loading screen shown while ttunes connects to Spotify: "ttunes" typed out in big pixel
//! letters, a caret after them, and what's being done with a spinner. It runs on a thread of its
//! own while the main thread connects, and hands the terminal over to the UI when it's dropped.

use std::io::{self, Stdout, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyModifiers};
use ratatui::crossterm::terminal::{
    Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::crossterm::{cursor, execute};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use crate::theme::Palette;
use crate::tui::style::{Styles, fill, put, width};

const LOGO: &str = "ttunes";
/// When the first letter appears, and the time between letters, as if typed.
const FIRST_LETTER: Duration = Duration::from_millis(150);
const LETTER_STEP: Duration = Duration::from_millis(110);
/// After the last letter, before the tagline shows: the intro is done.
const TAGLINE_DELAY: Duration = Duration::from_millis(220);
const SPINNER: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
const SPINNER_STEP: Duration = Duration::from_millis(80);

/// The letters in a 3×6 pixel font (one string per row, `#` lit), drawn with half blocks.
fn glyph(c: char) -> [&'static str; 6] {
    match c {
        't' => [".#.", "###", ".#.", ".#.", ".##", "..."],
        'u' => ["...", "#.#", "#.#", "#.#", ".##", "..."],
        'n' => ["...", "##.", "#.#", "#.#", "#.#", "..."],
        'e' => ["...", ".##", "###", "#..", ".##", "..."],
        's' => ["...", ".##", "#..", "..#", "##.", "..."],
        _ => [".#.", ".#.", ".#.", ".#.", ".#.", "..."], // The caret.
    }
}

/// Shows the loading screen until dropped; [Splash::status] says what's being done.
pub struct Splash {
    status: Arc<Mutex<String>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Splash {
    /// Take over the terminal and show the loading screen, saying `status`.
    pub fn start(palette: Palette, status: &str) -> io::Result<Self> {
        enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen, cursor::Hide)?;
        let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
        let status = Arc::new(Mutex::new(status.to_string()));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = std::thread::Builder::new().name("splash".into()).spawn({
            let (status, stop) = (status.clone(), stop.clone());
            move || run(&mut terminal, &palette, &status, &stop)
        })?;
        Ok(Self {
            status,
            stop,
            thread: Some(thread),
        })
    }

    /// Say what's being done now.
    pub fn status(&self, status: &str) {
        *self.status.lock().unwrap() = status.to_string();
    }
}

impl Drop for Splash {
    /// Finish typing the logo (it takes under a second), then clear the screen and give the
    /// terminal back: to the UI, which takes it over again, or to an error message.
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        let _ = execute!(
            io::stdout(),
            Clear(ClearType::All),
            cursor::Show,
            LeaveAlternateScreen
        );
        let _ = disable_raw_mode();
    }
}

fn run(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    palette: &Palette,
    status: &Mutex<String>,
    stop: &AtomicBool,
) {
    let started = Instant::now();
    loop {
        let t = started.elapsed();
        let status = status.lock().unwrap().clone();
        let _ = terminal.draw(|frame| {
            let area = frame.area();
            draw(frame.buffer_mut(), area, palette, t, &status);
        });
        if stop.load(Ordering::Relaxed) && t >= intro_done() {
            return;
        }
        // Ctrl+C or q quits while connecting: raw mode keeps Ctrl+C from interrupting.
        if event::poll(Duration::from_millis(40)).unwrap_or(false)
            && let Ok(Event::Key(key)) = event::read()
            && (key.code == KeyCode::Char('q')
                || (key.code == KeyCode::Char('c')
                    && key.modifiers.contains(KeyModifiers::CONTROL)))
        {
            let _ = execute!(
                io::stdout(),
                Clear(ClearType::All),
                cursor::Show,
                LeaveAlternateScreen
            );
            let _ = disable_raw_mode();
            let _ = io::stdout().flush();
            std::process::exit(130);
        }
    }
}

fn letter_at(i: usize) -> Duration {
    FIRST_LETTER + LETTER_STEP * i as u32
}

fn intro_done() -> Duration {
    letter_at(LOGO.len() - 1) + TAGLINE_DELAY
}

/// The loading screen `t` after it started.
fn draw(buf: &mut Buffer, area: Rect, palette: &Palette, t: Duration, status: &str) {
    let styles = Styles(palette);
    fill(buf, area, styles.primary());
    let letters = (0..LOGO.len()).take_while(|&i| t >= letter_at(i)).count();
    let done = t >= intro_done();

    // The biggest letters that fit, up to twice the size (4×3 cells per letter at scale 1), with
    // room for the caret.
    let slots = LOGO.len() as u16 + 1;
    let scale = (1..=2u16)
        .rev()
        .find(|s| 4 * s * slots <= area.width.saturating_sub(4) && 3 * s + 6 <= area.height);
    // Logo, a blank row, the tagline, a blank row, the status.
    let logo_rows = scale.map_or(1, |s| 3 * s);
    let block = logo_rows + 4;
    let free = area.height.saturating_sub(block);
    let top = area.y + free * 2 / 5;

    let accent = styles.accent();
    let bold_accent = accent.add_modifier(Modifier::BOLD);
    match scale {
        Some(scale) => {
            let glyph_width = 4 * scale;
            let x0 = area.x + area.width.saturating_sub(glyph_width * LOGO.len() as u16) / 2;
            for (i, c) in LOGO.chars().take(letters).enumerate() {
                let style = if i == 0 {
                    bold_accent
                } else {
                    styles.secondary()
                };
                draw_glyph(buf, area, x0 + i as u16 * glyph_width, top, c, style, scale);
            }
            let caret_x = x0 + letters as u16 * glyph_width;
            draw_glyph(buf, area, caret_x, top, '|', accent, scale);
        }
        // Too small for big letters: the word, then a block caret.
        None => {
            let x = area.x + area.width.saturating_sub(LOGO.len() as u16 + 1) / 2;
            let typed: String = LOGO.chars().take(letters).collect();
            let mut chars = typed.chars();
            if let Some(first) = chars.next() {
                put(buf, x, top, &first.to_string(), 1, bold_accent);
                put(
                    buf,
                    x + 1,
                    top,
                    chars.as_str(),
                    LOGO.len(),
                    styles.secondary(),
                );
            }
            let caret = Style::new().bg(palette.title).fg(palette.background);
            put(buf, x + letters as u16, top, " ", 1, caret);
        }
    }

    let centered = |buf: &mut Buffer, y: u16, text: &str, style: Style| {
        if y < area.bottom() {
            let x = area.x + area.width.saturating_sub(width(text) as u16) / 2;
            put(buf, x, y, text, usize::from(area.width), style);
        }
    };
    let below = top + logo_rows + 1;
    if done {
        centered(buf, below, "Spotify for the terminal", styles.secondary());
    }
    let spinner = SPINNER[(t.as_millis() / SPINNER_STEP.as_millis()) as usize % SPINNER.len()];
    centered(
        buf,
        below + 2,
        &format!("{spinner} {status}"),
        styles.primary(),
    );
    if area.height > block + 2 {
        centered(
            buf,
            area.bottom() - 1,
            "q or ctrl+c to quit",
            styles.secondary(),
        );
    }
}

/// Draw `c` from [glyph] at (`x`, `y`), each pixel `scale` cells wide and half a cell tall per
/// `scale`, clipped to `clip`.
fn draw_glyph(buf: &mut Buffer, clip: Rect, x: u16, y: u16, c: char, style: Style, scale: u16) {
    let rows = glyph(c);
    let lit = |px: u16, py: u16| {
        let (gx, gy) = (usize::from(px / scale), usize::from(py / scale));
        rows.get(gy)
            .and_then(|row| row.as_bytes().get(gx))
            .is_some_and(|&b| b == b'#')
    };
    for cy in 0..3 * scale {
        for cx in 0..4 * scale {
            let (top, bottom) = (lit(cx, 2 * cy), lit(cx, 2 * cy + 1));
            let symbol = match (top, bottom) {
                (true, true) => "█",
                (true, false) => "▀",
                (false, true) => "▄",
                (false, false) => continue,
            };
            let (tx, ty) = (x + cx, y + cy);
            if clip.contains((tx, ty).into()) {
                buf[(tx, ty)].set_symbol(symbol).set_style(style);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rows of the logo drawn at scale 1, as text.
    fn logo_rows(t: Duration) -> Vec<String> {
        let area = Rect::new(0, 0, 40, 12);
        let mut buf = Buffer::empty(area);
        let palette = crate::config::Config::new_for_test().build_theme();
        draw(&mut buf, area, &palette, t, "Connecting to Spotify");
        (0..area.height)
            .map(|y| {
                (0..area.width)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    #[test]
    fn test_logo_is_typed_out() {
        let start = logo_rows(Duration::ZERO).join("\n");
        assert!(!start.contains("Spotify for the terminal"));
        assert!(start.contains("Connecting to Spotify"));
        let done = logo_rows(intro_done()).join("\n");
        assert!(done.contains("Spotify for the terminal"));
        // More of the logo is lit once it's typed.
        let lit = |s: &str| s.chars().filter(|c| "█▀▄".contains(*c)).count();
        assert!(lit(&done) > lit(&start));
    }
}
