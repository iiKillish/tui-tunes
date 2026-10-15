//! The current track's lyrics (see [crate::lyrics]), following the song: the line being sung is
//! highlighted and kept a third of the way down. Used by the side panel's Lyrics tab and, bigger
//! and centered, by the cover screen.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock, RwLock};
use std::time::{Duration, Instant};

use cursive::event::{MouseButton, MouseEvent};
use cursive::theme::{ColorStyle, ColorType, Effect, PaletteColor};
use cursive::{Printer, Vec2};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::command::{Command, MoveAmount, MoveMode};
use crate::library::Library;
use crate::lyrics::{Lyrics, LyricsStore};
use crate::model::playable::Playable;
use crate::queue::Queue;
use crate::spotify::Spotify;

/// How long the lyrics stay where they were scrolled to before following the song again.
const MANUAL_SCROLL: Duration = Duration::from_secs(5);
/// Rows moved by one step of the mouse wheel.
const WHEEL_ROWS: usize = 3;

/// Lyrics fetched so far, shared by every [LyricsView] so each song is only downloaded once.
fn store() -> &'static LyricsStore {
    static STORE: OnceLock<LyricsStore> = OnceLock::new();
    STORE.get_or_init(LyricsStore::default)
}

pub struct LyricsView {
    queue: Arc<Queue>,
    spotify: Spotify,
    library: Arc<Library>,
    /// Lines centered with a blank row between them (the cover screen), rather than left-aligned
    /// and packed (the side panel).
    spacious: bool,
    /// Drawn on the side panel's background (`statusbar_bg`) rather than the main one.
    panel_background: bool,
    /// First row shown last, and a scroll position set with the mouse wheel or keys (and when),
    /// which applies for [MANUAL_SCROLL].
    top: AtomicUsize,
    manual_scroll: RwLock<Option<(usize, Instant)>>,
    /// For each row drawn last, the start time of the line shown there (for clicks).
    row_times: RwLock<Vec<Option<u32>>>,
}

/// One row of lyrics: text and, for lyrics (rather than messages), which line it's part of and
/// when that line starts.
struct Row {
    text: String,
    line: Option<usize>,
    time: Option<u32>,
}

impl Row {
    fn message(text: String) -> Self {
        Self {
            text,
            line: None,
            time: None,
        }
    }
}

impl LyricsView {
    pub fn new(queue: Arc<Queue>, library: Arc<Library>) -> Self {
        Self {
            spotify: queue.get_spotify(),
            queue,
            library,
            spacious: false,
            panel_background: false,
            top: AtomicUsize::new(0),
            manual_scroll: RwLock::new(None),
            row_times: RwLock::new(Vec::new()),
        }
    }

    /// Centered lines with blank rows between them.
    #[cfg_attr(not(feature = "cover"), allow(dead_code))]
    pub fn spacious(mut self) -> Self {
        self.spacious = true;
        self
    }

    /// Draw on a `statusbar_bg` background (the side panel's) instead of the main one.
    pub fn on_panel_background(mut self) -> Self {
        self.panel_background = true;
        self
    }

    /// Scroll by `rows` (negative for up). The lyrics follow the song again after a while.
    pub fn scroll(&self, rows: isize) {
        let top = self.top.load(Ordering::Relaxed).saturating_add_signed(rows);
        *self.manual_scroll.write().unwrap() = Some((top, Instant::now()));
    }

    /// Go back to following the song.
    pub fn follow(&self) {
        *self.manual_scroll.write().unwrap() = None;
    }

    /// Movement commands scroll; `move playing` (and Enter) follow the song again. Returns
    /// whether `cmd` applied.
    pub fn handle_command(&self, cmd: &Command) -> bool {
        match cmd {
            Command::Move(mode @ (MoveMode::Up | MoveMode::Down), amount) => {
                let step = match amount {
                    MoveAmount::Integer(n) => n.unsigned_abs() as usize,
                    MoveAmount::Float(_) => 10,
                    MoveAmount::Extreme => usize::MAX / 2,
                };
                self.scroll(if matches!(mode, MoveMode::Up) {
                    -(step as isize)
                } else {
                    step as isize
                });
            }
            Command::Move(MoveMode::Playing, _) | Command::Play => self.follow(),
            _ => return false,
        }
        true
    }

    /// Mouse `event` at `position` (within the area last drawn): clicking a line seeks to it,
    /// the wheel scrolls.
    pub fn on_mouse(&self, event: MouseEvent, position: Vec2) {
        match event {
            MouseEvent::Press(MouseButton::Left) => {
                let time = self
                    .row_times
                    .read()
                    .unwrap()
                    .get(position.y)
                    .copied()
                    .flatten();
                if let Some(time) = time {
                    self.spotify.seek(time);
                    self.follow();
                }
            }
            MouseEvent::WheelDown => self.scroll(WHEEL_ROWS as isize),
            MouseEvent::WheelUp => self.scroll(-(WHEEL_ROWS as isize)),
            _ => {}
        }
    }

    /// Lyrics to show, as rows of text with the line each came from, and the line being sung.
    fn rows(&self, width: usize) -> (Vec<Row>, Option<usize>) {
        let message =
            |text: &str| -> Vec<Row> { wrap(text, width).into_iter().map(Row::message).collect() };
        let track = match self.queue.get_current() {
            Some(Playable::Track(track)) => track,
            Some(Playable::Episode(_)) => return (message("No lyrics for podcasts"), None),
            None => return (message("Nothing playing"), None),
        };
        let library = self.library.clone();
        let lyrics = store().get(&track, move || library.trigger_redraw());

        let progress = self.spotify.get_current_progress().as_millis() as u32;
        let current = lyrics.current_line(progress);
        let gap = |rows: &mut Vec<Row>| {
            if self.spacious && !rows.is_empty() {
                rows.push(Row::message(String::new()));
            }
        };
        let mut rows = Vec::new();
        match &lyrics {
            Lyrics::Loading => rows = message("Loading lyrics…"),
            Lyrics::NotFound => rows = message("No lyrics found for this track"),
            Lyrics::Instrumental => rows = message("Instrumental"),
            Lyrics::Failed(e) => {
                rows = message("Couldn't load lyrics:");
                rows.extend(message(e));
            }
            Lyrics::Synced(lines) => {
                for (i, line) in lines.iter().enumerate() {
                    gap(&mut rows);
                    let text = if line.text.is_empty() {
                        "♪"
                    } else {
                        &line.text
                    };
                    rows.extend(wrap(text, width).into_iter().map(|text| Row {
                        text,
                        line: Some(i),
                        time: Some(line.time),
                    }));
                }
            }
            Lyrics::Plain(lines) => {
                rows.extend(message("Not synced to the song"));
                rows.push(Row::message(String::new()));
                for (i, line) in lines.iter().enumerate() {
                    rows.extend(wrap(line, width).into_iter().map(|text| Row {
                        text,
                        line: Some(i),
                        time: None,
                    }));
                }
            }
        }
        if matches!(lyrics, Lyrics::Synced(_) | Lyrics::Plain(_)) {
            rows.push(Row::message(String::new()));
            rows.push(Row::message("Lyrics from LRCLIB".to_string()));
        }
        (rows, current)
    }

    /// Draw the lyrics filling `printer`, `margin` columns in from either side.
    pub fn draw(&self, printer: &Printer<'_, '_>, margin: usize) {
        let size = printer.size;
        let background = if self.panel_background {
            ColorType::Color(*printer.theme.palette.custom("statusbar_bg").unwrap())
        } else {
            ColorType::Palette(PaletteColor::Background)
        };
        let style = |front: ColorType| ColorStyle::new(front, background);
        let primary = style(ColorType::Palette(PaletteColor::Primary));
        let secondary = style(ColorType::Palette(PaletteColor::Secondary));
        let accent = style(ColorType::Palette(PaletteColor::TitlePrimary));

        let width = size.x.saturating_sub(2 * margin);
        let (rows, current) = self.rows(width);
        let available = size.y;
        let max_top = rows.len().saturating_sub(available);

        // Follow the song, keeping the current line a third of the way down, unless scrolled by
        // hand recently.
        let manual = {
            let mut scroll = self.manual_scroll.write().unwrap();
            if scroll.is_some_and(|(_, at)| at.elapsed() > MANUAL_SCROLL) {
                *scroll = None;
            }
            scroll.map(|(top, _)| top)
        };
        let top = match (manual, current) {
            (Some(top), _) => top,
            (None, Some(line)) => rows
                .iter()
                .position(|row| row.line == Some(line))
                .unwrap_or(0)
                .saturating_sub(available / 3),
            (None, None) => 0,
        }
        .min(max_top);
        self.top.store(top, Ordering::Relaxed);
        if let Some(manual) = manual
            && manual != top
            && let Some((stored, _)) = self.manual_scroll.write().unwrap().as_mut()
        {
            // Scrolled past an end: keep the clamped position so scrolling back is immediate.
            *stored = top;
        }

        let mut times = vec![None; size.y];
        for (y, row) in rows.iter().skip(top).take(available).enumerate() {
            let row_style = match (row.line, current) {
                (None, _) => secondary,
                (Some(line), Some(current)) if line == current => accent,
                (Some(line), Some(current)) if line < current => secondary,
                _ => primary,
            };
            let x = if self.spacious {
                margin + width.saturating_sub(row.text.width()) / 2
            } else {
                margin
            };
            let is_current = row.line.is_some() && row.line == current;
            printer.with_color(row_style, |printer| {
                if is_current {
                    printer.with_effect(Effect::Bold, |printer| printer.print((x, y), &row.text));
                } else {
                    printer.print((x, y), &row.text);
                }
            });
            times[y] = row.time;
        }
        *self.row_times.write().unwrap() = times;
    }
}

/// Word-wrap `text` into lines at most `width` columns wide, breaking words only when they're
/// longer than a line. Always at least one line (empty for empty text).
fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut used = 0;
    for word in text.split_whitespace() {
        let word_width = word.width();
        if used > 0 && used + 1 + word_width > width {
            lines.push(std::mem::take(&mut current));
            used = 0;
        }
        if word_width > width {
            for c in word.chars() {
                let w = c.width().unwrap_or(0);
                if used + w > width {
                    lines.push(std::mem::take(&mut current));
                    used = 0;
                }
                current.push(c);
                used += w;
            }
            continue;
        }
        if used > 0 {
            current.push(' ');
            used += 1;
        }
        current.push_str(word);
        used += word_width;
    }
    if !current.is_empty() || lines.is_empty() {
        lines.push(current);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wrap() {
        assert_eq!(wrap("one two three", 7), vec!["one two", "three"]);
        assert_eq!(wrap("one two three", 20), vec!["one two three"]);
        assert_eq!(wrap("abcdefghij", 4), vec!["abcd", "efgh", "ij"]);
        assert_eq!(wrap("", 10), vec![""]);
        assert_eq!(wrap("  spaced   out  ", 20), vec!["spaced out"]);
    }
}
