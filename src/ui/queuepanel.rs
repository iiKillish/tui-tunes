#[cfg(feature = "cover")]
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use cursive::event::{Event, EventResult, MouseButton, MouseEvent};
use cursive::theme::{ColorStyle, ColorType, Effect, PaletteColor};
use cursive::traits::View;
use cursive::{Printer, Vec2};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::command::{Command, MoveAmount, MoveMode, ShiftMode};
use crate::config::Config;
use crate::library::Library;
use crate::lyrics::{Lyrics, LyricsStore};
use crate::model::playable::Playable;
use crate::queue::Queue;
use crate::spotify::Spotify;
#[cfg(feature = "cover")]
use crate::ui::cover::{
    Area, QUEUE_THUMBNAIL_IMAGE_ID, cached_cover_path, kitty_delete, kitty_place, kitty_supported,
    overlaps, terminal_font_size,
};

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
/// How long the lyrics stay where they were scrolled to before following the song again.
const LYRICS_MANUAL_SCROLL: Duration = Duration::from_secs(5);
/// Rows moved by one step of the mouse wheel in the lyrics.
const LYRICS_WHEEL_ROWS: usize = 3;
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
    spotify: Spotify,
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

    lyrics: LyricsStore,
    /// First lyrics row shown last, and a scroll position set with the mouse wheel or keys (and
    /// when), which applies for [LYRICS_MANUAL_SCROLL].
    lyrics_top: AtomicUsize,
    lyrics_scroll: RwLock<Option<(usize, Instant)>>,
    /// For each row drawn last in the lyrics, the start time of the line shown there (for clicks).
    lyrics_rows: RwLock<Vec<Option<u32>>>,

    #[cfg(feature = "cover")]
    thumbnails: ThumbnailLayer,
    #[cfg(feature = "cover")]
    font_size: Vec2,
}

/// An album thumbnail to show next to a queue item.
#[cfg(feature = "cover")]
#[derive(Clone, PartialEq)]
struct ThumbnailRequest {
    image_id: u32,
    url: String,
    offset: Vec2,
    cells: Vec2,
}

/// Album thumbnails drawn as Kitty images, kept like the player bar's icons: the panel says what
/// should be shown while drawing, and [ThumbnailLayer::render_to_terminal] updates the terminal
/// afterwards, touching only thumbnails that changed.
#[cfg(feature = "cover")]
#[derive(Default)]
struct ThumbnailLayer {
    desired: RwLock<Vec<ThumbnailRequest>>,
    rendered: RwLock<Vec<ThumbnailRequest>>,
    /// Resized thumbnails per cover URL and pixel size.
    cache: RwLock<HashMap<(String, u32, u32), image::RgbaImage>>,
}

#[cfg(feature = "cover")]
impl ThumbnailLayer {
    fn set(&self, thumbnails: Vec<ThumbnailRequest>) {
        *self.desired.write().unwrap() = thumbnails;
    }

    /// Show the requested thumbnails, except ones that would cover part of `overlays` (dialogs
    /// or menus) or whose cover is still downloading (they're retried on the next call).
    fn render_to_terminal(&self, overlays: &[Area], font_size: Vec2) {
        let desired: Vec<ThumbnailRequest> = self
            .desired
            .read()
            .unwrap()
            .iter()
            .filter(|t| !overlays.iter().any(|o| overlaps((t.offset, t.cells), *o)))
            .cloned()
            .collect();
        let mut rendered = self.rendered.write().unwrap();
        if *rendered == desired {
            return;
        }
        for old in rendered.iter().filter(|old| !desired.contains(old)) {
            let _ = kitty_delete(old.image_id);
        }
        let mut shown = Vec::new();
        let mut cache = self.cache.write().unwrap();
        for new in desired {
            if rendered.contains(&new) {
                shown.push(new);
                continue;
            }
            let Some(path) = cached_cover_path(&new.url) else {
                continue;
            };
            let size = (
                (new.cells.x * font_size.x) as u32,
                (new.cells.y * font_size.y) as u32,
            );
            let key = (new.url.clone(), size.0, size.1);
            if !cache.contains_key(&key) {
                let image = image::ImageReader::open(&path)
                    .and_then(|r| r.with_guessed_format())
                    .map_err(|e| e.to_string())
                    .and_then(|r| r.decode().map_err(|e| e.to_string()));
                match image {
                    // Fill the square, cropping rather than letterboxing.
                    Ok(image) => {
                        let thumbnail = image
                            .resize_to_fill(size.0, size.1, image::imageops::FilterType::Triangle)
                            .to_rgba8();
                        cache.insert(key.clone(), thumbnail);
                    }
                    Err(e) => {
                        log::error!("Failed to load cover {}: {e}", path.display());
                        continue;
                    }
                }
            }
            match kitty_place(&cache[&key], new.offset, new.cells, new.image_id, 0) {
                Ok(()) => shown.push(new),
                Err(e) => log::error!("Failed to draw thumbnail: {e}"),
            }
        }
        *rendered = shown;
    }
}

impl QueuePanel {
    pub fn new(queue: Arc<Queue>, library: Arc<Library>, config: &Config) -> Self {
        Self {
            spotify: queue.get_spotify(),
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
            lyrics: LyricsStore::default(),
            lyrics_top: AtomicUsize::new(0),
            lyrics_scroll: RwLock::new(None),
            lyrics_rows: RwLock::new(Vec::new()),
            #[cfg(feature = "cover")]
            thumbnails: ThumbnailLayer::default(),
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
                    PanelTab::Lyrics => self.lyrics_command(cmd),
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

    fn lyrics_command(&self, cmd: &Command) -> bool {
        match cmd {
            Command::Move(mode @ (MoveMode::Up | MoveMode::Down), amount) => {
                let step = match amount {
                    MoveAmount::Integer(n) => n.unsigned_abs() as usize,
                    MoveAmount::Float(_) => 10,
                    MoveAmount::Extreme => usize::MAX / 2,
                };
                self.scroll_lyrics(if matches!(mode, MoveMode::Up) {
                    -(step as isize)
                } else {
                    step as isize
                });
            }
            // Back to following the song.
            Command::Move(MoveMode::Playing, _) | Command::Play => {
                *self.lyrics_scroll.write().unwrap() = None;
            }
            _ => return false,
        }
        true
    }

    fn scroll_lyrics(&self, rows: isize) {
        let top = self.lyrics_top.load(Ordering::Relaxed);
        let top = top.saturating_add_signed(rows);
        *self.lyrics_scroll.write().unwrap() = Some((top, Instant::now()));
    }

    /// Lyrics to show, as rows of text with the line each came from, and the line being sung.
    fn lyrics_rows(&self, width: usize) -> (Vec<LyricsRow>, Option<usize>) {
        let message = |text: &str| -> Vec<LyricsRow> {
            wrap(text, width)
                .into_iter()
                .map(LyricsRow::message)
                .collect()
        };
        let track = match self.queue.get_current() {
            Some(Playable::Track(track)) => track,
            Some(Playable::Episode(_)) => return (message("No lyrics for podcasts"), None),
            None => return (message("Nothing playing"), None),
        };
        let library = self.library.clone();
        let lyrics = self.lyrics.get(&track, move || library.trigger_redraw());

        let progress = self.spotify.get_current_progress().as_millis() as u32;
        let current = lyrics.current_line(progress);
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
                    let text = if line.text.is_empty() {
                        "♪"
                    } else {
                        &line.text
                    };
                    rows.extend(wrap(text, width).into_iter().map(|text| LyricsRow {
                        text,
                        line: Some(i),
                        time: Some(line.time),
                    }));
                }
            }
            Lyrics::Plain(lines) => {
                rows.extend(message("Not synced to the song"));
                rows.push(LyricsRow::message(String::new()));
                for (i, line) in lines.iter().enumerate() {
                    rows.extend(wrap(line, width).into_iter().map(|text| LyricsRow {
                        text,
                        line: Some(i),
                        time: None,
                    }));
                }
            }
        }
        if matches!(lyrics, Lyrics::Synced(_) | Lyrics::Plain(_)) {
            rows.push(LyricsRow::message(String::new()));
            rows.push(LyricsRow::message("Lyrics from LRCLIB".to_string()));
        }
        (rows, current)
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
        let mut thumbnails: Vec<ThumbnailRequest> = Vec::new();
        #[allow(unused_variables, unused_mut)]
        let mut add_thumbnail = |track: &Playable, y: usize| {
            #[cfg(feature = "cover")]
            if thumbnail_width > 0
                && y + 2 <= size.y
                && let Some(url) = track.cover_url()
            {
                thumbnails.push(ThumbnailRequest {
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
        let palette = &printer.theme.palette;
        let background = ColorType::Color(*palette.custom("statusbar_bg").unwrap());
        let style = |front: ColorType| ColorStyle::new(front, background);
        let primary = style(ColorType::Palette(PaletteColor::Primary));
        let secondary = style(ColorType::Palette(PaletteColor::Secondary));
        let accent = style(ColorType::Palette(PaletteColor::TitlePrimary));

        let x = 2;
        let (rows, current) = self.lyrics_rows(size.x.saturating_sub(x + 2));
        let available = size.y.saturating_sub(CONTENT_ROW);
        let max_top = rows.len().saturating_sub(available);

        // Follow the song, keeping the current line a third of the way down, unless scrolled by
        // hand recently.
        let manual = {
            let mut scroll = self.lyrics_scroll.write().unwrap();
            if scroll.is_some_and(|(_, at)| at.elapsed() > LYRICS_MANUAL_SCROLL) {
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
        self.lyrics_top.store(top, Ordering::Relaxed);
        if let Some(manual) = manual
            && manual != top
            && let Some((stored, _)) = self.lyrics_scroll.write().unwrap().as_mut()
        {
            // Scrolled past an end: keep the clamped position so scrolling back is immediate.
            *stored = top;
        }

        let mut times = vec![None; size.y];
        for (i, row) in rows.iter().skip(top).take(available).enumerate() {
            let y = CONTENT_ROW + i;
            let row_style = match (row.line, current) {
                (None, _) => secondary,
                (Some(line), Some(current)) if line == current => accent,
                (Some(line), Some(current)) if line < current => secondary,
                _ => primary,
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
        *self.lyrics_rows.write().unwrap() = times;
        // No thumbnails on this tab.
        self.hide();
    }
}

/// One row of the lyrics tab: text and, for lyrics (rather than messages), which line it's part
/// of and when that line starts.
struct LyricsRow {
    text: String,
    line: Option<usize>,
    time: Option<u32>,
}

impl LyricsRow {
    fn message(text: String) -> Self {
        Self {
            text,
            line: None,
            time: None,
        }
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
            (PanelTab::Lyrics, MouseEvent::Press(MouseButton::Left)) => {
                // Jump to the clicked line, and follow the song from there.
                let time = self
                    .lyrics_rows
                    .read()
                    .unwrap()
                    .get(position.y)
                    .copied()
                    .flatten();
                if let Some(time) = time {
                    self.spotify.seek(time);
                    *self.lyrics_scroll.write().unwrap() = None;
                }
            }
            (PanelTab::Lyrics, MouseEvent::WheelDown) => {
                self.scroll_lyrics(LYRICS_WHEEL_ROWS as isize);
            }
            (PanelTab::Lyrics, MouseEvent::WheelUp) => {
                self.scroll_lyrics(-(LYRICS_WHEEL_ROWS as isize));
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

    #[test]
    fn test_wrap() {
        assert_eq!(wrap("one two three", 7), vec!["one two", "three"]);
        assert_eq!(wrap("one two three", 20), vec!["one two three"]);
        assert_eq!(wrap("abcdefghij", 4), vec!["abcd", "efgh", "ij"]);
        assert_eq!(wrap("", 10), vec![""]);
        assert_eq!(wrap("  spaced   out  ", 20), vec!["spaced out"]);
    }
}
