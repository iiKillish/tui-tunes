#[cfg(feature = "cover")]
use std::collections::HashMap;
use std::sync::Arc;
#[cfg(feature = "cover")]
use std::sync::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use cursive::event::{Event, EventResult, MouseButton, MouseEvent};
#[cfg(feature = "cover")]
use cursive::theme::{BaseColor, Color};
use cursive::theme::{ColorStyle, ColorType, Effect, PaletteColor};
use cursive::traits::View;
use cursive::{Printer, Vec2};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::config::Config;
use crate::library::Library;
use crate::model::playable::Playable;
use crate::queue::{Queue, RepeatSetting};
use crate::spotify::{PlayerEvent, Spotify, VOLUME_PERCENT};
#[cfg(feature = "cover")]
use crate::ui::cover::{
    CONTROL_ICON_IMAGE_ID, CoverRenderer, NOW_PLAYING_COVER_IMAGE_ID, kitty_delete, kitty_place,
    kitty_supported,
};
#[cfg(feature = "cover")]
use crate::ui::icons::{self, Icon};
use crate::utils::ms_to_hms;

/// Default height of the bar in rows.
const DEFAULT_HEIGHT: usize = 5;
/// Narrowest window the bar is shown in; the layout falls back to the status bar below this.
pub const MIN_WIDTH: usize = 60;
/// Width of one playback control button when drawn as text, and the gap between buttons.
const BUTTON_WIDTH: usize = 3;
const BUTTON_GAP: usize = 2;
/// Height of the playback control buttons when drawn as images (see [crate::ui::icons]).
#[cfg(feature = "cover")]
const ICON_ROWS: usize = 2;
const VOLUME_BAR_WIDTH: usize = 12;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Control {
    Shuffle,
    Previous,
    PlayPause,
    Next,
    Repeat,
}

const CONTROLS: [Control; 5] = [
    Control::Shuffle,
    Control::Previous,
    Control::PlayPause,
    Control::Next,
    Control::Repeat,
];

/// Where everything goes in a bar of a given size. Shared by drawing and mouse handling.
struct Geometry {
    /// Cover offset and size, if there's one to draw.
    #[cfg_attr(not(feature = "cover"), allow(dead_code))]
    cover: Option<(Vec2, Vec2)>,
    text_x: usize,
    text_width: usize,
    /// Rows of the title/artist/album lines, the controls and the progress bar.
    title_row: usize,
    controls_row: usize,
    progress_row: usize,
    /// x of the first control button, and the size of each button.
    controls_x: usize,
    button_width: usize,
    button_rows: usize,
    /// Progress bar start and width (times go on either side of it).
    bar_x: usize,
    bar_width: usize,
    /// Volume icon x and volume bar start (on `controls_row + 1`), if there's room.
    volume_x: Option<usize>,
}

/// Spotify-style player bar at the bottom of the window, replacing the status bar: cover art
/// (when built with the `cover` feature), title, artists and album on the left; shuffle,
/// previous, play/pause, next and repeat buttons over a progress bar in the middle; volume on
/// the right. All of it can be clicked.
pub struct NowPlaying {
    queue: Arc<Queue>,
    spotify: Spotify,
    library: Arc<Library>,
    height: usize,
    /// Whether to draw the small cover (off on the cover screen, which shows a big one).
    show_cover: AtomicBool,
    last_size: Vec2,
    #[cfg(feature = "cover")]
    cover: CoverRenderer,
    #[cfg(feature = "cover")]
    icons: IconLayer,
}

/// A control icon to show as an image.
#[cfg(feature = "cover")]
#[derive(Clone, PartialEq)]
struct IconRequest {
    image_id: u32,
    icon: Icon,
    color: [u8; 3],
    offset: Vec2,
    cells: Vec2,
    pixels: (u32, u32),
    /// Pixels to shift the icon down within its first cell.
    y_shift: usize,
}

/// Rendered icon images are cached per icon, color and pixel size.
#[cfg(feature = "cover")]
type IconKey = (Icon, [u8; 3], (u32, u32));

/// Control icons drawn as images, kept like [CoverRenderer]'s cover: views say what should be
/// shown while drawing, and [IconLayer::render_to_terminal] updates the terminal afterwards,
/// touching only icons that changed.
#[cfg(feature = "cover")]
#[derive(Default)]
struct IconLayer {
    desired: RwLock<Vec<IconRequest>>,
    rendered: RwLock<Vec<IconRequest>>,
    cache: RwLock<HashMap<IconKey, image::RgbaImage>>,
}

#[cfg(feature = "cover")]
impl IconLayer {
    fn set(&self, icons: Vec<IconRequest>) {
        *self.desired.write().unwrap() = icons;
    }

    fn render_to_terminal(&self) {
        let desired = self.desired.read().unwrap().clone();
        let mut rendered = self.rendered.write().unwrap();
        if *rendered == desired {
            return;
        }
        // Remove icons that changed or went away before placing new ones (possibly in the same
        // cells, like play becoming pause).
        for old in rendered.iter().filter(|old| !desired.contains(old)) {
            let _ = kitty_delete(old.image_id);
        }
        let mut cache = self.cache.write().unwrap();
        for new in desired.iter().filter(|new| !rendered.contains(new)) {
            let image = cache
                .entry((new.icon, new.color, new.pixels))
                .or_insert_with(|| icons::render(new.icon, new.pixels.0, new.pixels.1, new.color));
            if let Err(e) = kitty_place(image, new.offset, new.cells, new.image_id, new.y_shift) {
                log::error!("Failed to draw icon: {e}");
            }
        }
        *rendered = desired;
    }
}

/// RGB of a theme color, for drawing icons in it.
#[cfg(feature = "cover")]
fn rgb(color: Color) -> [u8; 3] {
    match color {
        Color::Rgb(r, g, b) => [r, g, b],
        Color::RgbLowRes(r, g, b) => [r * 51, g * 51, b * 51],
        Color::Dark(base) | Color::Light(base) => match base {
            BaseColor::Black => [0, 0, 0],
            BaseColor::Red => [205, 49, 49],
            BaseColor::Green => [13, 188, 121],
            BaseColor::Yellow => [229, 229, 16],
            BaseColor::Blue => [36, 114, 200],
            BaseColor::Magenta => [188, 63, 188],
            BaseColor::Cyan => [17, 168, 205],
            BaseColor::White => [229, 229, 229],
        },
        Color::TerminalDefault => [229, 229, 229],
    }
}

impl NowPlaying {
    pub fn new(queue: Arc<Queue>, library: Arc<Library>, config: &Config) -> Self {
        Self {
            spotify: queue.get_spotify(),
            queue,
            library,
            height: config.values().now_playing_height.unwrap_or(DEFAULT_HEIGHT),
            show_cover: AtomicBool::new(true),
            last_size: Vec2::zero(),
            #[cfg(feature = "cover")]
            cover: CoverRenderer::new(config, NOW_PLAYING_COVER_IMAGE_ID),
            #[cfg(feature = "cover")]
            icons: IconLayer::default(),
        }
    }

    /// Configured height in rows; 0 means the bar is disabled.
    pub fn height(&self) -> usize {
        self.height
    }

    pub fn set_show_cover(&self, show: bool) {
        self.show_cover.store(show, Ordering::Relaxed);
    }

    /// Stop showing the bar's images (cover and icons), e.g. while the bar itself isn't drawn.
    pub fn hide(&self) {
        #[cfg(feature = "cover")]
        {
            self.cover.clear_cover();
            self.icons.set(Vec::new());
        }
    }

    /// Draw (or remove) the cover and icons in the terminal. Must be called after cursive has
    /// drawn. `obscured` hides them while something is drawn on top of the layout, like a dialog:
    /// terminal images are always drawn over text.
    #[cfg(feature = "cover")]
    pub fn render_to_terminal(&self, obscured: bool) {
        if obscured {
            self.hide();
        }
        self.cover.render_to_terminal();
        self.icons.render_to_terminal();
    }

    /// Whether the controls can be drawn as (bigger) images rather than text.
    #[cfg(feature = "cover")]
    fn graphical_controls(&self) -> bool {
        kitty_supported()
    }

    fn nerdfont(&self) -> bool {
        self.library.cfg.values().use_nerdfont.unwrap_or(false)
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

    fn geometry(&self, size: Vec2) -> Geometry {
        let rows = size.y;
        let mid = rows / 2;

        let show_cover = self.show_cover.load(Ordering::Relaxed);
        let cover_cols = if show_cover && rows >= 3 {
            self.cover_width(rows)
        } else {
            0
        };
        let cover = (cover_cols > 0).then(|| (Vec2::new(1, 0), Vec2::new(cover_cols, rows)));
        let text_x = if cover_cols > 0 { cover_cols + 3 } else { 2 };

        // Center column: controls over the progress bar.
        let center_width = (size.x * 2 / 5).clamp(30, 80);
        let center_x = size.x.saturating_sub(center_width) / 2;
        let (button_width, button_rows) = self.button_size(rows);
        let controls_width = CONTROLS.len() * button_width + (CONTROLS.len() - 1) * BUTTON_GAP;
        let controls_x = (size.x.saturating_sub(controls_width)) / 2;
        // Room for " m:ss " on the left and " -m:ss" on the right of the bar.
        let bar_x = center_x + 6;
        let bar_width = center_width.saturating_sub(13);

        let text_width = center_x.saturating_sub(text_x + 2);

        let right_space = size.x.saturating_sub(center_x + center_width);
        let volume_x = (right_space >= VOLUME_BAR_WIDTH + 6)
            .then(|| size.x.saturating_sub(VOLUME_BAR_WIDTH + 5));

        Geometry {
            cover,
            text_x,
            text_width,
            title_row: mid.saturating_sub(1),
            controls_row: mid.saturating_sub(1),
            progress_row: (mid + 1).min(rows.saturating_sub(1)),
            controls_x,
            button_width,
            button_rows,
            bar_x,
            bar_width,
            volume_x,
        }
    }

    /// Size in cells of a control button: square images [ICON_ROWS] tall when the controls are
    /// drawn as images (and the bar is tall enough), else a few cells of text.
    fn button_size(&self, rows: usize) -> (usize, usize) {
        #[cfg(feature = "cover")]
        if self.graphical_controls() && rows >= ICON_ROWS + 2 {
            return (self.cover_width(ICON_ROWS), ICON_ROWS);
        }
        let _ = rows;
        (BUTTON_WIDTH, 1)
    }

    fn control_at(geometry: &Geometry, position: Vec2) -> Option<Control> {
        // Image buttons are drawn half a row higher (see `draw`), so they reach into the row above.
        let first_row = if geometry.button_rows > 1 {
            geometry.controls_row.saturating_sub(1)
        } else {
            geometry.controls_row
        };
        let rows = first_row..geometry.controls_row + geometry.button_rows;
        if !rows.contains(&position.y) {
            return None;
        }
        let offset = position.x.checked_sub(geometry.controls_x)?;
        let slot = offset / (geometry.button_width + BUTTON_GAP);
        (offset % (geometry.button_width + BUTTON_GAP) < geometry.button_width)
            .then(|| CONTROLS.get(slot).copied())
            .flatten()
    }

    /// The image icon for `control` in its current state.
    #[cfg(feature = "cover")]
    fn control_image(&self, control: Control) -> Icon {
        match control {
            Control::Shuffle => Icon::Shuffle,
            Control::Previous => Icon::Previous,
            Control::PlayPause
                if matches!(self.spotify.get_current_status(), PlayerEvent::Playing(_)) =>
            {
                Icon::Pause
            }
            Control::PlayPause => Icon::Play,
            Control::Next => Icon::Next,
            Control::Repeat if self.queue.get_repeat() == RepeatSetting::RepeatTrack => {
                Icon::RepeatOne
            }
            Control::Repeat => Icon::Repeat,
        }
    }

    fn control_icon(&self, control: Control) -> &'static str {
        let playing = matches!(self.spotify.get_current_status(), PlayerEvent::Playing(_));
        let repeat = self.queue.get_repeat();
        if self.nerdfont() {
            match control {
                Control::Shuffle => "\u{f049d}",
                Control::Previous => "\u{f04ae}",
                Control::PlayPause if playing => "\u{f03e4}",
                Control::PlayPause => "\u{f040a}",
                Control::Next => "\u{f04ad}",
                Control::Repeat if repeat == RepeatSetting::RepeatTrack => "\u{f0458}",
                Control::Repeat => "\u{f0456}",
            }
        } else {
            match control {
                Control::Shuffle => "⇄",
                Control::Previous => "⏮",
                Control::PlayPause if playing => "⏸",
                Control::PlayPause => "▶",
                Control::Next => "⏭",
                Control::Repeat if repeat == RepeatSetting::RepeatTrack => "↻¹",
                Control::Repeat => "↻",
            }
        }
    }

    fn control_active(&self, control: Control) -> bool {
        match control {
            Control::Shuffle => self.queue.get_shuffle(),
            Control::Repeat => self.queue.get_repeat() != RepeatSetting::None,
            _ => false,
        }
    }

    fn volume_icon(&self) -> &'static str {
        let volume = self.spotify.volume();
        if self.nerdfont() {
            match volume {
                0 => "\u{f0581}",
                v if v < u16::MAX / 3 => "\u{f057f}",
                v if v < u16::MAX / 3 * 2 => "\u{f0580}",
                _ => "\u{f057e}",
            }
        } else if volume == 0 {
            "×"
        } else {
            "♪"
        }
    }

    /// Run a control, the same way as the corresponding command.
    fn activate(&self, control: Control) {
        match control {
            Control::Shuffle => self.queue.set_shuffle(!self.queue.get_shuffle()),
            Control::Previous => {
                if self.spotify.get_current_progress() < Duration::from_secs(5) {
                    self.queue.previous();
                } else {
                    self.spotify.seek(0);
                }
            }
            Control::PlayPause => self.queue.toggleplayback(),
            Control::Next => self.queue.next(true),
            Control::Repeat => self.queue.set_repeat(match self.queue.get_repeat() {
                RepeatSetting::None => RepeatSetting::RepeatPlaylist,
                RepeatSetting::RepeatPlaylist => RepeatSetting::RepeatTrack,
                RepeatSetting::RepeatTrack => RepeatSetting::None,
            }),
        }
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

/// A horizontal bar `width` wide, filled to `fraction`.
fn draw_bar(printer: &Printer<'_, '_>, x: usize, y: usize, width: usize, fraction: f64) {
    let palette = &printer.theme.palette;
    let background = ColorType::Color(*palette.custom("statusbar_bg").unwrap());
    let filled_style = ColorStyle::new(
        ColorType::Color(*palette.custom("statusbar_progress").unwrap()),
        background,
    );
    let empty_style = ColorStyle::new(
        ColorType::Color(*palette.custom("statusbar_progress_bg").unwrap()),
        background,
    );
    let filled = ((fraction.clamp(0.0, 1.0) * width as f64).round() as usize).min(width);
    printer.with_color(empty_style, |printer| {
        printer.print((x, y), &"━".repeat(width));
    });
    printer.with_color(filled_style, |printer| {
        printer.print((x, y), &"━".repeat(filled));
    });
}

impl View for NowPlaying {
    fn draw(&self, printer: &Printer<'_, '_>) {
        let size = printer.size;
        let palette = &printer.theme.palette;
        let background = ColorType::Color(*palette.custom("statusbar_bg").unwrap());
        let style = |front: ColorType| ColorStyle::new(front, background);
        let primary = style(ColorType::Palette(PaletteColor::Primary));
        let secondary = style(ColorType::Palette(PaletteColor::Secondary));
        let accent = style(ColorType::Palette(PaletteColor::TitlePrimary));

        printer.with_color(primary, |printer| {
            for y in 0..size.y {
                printer.print_hline((0, y), size.x, " ");
            }
        });
        if size.y == 0 {
            self.hide();
            return;
        }

        let geometry = self.geometry(size);
        let track = self.queue.get_current();

        // Cover and track info on the left.
        #[cfg(feature = "cover")]
        match (geometry.cover, track.as_ref().and_then(|t| t.cover_url())) {
            (Some((offset, cover_size)), Some(url)) => {
                self.cover
                    .draw_cover(url, printer.offset + offset, cover_size)
            }
            _ => self.hide(),
        }

        let (title, artists, album) = match &track {
            Some(t) => (
                Playable::format(t, "%title", &self.library),
                Playable::format(t, "%artists", &self.library),
                Playable::format(t, "%album", &self.library),
            ),
            None => ("Nothing playing".to_string(), String::new(), String::new()),
        };
        let (x, w, row) = (geometry.text_x, geometry.text_width, geometry.title_row);
        printer.with_color(primary, |printer| {
            printer.with_effect(Effect::Bold, |printer| {
                printer.print((x, row), &truncate(&title, w));
            });
        });
        printer.with_color(secondary, |printer| {
            printer.print((x, row + 1), &truncate(&artists, w));
        });
        printer.with_color(accent, |printer| {
            printer.print((x, row + 2), &truncate(&album, w));
        });

        // Controls. Shuffle and repeat are colored when they're on.
        #[cfg(feature = "cover")]
        let mut icon_requests = Vec::new();
        for (i, control) in CONTROLS.iter().enumerate() {
            let slot_x = geometry.controls_x + i * (geometry.button_width + BUTTON_GAP);
            let active = self.control_active(*control);
            let (icon_style, icon_color) = match control {
                Control::Shuffle | Control::Repeat if active => {
                    (accent, PaletteColor::TitlePrimary)
                }
                Control::Shuffle | Control::Repeat => (secondary, PaletteColor::Secondary),
                _ => (primary, PaletteColor::Primary),
            };

            #[cfg(feature = "cover")]
            if geometry.button_rows > 1 {
                let font = self.cover.font_size();
                let cells = Vec2::new(geometry.button_width, geometry.button_rows);
                // Half a row higher than the controls row, to leave a gap above the progress
                // bar: start in the row above, shifted down by half a cell.
                let (row, y_shift) = match geometry.controls_row.checked_sub(1) {
                    Some(row) => (row, font.y / 2),
                    None => (geometry.controls_row, 0),
                };
                icon_requests.push(IconRequest {
                    image_id: CONTROL_ICON_IMAGE_ID + i as u32,
                    icon: self.control_image(*control),
                    color: rgb(palette[icon_color]),
                    offset: printer.offset + Vec2::new(slot_x, row),
                    cells,
                    pixels: ((cells.x * font.x) as u32, (cells.y * font.y) as u32),
                    y_shift,
                });
                continue;
            }
            let _ = icon_color;

            let icon = self.control_icon(*control);
            let icon_x = slot_x + (geometry.button_width.saturating_sub(icon.width())) / 2;
            printer.with_color(icon_style, |printer| {
                printer.with_effect(Effect::Bold, |printer| {
                    printer.print((icon_x, geometry.controls_row), icon);
                });
            });
        }
        #[cfg(feature = "cover")]
        self.icons.set(icon_requests);

        // Progress: elapsed, bar, remaining.
        if let Some(t) = &track {
            let duration = t.duration();
            let elapsed = (self.spotify.get_current_progress().as_millis() as u32).min(duration);
            let elapsed_text = ms_to_hms(elapsed);
            let remaining_text = format!("-{}", ms_to_hms(duration - elapsed));
            printer.with_color(secondary, |printer| {
                printer.print(
                    (
                        geometry.bar_x.saturating_sub(elapsed_text.width() + 1),
                        geometry.progress_row,
                    ),
                    &elapsed_text,
                );
                printer.print(
                    (
                        geometry.bar_x + geometry.bar_width + 1,
                        geometry.progress_row,
                    ),
                    &remaining_text,
                );
            });
            let fraction = if duration > 0 {
                elapsed as f64 / duration as f64
            } else {
                0.0
            };
            draw_bar(
                printer,
                geometry.bar_x,
                geometry.progress_row,
                geometry.bar_width,
                fraction,
            );
        } else {
            draw_bar(
                printer,
                geometry.bar_x,
                geometry.progress_row,
                geometry.bar_width,
                0.0,
            );
        }

        // Volume (and the library update indicator above it) on the right.
        if let Some(volume_x) = geometry.volume_x {
            let row = geometry.controls_row + 1;
            printer.with_color(primary, |printer| {
                printer.print((volume_x, row), self.volume_icon());
            });
            draw_bar(
                printer,
                volume_x + 3,
                row,
                VOLUME_BAR_WIDTH,
                self.spotify.volume() as f64 / u16::MAX as f64,
            );
            if !*self.library.is_done.read().unwrap() {
                printer.with_color(secondary, |printer| {
                    printer.print((volume_x, geometry.controls_row), "updating library…");
                });
            }
        }
    }

    fn layout(&mut self, size: Vec2) {
        self.last_size = size;
    }

    fn required_size(&mut self, constraint: Vec2) -> Vec2 {
        Vec2::new(constraint.x, self.height.min(constraint.y))
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
        let geometry = self.geometry(self.last_size);
        let on_volume = geometry
            .volume_x
            .is_some_and(|x| position.y == geometry.controls_row + 1 && position.x >= x);

        match event {
            MouseEvent::Press(MouseButton::Left) => {
                if let Some(control) = Self::control_at(&geometry, position) {
                    self.activate(control);
                } else if position.y == geometry.progress_row
                    && (geometry.bar_x..geometry.bar_x + geometry.bar_width).contains(&position.x)
                    && let Some(track) = self.queue.get_current()
                {
                    let fraction = (position.x - geometry.bar_x) as f64 / geometry.bar_width as f64;
                    self.spotify
                        .seek((track.duration() as f64 * fraction) as u32);
                } else if on_volume && let Some(volume_x) = geometry.volume_x {
                    let bar_x = volume_x + 3;
                    if position.x >= bar_x {
                        let fraction =
                            ((position.x - bar_x + 1) as f64 / VOLUME_BAR_WIDTH as f64).min(1.0);
                        self.spotify
                            .set_volume((u16::MAX as f64 * fraction) as u16, true);
                    }
                }
            }
            MouseEvent::WheelUp if on_volume => {
                let volume = self.spotify.volume().saturating_add(VOLUME_PERCENT);
                self.spotify.set_volume(volume, true);
            }
            MouseEvent::WheelDown if on_volume => {
                let volume = self.spotify.volume().saturating_sub(VOLUME_PERCENT);
                self.spotify.set_volume(volume, true);
            }
            MouseEvent::WheelUp if position.y == geometry.progress_row => {
                self.spotify.seek_relative(-5000);
            }
            MouseEvent::WheelDown if position.y == geometry.progress_row => {
                self.spotify.seek_relative(5000);
            }
            _ => {}
        }
        EventResult::Consumed(None)
    }
}
