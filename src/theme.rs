//! Colors from the configuration's theme (see [ConfigTheme]), as a [Palette] for the TUI.

use log::warn;
use ratatui::style::Color;

use crate::config::ConfigTheme;

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

/// The [Palette] for `theme_cfg`, with defaults for colors it doesn't set (or sets wrongly).
pub fn load(theme_cfg: &Option<ConfigTheme>) -> Palette {
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
    Palette {
        background: color!(background, Color::Reset),
        primary: color!(primary, Color::Reset),
        secondary: color!(secondary, Color::Blue),
        title: color!(title, Color::Red),
        playing: color!(playing, Color::Blue),
        playing_bg: color!(playing_bg, Color::Reset),
        highlight: color!(highlight, Color::Gray),
        highlight_bg: color!(highlight_bg, Color::Red),
        highlight_inactive_bg: color!(highlight_inactive_bg, Color::Blue),
        error: color!(error, Color::Reset),
        error_bg: color!(error_bg, Color::Red),
        statusbar_progress: color!(statusbar_progress, Color::Blue),
        statusbar_progress_bg: color!(statusbar_progress_bg, Color::DarkGray),
        statusbar_bg: color!(statusbar_bg, Color::Reset),
        cmdline: color!(cmdline, Color::Reset),
        cmdline_bg: color!(cmdline_bg, Color::Reset),
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
}
