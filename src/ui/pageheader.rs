//! The header above album and playlist pages, like Spotify's: a big cover, what the page is, its
//! name and details (artist or owner, year, number of songs, length).

use cursive::theme::{ColorStyle, ColorType, Effect, PaletteColor};
use cursive::{Printer, Vec2};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

#[cfg(feature = "cover")]
use crate::ui::cover::{
    Area, HEADER_COVER_IMAGE_ID, ImageLayer, ImageRequest, kitty_supported, terminal_font_size,
};

/// Rows the header takes: the cover, then a blank row above the page.
pub const HEADER_ROWS: usize = 8;
const COVER_ROWS: usize = HEADER_ROWS - 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageHeader {
    /// What the page is, e.g. "Album" or "Playlist".
    pub kind: String,
    pub title: String,
    /// Details joined with " • ", e.g. artist, year, "12 songs, 45 min".
    pub details: Vec<String>,
    pub cover_url: Option<String>,
}

impl PageHeader {
    /// "12 songs, 45 min" for tracks with these lengths in milliseconds.
    pub fn songs_and_length(durations: impl IntoIterator<Item = u32>) -> String {
        let (count, total_ms) = durations
            .into_iter()
            .fold((0usize, 0u64), |(n, ms), d| (n + 1, ms + u64::from(d)));
        let songs = if count == 1 { "song" } else { "songs" };
        format!("{count} {songs}, {}", length(total_ms))
    }
}

/// Spotify-style length: "1 hr 3 min", or "45 min 12 sec" under an hour.
fn length(ms: u64) -> String {
    let seconds = ms / 1000;
    let (hours, minutes, seconds) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if hours > 0 {
        format!("{hours} hr {minutes} min")
    } else {
        format!("{minutes} min {seconds} sec")
    }
}

/// Draws [PageHeader]s, with the cover as a terminal image when the terminal supports it.
#[derive(Default)]
pub struct HeaderRenderer {
    #[cfg(feature = "cover")]
    images: ImageLayer,
}

impl HeaderRenderer {
    /// Draw `header` into `printer`, which should be [HEADER_ROWS] tall.
    pub fn draw(&self, printer: &Printer<'_, '_>, header: &PageHeader) {
        let x = 2;
        let mut text_x = x;
        #[cfg(feature = "cover")]
        {
            let mut images = Vec::new();
            if kitty_supported()
                && let Some(url) = &header.cover_url
            {
                let font = terminal_font_size();
                let width = (COVER_ROWS * font.y).div_ceil(font.x.max(1));
                images.push(ImageRequest {
                    image_id: HEADER_COVER_IMAGE_ID,
                    url: url.clone(),
                    offset: printer.offset + Vec2::new(x, 0),
                    cells: Vec2::new(width, COVER_ROWS),
                });
                text_x = x + width + 3;
            }
            self.images.set(images);
        }

        let width = printer.size.x.saturating_sub(text_x + 2);
        let secondary = ColorStyle::new(
            ColorType::Palette(PaletteColor::Secondary),
            ColorType::Palette(PaletteColor::Background),
        );
        let accent = ColorStyle::new(
            ColorType::Palette(PaletteColor::TitlePrimary),
            ColorType::Palette(PaletteColor::Background),
        );
        printer.with_color(secondary, |printer| {
            printer.print((text_x, 1), &truncate(&header.kind, width));
        });
        printer.with_color(accent, |printer| {
            printer.with_effect(Effect::Bold, |printer| {
                printer.print((text_x, 3), &truncate(&header.title, width));
            });
        });
        printer.with_color(secondary, |printer| {
            printer.print((text_x, 5), &truncate(&header.details.join(" • "), width));
        });
    }

    /// Stop showing the cover, e.g. while no page with a header is shown.
    pub fn hide(&self) {
        #[cfg(feature = "cover")]
        self.images.set(Vec::new());
    }

    /// Update the cover in the terminal; must be called after cursive has drawn.
    #[cfg(feature = "cover")]
    pub fn render_to_terminal(&self, overlays: &[Area]) {
        self.images
            .render_to_terminal(overlays, terminal_font_size());
    }
}

/// Cut `text` to at most `width` columns, ending with "…" if it was cut.
fn truncate(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_string();
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
    if width > 0 {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_songs_and_length() {
        assert_eq!(
            PageHeader::songs_and_length([60_000, 125_000]),
            "2 songs, 3 min 5 sec"
        );
        assert_eq!(
            PageHeader::songs_and_length([3_780_000]),
            "1 song, 1 hr 3 min"
        );
        assert_eq!(PageHeader::songs_and_length([]), "0 songs, 0 min 0 sec");
    }
}
