//! Text and color helpers shared by every part of the UI.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::theme::Palette;

/// Named styles from the theme.
pub struct Styles<'a>(pub &'a Palette);

impl Styles<'_> {
    /// Normal text on the main background.
    pub fn primary(&self) -> Style {
        Style::new().fg(self.0.primary).bg(self.0.background)
    }

    /// Less important text (artists, lengths, hints).
    pub fn secondary(&self) -> Style {
        Style::new().fg(self.0.secondary).bg(self.0.background)
    }

    /// The accent: titles, the current tab, active toggles.
    pub fn accent(&self) -> Style {
        Style::new().fg(self.0.title).bg(self.0.background)
    }

    /// The track that's playing.
    pub fn playing(&self) -> Style {
        Style::new().fg(self.0.playing).bg(self.0.playing_bg)
    }

    /// The selected row: a darker background (drawn with [SELECTION_MARKER] in the accent at its
    /// left edge, see [Styles::marker]).
    pub fn selected(&self) -> Style {
        Style::new().fg(self.0.primary).bg(self.0.statusbar_bg).add_modifier(Modifier::BOLD)
    }

    /// The selected row while its list doesn't have the keyboard (no marker).
    pub fn selected_inactive(&self) -> Style {
        Style::new().fg(self.0.primary).bg(self.0.statusbar_bg)
    }

    /// The selection marker's style, on the selected row's background.
    pub fn marker(&self) -> Style {
        Style::new().fg(self.0.title).bg(self.0.statusbar_bg)
    }

    /// A filled button or chip in the accent (e.g. the header's Play button).
    pub fn button(&self) -> Style {
        Style::new()
            .fg(self.0.highlight)
            .bg(self.0.highlight_bg)
            .add_modifier(Modifier::BOLD)
    }

    /// Panels on the darker background: the player, menus, the command line.
    pub fn panel(&self) -> Style {
        Style::new().fg(self.0.primary).bg(self.0.statusbar_bg)
    }

    pub fn error(&self) -> Style {
        Style::new().fg(self.0.error).bg(self.0.error_bg)
    }
}

/// Drawn at the left edge of the selected row.
pub const SELECTION_MARKER: &str = "▌";

pub fn bold(style: Style) -> Style {
    style.add_modifier(Modifier::BOLD)
}

/// Columns `text` takes.
pub fn width(text: &str) -> usize {
    text.width()
}

/// Cut `text` to at most `width` columns, ending with "…" if it was cut.
pub fn truncate(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars() {
        let w = c.width().unwrap_or(0);
        if used + w + 1 > width {
            break;
        }
        out.push(c);
        used += w;
    }
    out.push('…');
    out
}

/// Word-wrap `text` into lines at most `width` columns wide, breaking words only when they're
/// longer than a line. Always at least one line (empty for empty text).
pub fn wrap(text: &str, width: usize) -> Vec<String> {
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

/// Write `text` at (`x`, `y`), cut to fit `max_width` columns and the buffer. Returns the
/// columns written.
pub fn put(buf: &mut Buffer, x: u16, y: u16, text: &str, max_width: usize, style: Style) -> usize {
    let area = buf.area;
    if y >= area.bottom() || x >= area.right() {
        return 0;
    }
    let room = max_width.min(usize::from(area.right() - x));
    let text = truncate(text, room);
    buf.set_stringn(x, y, &text, room, style);
    text.width()
}

/// Fill `area` with spaces in `style`.
pub fn fill(buf: &mut Buffer, area: Rect, style: Style) {
    let area = area.intersection(buf.area);
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.reset();
                cell.set_style(style);
            }
        }
    }
}

/// RGB of a theme color, for icons and blending. Named colors get typical terminal values.
pub fn rgb(color: Color) -> [u8; 3] {
    match color {
        Color::Rgb(r, g, b) => [r, g, b],
        Color::Black => [0, 0, 0],
        Color::Red => [205, 49, 49],
        Color::Green => [13, 188, 121],
        Color::Yellow => [229, 229, 16],
        Color::Blue => [36, 114, 200],
        Color::Magenta => [188, 63, 188],
        Color::Cyan => [17, 168, 205],
        Color::Gray => [204, 204, 204],
        Color::DarkGray => [102, 102, 102],
        Color::LightRed => [241, 76, 76],
        Color::LightGreen => [35, 209, 139],
        Color::LightYellow => [245, 245, 67],
        Color::LightBlue => [59, 142, 234],
        Color::LightMagenta => [214, 112, 214],
        Color::LightCyan => [41, 184, 219],
        Color::White => [229, 229, 229],
        Color::Indexed(_) | Color::Reset => [30, 30, 46],
    }
}

/// `a` blended towards `b` by `t` (0: all `a`, 1: all `b`).
pub fn mix(a: [u8; 3], b: [u8; 3], t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    let channel = |i: usize| (f32::from(a[i]) * (1.0 - t) + f32::from(b[i]) * t).round() as u8;
    Color::Rgb(channel(0), channel(1), channel(2))
}

/// Track length as m:ss (or h:mm:ss).
pub fn duration(ms: u32) -> String {
    crate::utils::ms_to_hms(ms)
}

/// Spotify-style total length: "1 hr 3 min", or "45 min 12 sec" under an hour.
pub fn total_length(ms: u64) -> String {
    let seconds = ms / 1000;
    let (hours, minutes, seconds) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if hours > 0 {
        format!("{hours} hr {minutes} min")
    } else {
        format!("{minutes} min {seconds} sec")
    }
}

/// "12 songs, 45 min 3 sec" for tracks with these lengths in milliseconds.
pub fn songs_and_length(durations: impl IntoIterator<Item = u32>) -> String {
    let (count, total) = durations
        .into_iter()
        .fold((0usize, 0u64), |(n, ms), d| (n + 1, ms + u64::from(d)));
    let songs = if count == 1 { "song" } else { "songs" };
    format!("{count} {songs}, {}", total_length(total))
}

/// The first item to show of a list scrolled to `first` so that `selected` is among the
/// `visible` items shown.
pub fn scroll_to(first: usize, selected: usize, visible: usize) -> usize {
    let visible = visible.max(1);
    if selected < first {
        selected
    } else if selected >= first + visible {
        selected + 1 - visible
    } else {
        first
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello world", 6), "hello…");
        assert_eq!(truncate("abc", 0), "");
    }

    #[test]
    fn test_wrap() {
        assert_eq!(wrap("one two three", 7), vec!["one two", "three"]);
        assert_eq!(wrap("abcdefghij", 4), vec!["abcd", "efgh", "ij"]);
        assert_eq!(wrap("", 10), vec![""]);
    }

    #[test]
    fn test_lengths() {
        assert_eq!(songs_and_length([60_000, 125_000]), "2 songs, 3 min 5 sec");
        assert_eq!(songs_and_length([3_780_000]), "1 song, 1 hr 3 min");
    }

    #[test]
    fn test_scroll_to() {
        assert_eq!(scroll_to(0, 7, 5), 3);
        assert_eq!(scroll_to(4, 2, 5), 2);
        assert_eq!(scroll_to(3, 7, 5), 3);
    }
}
