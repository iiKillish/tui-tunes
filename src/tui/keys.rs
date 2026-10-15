//! Keybindings: parsing the configuration's key names ("Shift+p", "Ctrl+s", "F1", "Space"),
//! matching terminal key presses against them, and the default bindings.

use std::collections::HashMap;

use log::{error, info};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::command::{
    Command, GotoMode, JumpMode, MoveAmount, MoveMode, PanelAction, SeekDirection, ShiftMode,
    TargetMode, parse,
};
use crate::config::Config;

/// Upper bound for a count prefix, bounding the repeat loop.
pub const MAX_COUNT: usize = 9999;

/// A key with its modifiers, normalized so a configured name and a key press compare equal:
/// shifted characters are the uppercase character without the Shift modifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Key {
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
}

impl Key {
    fn new(code: KeyCode, modifiers: KeyModifiers) -> Self {
        let mut modifiers =
            modifiers & (KeyModifiers::SHIFT | KeyModifiers::CONTROL | KeyModifiers::ALT);
        let code = match code {
            KeyCode::Char(c) if modifiers.contains(KeyModifiers::SHIFT) => {
                modifiers.remove(KeyModifiers::SHIFT);
                KeyCode::Char(c.to_uppercase().next().unwrap_or(c))
            }
            // Shift+Tab arrives as BackTab.
            KeyCode::BackTab => {
                modifiers.insert(KeyModifiers::SHIFT);
                KeyCode::Tab
            }
            code => code,
        };
        Self { code, modifiers }
    }

    /// The key pressed in `event`, or `None` for releases and repeats of other kinds.
    pub fn from_event(event: &KeyEvent) -> Option<Self> {
        (event.kind != KeyEventKind::Release).then(|| Self::new(event.code, event.modifiers))
    }

    /// Parse a configured key name such as `q`, `Shift+p`, `Ctrl+Left` or `F1`.
    pub fn parse(name: &str) -> Option<Self> {
        let (modifier, key) = match name.split_once('+') {
            Some((m, k)) if name != "+" && !k.is_empty() => (Some(m), k),
            _ => (None, name),
        };
        let modifiers = match modifier {
            None => KeyModifiers::NONE,
            Some("Shift") => KeyModifiers::SHIFT,
            Some("Ctrl") => KeyModifiers::CONTROL,
            Some("Alt") => KeyModifiers::ALT,
            Some(_) => return None,
        };
        let code = match key {
            "Enter" => KeyCode::Enter,
            "Space" => KeyCode::Char(' '),
            "Tab" => KeyCode::Tab,
            "Backspace" => KeyCode::Backspace,
            "Esc" => KeyCode::Esc,
            "Left" => KeyCode::Left,
            "Right" => KeyCode::Right,
            "Up" => KeyCode::Up,
            "Down" => KeyCode::Down,
            "Ins" => KeyCode::Insert,
            "Del" => KeyCode::Delete,
            "Home" => KeyCode::Home,
            "End" => KeyCode::End,
            "PageUp" => KeyCode::PageUp,
            "PageDown" => KeyCode::PageDown,
            f if f.len() > 1 && f.starts_with('F') => KeyCode::F(f[1..].parse().ok()?),
            s => {
                let mut chars = s.chars();
                let c = chars.next()?;
                if chars.next().is_some() {
                    return None;
                }
                KeyCode::Char(c)
            }
        };
        Some(Self::new(code, modifiers))
    }
}

/// Fold a digit into a count prefix, clamped to `max`. A leading `0` starts no count (vim rule).
pub fn accumulate_count(current: usize, digit: u32, max: usize) -> usize {
    if digit == 0 && current == 0 {
        return 0;
    }
    current
        .saturating_mul(10)
        .saturating_add(digit as usize)
        .min(max)
}

/// Keybindings by their configured names, sorted, for the help screen.
pub type BindingList = Vec<(String, Vec<Command>)>;

/// The keybindings: the defaults (unless `default_keybindings = false`), overridden by the
/// configuration's `[keybindings]`. Also returns the names as configured, for the help screen.
pub fn bindings(config: &Config) -> (HashMap<Key, Vec<Command>>, BindingList) {
    let values = config.values();
    let mut named = if values.default_keybindings.unwrap_or(true) {
        default_keybindings()
    } else {
        HashMap::new()
    };
    for (key, commands) in values.keybindings.clone().unwrap_or_default() {
        match parse(&commands) {
            Ok(cmds) => {
                info!("Custom keybinding: {key} -> {cmds:?}");
                named.insert(key, cmds);
            }
            Err(err) => error!("Invalid command(s) for key {key}-\"{commands}\": {err}"),
        }
    }
    let mut map = HashMap::new();
    for (name, commands) in &named {
        match Key::parse(name) {
            Some(key) => {
                map.insert(key, commands.clone());
            }
            None => error!("Could not parse keybinding: \"{name}\""),
        }
    }
    let mut list: Vec<(String, Vec<Command>)> = named.into_iter().collect();
    list.sort_by(|a, b| a.0.cmp(&b.0));
    (map, list)
}

#[allow(clippy::too_many_lines)]
fn default_keybindings() -> HashMap<String, Vec<Command>> {
    let mut kb = HashMap::new();

    kb.insert("q".into(), vec![Command::Quit]);
    kb.insert("Ctrl+l".into(), vec![Command::Redraw]);
    kb.insert("Shift+h".into(), vec![Command::Like]);
    kb.insert("Tab".into(), vec![Command::Screen(true)]);
    kb.insert("Shift+Tab".into(), vec![Command::Screen(false)]);
    kb.insert("Shift+l".into(), vec![Command::Panel(PanelAction::Lyrics)]);
    kb.insert("Alt+h".into(), vec![Command::Tab(false)]);
    kb.insert("Alt+l".into(), vec![Command::Tab(true)]);
    kb.insert("Shift+Left".into(), vec![Command::Tab(false)]);
    kb.insert("Shift+Right".into(), vec![Command::Tab(true)]);
    kb.insert("Shift+p".into(), vec![Command::TogglePlay]);
    kb.insert("Shift+u".into(), vec![Command::UpdateLibrary]);
    kb.insert("Shift+s".into(), vec![Command::Stop]);
    kb.insert("<".into(), vec![Command::Previous]);
    kb.insert(">".into(), vec![Command::Next]);
    kb.insert("c".into(), vec![Command::Clear]);
    kb.insert(
        "Space".into(),
        vec![
            Command::Queue,
            Command::Move(MoveMode::Down, Default::default()),
        ],
    );
    kb.insert(
        ".".into(),
        vec![
            Command::PlayNext,
            Command::Move(MoveMode::Down, Default::default()),
        ],
    );
    kb.insert("Enter".into(), vec![Command::Play]);
    kb.insert("n".into(), vec![Command::Jump(JumpMode::Next)]);
    kb.insert("Shift+n".into(), vec![Command::Jump(JumpMode::Previous)]);
    kb.insert("s".into(), vec![Command::Save]);
    kb.insert("Ctrl+s".into(), vec![Command::SaveQueue]);
    kb.insert("d".into(), vec![Command::Delete]);
    kb.insert(
        "f".into(),
        vec![Command::Seek(SeekDirection::Relative(1000))],
    );
    kb.insert(
        "b".into(),
        vec![Command::Seek(SeekDirection::Relative(-1000))],
    );
    kb.insert(
        "Shift+f".into(),
        vec![Command::Seek(SeekDirection::Relative(10000))],
    );
    kb.insert(
        "Shift+b".into(),
        vec![Command::Seek(SeekDirection::Relative(-10000))],
    );
    kb.insert("+".into(), vec![Command::VolumeUp(1)]);
    kb.insert("]".into(), vec![Command::VolumeUp(5)]);
    kb.insert("-".into(), vec![Command::VolumeDown(1)]);
    kb.insert("[".into(), vec![Command::VolumeDown(5)]);

    kb.insert("r".into(), vec![Command::Repeat(None)]);
    kb.insert("z".into(), vec![Command::Shuffle(None)]);

    #[cfg(feature = "share_clipboard")]
    {
        kb.insert("x".into(), vec![Command::Share(TargetMode::Selected)]);
        kb.insert("Shift+x".into(), vec![Command::Share(TargetMode::Current)]);
    }

    // F1-F5 in the order of the tabs.
    kb.insert("F1".into(), vec![Command::Focus("home".into())]);
    kb.insert("F2".into(), vec![Command::Focus("search".into())]);
    kb.insert("F3".into(), vec![Command::Focus("library".into())]);
    kb.insert("F4".into(), vec![Command::Focus("queue".into())]);
    kb.insert("F5".into(), vec![Command::Focus("lyrics".into())]);
    kb.insert("?".into(), vec![Command::Help]);
    kb.insert("Esc".into(), vec![Command::Back]);
    kb.insert("Backspace".into(), vec![Command::Back]);

    kb.insert("o".into(), vec![Command::Open(TargetMode::Selected)]);
    kb.insert("Shift+o".into(), vec![Command::Open(TargetMode::Current)]);
    kb.insert("a".into(), vec![Command::Goto(GotoMode::Album)]);
    kb.insert("Shift+a".into(), vec![Command::Goto(GotoMode::Artist)]);

    kb.insert(
        "m".into(),
        vec![Command::ShowRecommendations(TargetMode::Selected)],
    );
    kb.insert(
        "Shift+m".into(),
        vec![Command::ShowRecommendations(TargetMode::Current)],
    );

    kb.insert(
        "Up".into(),
        vec![Command::Move(MoveMode::Up, Default::default())],
    );
    kb.insert(
        "p".into(),
        vec![Command::Move(MoveMode::Playing, Default::default())],
    );
    kb.insert(
        "Down".into(),
        vec![Command::Move(MoveMode::Down, Default::default())],
    );
    kb.insert(
        "Left".into(),
        vec![Command::Move(MoveMode::Left, Default::default())],
    );
    kb.insert(
        "Right".into(),
        vec![Command::Move(MoveMode::Right, Default::default())],
    );
    kb.insert(
        "PageUp".into(),
        vec![Command::Move(MoveMode::Up, MoveAmount::Integer(5))],
    );
    kb.insert(
        "PageDown".into(),
        vec![Command::Move(MoveMode::Down, MoveAmount::Integer(5))],
    );
    kb.insert(
        "Home".into(),
        vec![Command::Move(MoveMode::Up, MoveAmount::Extreme)],
    );
    kb.insert(
        "End".into(),
        vec![Command::Move(MoveMode::Down, MoveAmount::Extreme)],
    );
    kb.insert(
        "k".into(),
        vec![Command::Move(MoveMode::Up, Default::default())],
    );
    kb.insert(
        "j".into(),
        vec![Command::Move(MoveMode::Down, Default::default())],
    );
    kb.insert(
        "h".into(),
        vec![Command::Move(MoveMode::Left, Default::default())],
    );
    kb.insert(
        "l".into(),
        vec![Command::Move(MoveMode::Right, Default::default())],
    );

    kb.insert(
        "Ctrl+p".into(),
        vec![Command::Move(MoveMode::Up, Default::default())],
    );
    kb.insert(
        "Ctrl+n".into(),
        vec![Command::Move(MoveMode::Down, Default::default())],
    );
    kb.insert(
        "Ctrl+a".into(),
        vec![Command::Move(MoveMode::Left, Default::default())],
    );
    kb.insert(
        "Ctrl+e".into(),
        vec![Command::Move(MoveMode::Right, Default::default())],
    );

    kb.insert("Shift+Up".into(), vec![Command::Shift(ShiftMode::Up, None)]);
    kb.insert(
        "Shift+Down".into(),
        vec![Command::Shift(ShiftMode::Down, None)],
    );

    kb.insert(
        "g".into(),
        vec![Command::Move(MoveMode::Up, MoveAmount::Extreme)],
    );
    kb.insert(
        "Shift+g".into(),
        vec![Command::Move(MoveMode::Down, MoveAmount::Extreme)],
    );

    #[cfg(feature = "share_clipboard")]
    kb.insert(
        "Ctrl+v".into(),
        vec![Command::Insert(crate::command::InsertSource::Clipboard)],
    );

    kb
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_keys() {
        assert_eq!(
            Key::parse("q"),
            Some(Key::new(KeyCode::Char('q'), KeyModifiers::NONE))
        );
        assert_eq!(
            Key::parse("Shift+p"),
            Some(Key::new(KeyCode::Char('P'), KeyModifiers::NONE))
        );
        assert_eq!(
            Key::parse("Ctrl+s"),
            Some(Key::new(KeyCode::Char('s'), KeyModifiers::CONTROL))
        );
        assert_eq!(
            Key::parse("F12"),
            Some(Key::new(KeyCode::F(12), KeyModifiers::NONE))
        );
        assert_eq!(
            Key::parse("+"),
            Some(Key::new(KeyCode::Char('+'), KeyModifiers::NONE))
        );
        assert_eq!(
            Key::parse("Space"),
            Some(Key::new(KeyCode::Char(' '), KeyModifiers::NONE))
        );
        assert_eq!(Key::parse("Hyper+x"), None);
    }

    #[test]
    fn test_key_presses_match_names() {
        let shift_p = KeyEvent::new(KeyCode::Char('P'), KeyModifiers::SHIFT);
        assert_eq!(Key::from_event(&shift_p), Key::parse("Shift+p"));
        let back_tab = KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT);
        assert_eq!(Key::from_event(&back_tab), Key::parse("Shift+Tab"));
    }

    #[test]
    fn test_count_prefix() {
        assert_eq!(accumulate_count(0, 1, MAX_COUNT), 1);
        assert_eq!(accumulate_count(1, 2, MAX_COUNT), 12);
        assert_eq!(accumulate_count(0, 0, MAX_COUNT), 0);
        assert_eq!(accumulate_count(9999, 9, MAX_COUNT), MAX_COUNT);
    }
}
