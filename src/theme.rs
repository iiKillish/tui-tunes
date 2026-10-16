//! Colors from the configuration's theme (see [ConfigTheme]), as a [Palette] for the TUI.
//!
//! By default ttunes has no colors of its own: text uses the terminal's foreground and background
//! and the accent is an ANSI color, so it matches whatever theme the terminal has. Surfaces (the
//! player, the selected row) are the terminal's real background moved a little towards its
//! foreground, when the terminal reports them (see [crate::terminal_colors]).

use log::warn;
use ratatui::style::Color;

use crate::config::ConfigTheme;
use crate::terminal_colors::{self, Rgb, TerminalColors};
use crate::tui::style;

/// The theme's colors. Names follow the configuration keys; see the user documentation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    pub background: Color,
    pub primary: Color,
    pub secondary: Color,
    /// Titles and the accent color (current tab, selected filter, focused things).
    pub title: Color,
    pub playing: Color,
    pub playing_bg: Color,
    pub highlight: Color,
    pub highlight_bg: Color,
    /// Selection in a part without focus, and surfaces like empty cover squares.
    pub highlight_inactive_bg: Color,
    pub error: Color,
    pub error_bg: Color,
    pub statusbar_progress: Color,
    pub statusbar_progress_bg: Color,
    /// The darker background of the player and other panels.
    pub statusbar_bg: Color,
    pub cmdline: Color,
    pub cmdline_bg: Color,
    /// The real background behind `background`, if known: what the page header's cover tint
    /// fades into.
    pub base: Option<Rgb>,
    /// The colors the terminal reported, to turn named colors into RGB (see [Palette::rgb]).
    pub terminal: TerminalColors,
}

impl Palette {
    /// RGB of a text color, for drawing icons in it: the terminal's own values where it reported
    /// them, typical ones otherwise.
    pub fn rgb(&self, color: Color) -> Rgb {
        let ansi = |index: usize| self.terminal.ansi[index].unwrap_or(style::rgb(color));
        match color {
            Color::Rgb(r, g, b) => [r, g, b],
            Color::Reset => self.terminal.foreground.unwrap_or(match self.base {
                Some(base) if is_light(base) => [0, 0, 0],
                _ => style::rgb(Color::Gray),
            }),
            Color::Indexed(index) if index < 16 => ansi(usize::from(index)),
            Color::Black => ansi(0),
            Color::Red => ansi(1),
            Color::Green => ansi(2),
            Color::Yellow => ansi(3),
            Color::Blue => ansi(4),
            Color::Magenta => ansi(5),
            Color::Cyan => ansi(6),
            Color::Gray => ansi(7),
            Color::DarkGray => ansi(8),
            Color::LightRed => ansi(9),
            Color::LightGreen => ansi(10),
            Color::LightYellow => ansi(11),
            Color::LightBlue => ansi(12),
            Color::LightMagenta => ansi(13),
            Color::LightCyan => ansi(14),
            Color::White => ansi(15),
            Color::Indexed(_) => style::rgb(color),
        }
    }
}

/// Relative luminance, 0 (black) to 1 (white).
fn luminance([r, g, b]: Rgb) -> f32 {
    (0.2126 * f32::from(r) + 0.7152 * f32::from(g) + 0.0722 * f32::from(b)) / 255.0
}

fn is_light(color: Rgb) -> bool {
    luminance(color) > 0.5
}

/// Background colors derived from the real background.
#[derive(Debug, PartialEq)]
struct Surfaces {
    /// The player, menus and the selected row.
    panel: Color,
    /// Selection without focus, empty cover squares.
    inactive: Color,
    /// Text on the accent (filled buttons).
    on_accent: Color,
}

/// Surfaces for a terminal with background `base` and text `foreground`: the background moved
/// towards the text, so lighter on dark themes and darker on light ones. Without a known
/// background, the terminal's default background and ANSI colors.
fn surfaces(base: Option<Rgb>, foreground: Option<Rgb>) -> Surfaces {
    let Some(base) = base else {
        return Surfaces {
            panel: Color::Reset,
            inactive: Color::DarkGray,
            on_accent: Color::Black,
        };
    };
    let light = is_light(base);
    let toward = foreground
        .filter(|&fg| (luminance(fg) < luminance(base)) == light)
        .unwrap_or(if light { [0, 0, 0] } else { [255, 255, 255] });
    Surfaces {
        panel: style::mix(base, toward, 0.07),
        inactive: style::mix(base, toward, 0.04),
        on_accent: Color::Rgb(base[0], base[1], base[2]),
    }
}

/// Parse a color: `#rrggbb`, `#rgb`, a name like `red` or `light blue`, or `default`.
pub fn parse_color(text: &str) -> Option<Color> {
    let text = text.trim().to_lowercase();
    if let Some(hex) = text.strip_prefix('#') {
        let channel = |s: &str| u8::from_str_radix(s, 16).ok();
        return match hex.len() {
            6 => Some(Color::Rgb(
                channel(&hex[0..2])?,
                channel(&hex[2..4])?,
                channel(&hex[4..6])?,
            )),
            3 => {
                let c = |i: usize| channel(&hex[i..=i]).map(|v| v * 17);
                Some(Color::Rgb(c(0)?, c(1)?, c(2)?))
            }
            _ => None,
        };
    }
    Some(match text.as_str() {
        "default" | "terminal default" => Color::Reset,
        "black" => Color::Black,
        "red" => Color::Red,
        "green" => Color::Green,
        "yellow" => Color::Yellow,
        "blue" => Color::Blue,
        "magenta" => Color::Magenta,
        "cyan" => Color::Cyan,
        "white" => Color::Gray,
        "light black" => Color::DarkGray,
        "light red" => Color::LightRed,
        "light green" => Color::LightGreen,
        "light yellow" => Color::LightYellow,
        "light blue" => Color::LightBlue,
        "light magenta" => Color::LightMagenta,
        "light cyan" => Color::LightCyan,
        "light white" => Color::White,
        _ => return None,
    })
}

/// The [Palette] for `theme_cfg` on this terminal (see [terminal_colors::detect]), with defaults
/// for colors it doesn't set (or sets wrongly).
pub fn load(theme_cfg: &Option<ConfigTheme>) -> Palette {
    load_for(theme_cfg, terminal_colors::detected())
}

/// The [Palette] for `theme_cfg` on a terminal with `terminal`'s colors.
fn load_for(theme_cfg: &Option<ConfigTheme>, terminal: TerminalColors) -> Palette {
    macro_rules! color {
        ($member:ident, $default:expr) => {
            match theme_cfg.as_ref().and_then(|t| t.$member.as_deref()) {
                Some(text) => parse_color(text).unwrap_or_else(|| {
                    warn!(
                        "Failed to parse color \"{text}\" in \"{}\", using the default",
                        stringify!($member)
                    );
                    $default
                }),
                None => $default,
            }
        };
    }
    /// The accent: titles, the current tab, the playing track, filled buttons.
    const ACCENT: Color = Color::Blue;

    let background = color!(background, Color::Reset);
    let base = match background {
        Color::Reset => terminal.background,
        Color::Rgb(r, g, b) => Some([r, g, b]),
        // A named background is whatever the terminal makes of it.
        _ => None,
    };
    let surfaces = surfaces(base, terminal.foreground);
    Palette {
        background,
        primary: color!(primary, Color::Reset),
        secondary: color!(secondary, Color::DarkGray),
        title: color!(title, ACCENT),
        playing: color!(playing, ACCENT),
        playing_bg: color!(playing_bg, background),
        highlight: color!(highlight, surfaces.on_accent),
        highlight_bg: color!(highlight_bg, ACCENT),
        highlight_inactive_bg: color!(highlight_inactive_bg, surfaces.inactive),
        error: color!(error, Color::Red),
        error_bg: color!(error_bg, background),
        statusbar_progress: color!(statusbar_progress, ACCENT),
        statusbar_progress_bg: color!(statusbar_progress_bg, Color::DarkGray),
        statusbar_bg: color!(statusbar_bg, surfaces.panel),
        cmdline: color!(cmdline, Color::Reset),
        cmdline_bg: color!(cmdline_bg, background),
        base,
        terminal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_color() {
        assert_eq!(parse_color("#cba6f7"), Some(Color::Rgb(0xcb, 0xa6, 0xf7)));
        assert_eq!(parse_color("#fff"), Some(Color::Rgb(255, 255, 255)));
        assert_eq!(parse_color("Light Blue"), Some(Color::LightBlue));
        assert_eq!(parse_color("nope"), None);
    }

    const DARK: Rgb = [0x1e, 0x1e, 0x2e];
    const LIGHT: Rgb = [0xef, 0xf1, 0xf5];

    fn terminal(foreground: Option<Rgb>, background: Option<Rgb>) -> TerminalColors {
        TerminalColors {
            foreground,
            background,
            ..Default::default()
        }
    }

    #[test]
    fn test_dark_terminal_gets_lighter_surfaces() {
        let palette = load_for(&None, terminal(Some([0xcd, 0xd6, 0xf4]), Some(DARK)));
        let Color::Rgb(r, g, b) = palette.statusbar_bg else {
            panic!("{:?}", palette.statusbar_bg)
        };
        assert!(luminance([r, g, b]) > luminance(DARK));
        // About 7% of the way to the foreground.
        assert_eq!([r, g, b], [0x2a, 0x2b, 0x3c]);
        assert_eq!(
            palette.highlight_inactive_bg,
            style::mix(DARK, [0xcd, 0xd6, 0xf4], 0.04)
        );
        assert_eq!(palette.background, Color::Reset);
        assert_eq!(palette.primary, Color::Reset);
        assert_eq!(palette.base, Some(DARK));
    }

    #[test]
    fn test_light_terminal_gets_darker_surfaces() {
        // Without a foreground, light themes still move towards black.
        for foreground in [Some([0x4c, 0x4f, 0x69]), None] {
            let palette = load_for(&None, terminal(foreground, Some(LIGHT)));
            let Color::Rgb(r, g, b) = palette.statusbar_bg else {
                panic!("{:?}", palette.statusbar_bg)
            };
            assert!(luminance([r, g, b]) < luminance(LIGHT));
            assert_eq!(palette.highlight, Color::Rgb(LIGHT[0], LIGHT[1], LIGHT[2]));
            assert_eq!(palette.rgb(Color::Reset), foreground.unwrap_or([0, 0, 0]));
        }
    }

    #[test]
    fn test_no_answer_falls_back_to_ansi() {
        let palette = load_for(&None, TerminalColors::default());
        assert_eq!(palette.statusbar_bg, Color::Reset);
        assert_eq!(palette.highlight_inactive_bg, Color::DarkGray);
        assert_eq!(palette.base, None);
        assert_eq!(palette.rgb(Color::Blue), style::rgb(Color::Blue));
    }

    #[test]
    fn test_theme_overrides_and_ansi_colors() {
        let theme = ConfigTheme {
            background: Some("#ffffff".into()),
            statusbar_bg: Some("red".into()),
            ..Default::default()
        };
        let mut colors = terminal(None, Some(DARK));
        colors.ansi[4] = Some([1, 2, 3]);
        let palette = load_for(&Some(theme), colors);
        // A configured background wins over the terminal's.
        assert_eq!(palette.base, Some([255, 255, 255]));
        assert_eq!(palette.statusbar_bg, Color::Red);
        assert_eq!(palette.rgb(palette.title), [1, 2, 3]);
    }
}
