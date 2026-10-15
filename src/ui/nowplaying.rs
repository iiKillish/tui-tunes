use std::sync::Arc;

use cursive::theme::{ColorStyle, ColorType, Effect, PaletteColor};
use cursive::traits::View;
use cursive::{Printer, Vec2};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::config::Config;
use crate::library::Library;
use crate::model::playable::Playable;
use crate::queue::{Queue, RepeatSetting};
use crate::spotify::{PlayerEvent, Spotify};
#[cfg(feature = "cover")]
use crate::ui::cover::CoverRenderer;
use crate::utils::ms_to_hms;

/// Default height of the card in rows.
const DEFAULT_HEIGHT: usize = 8;

/// Now-playing card shown above the status bar: album art (when built with the `cover` feature)
/// next to the title, artists, album and a progress bar.
pub struct NowPlaying {
    queue: Arc<Queue>,
    spotify: Spotify,
    library: Arc<Library>,
    height: usize,
    #[cfg(feature = "cover")]
    cover: CoverRenderer,
}

impl NowPlaying {
    pub fn new(queue: Arc<Queue>, library: Arc<Library>, config: &Config) -> Self {
        Self {
            spotify: queue.get_spotify(),
            queue,
            library,
            height: config
                .values()
                .now_playing_height
                .unwrap_or(DEFAULT_HEIGHT),
            #[cfg(feature = "cover")]
            cover: CoverRenderer::new(config),
        }
    }

    /// Rows the card takes up: 0 when it's disabled or nothing is loaded.
    pub fn height(&self) -> usize {
        if self.queue.get_current().is_some() {
            self.height
        } else {
            0
        }
    }

    /// Stop showing the card's cover, e.g. while the card itself isn't drawn.
    pub fn hide(&self) {
        #[cfg(feature = "cover")]
        self.cover.clear_cover();
    }

    /// Draw (or remove) the cover in the terminal. Must be called after cursive has drawn.
    /// `obscured` hides it while something is drawn on top of the layout, like a dialog: terminal
    /// images are always drawn over text.
    #[cfg(feature = "cover")]
    pub fn render_to_terminal(&self, obscured: bool) {
        if obscured {
            self.cover.clear_cover();
        }
        self.cover.render_to_terminal();
    }

    /// Width in cells of a square cover `rows` tall.
    fn cover_width(&self, rows: usize) -> usize {
        #[cfg(feature = "cover")]
        {
            let font = self.cover.font_size();
            (rows * font.y).div_ceil(font.x.max(1))
        }
        #[cfg(not(feature = "cover"))]
        {
            let _ = rows;
            0
        }
    }

    fn status_line(&self) -> String {
        let state = match self.spotify.get_current_status() {
            PlayerEvent::Playing(_) => "▶ Playing",
            PlayerEvent::Paused(_) => "▮▮ Paused",
            PlayerEvent::Stopped | PlayerEvent::FinishedTrack => "◼ Stopped",
        };
        let mut parts = vec![state.to_string()];
        if self.queue.get_shuffle() {
            parts.push("shuffle".to_string());
        }
        match self.queue.get_repeat() {
            RepeatSetting::None => {}
            RepeatSetting::RepeatPlaylist => parts.push("repeat".to_string()),
            RepeatSetting::RepeatTrack => parts.push("repeat track".to_string()),
        }
        parts.join("  ·  ")
    }
}

/// Cut `text` to at most `width` columns, ending with "…" if it was cut.
fn truncate(text: &str, width: usize) -> String {
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
        if used + w > width - 1 {
            break;
        }
        out.push(c);
        used += w;
    }
    out.push('…');
    out
}

impl View for NowPlaying {
    fn draw(&self, printer: &Printer<'_, '_>) {
        let size = printer.size;
        let background = ColorStyle::new(
            ColorType::Palette(PaletteColor::Primary),
            ColorType::Palette(PaletteColor::Background),
        );
        printer.with_color(background, |printer| {
            for y in 0..size.y {
                printer.print_hline((0, y), size.x, " ");
            }
        });

        let Some(track) = self.queue.get_current() else {
            self.hide();
            return;
        };
        if size.y == 0 || size.x < 20 {
            self.hide();
            return;
        }

        // Cover on the left, as a square filling the card's height (minus a blank row below
        // to separate it from the status bar), unless the window is too narrow for it.
        let cover_rows = size.y.saturating_sub(1).max(1);
        let mut cover_cols = self.cover_width(cover_rows);
        if cover_cols * 3 > size.x {
            cover_cols = 0;
        }

        #[cfg(feature = "cover")]
        match track.cover_url() {
            Some(url) if cover_cols > 0 => self.cover.draw_cover(
                url,
                printer.offset + Vec2::new(1, 0),
                Vec2::new(cover_cols, cover_rows),
            ),
            _ => self.hide(),
        }

        let text_x = if cover_cols > 0 { cover_cols + 3 } else { 1 };
        let text_width = size.x.saturating_sub(text_x + 1);

        let title = Playable::format(&track, "%title", &self.library);
        let artists = Playable::format(&track, "%artists", &self.library);
        let album = Playable::format(&track, "%album", &self.library);

        // Title, artists, album, gap, progress bar, status; vertically centered.
        let lines = 6;
        let top = cover_rows.saturating_sub(lines) / 2;

        printer.with_color(ColorStyle::primary(), |printer| {
            printer.with_effect(Effect::Bold, |printer| {
                printer.print((text_x, top), &truncate(&title, text_width));
            });
        });
        printer.with_color(ColorStyle::title_primary(), |printer| {
            printer.print((text_x, top + 1), &truncate(&artists, text_width));
        });
        printer.with_color(ColorStyle::secondary(), |printer| {
            printer.print((text_x, top + 2), &truncate(&album, text_width));
        });

        // Progress bar with elapsed / total on its right.
        let elapsed_ms = self.spotify.get_current_progress().as_millis() as u32;
        let times = format!(
            " {} / {}",
            ms_to_hms(elapsed_ms.min(track.duration())),
            track.duration_str()
        );
        let bar_width = text_width.saturating_sub(times.width());
        let filled = (elapsed_ms as u64 * bar_width as u64)
            .checked_div(track.duration() as u64)
            .unwrap_or(0)
            .min(bar_width as u64) as usize;
        let palette = &printer.theme.palette;
        let style_bar = ColorStyle::new(
            ColorType::Color(*palette.custom("statusbar_progress").unwrap()),
            ColorType::Palette(PaletteColor::Background),
        );
        let style_bar_bg = ColorStyle::new(
            ColorType::Color(*palette.custom("statusbar_progress_bg").unwrap()),
            ColorType::Palette(PaletteColor::Background),
        );
        printer.with_color(style_bar_bg, |printer| {
            printer.print((text_x, top + 4), &"┉".repeat(bar_width));
        });
        printer.with_color(style_bar, |printer| {
            printer.print((text_x, top + 4), &"━".repeat(filled));
        });
        printer.with_color(ColorStyle::secondary(), |printer| {
            printer.print((text_x + bar_width, top + 4), &times);
            printer.print(
                (text_x, top + 5),
                &truncate(&self.status_line(), text_width),
            );
        });
    }

    fn required_size(&mut self, constraint: Vec2) -> Vec2 {
        Vec2::new(constraint.x, self.height().min(constraint.y))
    }
}
