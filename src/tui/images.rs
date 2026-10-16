//! Pictures in the terminal with the Kitty graphics protocol: covers and the player's control
//! icons.
//!
//! Widgets ask for images while drawing a frame ([Frame::image], [Frame::icon] via
//! [request_image]); after ratatui has drawn the frame, [render] puts them on the terminal,
//! touching only images that changed. Images not requested in a frame are removed, so widgets
//! that stop being drawn never have to clean up after themselves.

// Without the `cover` feature there are no images: most of this compiles to nothing.
#![cfg_attr(
    not(feature = "cover"),
    allow(unused_imports, unused_variables, dead_code)
)]

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use log::{debug, error};
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

#[cfg(feature = "cover")]
#[derive(Clone, PartialEq, Debug)]
struct Request {
    source: Source,
    area: Rect,
    /// Pixels to move the image down within its first row (to place it between rows).
    y_shift: u16,
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
}

#[cfg(feature = "cover")]
static LAYER: Mutex<Option<Layer>> = Mutex::new(None);
static GENERATION: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "cover")]
fn with_layer<R>(f: impl FnOnce(&mut Layer) -> R) -> R {
    let mut layer = LAYER.lock().unwrap();
    f(layer.get_or_insert_with(|| Layer {
        frame: Vec::new(),
        frame_drawn: false,
        placed: Vec::new(),
        next_id: FIRST_IMAGE_ID,
        generation: 0,
        cache: HashMap::new(),
    }))
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
            });
        });
    }
}

/// Show the cover at `url` filling `area` in the frame being drawn.
pub fn cover(url: &str, area: Rect) {
    request_image(Source::Url(url.to_string()), area, 0);
}

/// Put the last frame's images on the terminal, except ones overlapping `overlays` (menus and
/// dialogs drawn over the page: images are always drawn over text). Call after ratatui has drawn.
/// Covers still downloading are retried on the next call.
pub fn render(overlays: &[Rect]) {
    #[cfg(not(feature = "cover"))]
    let _ = overlays;
    #[cfg(feature = "cover")]
    if supported() {
        render_images(overlays);
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
            .filter(|r| !overlays.iter().any(|o| r.area.intersects(grow(*o))))
            .cloned()
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
            match place(&layer.cache[&key], request.area, id, request.y_shift) {
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

/// Remove every image from the terminal and place them again on the next [render]. Needed after
/// a resize: terminals move or clip images when they reflow.
pub fn reset() {
    if supported() {
        let mut stdout = std::io::stdout().lock();
        let _ = write!(stdout, "\x1b_Ga=d,d=A,q=2\x1b\\");
        let _ = stdout.flush();
    }
    GENERATION.fetch_add(1, Ordering::Relaxed);
}

/// Remove all images, e.g. before quitting.
pub fn clear() {
    reset();
    #[cfg(feature = "cover")]
    with_layer(|layer| layer.placed.clear());
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

/// The pixels for `source` at `size`, or `None` while a cover is still downloading.
#[cfg(feature = "cover")]
fn load(source: &Source, size: Size) -> Option<image::RgbaImage> {
    let (width, height) = (u32::from(size.width.max(1)), u32::from(size.height.max(1)));
    match source {
        Source::Url(url) => {
            let path = cached_cover_path(url)?;
            let image = image::ImageReader::open(&path)
                .and_then(|r| r.with_guessed_format())
                .map_err(|e| e.to_string())
                .and_then(|r| r.decode().map_err(|e| e.to_string()));
            match image {
                // Fill the area, cropping rather than letterboxing.
                Ok(image) => Some(
                    image
                        .resize_to_fill(width, height, image::imageops::FilterType::Triangle)
                        .to_rgba8(),
                ),
                Err(e) => {
                    error!("Failed to load cover {}: {e}", path.display());
                    None
                }
            }
        }
        #[cfg(feature = "cover")]
        Source::Icon(icon, color) => Some(icons::render(*icon, width, height, *color)),
    }
}

/// Whether the terminal speaks the Kitty graphics protocol (Kitty, Ghostty, WezTerm, Konsole),
/// judged from the environment so nothing has to be read back from the terminal.
pub fn supported() -> bool {
    static SUPPORTED: OnceLock<bool> = OnceLock::new();
    *SUPPORTED.get_or_init(|| {
        if !cfg!(feature = "cover") {
            return false;
        }
        let var = |name: &str| std::env::var(name).unwrap_or_default();
        let term = var("TERM");
        let program = var("TERM_PROGRAM").to_lowercase();
        let supported = term.contains("kitty")
            || term.contains("ghostty")
            || !var("KITTY_WINDOW_ID").is_empty()
            || !var("GHOSTTY_RESOURCES_DIR").is_empty()
            || !var("KONSOLE_VERSION").is_empty()
            || program == "ghostty"
            || program == "wezterm";
        debug!("Kitty graphics: {supported}");
        supported
    })
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

/// Local path of the cover image at `url`, or `None` while it's still being downloaded (a
/// download is started in the background the first time). Each cover is downloaded once.
pub fn cached_cover_path(url: &str) -> Option<PathBuf> {
    static LOADING: OnceLock<Arc<RwLock<HashSet<String>>>> = OnceLock::new();
    let loading = LOADING.get_or_init(Default::default).clone();

    let path = crate::utils::cache_path_for_url(url.to_string());
    let mut in_progress = loading.write().unwrap();
    if in_progress.contains(url) {
        return None;
    }
    if path.exists() {
        return Some(path);
    }
    in_progress.insert(url.to_string());
    drop(in_progress);

    let url = url.to_string();
    std::thread::spawn(move || {
        if let Err(e) = crate::utils::download(url.clone(), path) {
            error!("Failed to download cover: {e}");
        }
        loading.write().unwrap().remove(&url);
        crate::tui::request_redraw();
    });
    None
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
    std::thread::spawn(move || {
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
    });
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
fn place(pixels: &image::RgbaImage, area: Rect, id: u32, y_shift: u16) -> std::io::Result<()> {
    let data = base64_encode(pixels.as_raw());
    let mut out: Vec<u8> = Vec::with_capacity(data.len() + 256);
    write!(out, "\x1b_Ga=d,d=I,i={id},q=2\x1b\\")?;
    // Save the cursor, move to the top-left cell, and restore the cursor afterwards.
    let Position { x, y } = area.as_position();
    write!(out, "\x1b7\x1b[{};{}H", y + 1, x + 1)?;
    let chunks: Vec<&[u8]> = data.as_bytes().chunks(4096).collect();
    for (i, chunk) in chunks.iter().enumerate() {
        let more = u8::from(i + 1 < chunks.len());
        if i == 0 {
            write!(
                out,
                "\x1b_Ga=T,i={id},f=32,s={},v={},c={},r={},Y={y_shift},C=1,q=2,m={more};",
                pixels.width(),
                pixels.height(),
                area.width,
                area.height
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
    fn test_average_color_skips_extremes() {
        let pixels = [[0, 0, 0], [255, 255, 255], [100, 50, 150], [120, 70, 170]];
        assert_eq!(average_color(pixels.into_iter()), Some([110, 60, 160]));
        assert_eq!(average_color([[0, 0, 0]].into_iter()), None);
    }
}
