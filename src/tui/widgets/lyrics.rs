//! The current track's lyrics (see [crate::lyrics]), following the song: the line being sung is
//! bold in the accent, lines already sung are dimmed, and the current line is kept a third of the
//! way down. Scrolling by hand (keys or the wheel) holds the view for [MANUAL_SCROLL]; clicking a
//! line seeks to it. Used by the Lyrics screen; reusable wherever lyrics are shown.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use ratatui::Frame;
use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

use crate::command::{Command, MoveAmount, MoveMode};
use crate::lyrics::{Lyrics, LyricsStore};
use crate::model::playable::Playable;
use crate::tui::page::{Ctx, Outcome};
use crate::tui::pointer;
use crate::tui::style::{Styles, bold, put, width, wrap};

/// How long the lyrics stay where they were scrolled to before following the song again.
const MANUAL_SCROLL: Duration = Duration::from_secs(5);
/// Rows moved by one step of the mouse wheel.
const WHEEL_ROWS: usize = 3;
/// Credit shown under the lyrics.
const CREDIT: &str = "Lyrics from LRCLIB";

/// Lyrics fetched so far, shared by every [LyricsView] so each song is only downloaded once.
fn store() -> &'static LyricsStore {
    static STORE: OnceLock<LyricsStore> = OnceLock::new();
    STORE.get_or_init(LyricsStore::default)
}

/// One row of lyrics: its text and, for lyrics (rather than messages), which line it's part of
/// and when that line starts (for synced lyrics).
#[derive(Clone, Debug, PartialEq, Eq)]
struct Row {
    text: String,
    line: Option<usize>,
    time: Option<u32>,
}

impl Row {
    fn message(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            line: None,
            time: None,
        }
    }
}

/// `text` wrapped to `width`, as message rows.
fn message(text: &str, width: usize) -> Vec<Row> {
    wrap(text, width).into_iter().map(Row::message).collect()
}

/// The rows showing `lyrics` in `width` columns. `spacious` puts a blank row between lines.
fn layout(lyrics: &Lyrics, width: usize, spacious: bool) -> Vec<Row> {
    let mut rows = Vec::new();
    match lyrics {
        Lyrics::Loading => return message("Loading lyrics…", width),
        Lyrics::NotFound => return message("No lyrics found for this track", width),
        Lyrics::Instrumental => return message("Instrumental", width),
        Lyrics::Failed(e) => {
            rows = message("Couldn't load lyrics:", width);
            rows.extend(message(e, width));
            return rows;
        }
        Lyrics::Synced(lines) => {
            for (i, line) in lines.iter().enumerate() {
                if spacious && !rows.is_empty() {
                    rows.push(Row::message(""));
                }
                // An empty line is an instrumental break.
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
            rows.extend(message("Not synced to the song", width));
            rows.push(Row::message(""));
            for (i, line) in lines.iter().enumerate() {
                if spacious && i > 0 {
                    rows.push(Row::message(""));
                }
                rows.extend(wrap(line, width).into_iter().map(|text| Row {
                    text,
                    line: Some(i),
                    time: None,
                }));
            }
        }
    }
    rows.push(Row::message(""));
    rows.extend(message(CREDIT, width));
    rows
}

/// The first row to show to follow the song: the current line's first row a third of the way
/// down `visible` rows (or the top before the first line), within the scrollable range.
fn follow_top(rows: &[Row], current: Option<usize>, visible: usize) -> usize {
    let top = current
        .and_then(|line| rows.iter().position(|row| row.line == Some(line)))
        .map_or(0, |row| row.saturating_sub(visible / 3));
    top.min(max_top(rows.len(), visible))
}

/// The furthest the view can scroll: until the last row is at the bottom.
fn max_top(rows: usize, visible: usize) -> usize {
    rows.saturating_sub(visible)
}

pub struct LyricsView {
    /// Lines centered with a blank row between them, rather than left-aligned and packed.
    spacious: bool,
    /// First row shown last.
    top: usize,
    /// A scroll position set by hand, and when; it applies for [MANUAL_SCROLL].
    manual_scroll: Option<(usize, Instant)>,
    /// Where the lyrics were drawn last, and for each row of it the start time of the line shown
    /// there (for clicks).
    area: Rect,
    row_times: Vec<Option<u32>>,
}

impl Default for LyricsView {
    fn default() -> Self {
        Self::new()
    }
}

impl LyricsView {
    pub fn new() -> Self {
        Self {
            spacious: false,
            top: 0,
            manual_scroll: None,
            area: Rect::default(),
            row_times: Vec::new(),
        }
    }

    /// Centered lines with blank rows between them.
    pub fn spacious(mut self) -> Self {
        self.spacious = true;
        self
    }

    /// Scroll by `rows` (negative for up). The lyrics follow the song again after a while.
    pub fn scroll(&mut self, rows: isize) {
        let top = self.top.saturating_add_signed(rows);
        self.manual_scroll = Some((top, Instant::now()));
    }

    /// Go back to following the song.
    pub fn follow(&mut self) {
        self.manual_scroll = None;
    }

    /// Movement commands scroll; `move playing` and play follow the song again.
    pub fn command(&mut self, cmd: &Command, _ctx: &Ctx) -> Outcome {
        match cmd {
            Command::Move(mode @ (MoveMode::Up | MoveMode::Down), amount) => {
                // In the spacious layout a line takes two rows.
                let line_rows = if self.spacious { 2 } else { 1 };
                let step = match amount {
                    MoveAmount::Integer(n) => n.unsigned_abs() as usize * line_rows,
                    MoveAmount::Float(scale) => (f32::from(self.area.height) * scale) as usize,
                    MoveAmount::Extreme => usize::MAX / 2,
                }
                .max(1) as isize;
                self.scroll(if matches!(mode, MoveMode::Up) {
                    -step
                } else {
                    step
                });
            }
            Command::Move(MoveMode::Playing, _) | Command::Play => self.follow(),
            _ => return Outcome::Ignored,
        }
        Outcome::Consumed
    }

    /// Clicking a synced line seeks to it, the wheel scrolls.
    pub fn mouse(&mut self, event: MouseEvent, ctx: &Ctx) -> Outcome {
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let row = event.row.checked_sub(self.area.y).map(usize::from);
                if let Some(time) = row.and_then(|row| self.row_times.get(row).copied().flatten()) {
                    ctx.spotify.seek(time);
                    self.follow();
                }
            }
            MouseEventKind::ScrollDown => self.scroll(WHEEL_ROWS as isize),
            MouseEventKind::ScrollUp => self.scroll(-(WHEEL_ROWS as isize)),
            _ => return Outcome::Ignored,
        }
        Outcome::Consumed
    }

    /// The rows to show for what's playing, and the line being sung.
    fn rows(&self, width: usize, ctx: &Ctx) -> (Vec<Row>, Option<usize>) {
        let track = match ctx.queue.get_current() {
            Some(Playable::Track(track)) => track,
            Some(Playable::Episode(_)) => return (message("No lyrics for podcasts", width), None),
            None => return (message("Nothing playing", width), None),
        };
        let lyrics = store().get(&track, crate::tui::request_redraw);
        let progress = ctx.spotify.get_current_progress().as_millis() as u32;
        (
            layout(&lyrics, width, self.spacious),
            lyrics.current_line(progress),
        )
    }

    /// Draw the lyrics filling `area`, `margin` columns in from either side.
    pub fn draw(&mut self, frame: &mut Frame, area: Rect, ctx: &Ctx, margin: u16) {
        self.area = area;
        let styles = Styles(&ctx.palette);
        let left = area.x + margin.min(area.width / 2);
        let text_width = usize::from(area.width.saturating_sub(2 * margin));
        let visible = usize::from(area.height);
        let (rows, current) = self.rows(text_width, ctx);

        // Follow the song unless scrolled by hand recently.
        if self
            .manual_scroll
            .is_some_and(|(_, at)| at.elapsed() > MANUAL_SCROLL)
        {
            self.manual_scroll = None;
        }
        self.top = match &mut self.manual_scroll {
            Some((top, _)) => {
                // Scrolled past an end: keep the clamped position so scrolling back is immediate.
                *top = (*top).min(max_top(rows.len(), visible));
                *top
            }
            None => follow_top(&rows, current, visible),
        };

        // Messages (nothing playing, loading…) sit a third of the way down in the spacious layout.
        let only_messages = rows.iter().all(|row| row.line.is_none());
        let offset = if self.spacious && only_messages {
            visible.saturating_sub(rows.len()) / 3
        } else {
            0
        };

        let buf = frame.buffer_mut();
        self.row_times = vec![None; visible];
        for (i, row) in rows
            .iter()
            .skip(self.top)
            .take(visible.saturating_sub(offset))
            .enumerate()
        {
            let y = offset + i;
            let style = match (row.line, current) {
                (None, _) => styles.secondary(),
                (Some(line), Some(current)) if line == current => bold(styles.accent()),
                (Some(line), Some(current)) if line < current => styles.secondary(),
                _ => styles.primary(),
            };
            let x = if self.spacious {
                left + (text_width.saturating_sub(width(&row.text)) / 2) as u16
            } else {
                left
            };
            let written = put(buf, x, area.y + y as u16, &row.text, text_width, style);
            // Clicking a synced line seeks to it.
            if row.time.is_some() {
                pointer::clickable(Rect::new(x, area.y + y as u16, written as u16, 1));
            }
            self.row_times[y] = row.time;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lyrics::SyncedLine;

    fn synced(lines: &[(u32, &str)]) -> Lyrics {
        Lyrics::Synced(
            lines
                .iter()
                .map(|(time, text)| SyncedLine {
                    time: *time,
                    text: text.to_string(),
                })
                .collect(),
        )
    }

    fn texts(rows: &[Row]) -> Vec<&str> {
        rows.iter().map(|row| row.text.as_str()).collect()
    }

    #[test]
    fn test_layout_packed() {
        let rows = layout(
            &synced(&[(0, "one two"), (1000, ""), (2000, "three")]),
            20,
            false,
        );
        assert_eq!(texts(&rows), vec!["one two", "♪", "three", "", CREDIT]);
        assert_eq!(rows[1].line, Some(1));
        assert_eq!(rows[2].time, Some(2000));
        assert_eq!(rows[4].line, None);
    }

    #[test]
    fn test_layout_spacious_wraps_and_spaces_lines() {
        let rows = layout(&synced(&[(0, "one two three"), (1000, "four")]), 7, true);
        assert_eq!(
            texts(&rows),
            vec![
                "one two", "three", "", "four", "", "Lyrics", "from", "LRCLIB"
            ]
        );
        assert_eq!(rows[1].line, Some(0));
        assert_eq!(rows[2].line, None);
        assert_eq!(rows[3].time, Some(1000));
    }

    #[test]
    fn test_layout_messages() {
        assert_eq!(
            texts(&layout(&Lyrics::Loading, 40, true)),
            vec!["Loading lyrics…"]
        );
        let plain = layout(&Lyrics::Plain(vec!["a".into(), "b".into()]), 40, false);
        assert_eq!(
            texts(&plain),
            vec!["Not synced to the song", "", "a", "b", "", CREDIT]
        );
        assert!(plain.iter().all(|row| row.time.is_none()));
    }

    #[test]
    fn test_follow_top_keeps_current_line_a_third_down() {
        let lines: Vec<(u32, String)> = (0..30).map(|i| (i * 1000, format!("line {i}"))).collect();
        let lyrics = synced(
            &lines
                .iter()
                .map(|(t, s)| (*t, s.as_str()))
                .collect::<Vec<_>>(),
        );
        let rows = layout(&lyrics, 40, true);
        // Line 10 starts at row 20; with 9 rows visible it's shown 3 rows down.
        assert_eq!(follow_top(&rows, Some(10), 9), 17);
        assert_eq!(follow_top(&rows, Some(0), 9), 0);
        assert_eq!(follow_top(&rows, None, 9), 0);
        // Near the end the view stops with the last row at the bottom.
        assert_eq!(follow_top(&rows, Some(29), 9), rows.len() - 9);
        // Everything fits: no scrolling.
        assert_eq!(follow_top(&rows, Some(29), 200), 0);
    }
}
