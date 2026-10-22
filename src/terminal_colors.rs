//! The terminal's own colors, asked for once at startup so the theme can match them: the
//! foreground (OSC 10), the background (OSC 11) and the 16 ANSI colors (OSC 4).

use std::sync::OnceLock;
#[cfg(unix)]
use std::time::Duration;

/// An RGB color.
pub type Rgb = [u8; 3];

/// The colors the terminal reported; `None` for those it didn't.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TerminalColors {
    pub foreground: Option<Rgb>,
    pub background: Option<Rgb>,
    /// ANSI colors 0-15 (black, red, … white, then the bright variants).
    pub ansi: [Option<Rgb>; 16],
}

/// How long to wait for the terminal's answers. Terminals answer in a few milliseconds; ones that
/// don't support the queries (or tmux without passthrough) never do.
#[cfg(unix)]
const TIMEOUT: Duration = Duration::from_millis(100);

static DETECTED: OnceLock<TerminalColors> = OnceLock::new();

/// Ask the terminal for its colors. Call once at startup, before the UI takes over the terminal
/// (raw mode, the alternate screen and the input thread), which would swallow the answers.
pub fn detect() {
    DETECTED.get_or_init(|| {
        let colors = query()
            .map(|reply| parse_replies(&reply))
            .unwrap_or_default();
        log::info!("Terminal colors: {colors:?}");
        colors
    });
}

/// The colors found by [detect]; none if it wasn't called or the terminal didn't answer.
pub fn detected() -> TerminalColors {
    DETECTED.get().copied().unwrap_or_default()
}

/// Send the color queries, followed by a primary device attributes request (DA1). Every terminal
/// answers DA1, and answers in order, so its reply means all color replies have arrived.
#[cfg(unix)]
fn query() -> Option<Vec<u8>> {
    use std::fs::OpenOptions;
    use std::io::{IsTerminal, Read, Write};
    use std::os::fd::AsRawFd;
    use std::time::Instant;

    use ratatui::crossterm::terminal::{disable_raw_mode, enable_raw_mode};

    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        return None;
    }
    let mut tty = OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .ok()?;

    let mut request = String::from("\x1b]10;?\x1b\\\x1b]11;?\x1b\\");
    for i in 0..16 {
        request.push_str(&format!("\x1b]4;{i};?\x1b\\"));
    }
    request.push_str("\x1b[c");

    // Raw mode, so the answers aren't echoed or held back until a newline.
    enable_raw_mode().ok()?;
    let mut reply = Vec::new();
    if tty
        .write_all(request.as_bytes())
        .and_then(|()| tty.flush())
        .is_ok()
    {
        let deadline = Instant::now() + TIMEOUT;
        let mut chunk = [0u8; 1024];
        while !has_device_attributes(&reply) {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            let mut fd = libc::pollfd {
                fd: tty.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            // SAFETY: `fd` is a valid pollfd for an open file for the duration of the call.
            let ready = unsafe { libc::poll(&mut fd, 1, left.as_millis() as libc::c_int) };
            if ready <= 0 {
                break;
            }
            match tty.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => reply.extend_from_slice(&chunk[..n]),
            }
        }
    }
    let _ = disable_raw_mode();
    Some(reply)
}

#[cfg(not(unix))]
fn query() -> Option<Vec<u8>> {
    None
}

/// Whether `reply` contains a DA1 answer (`ESC [ ? … c`).
#[cfg(any(unix, test))]
fn has_device_attributes(reply: &[u8]) -> bool {
    reply
        .windows(3)
        .position(|w| w == b"\x1b[?")
        .is_some_and(|start| reply[start..].contains(&b'c'))
}

/// Collect the colors from the terminal's OSC replies, each ended by BEL or ST (`ESC \`).
pub fn parse_replies(reply: &[u8]) -> TerminalColors {
    let text = String::from_utf8_lossy(reply);
    let mut colors = TerminalColors::default();
    for sequence in text.split("\x1b]").skip(1) {
        let body = sequence.split(['\x07', '\x1b']).next().unwrap_or_default();
        let fields: Vec<&str> = body.split(';').collect();
        match fields.as_slice() {
            ["10", spec] => colors.foreground = parse_color_spec(spec),
            ["11", spec] => colors.background = parse_color_spec(spec),
            ["4", index, spec] => {
                if let Some(slot) = index
                    .parse::<usize>()
                    .ok()
                    .and_then(|i| colors.ansi.get_mut(i))
                {
                    *slot = parse_color_spec(spec);
                }
            }
            _ => {}
        }
    }
    colors
}

/// Parse an X11 color spec as terminals report them: `rgb:rrrr/gggg/bbbb`, with 1 to 4 hex
/// digits per channel (`rgb:rr/gg/bb` is common too). `rgba:` replies drop the alpha.
pub fn parse_color_spec(spec: &str) -> Option<Rgb> {
    let channels = spec
        .strip_prefix("rgb:")
        .or_else(|| spec.strip_prefix("rgba:"))?;
    let mut channels = channels.split('/').map(|hex| {
        if hex.is_empty() || hex.len() > 4 {
            return None;
        }
        let value = u32::from_str_radix(hex, 16).ok()?;
        let max = (1u32 << (4 * hex.len())) - 1;
        Some(((value * 255 + max / 2) / max) as u8)
    });
    Some([channels.next()??, channels.next()??, channels.next()??])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_color_spec() {
        assert_eq!(
            parse_color_spec("rgb:1e1e/1e1e/2e2e"),
            Some([0x1e, 0x1e, 0x2e])
        );
        assert_eq!(parse_color_spec("rgb:ff/80/00"), Some([255, 128, 0]));
        assert_eq!(parse_color_spec("rgb:f/0/8"), Some([255, 0, 136]));
        assert_eq!(
            parse_color_spec("rgba:ffff/0000/0000/ffff"),
            Some([255, 0, 0])
        );
        assert_eq!(parse_color_spec("rgb:ff/80"), None);
        assert_eq!(parse_color_spec("rgb:ff/zz/00"), None);
        assert_eq!(parse_color_spec("#ffffff"), None);
    }

    #[test]
    fn test_parse_replies() {
        let reply = b"\x1b]10;rgb:cdcd/d6d6/f4f4\x1b\\\x1b]11;rgb:1e/1e/2e\x07\
                      \x1b]4;4;rgb:8989/b4b4/fafa\x1b\\\x1b]4;99;rgb:0/0/0\x1b\\\x1b[?62;22c";
        let colors = parse_replies(reply);
        assert_eq!(colors.foreground, Some([0xcd, 0xd6, 0xf4]));
        assert_eq!(colors.background, Some([0x1e, 0x1e, 0x2e]));
        assert_eq!(colors.ansi[4], Some([0x89, 0xb4, 0xfa]));
        assert_eq!(colors.ansi[0], None);
        assert!(has_device_attributes(reply));

        assert_eq!(parse_replies(b"\x1b[?1;2c"), TerminalColors::default());
        assert!(!has_device_attributes(b"\x1b]11;rgb:0/0/0\x07"));
    }
}
