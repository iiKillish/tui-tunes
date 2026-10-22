//! What keys do, for people: key names as shown ("Shift+P", "Space"), what a binding does in
//! words, and key hints on screen.
//!
//! Key hints label what's on screen with the key that does it, like Vimium's link hints, to help
//! learn the keys. They show while Ctrl+Shift is held (in terminals that report it) or after the
//! `hints` command (`;`). Widgets say where their things are while drawing ([add], [add_row]),
//! like they register clickable areas with [crate::tui::pointer]; [draw] labels them after the
//! frame.

use std::cell::RefCell;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

use crate::command::{
    Command, GotoMode, JumpMode, MoveAmount, MoveMode, PanelAction, SeekDirection, ShiftMode,
    SleepMode, TargetMode,
};
use crate::theme::Palette;
use crate::tui::style::{fill, put, width};

/// A key name from the configuration ("Shift+p", "Alt+l", "Up") as shown: "Shift+P", "Alt+L",
/// "↑". On a Mac, Alt is the Option key, so it's shown as ⌥.
pub fn key_label(name: &str) -> String {
    let (modifier, key) = match name.split_once('+') {
        Some((m, k)) if name != "+" && !k.is_empty() => (Some(m), k),
        _ => (None, name),
    };
    let key = match key {
        "Up" => "↑".to_string(),
        "Down" => "↓".to_string(),
        "Left" => "←".to_string(),
        "Right" => "→".to_string(),
        "Backspace" => "⌫".to_string(),
        k => k.to_string(),
    };
    match modifier {
        // Shift+p is P, as typed.
        Some("Shift") if key.chars().count() == 1 => format!("Shift+{}", key.to_uppercase()),
        Some("Alt") if cfg!(target_os = "macos") => format!("⌥{key}"),
        Some(m) => format!("{m}+{key}"),
        None => key,
    }
}

/// Where a binding goes in the key list.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Group {
    Playback,
    Navigation,
    Library,
    Other,
}

impl Group {
    pub const ALL: [Self; 4] = [Self::Playback, Self::Navigation, Self::Library, Self::Other];

    /// The name on the key list's tab.
    pub fn short_title(self) -> &'static str {
        match self {
            Self::Library => "Library",
            group => group.title(),
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Playback => "Playback",
            Self::Navigation => "Getting around",
            Self::Library => "Queue, library and playlists",
            Self::Other => "Other",
        }
    }
}

/// The command of a binding that says what it does: bindings like Space in some setups (queue,
/// then move down) do something, then move on.
fn main_command(commands: &[Command]) -> Option<&Command> {
    commands
        .iter()
        .find(|c| !matches!(c, Command::Move(..)))
        .or(commands.first())
}

/// Which [Group] the binding running `commands` goes in.
pub fn group(commands: &[Command]) -> Group {
    match main_command(commands) {
        Some(
            Command::TogglePlay
            | Command::Stop
            | Command::Previous
            | Command::Next
            | Command::Seek(_)
            | Command::VolumeUp(_)
            | Command::VolumeDown(_)
            | Command::Repeat(_)
            | Command::Shuffle(_)
            | Command::Like
            | Command::Sleep(_),
        ) => Group::Playback,
        Some(
            Command::Move(..)
            | Command::Focus(_)
            | Command::Screen(_)
            | Command::Tab(_)
            | Command::Back
            | Command::Open(_)
            | Command::Goto(_)
            | Command::Jump(_)
            | Command::Search(_)
            | Command::Panel(_)
            | Command::Help
            | Command::Hints
            | Command::Settings,
        ) => Group::Navigation,
        Some(
            Command::Queue
            | Command::PlayNext
            | Command::Play
            | Command::Clear
            | Command::Save
            | Command::SaveCurrent
            | Command::SaveQueue
            | Command::Add
            | Command::AddCurrent
            | Command::Delete
            | Command::Shift(..)
            | Command::NewPlaylist(_)
            | Command::Rename(_)
            | Command::Sort(..)
            | Command::Insert(_)
            | Command::UpdateLibrary,
        ) => Group::Library,
        #[cfg(feature = "share_clipboard")]
        Some(Command::Share(_)) => Group::Library,
        _ => Group::Other,
    }
}

/// What the binding running `commands` does, in a few words.
pub fn describe(commands: &[Command]) -> String {
    let Some(command) = main_command(commands) else {
        return "Nothing".into();
    };
    let seconds = |ms: i32| {
        let s = ms.unsigned_abs() / 1000;
        if s.is_multiple_of(60) && s >= 60 {
            format!("{} min", s / 60)
        } else {
            format!("{s} s")
        }
    };
    match command {
        Command::Quit => "Quit".into(),
        Command::TogglePlay => "Play / pause".into(),
        Command::Stop => "Stop".into(),
        Command::Previous => "Previous song".into(),
        Command::Next => "Next song".into(),
        Command::Clear => "Clear the queue".into(),
        Command::Queue => "Add to the queue".into(),
        Command::PlayNext => "Play next".into(),
        Command::Play => "Play / open".into(),
        Command::UpdateLibrary => "Refresh the library".into(),
        Command::Save => "Save or like the selected item".into(),
        Command::SaveCurrent => "Like the playing song".into(),
        Command::SaveQueue => "Save the queue as a playlist".into(),
        Command::Add => "Add the selected song to a playlist".into(),
        Command::AddCurrent => "Add the playing song to a playlist".into(),
        Command::Delete => "Remove the selected item".into(),
        Command::Focus(screen) => format!("Go to {}", screen.label()),
        Command::Seek(SeekDirection::Relative(ms)) if *ms >= 0 => {
            format!("Forward {}", seconds(*ms))
        }
        Command::Seek(SeekDirection::Relative(ms)) => format!("Back {}", seconds(*ms)),
        Command::Seek(SeekDirection::Absolute(ms)) => {
            format!("Go to {}", crate::utils::ms_to_hms(*ms))
        }
        Command::VolumeUp(n) => format!("Volume up {n}%"),
        Command::VolumeDown(n) => format!("Volume down {n}%"),
        Command::Repeat(_) => "Repeat: off, all, one".into(),
        Command::Shuffle(_) => "Shuffle".into(),
        #[cfg(feature = "share_clipboard")]
        Command::Share(TargetMode::Selected) => "Copy a link to the selected item".into(),
        #[cfg(feature = "share_clipboard")]
        Command::Share(TargetMode::Current) => "Copy a link to the playing song".into(),
        Command::Back => "Back".into(),
        Command::Open(TargetMode::Selected) => "Open, or the menu of a song".into(),
        Command::Open(TargetMode::Current) => "Menu of the playing song".into(),
        Command::Goto(GotoMode::Album) => "Go to the album".into(),
        Command::Goto(GotoMode::Artist) => "Go to the artist".into(),
        Command::Move(mode, amount) => {
            let extreme = matches!(amount, MoveAmount::Extreme);
            let page = matches!(amount, MoveAmount::Float(_));
            let by = match amount {
                MoveAmount::Integer(n) if n.unsigned_abs() > 1 => format!(" {}", n.unsigned_abs()),
                _ => String::new(),
            };
            let direction: String = match (mode, extreme, page) {
                (MoveMode::Playing, ..) => "Select the playing song".into(),
                (MoveMode::Up, true, _) => "Top".into(),
                (MoveMode::Down, true, _) => "Bottom".into(),
                (MoveMode::Up, _, true) => "Page up".into(),
                (MoveMode::Down, _, true) => "Page down".into(),
                (MoveMode::Up, ..) => "Up".into(),
                (MoveMode::Down, ..) => "Down".into(),
                (MoveMode::Left, ..) => "Left".into(),
                (MoveMode::Right, ..) => "Right".into(),
            };
            match mode {
                MoveMode::Playing => direction,
                _ if extreme || page => direction,
                _ => format!("{direction}{by}"),
            }
        }
        Command::Shift(ShiftMode::Up, _) => "Move the song up".into(),
        Command::Shift(ShiftMode::Down, _) => "Move the song down".into(),
        Command::Search(_) => "Search".into(),
        Command::Jump(JumpMode::Next) => "Next match".into(),
        Command::Jump(JumpMode::Previous) => "Previous match".into(),
        Command::Jump(JumpMode::Query(_)) => "Find in the list".into(),
        Command::Help => "All keys".into(),
        Command::Hints => "Show keys on screen".into(),
        Command::Settings => "Settings".into(),
        Command::ReloadConfig => "Reload the configuration".into(),
        Command::Noop => "Nothing".into(),
        Command::Insert(_) => "Open a Spotify link".into(),
        Command::NewPlaylist(_) => "New playlist".into(),
        Command::Rename(_) => "Rename the playlist".into(),
        Command::Sort(key, _) => format!("Sort by {key}"),
        Command::Logout => "Log out".into(),
        Command::Redraw => "Redraw the screen".into(),
        Command::Execute(_) => "Run a shell command".into(),
        Command::Reconnect => "Reconnect to Spotify".into(),
        Command::Like => "Like the playing song".into(),
        Command::Panel(PanelAction::Queue) => "Show Up next".into(),
        Command::Panel(PanelAction::Lyrics) => "Show the lyrics".into(),
        Command::Tab(true) => "Next tab".into(),
        Command::Tab(false) => "Previous tab".into(),
        Command::Screen(true) => "Next screen".into(),
        Command::Screen(false) => "Previous screen".into(),
        Command::Sleep(SleepMode::Off) => "Sleep timer off".into(),
        Command::Sleep(SleepMode::After(s)) => format!("Pause in {} min", s / 60),
        Command::Sleep(_) => "Pause when this song ends".into(),
    }
}

/// What a binding does, and every key that does it.
pub struct Action {
    pub group: Group,
    /// Key names as in the configuration ("Shift+p").
    pub keys: Vec<String>,
    pub label: String,
    pub commands: Vec<Command>,
}

impl Action {
    /// The keys as shown, side by side.
    pub fn keys_label(&self) -> String {
        let mut labels: Vec<String> = self.keys.iter().map(|k| key_label(k)).collect();
        labels.sort_by_key(|k| (width(k), k.clone()));
        labels.join("  ")
    }

    /// The commands as the configuration writes them.
    pub fn commands_text(&self) -> String {
        self.commands
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("; ")
    }
}

/// The bindings (key name, commands) as actions: keys doing the same on one row, in groups,
/// leaving out keys that do nothing.
pub fn actions(bindings: &[(String, Vec<Command>)]) -> Vec<Action> {
    let mut actions: Vec<Action> = Vec::new();
    for (key, commands) in bindings {
        if matches!(commands.as_slice(), [Command::Noop]) {
            continue;
        }
        let text = commands
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("; ");
        match actions.iter_mut().find(|a| a.commands_text() == text) {
            Some(action) => action.keys.push(key.clone()),
            None => actions.push(Action {
                group: group(commands),
                keys: vec![key.clone()],
                label: describe(commands),
                commands: commands.clone(),
            }),
        }
    }
    actions.sort_by_cached_key(|a| (a.group, sort_key(a)));
    actions
}

/// Where an action goes in its group: the most used first, related ones together (up, down,
/// left, right; the screens in their order, F1, F2, …), then by what it does.
fn sort_key(action: &Action) -> (u8, String) {
    let Some(command) = main_command(&action.commands) else {
        return (u8::MAX, action.label.clone());
    };
    let rank = match command {
        Command::TogglePlay | Command::Play => 0,
        Command::Next | Command::Queue => 1,
        Command::Previous | Command::PlayNext => 2,
        Command::Stop => 3,
        Command::Seek(_) => 4,
        Command::VolumeUp(_) | Command::VolumeDown(_) => 5,
        Command::Move(MoveMode::Up | MoveMode::Down, MoveAmount::Integer(_)) => 0,
        Command::Move(MoveMode::Left | MoveMode::Right, _) => 1,
        Command::Move(..) => 2,
        Command::Back | Command::Open(_) => 3,
        Command::Focus(_) => 4,
        Command::Screen(_) | Command::Tab(_) | Command::Panel(_) => 5,
        Command::Goto(_) => 6,
        Command::Search(_) | Command::Jump(_) => 7,
        Command::Help | Command::Hints | Command::Settings => 8,
        _ => 10,
    };
    let order = match command {
        Command::Focus(screen) => format!("{:02}", *screen as usize),
        Command::Move(mode, _) => format!("{} {}", mode.clone() as u8, action.label),
        _ => action.label.clone(),
    };
    (rank, order)
}

/// The keys bound to `command` (alone, or first followed by moves), shortest first.
pub fn keys_for(bindings: &[(String, Vec<Command>)], command: &Command) -> Vec<String> {
    let wanted = command.to_string();
    let mut keys: Vec<String> = bindings
        .iter()
        .filter(|(_, commands)| main_command(commands).is_some_and(|c| c.to_string() == wanted))
        .map(|(key, _)| key_label(key))
        .collect();
    keys.sort_by_key(|k| (width(k), k.clone()));
    keys
}

enum Hint {
    /// Label `area` with the key of a command, at its top left.
    At(Rect, Command),
    /// The same, centered on the row under `area`, so the label doesn't hide it (tabs, buttons).
    Below(Rect, Command),
    /// Label a row with the keys of some commands, at its right end.
    Row(Rect, Vec<Command>),
}

thread_local! {
    /// This frame's hints. Frames are drawn on one thread (tests draw on several at once).
    static HINTS: RefCell<Vec<Hint>> = const { RefCell::new(Vec::new()) };
}

/// Forget the last frame's hints; called as a frame starts.
pub fn clear() {
    HINTS.with_borrow_mut(Vec::clear);
}

/// `command` is what clicking `area` does: label it with its key.
pub fn add(area: Rect, command: Command) {
    HINTS.with_borrow_mut(|hints| hints.push(Hint::At(area, command)));
}

/// Like [add], but the label goes under `area`, centered: for tabs and buttons whose names or
/// icons it would hide, with a blank row under them.
pub fn add_below(area: Rect, command: Command) {
    HINTS.with_borrow_mut(|hints| hints.push(Hint::Below(area, command)));
}

/// `commands` act on what's in `row` (the selected row of a list): label them at its end.
pub fn add_row(row: Rect, commands: Vec<Command>) {
    HINTS.with_borrow_mut(|hints| hints.push(Hint::Row(row, commands)));
}

/// Label this frame's hints with their keys in `bindings`, and say how to see the rest on the
/// bottom row of `screen` (and how to hide them: by letting go of Ctrl+Shift if `held`, else with
/// any key). Labels go beside images (under buttons, on a card's text), never on them, so images
/// stay.
pub fn draw(
    buf: &mut Buffer,
    screen: Rect,
    bindings: &[(String, Vec<Command>)],
    palette: &Palette,
    held: bool,
) {
    // Black on yellow, like Vimium's, to stand out from everything else.
    let badge = Style::new()
        .fg(Color::Black)
        .bg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let mut drawn: Vec<Rect> = Vec::new();
    // Labels that would overlap one already drawn move right, as far as the screen goes.
    let mut label = |buf: &mut Buffer, x: u16, y: u16, text: &str| -> u16 {
        let text = format!(" {text} ");
        let w = width(&text) as u16;
        let mut rect = Rect::new(x, y, w, 1).intersection(screen);
        while let Some(taken) = drawn.iter().find(|d| d.intersects(rect)) {
            rect = Rect::new(taken.right() + 1, y, w, 1).intersection(screen);
        }
        if rect.width < w {
            return 0;
        }
        put(buf, rect.x, rect.y, &text, usize::from(rect.width), badge);
        drawn.push(rect);
        w
    };
    let hints = HINTS.take();
    for hint in &hints {
        match hint {
            Hint::At(area, command) => {
                if let Some(key) = keys_for(bindings, command).first() {
                    label(buf, area.x, area.y, key);
                }
            }
            Hint::Below(area, command) => {
                if let Some(key) = keys_for(bindings, command).first() {
                    let w = width(key) as u16 + 2;
                    let x = (area.x + area.width / 2).saturating_sub(w / 2);
                    label(buf, x, area.bottom(), key);
                }
            }
            Hint::Row(row, commands) => {
                let labels: Vec<String> = commands
                    .iter()
                    .filter_map(|c| {
                        let key = keys_for(bindings, c).into_iter().next()?;
                        Some(format!(
                            "{key} {}",
                            describe(std::slice::from_ref(c)).to_lowercase()
                        ))
                    })
                    .collect();
                let total: u16 = labels.iter().map(|l| width(l) as u16 + 3).sum();
                let mut x = row.right().saturating_sub(total + 1).max(row.x);
                for text in &labels {
                    x += label(buf, x, row.y, text) + 1;
                }
            }
        }
    }

    // How to see every key, on the bottom row.
    if screen.height > 2 {
        let y = screen.bottom() - 1;
        let row = Rect::new(screen.x, y, screen.width, 1);
        fill(buf, row, Style::new().bg(palette.statusbar_bg));
        let help = keys_for(bindings, &Command::Help);
        let hide = if held {
            "let go to hide"
        } else {
            "any key to hide"
        };
        let text = match help.first() {
            Some(key) => format!(" Keys on screen · {key} for all keys · {hide} "),
            None => format!(" Keys on screen · {hide} "),
        };
        put(
            buf,
            screen.x + 1,
            y,
            &text,
            usize::from(screen.width.saturating_sub(2)),
            Style::new().fg(Color::Yellow).bg(palette.statusbar_bg),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::ScreenId;

    #[test]
    fn test_key_labels() {
        assert_eq!(key_label("Shift+p"), "Shift+P");
        assert_eq!(key_label("Space"), "Space");
        assert_eq!(key_label("Up"), "↑");
        assert_eq!(key_label("q"), "q");
        assert_eq!(key_label("+"), "+");
    }

    #[test]
    fn test_keys_for_a_command() {
        let bindings = vec![
            ("F1".to_string(), vec![Command::Focus(ScreenId::Home)]),
            (
                "e".to_string(),
                vec![
                    Command::Queue,
                    Command::Move(MoveMode::Down, MoveAmount::default()),
                ],
            ),
        ];
        assert_eq!(keys_for(&bindings, &Command::Focus(ScreenId::Home)), ["F1"]);
        assert_eq!(keys_for(&bindings, &Command::Queue), ["e"]);
        assert!(keys_for(&bindings, &Command::Next).is_empty());
        assert_eq!(describe(&bindings[1].1), "Add to the queue");
        assert_eq!(group(&bindings[1].1), Group::Library);
    }
}
