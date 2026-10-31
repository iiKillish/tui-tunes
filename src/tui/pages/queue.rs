//! The Up next screen, like Spotify's queue: the current track under "Now playing", then what's
//! coming up, grouped by where it was queued from ("Next from: <album or playlist>", or "Next up"
//! for tracks queued one by one).
//!
//! Items take two rows (title, then artists) next to a two-row cover, with a blank row between
//! them. The selection moves over the current track and the upcoming ones; play, delete and shift
//! work on it (upcoming items only shift among themselves). Clicking an item plays it. A Clear
//! button at the top right clears what's up next.

use std::sync::Arc;

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use ratatui::style::Style;

use crate::command::{Command, MoveAmount, MoveMode, ShiftMode, TargetMode};
use crate::model::playable::Playable;
use crate::tui::hints;
use crate::tui::images;
use crate::tui::nav;
use crate::tui::page::{Ctx, Outcome, Page};
use crate::tui::pointer;
use crate::tui::style::{
    SELECTION_MARKER, Styles, bold, duration, fill, put, songs_and_length, width,
};
use crate::tui::wheel;

/// Most upcoming items looked at; more than ever fit on screen.
const MAX_UPCOMING: usize = 200;
/// Rows an item takes: title and artists, then a blank row before whatever comes next.
const ITEM_ROWS: usize = 3;
/// Columns the headings and items are indented by.
const INDENT: u16 = 2;
/// Narrowest page that shows covers before its items.
const MIN_WIDTH_FOR_THUMBNAILS: u16 = 32;

pub struct QueuePage {
    /// Selected item: 0 is the current track (if there is one), then the upcoming ones.
    selected: usize,
    /// Set when the selection moves, so the next draw scrolls it into view.
    follow_selection: bool,
    /// Number of upcoming items scrolled past, and the most that's useful (beyond that the list
    /// would end before the bottom of the page).
    scroll: usize,
    max_scroll: usize,
    /// Where the page was drawn last, and for each of its rows the queue index of the item shown
    /// there (for clicks).
    area: Rect,
    row_items: Vec<Option<usize>>,
    /// Where the Clear button was drawn last, if it was.
    clear_button: Option<Rect>,
    /// Upcoming items drawn last.
    upcoming_shown: usize,
    /// "12 songs, 45 min" for what's coming up, worked out while drawing.
    subtitle: String,
}

pub fn new(_ctx: &Ctx) -> QueuePage {
    QueuePage {
        selected: 0,
        follow_selection: true,
        scroll: 0,
        max_scroll: 0,
        area: Rect::default(),
        row_items: Vec::new(),
        clear_button: None,
        upcoming_shown: 0,
        subtitle: String::new(),
    }
}

/// What the page shows: the current item and the upcoming ones, with their queue indices.
struct Items {
    current: Option<(usize, Playable)>,
    upcoming: Vec<(usize, Playable)>,
}

impl Items {
    fn load(ctx: &Ctx) -> Self {
        let current = ctx.queue.get_current_index().zip(ctx.queue.get_current());
        Self {
            current,
            upcoming: ctx.queue.upcoming(MAX_UPCOMING),
        }
    }

    /// Position of the first upcoming item in the selection (after the current one, if any).
    fn first_upcoming(&self) -> usize {
        usize::from(self.current.is_some())
    }

    /// The last position that can be selected.
    fn last(&self) -> usize {
        (self.first_upcoming() + self.upcoming.len()).saturating_sub(1)
    }

    /// The queue index of the item at `position` in the selection.
    fn index(&self, position: usize) -> Option<usize> {
        match (&self.current, position.checked_sub(self.first_upcoming())) {
            (Some((index, _)), None) => Some(*index),
            (_, Some(u)) => self.upcoming.get(u).map(|(index, _)| *index),
            (None, None) => None,
        }
    }

    fn get(&self, position: usize) -> Option<&Playable> {
        match (&self.current, position.checked_sub(self.first_upcoming())) {
            (Some((_, current)), None) => Some(current),
            (_, Some(u)) => self.upcoming.get(u).map(|(_, item)| item),
            (None, None) => None,
        }
    }
}

/// Rows needed to list items with these sources: [ITEM_ROWS] per item (without the last one's
/// blank row), plus a heading wherever the source changes.
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

/// The scroll position from `scroll` that shows upcoming item `item` in `rows`.
fn scroll_to_item(sources: &[Option<Arc<str>>], scroll: usize, item: usize, rows: usize) -> usize {
    if item >= sources.len() {
        return scroll;
    }
    let mut scroll = scroll.min(item);
    while scroll < item && rows_needed(&sources[scroll..=item]) > rows {
        scroll += 1;
    }
    scroll
}

/// Everything needed to draw an item.
struct ItemLook<'a> {
    item: &'a Playable,
    selected: bool,
    focused: bool,
    title_style: Style,
}

impl QueuePage {
    /// Select `position` (clamped to the items) and scroll it into view on the next draw.
    fn select(&mut self, position: usize, items: &Items) {
        self.selected = position.min(items.last());
        self.follow_selection = true;
    }

    /// Upcoming items fitting in the page, for page-wise moves.
    fn page_items(&self) -> usize {
        (usize::from(self.area.height) / ITEM_ROWS).max(1)
    }

    fn shift(&mut self, mode: &ShiftMode, amount: Option<i32>, ctx: &Ctx) {
        let items = Items::load(ctx);
        let first = items.first_upcoming();
        let selected = self.selected.min(items.last());
        // Upcoming items move among themselves; the current track stays put.
        if selected < first {
            return;
        }
        let Some(from) = items.index(selected) else {
            return;
        };
        let amount = amount.unwrap_or(1);
        let up = matches!(mode, ShiftMode::Up) == (amount >= 0);
        let amount = amount.unsigned_abs() as usize;
        let target = if up {
            selected.saturating_sub(amount).max(first)
        } else {
            (selected + amount).min(items.last())
        };
        if let Some(to) = items.index(target)
            && to != from
        {
            ctx.queue.shift(from, to);
            self.select(target, &items);
        }
    }

    /// Play the item at queue `index` unless it's already the current one.
    fn play(&mut self, index: usize, ctx: &Ctx) {
        if Some(index) != ctx.queue.get_current_index() {
            ctx.queue.play(index, false, false);
            self.selected = 0;
            self.scroll = 0;
            self.follow_selection = true;
        }
    }

    fn draw_item(&self, buf: &mut Buffer, y: u16, look: ItemLook, ctx: &Ctx) {
        let styles = Styles(&ctx.palette);
        let area = self.area;
        let rows = 2.min(area.bottom().saturating_sub(y));
        let (title_style, details_style) = if look.selected {
            let style = if look.focused {
                styles.selected()
            } else {
                styles.selected_inactive()
            };
            fill(buf, Rect::new(area.x, y, area.width, rows), style);
            if look.focused {
                for row in y..y + rows {
                    put(buf, area.x, row, SELECTION_MARKER, 1, styles.marker());
                }
            }
            (style, styles.selected_inactive().fg(ctx.palette.secondary))
        } else {
            (look.title_style, styles.secondary())
        };

        let mut x = area.x + INDENT;
        let thumb = thumbnail_width(area.width);
        if thumb > 0 {
            if rows == 2
                && let Some(url) = look.item.cover_url()
            {
                images::cover(&url, Rect::new(x, y, thumb, 2));
            }
            x += thumb + 1;
        }

        let end = area.right().saturating_sub(INDENT);
        let length = duration(look.item.duration());
        let length_x = end.saturating_sub(width(&length) as u16);
        put(buf, length_x, y, &length, width(&length), details_style);

        let room = usize::from(length_x.saturating_sub(x + 1));
        let title = Playable::format(look.item, "%title", &ctx.library);
        put(buf, x, y, &title, room, title_style);
        if rows == 2 {
            let artists = Playable::format(look.item, "%artists", &ctx.library);
            let room = usize::from(end.saturating_sub(x));
            put(buf, x, y + 1, &artists, room, details_style);
        }
    }
}

/// Width of an item's two-row cover, or 0 when the terminal can't show images or the page is
/// too narrow to spare the room.
fn thumbnail_width(page_width: u16) -> u16 {
    if images::supported() && page_width >= MIN_WIDTH_FOR_THUMBNAILS {
        images::square_width(2)
    } else {
        0
    }
}

impl Page for QueuePage {
    fn title(&self) -> String {
        "Up next".into()
    }

    fn subtitle(&self) -> String {
        self.subtitle.clone()
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect, ctx: &Ctx, focused: bool) {
        self.area = area;
        let styles = Styles(&ctx.palette);
        let heading = bold(styles.accent());
        let items = Items::load(ctx);
        self.selected = self.selected.min(items.last());
        self.subtitle = if items.upcoming.len() >= MAX_UPCOMING {
            format!("{MAX_UPCOMING}+ songs")
        } else if items.upcoming.is_empty() {
            String::new()
        } else {
            songs_and_length(items.upcoming.iter().map(|(_, item)| item.duration()))
        };

        let height = usize::from(area.height);
        let x = area.x + INDENT;
        let text_width = usize::from(area.width.saturating_sub(2 * INDENT));
        let mut rows: Vec<Option<usize>> = vec![None; height];
        // Rows within the page, from the top.
        let mut y = 0;
        let buf = frame.buffer_mut();

        // Clear, on the top row at the right, while there's anything to clear.
        self.clear_button = None;
        if !items.upcoming.is_empty() && height >= 3 {
            let label = " ✕ Clear ";
            let bx = area.right().saturating_sub(INDENT + width(label) as u16);
            if bx >= x + 16 {
                let written = put(buf, bx, area.y, label, usize::MAX, styles.primary());
                let button = Rect::new(bx, area.y, written as u16, 1);
                pointer::clickable(button);
                hints::add(button, Command::Clear);
                self.clear_button = Some(button);
            }
        }
        let heading_width = match self.clear_button {
            Some(button) => usize::from(button.x.saturating_sub(x + 1)),
            None => text_width,
        };

        if let Some((index, current)) = &items.current
            && height >= 3
        {
            put(buf, x, area.y, "Now playing", heading_width, heading);
            y += 1;
            let look = ItemLook {
                item: current,
                selected: self.selected == 0,
                focused,
                title_style: bold(
                    Style::new()
                        .fg(ctx.palette.playing)
                        .bg(ctx.palette.background),
                ),
            };
            self.draw_item(buf, area.y + y as u16, look, ctx);
            rows.iter_mut()
                .skip(y)
                .take(2)
                .for_each(|r| *r = Some(*index));
            y += ITEM_ROWS;
        }

        let sources: Vec<Option<Arc<str>>> = items
            .upcoming
            .iter()
            .map(|(index, _)| ctx.queue.source(*index))
            .collect();
        let available = height.saturating_sub(y);
        self.max_scroll = max_scroll(&sources, available);
        // Keep a selection made with the keyboard in view.
        if focused && std::mem::take(&mut self.follow_selection) {
            match self.selected.checked_sub(items.first_upcoming()) {
                Some(u) => self.scroll = scroll_to_item(&sources, self.scroll, u, available),
                None => self.scroll = 0,
            }
        }
        self.scroll = self.scroll.min(self.max_scroll);

        if items.upcoming.is_empty() && y < height {
            put(
                buf,
                x,
                area.y + y as u16,
                "Nothing up next",
                text_width,
                styles.secondary(),
            );
        }

        // Group consecutive items by source: a heading whenever it changes (and at the top).
        self.upcoming_shown = 0;
        let mut previous_source: Option<&Option<Arc<str>>> = None;
        let upcoming = items
            .upcoming
            .iter()
            .zip(&sources)
            .enumerate()
            .skip(self.scroll);
        for (u, ((index, item), source)) in upcoming {
            if previous_source != Some(source) {
                // A heading plus at least one item has to fit.
                if y + 3 > height {
                    break;
                }
                let text = match source {
                    Some(name) => format!("Next from: {name}"),
                    None => "Next up".to_string(),
                };
                let room = if y == 0 { heading_width } else { text_width };
                put(buf, x, area.y + y as u16, &text, room, heading);
                y += 1;
                previous_source = Some(source);
            }
            if y + 2 > height {
                break;
            }
            let look = ItemLook {
                item,
                selected: self.selected == u + items.first_upcoming(),
                focused,
                title_style: styles.primary(),
            };
            self.draw_item(buf, area.y + y as u16, look, ctx);
            self.upcoming_shown += 1;
            rows.iter_mut()
                .skip(y)
                .take(2)
                .for_each(|r| *r = Some(*index));
            y += ITEM_ROWS;
        }
        for (y, _) in rows.iter().enumerate().filter(|(_, item)| item.is_some()) {
            pointer::clickable(Rect::new(area.x, area.y + y as u16, area.width, 1));
        }
        self.row_items = rows;
    }

    fn command(&mut self, cmd: &Command, ctx: &Ctx) -> Outcome {
        match cmd {
            Command::Move(mode @ (MoveMode::Up | MoveMode::Down), amount) => {
                let items = Items::load(ctx);
                let step = match amount {
                    MoveAmount::Integer(n) => n.unsigned_abs() as usize,
                    MoveAmount::Float(scale) => {
                        ((self.page_items() as f32 * scale) as usize).max(1)
                    }
                    MoveAmount::Extreme => usize::MAX,
                };
                let position = match mode {
                    MoveMode::Up => self.selected.saturating_sub(step),
                    _ => self.selected.saturating_add(step),
                };
                self.select(position, &items);
            }
            Command::Move(MoveMode::Playing, _) => {
                self.selected = 0;
                self.scroll = 0;
                self.follow_selection = true;
            }
            Command::Play => {
                if let Some(index) = Items::load(ctx).index(self.selected) {
                    self.play(index, ctx);
                }
            }
            Command::Delete => {
                if let Some(index) = Items::load(ctx).index(self.selected) {
                    ctx.queue.remove(index);
                }
            }
            Command::Shift(mode, amount) => self.shift(mode, *amount, ctx),
            // Already queued.
            Command::Queue | Command::PlayNext => {}
            Command::Open(TargetMode::Selected) => {
                return match Items::load(ctx).get(self.selected) {
                    Some(item) => Outcome::Overlay(nav::context_menu(item.as_listitem(), ctx)),
                    None => Outcome::Consumed,
                };
            }
            _ => return Outcome::Ignored,
        }
        Outcome::Consumed
    }

    fn mouse(&mut self, event: MouseEvent, ctx: &Ctx) -> Outcome {
        match event.kind {
            MouseEventKind::Down(MouseButton::Left)
                if self
                    .clear_button
                    .is_some_and(|r| r.contains((event.column, event.row).into())) =>
            {
                return Outcome::Run(vec![Command::Clear]);
            }
            MouseEventKind::Down(MouseButton::Left) => {
                let row = event.row.checked_sub(self.area.y).map(usize::from);
                if let Some(index) = row.and_then(|row| self.row_items.get(row).copied().flatten())
                {
                    let menu = ctx.config.values().queue_click.as_deref() == Some("menu");
                    if !menu {
                        self.play(index, ctx);
                        return Outcome::Consumed;
                    }
                    // Select it, so the menu is seen to be for that song.
                    let items = Items::load(ctx);
                    let position = (0..=items.last()).find(|&p| items.index(p) == Some(index));
                    if let Some(position) = position {
                        self.selected = position;
                    }
                    if let Some(item) = position.and_then(|p| items.get(p)) {
                        return Outcome::Overlay(nav::context_menu(item.as_listitem(), ctx));
                    }
                }
            }
            // Only while there's more below (or above).
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp
                if !wheel::moves(&event, self.upcoming_shown) => {}
            MouseEventKind::ScrollDown => {
                if self.scroll < self.max_scroll {
                    self.scroll += 1;
                }
            }
            MouseEventKind::ScrollUp => self.scroll = self.scroll.saturating_sub(1),
            _ => return Outcome::Ignored,
        }
        Outcome::Consumed
    }

    fn on_show(&mut self, _ctx: &Ctx) {
        // Start from the current track, like opening Spotify's queue.
        self.selected = 0;
        self.scroll = 0;
        self.follow_selection = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sources(names: &[Option<&str>]) -> Vec<Option<Arc<str>>> {
        names.iter().map(|n| n.map(Arc::from)).collect()
    }

    #[test]
    fn test_rows_needed_counts_headings_and_gaps() {
        // heading, item, gap, item, gap, heading, item (the last item's gap isn't needed).
        let s = sources(&[Some("A"), Some("A"), None]);
        assert_eq!(rows_needed(&s), 1 + 3 + 3 + 1 + 2);
        assert_eq!(rows_needed(&[]), 0);
    }

    #[test]
    fn test_no_scrolling_when_everything_fits() {
        // heading + 3 items with gaps between = 1 + 3 + 3 + 2 rows.
        let s = sources(&[Some("A"); 3]);
        assert_eq!(max_scroll(&s, 9), 0);
        assert_eq!(max_scroll(&s, 50), 0);
    }

    #[test]
    fn test_scrolls_only_until_the_last_item_is_at_the_bottom() {
        // 10 rows fit a heading and 3 items (1 + 3 + 3 + 2 = 9), not 4 (12).
        let s = sources(&[Some("A"); 10]);
        assert_eq!(max_scroll(&s, 10), 7);
    }

    #[test]
    fn test_scroll_to_item() {
        let s = sources(&[Some("A"); 10]);
        // Already in view.
        assert_eq!(scroll_to_item(&s, 0, 2, 10), 0);
        // Below: scroll just far enough for it to be the last one shown.
        assert_eq!(scroll_to_item(&s, 0, 5, 10), 3);
        // Above: it becomes the first one shown.
        assert_eq!(scroll_to_item(&s, 6, 4, 10), 4);
        // Out of range: unchanged.
        assert_eq!(scroll_to_item(&s, 2, 20, 10), 2);
    }
}
