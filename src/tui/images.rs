//! Pictures in the terminal: covers and the player's control icons, with the Kitty graphics
//! protocol, sixel or iTerm2's inline images, whichever the terminal speaks ([protocol]).
//!
//! Widgets ask for images while drawing a frame ([Frame::image], [Frame::icon] via
//! [request_image]). Images not requested in a frame are removed, so widgets that stop being
//! drawn never have to clean up after themselves. How they get on the terminal depends on the
//! protocol:
//!
//! - Kitty images are a layer above the text: after ratatui has drawn the frame, [render] places
//!   them, touching only images that changed, and deletes the ones no longer wanted by ID.
//! - Sixel and iTerm2 images are painted into the cells like text, and text drawn over them
//!   erases them. [draw_into_buffer] puts each image's escape sequence in its top-left cell of
//!   ratatui's buffer and marks the rest of its cells as skipped, so ratatui's diff redraws an
//!   image only when it changes, and redraws the text under it (erasing it) once it's gone.

// Without the `cover` feature there are no images: most of this compiles to nothing.
#![cfg_attr(
    not(feature = "cover"),
    allow(unused_imports, unused_variables, dead_code)
)]

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::num::NonZeroU16;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock, RwLock};

use log::{debug, error};
use ratatui::buffer::{Buffer, CellDiffOption};
use ratatui::layout::{Position, Rect, Size};

#[cfg(feature = "cover")]
use crate::tui::icons::{self, Icon};

/// What to show in an image.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum Source {
    /// A cover (or artist picture) by URL, downloaded and cached on first use.
    Url(String),
    /// A player control icon in an RGB color.
    #[cfg(feature = "cover")]
    Icon(Icon, [u8; 3]),
}

/// How images are put on the terminal.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Protocol {
    Kitty,
    Sixel,
    Iterm2,
}

#[cfg(feature = "cover")]
#[derive(Clone, PartialEq, Debug)]
struct Request {
    source: Source,
    area: Rect,
    /// Pixels to move the image down within its first row (to place it between rows). Kitty only.
    y_shift: u16,
    /// The part of `area` shown: all of it, unless a menu covers some (see [visible_part]).
    shown: Rect,
}

#[cfg(feature = "cover")]
#[derive(Clone, PartialEq, Debug)]
struct Placed {
    id: u32,
    request: Request,
}

/// First Kitty image ID used; IDs are handed out per frame.
#[cfg(feature = "cover")]
const FIRST_IMAGE_ID: u32 = 1000;

#[cfg(feature = "cover")]
struct Layer {
    /// Requested for the frame being drawn, and whether a frame was drawn since the last render.
    frame: Vec<Request>,
    frame_drawn: bool,
    placed: Vec<Placed>,
    next_id: u32,
    /// [GENERATION] when the images were last placed; see [reset].
    generation: u64,
    /// Scaled pixels per source and pixel size.
    cache: HashMap<(Source, u32, u32), image::RgbaImage>,
    /// The same, as sixel or iTerm2 escape sequences, for the part shown (the cells it starts
    /// at, across and down, and their number across and down).
    encoded: HashMap<EncodedKey, String>,
}

#[cfg(feature = "cover")]
static LAYER: Mutex<Option<Layer>> = Mutex::new(None);
static GENERATION: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "cover")]
impl Layer {
    fn new() -> Self {
        Self {
            frame: Vec::new(),
            frame_drawn: false,
            placed: Vec::new(),
            next_id: FIRST_IMAGE_ID,
            generation: 0,
            cache: HashMap::new(),
            encoded: HashMap::new(),
        }
    }
}

#[cfg(feature = "cover")]
fn with_layer<R>(f: impl FnOnce(&mut Layer) -> R) -> R {
    let mut layer = LAYER.lock().unwrap();
    f(layer.get_or_insert_with(Layer::new))
}

/// Start a frame: forget the images asked for in the last one.
pub fn begin_frame() {
    #[cfg(feature = "cover")]
    with_layer(|layer| {
        layer.frame.clear();
        layer.frame_drawn = true;
    });
}

/// Show `source` filling `area` (cropped to fit) in the frame being drawn. Does nothing when the
/// terminal can't show images; check [supported] to draw something else instead.
#[cfg_attr(not(feature = "cover"), allow(unused_variables))]
pub fn request_image(source: Source, area: Rect, y_shift: u16) {
    #[cfg(feature = "cover")]
    if supported() && !area.is_empty() {
        with_layer(|layer| {
            layer.frame.push(Request {
                source,
                area,
                y_shift,
                shown: area,
            });
        });
    }
}

/// Show the cover at `url` filling `area` in the frame being drawn. A smaller size of it is
/// shown when that's big enough (see [smaller_image]), unless that one couldn't be downloaded.
pub fn cover(url: &str, area: Rect) {
    if !supported() || area.is_empty() {
        return;
    }
    let font = font_size();
    let pixels = (area.width * font.width).max(area.height * font.height);
    let url = smaller_image(url, u32::from(pixels))
        .filter(|smaller| !downloads().failed.contains_key(smaller))
        .unwrap_or_else(|| url.to_string());
    request_image(Source::Url(url), area, 0);
}

/// Codes that Spotify's image IDs start with for each size of an image (the rest of the ID is
/// the same for every size), largest first: for covers, artist pictures and podcast covers.
/// The models keep the largest, which a thumbnail in a list doesn't need: downloading and
/// scaling it costs ten times as much as the smallest.
const IMAGE_SIZE_CODES: &[&[(u32, &str)]] = &[
    &[
        (640, "ab67616d0000b273"),
        (300, "ab67616d00001e02"),
        (64, "ab67616d00004851"),
    ],
    &[
        (640, "ab6761610000e5eb"),
        (320, "ab67616100005174"),
        (160, "ab6761610000f178"),
    ],
    &[
        (640, "ab6765630000ba8a"),
        (300, "ab67656300005f1f"),
        (64, "ab6765630000f68d"),
    ],
];

/// The URL of the smallest size of Spotify's image at `url` that is still at least `pixels`
/// across, if that's smaller than the one at `url`.
fn smaller_image(url: &str, pixels: u32) -> Option<String> {
    let (base, id) = url.rsplit_once('/')?;
    if base != "https://i.scdn.co/image" {
        return None;
    }
    let sizes = IMAGE_SIZE_CODES
        .iter()
        .find(|sizes| sizes.iter().any(|(_, code)| id.starts_with(code)))?;
    let current = sizes.iter().position(|(_, code)| id.starts_with(code))?;
    let fitting = sizes[current..]
        .iter()
        .rposition(|&(size, _)| size >= pixels)?
        + current;
    (fitting != current).then(|| {
        let rest = &id[sizes[current].1.len()..];
        format!("{base}/{}{rest}", sizes[fitting].1)
    })
}

/// Put the last frame's Kitty images on the terminal, except ones overlapping `overlays` (menus
/// and dialogs drawn over the page: images are always drawn over text). Call after ratatui has
/// drawn. Covers still downloading or being scaled are left out until a later call.
pub fn render(overlays: &[Rect]) {
    #[cfg(not(feature = "cover"))]
    let _ = overlays;
    #[cfg(feature = "cover")]
    if protocol() == Some(Protocol::Kitty) {
        render_images(overlays);
    }
}

/// Write the frame's sixel or iTerm2 images into `buf`, except ones overlapping `overlays`; see
/// the module documentation. Call last while drawing the frame, once the overlays are drawn.
pub fn draw_into_buffer(buf: &mut Buffer, overlays: &[Rect]) {
    #[cfg(not(feature = "cover"))]
    let _ = (buf, overlays);
    #[cfg(feature = "cover")]
    if let Some(protocol @ (Protocol::Sixel | Protocol::Iterm2)) = protocol() {
        let font = font_size();
        with_layer(|layer| draw_cells(layer, protocol, font, buf, overlays));
    }
}

#[cfg(feature = "cover")]
fn draw_cells(
    layer: &mut Layer,
    protocol: Protocol,
    font: Size,
    buf: &mut Buffer,
    overlays: &[Rect],
) {
    for request in &layer.frame {
        let area = request.area;
        // Escape sequences can't be clipped: leave out images that don't fit.
        if area.intersection(buf.area) != area {
            continue;
        }
        // Only the part a menu leaves uncovered, cut out of the pixels.
        let Some(shown) = visible_part(area, overlays) else {
            continue;
        };
        let (width, height) = (
            u32::from(area.width * font.width),
            u32::from(area.height * font.height),
        );
        let key = (
            request.source.clone(),
            width,
            height,
            shown.x - area.x,
            shown.y - area.y,
            shown.width,
            shown.height,
        );
        if !layer.encoded.contains_key(&key) {
            // Kept, like Kitty's, so showing another part of it (a menu opened over it) doesn't
            // wait for it to be scaled again.
            let pixels_key = (request.source.clone(), width, height);
            if !layer.cache.contains_key(&pixels_key) {
                let Some(pixels) = load(&request.source, Size::new(width as u16, height as u16))
                else {
                    continue;
                };
                layer.cache.insert(pixels_key.clone(), pixels);
            }
            let pixels = image::imageops::crop_imm(
                &layer.cache[&pixels_key],
                u32::from((shown.x - area.x) * font.width),
                u32::from((shown.y - area.y) * font.height),
                u32::from(shown.width * font.width),
                u32::from(shown.height * font.height),
            )
            .to_image();
            match encode(protocol, &pixels) {
                Ok(encoded) => layer.encoded.insert(key.clone(), encoded),
                Err(e) => {
                    error!("Failed to encode image: {e}");
                    continue;
                }
            };
        }
        let symbol = cell_sequence(&layer.encoded[&key], shown);
        for position in shown.positions() {
            buf[position].set_diff_option(CellDiffOption::Skip);
        }
        buf[shown.as_position()]
            .set_symbol(&symbol)
            .set_diff_option(CellDiffOption::ForcedWidth(NonZeroU16::MIN));
    }
    if layer.encoded.len() > 400 {
        layer.encoded.clear();
    }
    if layer.cache.len() > 400 {
        layer.cache.clear();
    }
}

/// What to print in the top-left cell of `area` to show `image` (a sixel or iTerm2 sequence)
/// over it. The area is cleared first, in the cell's colors, so transparent pixels show the
/// right background and nothing of an older image is left. The cursor and colors are saved and
/// restored around it (DECSC/DECRC), then the cursor steps one cell right, as ratatui expects
/// after printing a cell.
#[cfg(feature = "cover")]
fn cell_sequence(image: &str, area: Rect) -> String {
    let mut out = String::with_capacity(image.len() + area.area() as usize + 64);
    out.push_str("\x1b7");
    for y in area.top()..area.bottom() {
        out.push_str(&format!("\x1b[{};{}H", y + 1, area.x + 1));
        out.extend(std::iter::repeat_n(' ', usize::from(area.width)));
    }
    out.push_str(&format!("\x1b[{};{}H", area.y + 1, area.x + 1));
    out.push_str(image);
    out.push_str("\x1b8\x1b[1C");
    out
}

/// `pixels` as a sixel or iTerm2 escape sequence.
#[cfg(feature = "cover")]
fn encode(protocol: Protocol, pixels: &image::RgbaImage) -> Result<String, String> {
    let (width, height) = pixels.dimensions();
    match protocol {
        Protocol::Sixel => icy_sixel::sixel_encode(
            pixels.as_raw(),
            width as usize,
            height as usize,
            &icy_sixel::EncodeOptions::default(),
        )
        .map_err(|e| e.to_string()),
        Protocol::Iterm2 => {
            let mut png = std::io::Cursor::new(Vec::new());
            pixels
                .write_to(&mut png, image::ImageFormat::Png)
                .map_err(|e| e.to_string())?;
            let png = png.into_inner();
            // Sized in pixels, so the image keeps its size instead of being scaled to the cells.
            Ok(format!(
                "\x1b]1337;File=inline=1;size={};width={width}px;height={height}px;\
                 preserveAspectRatio=0;doNotMoveCursor=1:{}\x07",
                png.len(),
                base64_encode(&png)
            ))
        }
        Protocol::Kitty => unreachable!("Kitty images aren't drawn into cells"),
    }
}

#[cfg(feature = "cover")]
fn render_images(overlays: &[Rect]) {
    let font = font_size();
    with_layer(|layer| {
        let generation = GENERATION.load(Ordering::Relaxed);
        if layer.generation != generation {
            // Everything was deleted (see reset).
            layer.placed.clear();
            layer.generation = generation;
        }
        let desired: Vec<Request> = layer
            .frame
            .iter()
            .filter_map(|r| {
                let shown = visible_part(r.area, overlays)?;
                Some(Request { shown, ..r.clone() })
            })
            .collect();

        // Keep what's already placed and still wanted; remove the rest.
        let mut kept = Vec::new();
        for placed in std::mem::take(&mut layer.placed) {
            if desired.contains(&placed.request)
                && !kept.iter().any(|k: &Placed| k.request == placed.request)
            {
                kept.push(placed);
            } else {
                let _ = delete(placed.id);
            }
        }
        for request in desired {
            if kept.iter().any(|k| k.request == request) {
                continue;
            }
            let pixels = Size::new(
                request.area.width * font.width,
                request.area.height * font.height,
            );
            let key = (
                request.source.clone(),
                u32::from(pixels.width),
                u32::from(pixels.height),
            );
            if !layer.cache.contains_key(&key) {
                let Some(image) = load(&request.source, pixels) else {
                    continue;
                };
                layer.cache.insert(key.clone(), image);
            }
            let id = layer.next_id;
            layer.next_id = layer.next_id.wrapping_add(1).max(FIRST_IMAGE_ID);
            match place(&layer.cache[&key], &request, font, id) {
                Ok(()) => kept.push(Placed { id, request }),
                Err(e) => error!("Failed to draw image: {e}"),
            }
        }
        layer.placed = kept;
        // Thumbnails of covers no longer shown can go; keep the cache from growing forever.
        if layer.cache.len() > 400 {
            layer.cache.clear();
        }
        layer.frame_drawn = false;
    });
}

/// Remove every Kitty image from the terminal and place them again on the next [render]. Needed
/// after a resize: terminals move or clip images when they reflow. (Sixel and iTerm2 images go
/// with the text: ratatui redraws everything after a resize or a clear.)
pub fn reset() {
    if protocol() == Some(Protocol::Kitty) {
        let mut stdout = std::io::stdout().lock();
        let _ = write!(stdout, "\x1b_Ga=d,d=A,q=2\x1b\\");
        let _ = stdout.flush();
    }
    GENERATION.fetch_add(1, Ordering::Relaxed);
}

/// Set the terminal up for images when the UI starts: with sixel, keep the cursor beside an
/// image instead of below it (mode 8452), so an image at the bottom doesn't scroll the screen.
pub fn start() {
    if protocol() == Some(Protocol::Sixel) {
        let mut stdout = std::io::stdout().lock();
        let _ = write!(stdout, "\x1b[?8452h");
        let _ = stdout.flush();
    }
}

/// Undo [start] when the UI stops.
pub fn stop() {
    if protocol() == Some(Protocol::Sixel) {
        let mut stdout = std::io::stdout().lock();
        let _ = write!(stdout, "\x1b[?8452l");
        let _ = stdout.flush();
    }
}

/// Remove all images, e.g. before quitting.
pub fn clear() {
    reset();
    #[cfg(feature = "cover")]
    with_layer(|layer| layer.placed.clear());
}

#[cfg(feature = "cover")]
type EncodedKey = (Source, u32, u32, u16, u16, u16, u16);

/// The biggest part of `area` that `overlays` (and their borders) leave uncovered, if any: an
/// image under a menu shows what's still in view rather than nothing.
#[cfg(feature = "cover")]
fn visible_part(area: Rect, overlays: &[Rect]) -> Option<Rect> {
    let mut shown = area;
    for overlay in overlays.iter().map(|o| grow(*o)) {
        if !shown.intersects(overlay) {
            continue;
        }
        let above = Rect::new(
            shown.x,
            shown.y,
            shown.width,
            overlay.y.saturating_sub(shown.y),
        );
        let below_y = overlay.bottom().max(shown.y);
        let below = Rect::new(
            shown.x,
            below_y,
            shown.width,
            shown.bottom().saturating_sub(below_y),
        );
        let left = Rect::new(
            shown.x,
            shown.y,
            overlay.x.saturating_sub(shown.x),
            shown.height,
        );
        let right_x = overlay.right().max(shown.x);
        let right = Rect::new(
            right_x,
            shown.y,
            shown.right().saturating_sub(right_x),
            shown.height,
        );
        shown = [above, below, left, right]
            .into_iter()
            .filter(|part| !part.is_empty())
            .max_by_key(|part| part.area())?;
    }
    Some(shown)
}

/// Menus and dialogs get a border and shadow around their area.
#[cfg(feature = "cover")]
fn grow(rect: Rect) -> Rect {
    Rect::new(
        rect.x.saturating_sub(1),
        rect.y.saturating_sub(1),
        rect.width + 2,
        rect.height + 2,
    )
}

/// The pixels for `source` at `size`, or `None` while a cover is still downloading or being
/// scaled (see [scaled_cover]).
#[cfg(feature = "cover")]
fn load(source: &Source, size: Size) -> Option<image::RgbaImage> {
    let (width, height) = (u32::from(size.width.max(1)), u32::from(size.height.max(1)));
    match source {
        Source::Url(url) => scaled_cover(url, width, height),
        #[cfg(feature = "cover")]
        Source::Icon(icon, color) => Some(icons::render(*icon, width, height, *color)),
    }
}

/// Threads that decode and scale covers, away from the UI thread: a large cover takes long
/// enough to stall the frame drawing it.
#[cfg(feature = "cover")]
fn scaling_pool() -> &'static crossbeam_channel::Sender<Job> {
    static POOL: OnceLock<crossbeam_channel::Sender<Job>> = OnceLock::new();
    POOL.get_or_init(|| start_pool("cover scaling", 2))
}

/// The cover at `url` scaled to `width` by `height` pixels, or `None` until it's been
/// downloaded and scaled in the background, which asks for a redraw once done. Each scaled
/// cover is handed out once: callers keep it (see [Layer::cache]).
#[cfg(feature = "cover")]
fn scaled_cover(url: &str, width: u32, height: u32) -> Option<image::RgbaImage> {
    type Scaled = HashMap<(String, u32, u32), Option<image::RgbaImage>>;
    /// Covers being scaled (`None`), and scaled ones not yet handed out.
    static SCALED: OnceLock<Mutex<Scaled>> = OnceLock::new();
    let scaled = || SCALED.get_or_init(Default::default).lock().unwrap();

    let key = (url.to_string(), width, height);
    {
        let mut scaled = scaled();
        match scaled.get(&key) {
            Some(None) => return None,
            Some(Some(_)) => return scaled.remove(&key).flatten(),
            None => {}
        }
    }
    let path = cached_cover_path(url)?;
    {
        let mut scaled = scaled();
        // Ones scaled but no longer drawn (scrolled away) would otherwise be kept forever.
        if scaled.len() > 400 {
            scaled.retain(|_, pixels| pixels.is_none());
        }
        scaled.insert(key.clone(), None);
    }
    let url = url.to_string();
    let job = move || {
        let image = image::ImageReader::open(&path)
            .and_then(|r| r.with_guessed_format())
            .map_err(|e| e.to_string())
            .and_then(|r| r.decode().map_err(|e| e.to_string()));
        match image {
            // Fill the area, cropping rather than letterboxing.
            Ok(image) => {
                let pixels = image
                    .resize_to_fill(width, height, image::imageops::FilterType::Triangle)
                    .to_rgba8();
                scaled().insert(key, Some(pixels));
            }
            Err(e) => {
                error!("Failed to load cover {}: {e}", path.display());
                scaled().remove(&key);
                forget_cover(&url, &path);
            }
        }
        crate::tui::request_redraw();
    };
    let _ = scaling_pool().send(Box::new(job));
    None
}

/// Whether the terminal can show images; see [protocol].
pub fn supported() -> bool {
    protocol().is_some()
}

/// The `image_protocol` setting, when it isn't `auto`: `None` turns images off.
static CONFIGURED: OnceLock<Option<Protocol>> = OnceLock::new();

/// Apply the `image_protocol` setting (`auto`, `kitty`, `sixel`, `iterm2` or `none`). Called
/// once at startup, before anything is drawn; an unknown value is the same as `auto`.
pub fn configure(setting: Option<&str>) {
    let protocol = match setting.map(str::to_lowercase).as_deref() {
        Some("kitty") => Some(Protocol::Kitty),
        Some("sixel") => Some(Protocol::Sixel),
        Some("iterm2") => Some(Protocol::Iterm2),
        Some("none") => None,
        Some("auto") | None => return,
        Some(other) => {
            error!("Unknown image_protocol {other:?}, detecting the terminal's instead");
            return;
        }
    };
    let _ = CONFIGURED.set(protocol);
}

/// How the terminal shows images, if it can: the `image_protocol` setting, or else judged from
/// the environment, so nothing has to be read back from the terminal.
pub fn protocol() -> Option<Protocol> {
    #[cfg(test)]
    if let Some(forced) = FORCE_SUPPORTED.with(std::cell::Cell::get) {
        return forced.then_some(Protocol::Kitty);
    }
    if !cfg!(feature = "cover") {
        return None;
    }
    static DETECTED: OnceLock<Option<Protocol>> = OnceLock::new();
    if let Some(configured) = CONFIGURED.get() {
        return *configured;
    }
    *DETECTED.get_or_init(|| {
        let var = |name: &str| std::env::var(name).unwrap_or_default();
        let protocol = detect(&var("TERM"), &var("TERM_PROGRAM"), |name| {
            !var(name).is_empty()
        });
        debug!("Image protocol: {protocol:?}");
        protocol
    })
}

/// The protocol for a terminal with these `TERM` and `TERM_PROGRAM` values and environment
/// variables (`is_set`).
fn detect(term: &str, program: &str, is_set: impl Fn(&str) -> bool) -> Option<Protocol> {
    let program = program.to_lowercase();
    if term.contains("kitty")
        || term.contains("ghostty")
        || is_set("KITTY_WINDOW_ID")
        || is_set("GHOSTTY_RESOURCES_DIR")
        || is_set("KONSOLE_VERSION")
        || program == "ghostty"
        || program == "wezterm"
    {
        Some(Protocol::Kitty)
    } else if program == "iterm.app" || is_set("ITERM_SESSION_ID") {
        Some(Protocol::Iterm2)
    } else if term.starts_with("foot")
        || term.contains("mlterm")
        || term.starts_with("contour")
        || is_set("WT_SESSION")
        || program == "mintty"
    {
        Some(Protocol::Sixel)
    } else {
        None
    }
}

#[cfg(test)]
thread_local! {
    static FORCE_SUPPORTED: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
}

/// Make [supported] answer `supported` on this thread, to test both layouts.
#[cfg(test)]
pub fn force_supported(supported: bool) {
    FORCE_SUPPORTED.with(|f| f.set(Some(supported)));
}

/// Size of a terminal cell in pixels (or at least its aspect ratio).
pub fn font_size() -> Size {
    #[cfg(feature = "cover")]
    {
        // SAFETY: TIOCGWINSZ only writes a `winsize` to the pointer it's given.
        let mut size: libc::winsize = unsafe { std::mem::zeroed() };
        unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut size) };
        let libc::winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: xpixels,
            ws_ypixel: ypixels,
        } = size;
        if cols > 0 && rows > 0 && xpixels > 0 && ypixels > 0 {
            return Size::new((xpixels / cols).max(1), (ypixels / rows).max(1));
        }
    }
    Size::new(8, 16)
}

/// Width in cells of a square picture `rows` tall.
pub fn square_width(rows: u16) -> u16 {
    let font = font_size();
    (rows * font.height).div_ceil(font.width.max(1))
}

type Job = Box<dyn FnOnce() + Send>;

/// Start `threads` threads called `name` that run the jobs sent to the returned channel, any
/// thread taking the next job, so at most `threads` run at once.
fn start_pool(name: &str, threads: usize) -> crossbeam_channel::Sender<Job> {
    let (tx, rx) = crossbeam_channel::unbounded::<Job>();
    for _ in 0..threads {
        let rx = rx.clone();
        std::thread::Builder::new()
            .name(name.into())
            .spawn(move || rx.into_iter().for_each(|job| job()))
            .expect("could not start a background thread");
    }
    tx
}

/// Covers downloaded at once: a page of thumbnails would otherwise start dozens of connections.
const DOWNLOAD_THREADS: usize = 4;

/// How long a cover that couldn't be downloaded (or read) is left before it's tried again, so
/// being offline doesn't start a download for every cover on every frame.
const RETRY_AFTER: std::time::Duration = std::time::Duration::from_secs(60);

/// Covers being downloaded (or waiting to be), and when ones that failed last were tried.
#[derive(Default)]
struct Downloads {
    pending: HashSet<String>,
    /// Kept until a download succeeds, also while it's tried again.
    failed: HashMap<String, std::time::Instant>,
}

impl Downloads {
    /// Whether to start downloading `url` (not on disk) now: not if it's already on its way, or
    /// failed less than [RETRY_AFTER] ago. Counts it as pending if so.
    fn start(&mut self, url: &str, now: std::time::Instant) -> bool {
        if self.pending.contains(url)
            || self
                .failed
                .get(url)
                .is_some_and(|&at| now.duration_since(at) < RETRY_AFTER)
        {
            return false;
        }
        self.pending.insert(url.to_string())
    }

    /// `url` is done downloading, or failed (`ok` false) at `now`.
    fn finish(&mut self, url: &str, ok: bool, now: std::time::Instant) {
        self.pending.remove(url);
        if ok {
            self.failed.remove(url);
        } else {
            self.failed.insert(url.to_string(), now);
        }
    }
}

fn downloads() -> std::sync::MutexGuard<'static, Downloads> {
    static DOWNLOADS: OnceLock<Mutex<Downloads>> = OnceLock::new();
    DOWNLOADS.get_or_init(Default::default).lock().unwrap()
}

/// Local path of the cover image at `url`, or `None` while it's still being downloaded (a
/// download is started in the background the first time). Each cover is downloaded once.
pub fn cached_cover_path(url: &str) -> Option<PathBuf> {
    static POOL: OnceLock<crossbeam_channel::Sender<Job>> = OnceLock::new();

    let path = crate::utils::cache_path_for_url(url.to_string());
    // Files only appear there once complete (see utils::download).
    if path.exists() {
        return Some(path);
    }
    if !downloads().start(url, std::time::Instant::now()) {
        return None;
    }
    let url = url.to_string();
    let job = move || {
        let result = crate::utils::download(url.clone(), path);
        if let Err(e) = &result {
            error!("Failed to download cover {url}: {e}");
        }
        downloads().finish(&url, result.is_ok(), std::time::Instant::now());
        crate::tui::request_redraw();
    };
    let pool = POOL.get_or_init(|| start_pool("cover download", DOWNLOAD_THREADS));
    let _ = pool.send(Box::new(job));
    None
}

/// The cached cover for `url`, at `path`, couldn't be read: delete it so it's downloaded again,
/// after [RETRY_AFTER] so a server sending something unreadable isn't asked again every frame.
fn forget_cover(url: &str, path: &std::path::Path) {
    let _ = std::fs::remove_file(path);
    let mut downloads = downloads();
    downloads.pending.remove(url);
    downloads
        .failed
        .insert(url.to_string(), std::time::Instant::now());
}

/// The main color of the cover at `url` (its average, skipping near-black and near-white pixels),
/// for tinting page headers. `None` until it's been worked out in the background.
#[cfg(not(feature = "cover"))]
pub fn cover_tint(_url: &str) -> Option<[u8; 3]> {
    None
}

/// The main color of the cover at `url`; see the version without images.
#[cfg(feature = "cover")]
pub fn cover_tint(url: &str) -> Option<[u8; 3]> {
    static TINTS: OnceLock<RwLock<HashMap<String, Option<[u8; 3]>>>> = OnceLock::new();
    let tints = TINTS.get_or_init(Default::default);
    if let Some(tint) = tints.read().unwrap().get(url) {
        return *tint;
    }
    let path = cached_cover_path(url)?;
    tints.write().unwrap().insert(url.to_string(), None);
    let url = url.to_string();
    let job = move || {
        let tint = image::open(&path).ok().map(|image| {
            let small = image.thumbnail(32, 32).to_rgb8();
            average_color(small.pixels().map(|p| p.0))
        });
        TINTS
            .get()
            .unwrap()
            .write()
            .unwrap()
            .insert(url, tint.flatten());
        crate::tui::request_redraw();
    };
    let _ = scaling_pool().send(Box::new(job));
    None
}

#[cfg_attr(not(feature = "cover"), allow(dead_code))]
/// Average of `pixels`, leaving out ones too dark or light to say much about the color.
fn average_color(pixels: impl Iterator<Item = [u8; 3]>) -> Option<[u8; 3]> {
    let (mut sum, mut count) = ([0u64; 3], 0u64);
    for [r, g, b] in pixels {
        let brightness = (u16::from(r) + u16::from(g) + u16::from(b)) / 3;
        if (24..=232).contains(&brightness) {
            sum[0] += u64::from(r);
            sum[1] += u64::from(g);
            sum[2] += u64::from(b);
            count += 1;
        }
    }
    (count > 0).then(|| sum.map(|c| (c / count) as u8))
}

/// Place `pixels` over `area` with the Kitty graphics protocol as image `id` (replacing any image
/// with that ID), without moving the cursor (`C=1`) or asking the terminal for a reply (`q=2`).
#[cfg(feature = "cover")]
fn place(pixels: &image::RgbaImage, request: &Request, font: Size, id: u32) -> std::io::Result<()> {
    let (area, shown) = (request.area, request.shown);
    let data = base64_encode(pixels.as_raw());
    let mut out: Vec<u8> = Vec::with_capacity(data.len() + 256);
    write!(out, "\x1b_Ga=d,d=I,i={id},q=2\x1b\\")?;
    // Only part of it is shown: that part of the pixels (x, y, w, h), over its cells.
    let (crop, y_shift) = if shown == area {
        (String::new(), request.y_shift)
    } else {
        let crop = format!(
            "x={},y={},w={},h={},",
            (shown.x - area.x) * font.width,
            (shown.y - area.y) * font.height,
            shown.width * font.width,
            shown.height * font.height,
        );
        (crop, 0)
    };
    // Save the cursor, move to the top-left cell, and restore the cursor afterwards.
    let Position { x, y } = shown.as_position();
    write!(out, "\x1b7\x1b[{};{}H", y + 1, x + 1)?;
    let chunks: Vec<&[u8]> = data.as_bytes().chunks(4096).collect();
    for (i, chunk) in chunks.iter().enumerate() {
        let more = u8::from(i + 1 < chunks.len());
        if i == 0 {
            write!(
                out,
                "\x1b_Ga=T,i={id},f=32,s={},v={},{crop}c={},r={},Y={y_shift},C=1,q=2,m={more};",
                pixels.width(),
                pixels.height(),
                shown.width,
                shown.height
            )?;
        } else {
            write!(out, "\x1b_Gm={more};")?;
        }
        out.extend_from_slice(chunk);
        out.extend_from_slice(b"\x1b\\");
    }
    out.extend_from_slice(b"\x1b8");
    let mut stdout = std::io::stdout().lock();
    stdout.write_all(&out)?;
    stdout.flush()
}

#[cfg(feature = "cover")]
/// Delete image `id`. Images are deleted by ID: deleting "whatever covers this cell" removes
/// neighbouring images too in some terminals (Ghostty).
fn delete(id: u32) -> std::io::Result<()> {
    let mut stdout = std::io::stdout().lock();
    write!(stdout, "\x1b_Ga=d,d=I,i={id},q=2\x1b\\")?;
    stdout.flush()
}

/// Standard base64 with padding.
#[cfg_attr(not(feature = "cover"), allow(dead_code))]
fn base64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = match *chunk {
            [a, b, c] => u32::from_be_bytes([0, a, b, c]),
            [a, b] => u32::from_be_bytes([0, a, b, 0]),
            [a] => u32::from_be_bytes([0, a, 0, 0]),
            _ => unreachable!(),
        };
        let symbols = [18, 12, 6, 0].map(|shift| ALPHABET[(n >> shift) as usize & 63] as char);
        let kept = chunk.len() + 1;
        out.extend(&symbols[..kept]);
        out.extend(std::iter::repeat_n('=', 4 - kept));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_base64() {
        assert_eq!(base64_encode(b"Man"), "TWFu");
        assert_eq!(base64_encode(b"Ma"), "TWE=");
        assert_eq!(base64_encode(b"M"), "TQ==");
    }

    #[test]
    fn test_detect() {
        let none = |_: &str| false;
        assert_eq!(detect("xterm-kitty", "", none), Some(Protocol::Kitty));
        assert_eq!(
            detect("xterm-256color", "WezTerm", none),
            Some(Protocol::Kitty)
        );
        assert_eq!(
            detect("xterm-256color", "iTerm.app", none),
            Some(Protocol::Iterm2)
        );
        assert_eq!(detect("foot", "", none), Some(Protocol::Sixel));
        assert_eq!(
            detect("xterm-256color", "", |name| name == "WT_SESSION"),
            Some(Protocol::Sixel)
        );
        assert_eq!(detect("xterm-256color", "", none), None);
    }

    #[test]
    #[cfg(feature = "cover")]
    fn test_sixel_in_cells() {
        let area = Rect::new(0, 0, 10, 4);
        let image = Rect::new(2, 1, 3, 2);
        let mut layer = Layer::new();
        let font = Size::new(8, 16);
        layer.frame.push(Request {
            source: Source::Icon(Icon::Play, [255, 255, 255]),
            area: image,
            y_shift: 0,
            shown: image,
        });
        let request = layer.frame[0].clone();
        let mut buf = Buffer::empty(area);
        draw_cells(&mut layer, Protocol::Sixel, font, &mut buf, &[]);
        let first = &buf[(2, 1)];
        assert!(first.symbol().starts_with("\x1b7"));
        assert!(first.symbol().contains("\x1bP"));
        assert_eq!(
            first.diff_option,
            CellDiffOption::ForcedWidth(NonZeroU16::MIN)
        );
        for position in image.positions().skip(1) {
            assert_eq!(buf[position].diff_option, CellDiffOption::Skip);
        }
        assert_eq!(buf[(0, 0)].diff_option, CellDiffOption::None);

        // Unchanged in the next frame: nothing to redraw. Once it's gone, its cells are redrawn.
        let mut next = Buffer::empty(area);
        draw_cells(&mut layer, Protocol::Sixel, font, &mut next, &[]);
        assert!(buf.diff(&next).is_empty());
        layer.frame.clear();
        let empty = Buffer::empty(area);
        assert_eq!(buf.diff(&empty).len(), 6);

        // Under a menu (and its border), only the column left of it shows.
        let mut covered = Buffer::empty(area);
        layer.frame.push(request);
        draw_cells(
            &mut layer,
            Protocol::Sixel,
            font,
            &mut covered,
            &[Rect::new(4, 2, 2, 1)],
        );
        assert!(covered[(2, 1)].symbol().contains("\x1bP"));
        assert_eq!(covered[(3, 1)], Buffer::empty(area)[(3, 1)]);
    }

    #[test]
    #[cfg(feature = "cover")]
    fn test_visible_part() {
        let image = Rect::new(10, 0, 20, 10);
        assert_eq!(visible_part(image, &[]), Some(image));
        // A menu over its lower part, and its border: the rows above stay.
        assert_eq!(
            visible_part(image, &[Rect::new(0, 6, 50, 10)]),
            Some(Rect::new(10, 0, 20, 5))
        );
        // Over its right side: the columns at the left.
        assert_eq!(
            visible_part(image, &[Rect::new(20, 0, 30, 20)]),
            Some(Rect::new(10, 0, 9, 10))
        );
        assert_eq!(visible_part(image, &[Rect::new(5, 0, 40, 20)]), None);
    }

    #[test]
    #[cfg(feature = "cover")]
    fn test_cell_sequence() {
        let sequence = cell_sequence("IMAGE", Rect::new(2, 1, 3, 2));
        assert_eq!(
            sequence,
            "\x1b7\x1b[2;3H   \x1b[3;3H   \x1b[2;3HIMAGE\x1b8\x1b[1C"
        );
    }

    #[test]
    fn test_smaller_image() {
        let cover = "https://i.scdn.co/image/ab67616d0000b273c0ffee";
        assert_eq!(
            smaller_image(cover, 40).as_deref(),
            Some("https://i.scdn.co/image/ab67616d00004851c0ffee")
        );
        assert_eq!(
            smaller_image(cover, 140).as_deref(),
            Some("https://i.scdn.co/image/ab67616d00001e02c0ffee")
        );
        assert_eq!(smaller_image(cover, 500), None);
        assert_eq!(
            smaller_image("https://i.scdn.co/image/ab6761610000e5ebc0ffee", 140).as_deref(),
            Some("https://i.scdn.co/image/ab6761610000f178c0ffee")
        );
        // Already small, or not one of Spotify's.
        assert_eq!(
            smaller_image("https://i.scdn.co/image/ab67616d00004851c0ffee", 40),
            None
        );
        assert_eq!(
            smaller_image("https://example.com/image/ab67616d0000b273", 40),
            None
        );
        assert_eq!(
            smaller_image("https://i.scdn.co/image/ab67706c0000da84", 40),
            None
        );
    }

    #[test]
    fn test_downloads_wait_before_retrying() {
        let mut downloads = Downloads::default();
        let start = std::time::Instant::now();
        assert!(downloads.start("a", start));
        // Already on its way.
        assert!(!downloads.start("a", start));
        downloads.finish("a", false, start);
        // Failed: left alone for a while, then tried again.
        assert!(!downloads.start("a", start + RETRY_AFTER / 2));
        assert!(downloads.start("a", start + RETRY_AFTER));
        downloads.finish("a", true, start + RETRY_AFTER);
        assert!(downloads.start("a", start + RETRY_AFTER));
    }

    #[test]
    fn test_average_color_skips_extremes() {
        let pixels = [[0, 0, 0], [255, 255, 255], [100, 50, 150], [120, 70, 170]];
        assert_eq!(average_color(pixels.into_iter()), Some([110, 60, 160]));
        assert_eq!(average_color([[0, 0, 0]].into_iter()), None);
    }
}
