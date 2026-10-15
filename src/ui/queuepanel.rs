use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};

use cursive::event::{Event, EventResult, MouseButton, MouseEvent};
use cursive::theme::{ColorStyle, ColorType, Effect, PaletteColor};
use cursive::traits::View;
use cursive::{Printer, Vec2};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::command::{Command, MoveAmount, MoveMode, ShiftMode};
use crate::config::Config;
use crate::library::Library;
use crate::model::playable::Playable;
use crate::queue::Queue;
#[cfg(feature = "cover")]
use crate::ui::cover::{
    Area, ImageLayer, ImageRequest, QUEUE_THUMBNAIL_IMAGE_ID, kitty_supported, terminal_font_size,
};
use crate::ui::lyricsview::LyricsView;

/// Default width of the panel in columns.
const DEFAULT_WIDTH: usize = 40;
/// Most upcoming items looked at for the panel; more than ever fit on screen.
const MAX_UPCOMING: usize = 200;
/// Row of the tabs, and the first row of a tab's content.
const TABS_ROW: usize = 1;
const CONTENT_ROW: usize = 3;
/// Tab labels and where they start; each is drawn with a space either side.
const QUEUE_TAB: (&str, usize) = ("Queue", 1);
const LYRICS_TAB: (&str, usize) = ("Lyrics", 9);
/// Items moved by a page up/down (`MoveAmount::Float`) in the queue.
const QUEUE_PAGE: usize = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelTab {
    Queue,
    Lyrics,
}

/// Side panel on the right, like Spotify's, with two tabs:
///
/// * Queue: the current track under "Now playing", then what's coming up, grouped by where it was
///   queued from ("Next from: <album or playlist>").
/// * Lyrics: the current track's lyrics from LRCLIB, with the line being sung highlighted.
///
/// Everything can be clicked (items play, lyrics lines seek) and scrolled. With keyboard focus
/// (see [crate::command::PanelAction::Focus]) the usual movement, play, delete and shift commands
/// work on the panel instead of the main view.
pub struct QueuePanel {
    queue: Arc<Queue>,
    library: Arc<Library>,
    width: usize,
    tab: RwLock<PanelTab>,
    focused: AtomicBool,

    /// Number of upcoming items scrolled past, and the most that's useful (beyond that the list
    /// would end before the bottom of the panel).
    scroll: AtomicUsize,
    max_scroll: AtomicUsize,
    /// Selected item while focused: 0 is the current track, n the nth upcoming one.
    selected: AtomicUsize,
    /// Set when the selection moves, so the next draw scrolls it into view.
    follow_selection: AtomicBool,
    /// For each row drawn last, the queue index of the item shown there (for clicks).
    row_items: RwLock<Vec<Option<usize>>>,

    lyrics: LyricsView,

    #[cfg(feature = "cover")]
    thumbnails: ImageLayer,
    #[cfg(feature = "cover")]
    font_size: Vec2,
}

impl QueuePanel {
    pub fn new(queue: Arc<Queue>, library: Arc<Library>, config: &Config) -> Self {
        Self {
            lyrics: LyricsView::new(queue.clone(), library.clone()).on_panel_background(),
            queue,
            library,
            width: config.values().queue_panel_width.unwrap_or(DEFAULT_WIDTH),
            tab: RwLock::new(PanelTab::Queue),
            focused: AtomicBool::new(false),
            scroll: AtomicUsize::new(0),
            max_scroll: AtomicUsize::new(0),
            selected: AtomicUsize::new(0),
            follow_selection: AtomicBool::new(false),
            row_items: RwLock::new(Vec::new()),
            #[cfg(feature = "cover")]
            thumbnails: ImageLayer::default(),
            #[cfg(feature = "cover")]
            font_size: terminal_font_size(),
        }
    }

    /// Configured width in columns; 0 means the panel is disabled.
    pub fn width(&self) -> usize {
        self.width
    }

    pub fn tab(&self) -> PanelTab {
        *self.tab.read().unwrap()
    }

    pub fn set_tab(&self, tab: PanelTab) {
        *self.tab.write().unwrap() = tab;
    }

    /// Whether keyboard commands go to the panel rather than the main view.
    pub fn is_focused(&self) -> bool {
        self.focused.load(Ordering::Relaxed)
    }

    pub fn set_focused(&self, focused: bool) {
        if focused && !self.is_focused() {
            // Start from the current track, like opening Spotify's queue.
            self.selected.store(0, Ordering::Relaxed);
            self.follow_selection.store(true, Ordering::Relaxed);
        }
        self.focused.store(focused, Ordering::Relaxed);
    }

    /// Stop showing thumbnails, e.g. while the panel isn't drawn.
    pub fn hide(&self) {
        #[cfg(feature = "cover")]
        self.thumbnails.set(Vec::new());
    }

    /// Draw (or remove) thumbnails in the terminal; must be called after cursive has drawn.
    /// Thumbnails that would cover part of `overlays` (dialogs or menus) are left out.
    #[cfg(feature = "cover")]
    pub fn render_to_terminal(&self, overlays: &[Area]) {
        self.thumbnails.render_to_terminal(overlays, self.font_size);
    }

    /// Width in cells of an item's square thumbnail (two rows tall), or 0 without image support.
    fn thumbnail_width(&self) -> usize {
        #[cfg(feature = "cover")]
        if kitty_supported() {
            return (2 * self.font_size.y).div_ceil(self.font_size.x.max(1));
        }
        0
    }

    /// Run `cmd` on the panel while it has keyboard focus. Returns whether it applied; commands
    /// that don't are left to the main view.
    pub fn handle_command(&self, cmd: &Command) -> bool {
        match cmd {
            Command::Back => self.set_focused(false),
            Command::Move(MoveMode::Left, _) => self.set_tab(PanelTab::Queue),
            Command::Move(MoveMode::Right, _) => self.set_tab(PanelTab::Lyrics),
            _ => {
                return match self.tab() {
                    PanelTab::Queue => self.queue_command(cmd),
                    PanelTab::Lyrics => self.lyrics.handle_command(cmd),
                };
            }
        }
        true
    }

    fn queue_command(&self, cmd: &Command) -> bool {
        let upcoming = self.queue.upcoming(MAX_UPCOMING);
        // Selectable items: the current track (if any) and the upcoming ones.
        let has_current = self.queue.get_current_index().is_some();
        let last = upcoming.len().saturating_sub(usize::from(!has_current));
        let selected = self.selected.load(Ordering::Relaxed).min(last);
        // The queue index of an item by its position in the panel.
        let index_of = |position: usize| match position {
            0 if has_current => self.queue.get_current_index(),
            p => upcoming
                .get(p - usize::from(has_current))
                .map(|(index, _)| *index),
        };
        let select = |position: usize| {
            self.selected.store(position.min(last), Ordering::Relaxed);
            self.follow_selection.store(true, Ordering::Relaxed);
        };

        match cmd {
            Command::Move(mode @ (MoveMode::Up | MoveMode::Down), amount) => {
                let step = match amount {
                    MoveAmount::Integer(n) => n.unsigned_abs() as usize,
                    MoveAmount::Float(_) => QUEUE_PAGE,
                    MoveAmount::Extreme => usize::MAX,
                };
                match mode {
                    MoveMode::Up => select(selected.saturating_sub(step)),
                    _ => select(selected.saturating_add(step)),
                }
            }
            Command::Move(MoveMode::Playing, _) => select(0),
            Command::Play => {
                if let Some(index) = index_of(selected)
                    && Some(index) != self.queue.get_current_index()
                {
                    self.queue.play(index, false, false);
                    select(0);
                }
            }
            Command::Delete => {
                if let Some(index) = index_of(selected) {
                    self.queue.remove(index);
                }
            }
            Command::Shift(mode, amount) => {
                // Upcoming items move among themselves; the current track stays put.
                let first = usize::from(has_current);
                let Some(from) = index_of(selected).filter(|_| selected >= first) else {
                    return true;
                };
                let amount = amount.unwrap_or(1);
                let up = matches!(mode, ShiftMode::Up) == (amount >= 0);
                let amount = amount.unsigned_abs() as usize;
                let target = if up {
                    selected.saturating_sub(amount).max(first)
                } else {
                    (selected + amount).min(last)
                };
                if let Some(to) = index_of(target)
                    && to != from
                {
                    self.queue.shift(from, to);
                    select(target);
                }
            }
            _ => return false,
        }
        true
    }

    fn draw_tabs(&self, printer: &Printer<'_, '_>) {
        let palette = &printer.theme.palette;
        let background = ColorType::Color(*palette.custom("statusbar_bg").unwrap());
        let tab = self.tab();
        for (label, x, this) in [
            (QUEUE_TAB.0, QUEUE_TAB.1, PanelTab::Queue),
            (LYRICS_TAB.0, LYRICS_TAB.1, PanelTab::Lyrics),
        ] {
            let text = format!(" {label} ");
            let style = if tab != this {
                ColorStyle::new(ColorType::Palette(PaletteColor::Secondary), background)
            } else if self.is_focused() {
                // Focus shows as the active tab drawn like a selected list item.
                ColorStyle::highlight()
            } else {
                ColorStyle::new(ColorType::Palette(PaletteColor::TitlePrimary), background)
            };
            printer.with_color(style, |printer| {
                if tab == this {
                    printer
                        .with_effect(Effect::Bold, |printer| printer.print((x, TABS_ROW), &text));
                } else {
                    printer.print((x, TABS_ROW), &text);
                }
            });
        }
    }

    fn draw_queue(&self, printer: &Printer<'_, '_>) {
        let size = printer.size;
        let palette = &printer.theme.palette;
        let background = ColorType::Color(*palette.custom("statusbar_bg").unwrap());
        let style = |front: ColorType| ColorStyle::new(front, background);
        let primary = style(ColorType::Palette(PaletteColor::Primary));
        let secondary = style(ColorType::Palette(PaletteColor::Secondary));
        let accent = style(ColorType::Palette(PaletteColor::TitlePrimary));
        let playing = style(ColorType::Color(*palette.custom("playing").unwrap()));
        let highlight = ColorStyle::highlight();

        let x = 2;
        // Items have a thumbnail on the left (when images are supported), their text after it.
        let thumbnail_width = self.thumbnail_width();
        let item_x = if thumbnail_width > 0 {
            x + thumbnail_width + 1
        } else {
            x
        };
        let mut rows: Vec<Option<usize>> = vec![None; size.y];
        let mut y = CONTENT_ROW;

        let line =
            |printer: &Printer<'_, '_>, x: usize, y: usize, style, bold: bool, text: &str| {
                if y >= size.y {
                    return;
                }
                printer.with_color(style, |printer| {
                    let text = truncate(text, size.x.saturating_sub(x + 2));
                    if bold {
                        printer.with_effect(Effect::Bold, |printer| printer.print((x, y), &text));
                    } else {
                        printer.print((x, y), &text);
                    }
                });
            };
        // An item's two rows: title and artists, in the highlight colors when it's selected.
        let focused = self.is_focused();
        let selected = self.selected.load(Ordering::Relaxed);
        let item = |printer: &Printer<'_, '_>,
                    y: usize,
                    position: usize,
                    track: &Playable,
                    title_style: ColorStyle,
                    bold: bool| {
            let is_selected = focused && position == selected;
            if is_selected {
                printer.with_color(highlight, |printer| {
                    for row in y..(y + 2).min(size.y) {
                        printer.print_hline((x - 1, row), size.x.saturating_sub(x), " ");
                    }
                });
            }
            let (title_style, artists_style) = if is_selected {
                (highlight, highlight)
            } else {
                (title_style, secondary)
            };
            let format = |format: &str| Playable::format(track, format, &self.library);
            line(printer, item_x, y, title_style, bold, &format("%title"));
            line(
                printer,
                item_x,
                y + 1,
                artists_style,
                false,
                &format("%artists"),
            );
        };
        #[cfg(feature = "cover")]
        let mut thumbnails: Vec<ImageRequest> = Vec::new();
        #[allow(unused_variables, unused_mut)]
        let mut add_thumbnail = |track: &Playable, y: usize| {
            #[cfg(feature = "cover")]
            if thumbnail_width > 0
                && y + 2 <= size.y
                && let Some(url) = track.cover_url()
            {
                thumbnails.push(ImageRequest {
                    image_id: QUEUE_THUMBNAIL_IMAGE_ID + thumbnails.len() as u32,
                    url,
                    offset: printer.offset + Vec2::new(x, y),
                    cells: Vec2::new(thumbnail_width, 2),
                });
            }
        };

        let has_current = match self.queue.get_current() {
            Some(current) => {
                line(printer, x, y, accent, true, "Now playing");
                y += 1;
                add_thumbnail(&current, y);
                item(printer, y, 0, &current, playing, true);
                if let Some(index) = self.queue.get_current_index() {
                    rows.iter_mut()
                        .skip(y)
                        .take(2)
                        .for_each(|r| *r = Some(index));
                }
                y += ITEM_ROWS;
                true
            }
            None => false,
        };

        let upcoming = self.queue.upcoming(MAX_UPCOMING);
        let sources: Vec<Option<Arc<str>>> = upcoming
            .iter()
            .map(|(i, _)| self.queue.source(*i))
            .collect();
        let first_upcoming = usize::from(has_current);
        let last = upcoming.len().saturating_sub(usize::from(!has_current));
        let selected = selected.min(last);
        self.selected.store(selected, Ordering::Relaxed);

        let available = size.y.saturating_sub(y);
        let max_scroll = max_scroll(&sources, available);
        let mut scroll = self.scroll.load(Ordering::Relaxed);
        // Keep a selection made with the keyboard in view.
        if focused
            && self.follow_selection.swap(false, Ordering::Relaxed)
            && let Some(u) = selected.checked_sub(first_upcoming)
        {
            if u < scroll {
                scroll = u;
            }
            while scroll < u && rows_needed(&sources[scroll..=u]) > available {
                scroll += 1;
            }
        }
        let scroll = scroll.min(max_scroll);
        self.scroll.store(scroll, Ordering::Relaxed);
        self.max_scroll.store(max_scroll, Ordering::Relaxed);

        if upcoming.is_empty() {
            line(printer, x, y, secondary, false, "Nothing up next");
        }

        // Group consecutive items by source: a header whenever it changes (and at the top).
        let mut previous_source: Option<Option<Arc<str>>> = None;
        for (u, ((index, track), source)) in upcoming.iter().zip(sources).enumerate().skip(scroll) {
            if y + 2 > size.y {
                break;
            }
            if previous_source.as_ref() != Some(&source) {
                // A header plus at least one item has to fit.
                if y + 3 > size.y {
                    break;
                }
                let header = match &source {
                    Some(name) => format!("Next from: {name}"),
                    None => "Next up".to_string(),
                };
                line(printer, x, y, accent, true, &header);
                y += 1;
                previous_source = Some(source);
            }
            add_thumbnail(track, y);
            item(printer, y, u + first_upcoming, track, primary, false);
            rows.iter_mut()
                .skip(y)
                .take(2)
                .for_each(|r| *r = Some(*index));
            y += ITEM_ROWS;
        }

        *self.row_items.write().unwrap() = rows;
        #[cfg(feature = "cover")]
        self.thumbnails.set(thumbnails);
    }

    fn draw_lyrics(&self, printer: &Printer<'_, '_>) {
        let size = printer.size;
        self.lyrics.draw(
            &printer
                .offset((0, CONTENT_ROW))
                .cropped((size.x, size.y.saturating_sub(CONTENT_ROW))),
            2,
        );
        // No thumbnails on this tab.
        self.hide();
    }
}

/// Rows an item takes: title and artists, then a blank row before whatever comes next.
const ITEM_ROWS: usize = 3;

/// Rows needed to list items with these sources: [ITEM_ROWS] per item (without the last one's
/// blank row), plus a header wherever the source changes.
fn rows_needed(sources: &[Option<Arc<str>>]) -> usize {
    let mut rows = 0;
    let mut previous: Option<&Option<Arc<str>>> = None;
    for source in sources {
        if previous != Some(source) {
            rows += 1;
            previous = Some(source);
        }
        rows += ITEM_ROWS;
    }
    rows.saturating_sub(1)
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
        printer.with_color(
            ColorStyle::new(ColorType::Palette(PaletteColor::Primary), background),
            |printer| {
                for y in 0..size.y {
                    printer.print_hline((0, y), size.x, " ");
                }
            },
        );

        self.draw_tabs(printer);
        match self.tab() {
            PanelTab::Queue => self.draw_queue(printer),
            PanelTab::Lyrics => self.draw_lyrics(printer),
        }
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

        if position.y == TABS_ROW && event == MouseEvent::Press(MouseButton::Left) {
            for (label, x, tab) in [
                (QUEUE_TAB.0, QUEUE_TAB.1, PanelTab::Queue),
                (LYRICS_TAB.0, LYRICS_TAB.1, PanelTab::Lyrics),
            ] {
                if (x..x + label.width() + 2).contains(&position.x) {
                    self.set_tab(tab);
                }
            }
            return EventResult::Consumed(None);
        }

        match (self.tab(), event) {
            (PanelTab::Queue, MouseEvent::Press(MouseButton::Left)) => {
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
                    self.selected.store(0, Ordering::Relaxed);
                }
            }
            (PanelTab::Queue, MouseEvent::WheelDown) => {
                // Only scroll while there's more below.
                let scroll = self.scroll.load(Ordering::Relaxed);
                if scroll < self.max_scroll.load(Ordering::Relaxed) {
                    self.scroll.store(scroll + 1, Ordering::Relaxed);
                }
            }
            (PanelTab::Queue, MouseEvent::WheelUp) => {
                let scroll = self.scroll.load(Ordering::Relaxed);
                self.scroll
                    .store(scroll.saturating_sub(1), Ordering::Relaxed);
            }
            (PanelTab::Lyrics, event) => {
                if let Some(position) = position.checked_sub((0, CONTENT_ROW)) {
                    self.lyrics.on_mouse(event, position);
                }
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
    fn test_rows_needed_counts_headers_and_gaps() {
        // header, item, gap, item, gap, header, item (the last item's gap isn't needed).
        let s = sources(&[Some("A"), Some("A"), None]);
        assert_eq!(rows_needed(&s), 1 + 3 + 3 + 1 + 2);
    }

    #[test]
    fn test_no_scrolling_when_everything_fits() {
        // header + 3 items with gaps between = 1 + 3 + 3 + 2 rows.
        let s = sources(&[Some("A"); 3]);
        assert_eq!(max_scroll(&s, 9), 0);
        assert_eq!(max_scroll(&s, 50), 0);
    }

    #[test]
    fn test_scrolls_only_until_the_last_item_is_at_the_bottom() {
        // 10 rows fit a header and 3 items (1 + 3 + 3 + 2 = 9), not 4 (12).
        let s = sources(&[Some("A"); 10]);
        assert_eq!(max_scroll(&s, 10), 7);
    }
}
