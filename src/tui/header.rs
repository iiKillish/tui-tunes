//! The header above album, playlist and artist pages: the cover on the left; what the page is,
//! its name and details on the right; Play, Shuffle and Like buttons under them. The background
//! takes on the cover's main color and fades into the page.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;

use crate::theme::Palette;
use crate::tui::images;
use crate::tui::style::{Styles, bold, fill, mix, put, rgb, width};

/// Rows the header takes, including a blank row under it.
pub const HEADER_ROWS: u16 = 9;
/// Rows of the cover.
const COVER_ROWS: u16 = 7;
/// Narrowest text beside the cover.
const MIN_TEXT_WIDTH: u16 = 24;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeaderAction {
    Play,
    Shuffle,
    Like,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageHeader {
    /// What the page is: "Album", "Playlist", "Artist".
    pub kind: String,
    pub title: String,
    /// Shown joined with " • ": artist or owner, year, "12 songs, 45 min".
    pub details: Vec<String>,
    pub cover_url: Option<String>,
    /// Whether to show the Play and Shuffle buttons.
    pub playable: bool,
    /// The Like button's state, or `None` for no Like button.
    pub liked: Option<bool>,
}

/// Draw `header` into `area` (normally [HEADER_ROWS] tall). Returns where each button was drawn,
/// for clicks.
pub fn draw(
    frame: &mut Frame,
    area: Rect,
    header: &PageHeader,
    palette: &Palette,
) -> Vec<(Rect, HeaderAction)> {
    let buf = frame.buffer_mut();

    // Background: the cover's color fading into the page, row by row.
    // Without a known background there is nothing to fade into: no tint.
    let tint = palette
        .base
        .and_then(|base| Some((base, images::cover_tint(header.cover_url.as_deref()?)?)));
    for row in 0..area.height {
        let bg = match tint {
            Some((base, tint)) => {
                let fade = f32::from(row) / f32::from(area.height.max(1));
                mix(mix_rgb(base, tint, 0.45), base, fade)
            }
            None => palette.background,
        };
        fill(
            buf,
            Rect::new(area.x, area.y + row, area.width, 1),
            Style::new().bg(bg),
        );
    }
    // Text styles keep the tinted background already in the cells.
    let fg = |color| Style::new().fg(color);

    let mut text_x = area.x + 2;
    let cover_width = images::square_width(COVER_ROWS);
    if let Some(url) = &header.cover_url
        && images::supported()
        && area.height > COVER_ROWS
        // Narrow windows leave the cover out rather than squeeze the text.
        && area.width >= 2 + cover_width + 3 + MIN_TEXT_WIDTH + 2
    {
        let width = cover_width;
        images::cover(url, Rect::new(area.x + 2, area.y + 1, width, COVER_ROWS));
        text_x = area.x + 2 + width + 3;
    }
    let width = usize::from(area.right().saturating_sub(text_x + 2));

    put(
        buf,
        text_x,
        area.y + 1,
        &header.kind,
        width,
        fg(palette.secondary),
    );
    put(
        buf,
        text_x,
        area.y + 2,
        &header.title,
        width,
        bold(fg(palette.title)),
    );
    put(
        buf,
        text_x,
        area.y + 3,
        &header.details.join(" • "),
        width,
        fg(palette.secondary),
    );

    let mut buttons = Vec::new();
    let mut x = text_x;
    for (label, style, action) in button_labels(header, palette, false) {
        let written = put(
            buf,
            x,
            area.y + 5,
            &label,
            usize::from(area.right().saturating_sub(x)),
            style,
        );
        if written > 0 {
            buttons.push((Rect::new(x, area.y + 5, written as u16, 1), action));
        }
        x += written as u16 + 2;
    }
    buttons
}

/// The header's buttons, right-aligned in the one-row `area`, for windows too short for the whole
/// header: Play and Shuffle. Labelled if that leaves `keep` columns on the left (for the page's
/// title), just icons if not; nothing if even those don't fit. Returns where each was drawn, for
/// clicks.
pub fn draw_buttons(
    frame: &mut Frame,
    area: Rect,
    header: &PageHeader,
    palette: &Palette,
    keep: u16,
) -> Vec<(Rect, HeaderAction)> {
    let total = |labels: &[(String, Style, HeaderAction)]| {
        labels
            .iter()
            .map(|(label, ..)| width(label) as u16 + 2)
            .sum::<u16>()
            .saturating_sub(2)
    };
    let play_and_shuffle = |short| {
        let mut labels = button_labels(header, palette, short);
        labels.retain(|(.., action)| *action != HeaderAction::Like);
        labels
    };
    let mut labels = play_and_shuffle(false);
    if total(&labels) + keep > area.width {
        labels = play_and_shuffle(true);
    }
    let total = total(&labels);
    if labels.is_empty() || total > area.width {
        return Vec::new();
    }
    let buf = frame.buffer_mut();
    let mut buttons = Vec::new();
    let mut x = area.right() - total;
    for (label, style, action) in labels {
        let written = put(buf, x, area.y, &label, usize::MAX, style) as u16;
        buttons.push((Rect::new(x, area.y, written, 1), action));
        x += written + 2;
    }
    buttons
}

/// The buttons' labels (padded, and just the icons if `short`), styles and actions: Play stands
/// out in the accent, the others are plain.
fn button_labels(
    header: &PageHeader,
    palette: &Palette,
    short: bool,
) -> Vec<(String, Style, HeaderAction)> {
    let fg = |color| Style::new().fg(color);
    let label = |icon: &str, text: &str| match short {
        true => format!(" {icon} "),
        false => format!(" {icon} {text} "),
    };
    let mut buttons = Vec::new();
    if header.playable {
        buttons.push((
            label("▶", "Play"),
            Styles(palette).button(),
            HeaderAction::Play,
        ));
        buttons.push((
            label("⇄", "Shuffle"),
            fg(palette.primary),
            HeaderAction::Shuffle,
        ));
    }
    if let Some(liked) = header.liked {
        let (icon, text, style) = if liked {
            ("♥", "Liked", fg(palette.title))
        } else {
            ("♡", "Like", fg(palette.primary))
        };
        buttons.push((label(icon, text), style, HeaderAction::Like));
    }
    buttons
}

fn mix_rgb(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
    rgb(mix(a, b, t))
}
