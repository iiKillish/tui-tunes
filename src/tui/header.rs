//! The header above album, playlist and artist pages: the cover on the left; what the page is,
//! its name and details on the right; Play, Shuffle and Like buttons under them. The background
//! takes on the cover's main color and fades into the page.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;

use crate::theme::Palette;
use crate::tui::images;
use crate::tui::style::{Styles, bold, fill, mix, put, rgb};

/// Rows the header takes, including a blank row under it.
pub const HEADER_ROWS: u16 = 9;
/// Rows of the cover.
const COVER_ROWS: u16 = 7;

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
    let styles = Styles(palette);
    let buf = frame.buffer_mut();

    // Background: the cover's color fading into the page, row by row.
    let base = rgb(palette.background);
    let tint = header.cover_url.as_deref().and_then(images::cover_tint);
    for row in 0..area.height {
        let bg = match tint {
            Some(tint) => {
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
    if let Some(url) = &header.cover_url
        && images::supported()
        && area.height > COVER_ROWS
    {
        let width = images::square_width(COVER_ROWS);
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

    // Buttons: Play stands out in the accent, the others are plain.
    let mut buttons = Vec::new();
    let mut x = text_x;
    let y = area.y + 5;
    let mut button = |label: &str, style: Style, action: HeaderAction, x: &mut u16| {
        let text = format!(" {label} ");
        let written = put(
            buf,
            *x,
            y,
            &text,
            usize::from(area.right().saturating_sub(*x)),
            style,
        );
        if written > 0 {
            buttons.push((Rect::new(*x, y, written as u16, 1), action));
        }
        *x += written as u16 + 2;
    };
    if header.playable {
        button("▶ Play", styles.button(), HeaderAction::Play, &mut x);
        button(
            "⇄ Shuffle",
            fg(palette.primary),
            HeaderAction::Shuffle,
            &mut x,
        );
    }
    if let Some(liked) = header.liked {
        let (label, style) = if liked {
            ("♥ Liked", fg(palette.title))
        } else {
            ("♡ Like", fg(palette.primary))
        };
        button(label, style, HeaderAction::Like, &mut x);
    }
    buttons
}

fn mix_rgb(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
    rgb(mix(a, b, t))
}
