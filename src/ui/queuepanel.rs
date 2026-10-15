use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};

use cursive::event::{Event, EventResult, MouseButton, MouseEvent};
use cursive::theme::{ColorStyle, ColorType, Effect, PaletteColor};
use cursive::traits::View;
use cursive::{Printer, Vec2};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::config::Config;
use crate::library::Library;
use crate::model::playable::Playable;
use crate::queue::Queue;

/// Default width of the panel in columns.
const DEFAULT_WIDTH: usize = 40;
/// Most upcoming items looked at for the panel; more than ever fit on screen.
const MAX_UPCOMING: usize = 200;

/// Queue panel on the right, like Spotify's: the current track under "Now playing", then what's
/// coming up, grouped by where it was queued from ("Next from: <album or playlist>"). Clicking an
/// item plays it; the mouse wheel scrolls.
pub struct QueuePanel {
    queue: Arc<Queue>,
    library: Arc<Library>,
    width: usize,
    /// Number of upcoming items scrolled past, and the most that's useful (beyond that the list
    /// would end before the bottom of the panel).
    scroll: AtomicUsize,
    max_scroll: AtomicUsize,
    /// For each row drawn last, the queue index of the item shown there (for clicks).
    row_items: RwLock<Vec<Option<usize>>>,
}

impl QueuePanel {
    pub fn new(queue: Arc<Queue>, library: Arc<Library>, config: &Config) -> Self {
        Self {
            queue,
            library,
            width: config.values().queue_panel_width.unwrap_or(DEFAULT_WIDTH),
            scroll: AtomicUsize::new(0),
            max_scroll: AtomicUsize::new(0),
            row_items: RwLock::new(Vec::new()),
        }
    }

    /// Configured width in columns; 0 means the panel is disabled.
    pub fn width(&self) -> usize {
        self.width
    }
}

/// Rows needed to list items with these sources: two per item, plus a header (and a blank line
/// before every header but the first) wherever the source changes.
fn rows_needed(sources: &[Option<Arc<str>>]) -> usize {
    let mut rows = 0;
    let mut previous: Option<&Option<Arc<str>>> = None;
    for source in sources {
        if previous != Some(source) {
            rows += if previous.is_some() { 2 } else { 1 };
            previous = Some(source);
        }
        rows += 2;
    }
    rows
}

/// The furthest the list can usefully scroll: the first start position from which the rest fits
/// in `rows`. 0 when everything fits.
fn max_scroll(sources: &[Option<Arc<str>>], rows: usize) -> usize {
    (0..sources.len())
        .find(|&start| rows_needed(&sources[start..]) <= rows)
        .unwrap_or(sources.len().saturating_sub(1))
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

impl View for QueuePanel {
    fn draw(&self, printer: &Printer<'_, '_>) {
        let size = printer.size;
        let palette = &printer.theme.palette;
        let background = ColorType::Color(*palette.custom("statusbar_bg").unwrap());
        let style = |front: ColorType| ColorStyle::new(front, background);
        let primary = style(ColorType::Palette(PaletteColor::Primary));
        let secondary = style(ColorType::Palette(PaletteColor::Secondary));
        let accent = style(ColorType::Palette(PaletteColor::TitlePrimary));
        let playing = style(ColorType::Color(*palette.custom("playing").unwrap()));

        printer.with_color(primary, |printer| {
            for y in 0..size.y {
                printer.print_hline((0, y), size.x, " ");
            }
        });

        let x = 2;
        let text_width = size.x.saturating_sub(x + 2);
        let mut rows: Vec<Option<usize>> = vec![None; size.y];
        let mut y = 1;

        let line = |printer: &Printer<'_, '_>, y: usize, style, bold: bool, text: &str| {
            if y >= size.y {
                return;
            }
            printer.with_color(style, |printer| {
                let text = truncate(text, text_width);
                if bold {
                    printer.with_effect(Effect::Bold, |printer| printer.print((x, y), &text));
                } else {
                    printer.print((x, y), &text);
                }
            });
        };
        let format =
            |track: &Playable, format: &str| Playable::format(track, format, &self.library);

        line(printer, y, primary, true, "Queue");
        y += 2;

        if let Some(current) = self.queue.get_current() {
            line(printer, y, accent, true, "Now playing");
            y += 1;
            line(printer, y, playing, true, &format(&current, "%title"));
            line(
                printer,
                y + 1,
                secondary,
                false,
                &format(&current, "%artists"),
            );
            if let Some(index) = self.queue.get_current_index() {
                rows.iter_mut()
                    .skip(y)
                    .take(2)
                    .for_each(|r| *r = Some(index));
            }
            y += 3;
        }

        let upcoming = self.queue.upcoming(MAX_UPCOMING);
        let sources: Vec<Option<Arc<str>>> = upcoming
            .iter()
            .map(|(i, _)| self.queue.source(*i))
            .collect();
        let max_scroll = max_scroll(&sources, size.y.saturating_sub(y));
        let scroll = self.scroll.load(Ordering::Relaxed).min(max_scroll);
        self.scroll.store(scroll, Ordering::Relaxed);
        self.max_scroll.store(max_scroll, Ordering::Relaxed);

        if upcoming.is_empty() {
            line(printer, y, secondary, false, "Nothing up next");
        }

        // Group consecutive items by source: a header whenever it changes (and at the top).
        let mut previous_source: Option<Option<Arc<str>>> = None;
        for ((index, track), source) in upcoming.iter().zip(sources).skip(scroll) {
            if y + 2 > size.y {
                break;
            }
            if previous_source.as_ref() != Some(&source) {
                if previous_source.is_some() {
                    y += 1;
                }
                if y + 3 > size.y {
                    break;
                }
                let header = match &source {
                    Some(name) => format!("Next from: {name}"),
                    None => "Next up".to_string(),
                };
                line(printer, y, accent, true, &header);
                y += 1;
                previous_source = Some(source);
            }
            line(printer, y, primary, false, &format(track, "%title"));
            line(printer, y + 1, secondary, false, &format(track, "%artists"));
            rows.iter_mut()
                .skip(y)
                .take(2)
                .for_each(|r| *r = Some(*index));
            y += 2;
        }

        *self.row_items.write().unwrap() = rows;
    }

    fn required_size(&mut self, constraint: Vec2) -> Vec2 {
        Vec2::new(self.width.min(constraint.x), constraint.y)
    }

    fn on_event(&mut self, event: Event) -> EventResult {
        let Event::Mouse {
            offset,
            position,
            event,
        } = event
        else {
            return EventResult::Ignored;
        };
        let Some(position) = position.checked_sub(offset) else {
            return EventResult::Ignored;
        };
        match event {
            MouseEvent::Press(MouseButton::Left) => {
                let item = self
                    .row_items
                    .read()
                    .unwrap()
                    .get(position.y)
                    .copied()
                    .flatten();
                if let Some(index) = item
                    && Some(index) != self.queue.get_current_index()
                {
                    self.queue.play(index, false, false);
                }
            }
            MouseEvent::WheelDown => {
                // Only scroll while there's more below.
                let scroll = self.scroll.load(Ordering::Relaxed);
                if scroll < self.max_scroll.load(Ordering::Relaxed) {
                    self.scroll.store(scroll + 1, Ordering::Relaxed);
                }
            }
            MouseEvent::WheelUp => {
                let scroll = self.scroll.load(Ordering::Relaxed);
                self.scroll
                    .store(scroll.saturating_sub(1), Ordering::Relaxed);
            }
            _ => {}
        }
        EventResult::Consumed(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sources(names: &[Option<&str>]) -> Vec<Option<Arc<str>>> {
        names.iter().map(|n| n.map(Arc::from)).collect()
    }

    #[test]
    fn test_rows_needed_counts_headers() {
        // header + 2 items, then gap + header + 1 item.
        let s = sources(&[Some("A"), Some("A"), None]);
        assert_eq!(rows_needed(&s), 1 + 4 + 2 + 2);
    }

    #[test]
    fn test_no_scrolling_when_everything_fits() {
        let s = sources(&[Some("A"); 3]);
        assert_eq!(max_scroll(&s, 7), 0);
        assert_eq!(max_scroll(&s, 50), 0);
    }

    #[test]
    fn test_scrolls_only_until_the_last_item_is_at_the_bottom() {
        // 10 items from one source need 21 rows; 9 rows fit a header and 4 items.
        let s = sources(&[Some("A"); 10]);
        assert_eq!(max_scroll(&s, 9), 6);
    }
}
