use crate::queue::RepeatSetting;
use crate::spotify_url::SpotifyUrl;
use std::fmt;

use strum::VariantNames;
use strum_macros::{Display, EnumString, IntoStaticStr};

const QUIT_CMD: &str = "quit";
const TOGGLE_PLAY_CMD: &str = "playpause";
const STOP_CMD: &str = "stop";
const PREVIOUS_CMD: &str = "previous";
const NEXT_CMD: &str = "next";
const CLEAR_CMD: &str = "clear";
const QUEUE_CMD: &str = "queue";
const PLAY_NEXT_CMD: &str = "playnext";
const PLAY: &str = "play";
const UPDATE_LIBRARY_CMD: &str = "update";
const SAVE_CMD: &str = "save";
const SAVE_CURRENT_CMD: &str = "save current";
const SAVE_QUEUE_CMD: &str = "save queue";
const ADD: &str = "add";
const ADD_CURRENT_CMD: &str = "add current";
const DELETE_CMD: &str = "delete";
const FOCUS: &str = "focus";
const SEEK: &str = "seek";
const VOLUME_UP_CMD: &str = "volup";
const VOLUME_DOWN_CMD: &str = "voldown";
const REPEAT_CMD: &str = "repeat";
const SHUFFLE_CMD: &str = "shuffle";
#[cfg(feature = "share_clipboard")]
const SHARE_CMD: &str = "share";
const BACK_CMD: &str = "back";
const OPEN_CMD: &str = "open";
const GOTO_CMD: &str = "goto";
const MOVE_CMD: &str = "move";
const SHIFT_CMD: &str = "shift";
const SEARCH_CMD: &str = "search";
const JUMP_PREVIOUS_CMD: &str = "jumpprevious";
const JUMP_NEXT_CMD: &str = "jumpnext";
const JUMP_CMD: &str = "jump";
const HELP_CMD: &str = "help";
const HINTS_CMD: &str = "hints";
const SETTINGS_CMD: &str = "settings";
const RELOAD_CONFIG_CMD: &str = "reload";
const NOOP_CMD: &str = "noop";
const INSERT_CMD: &str = "insert";
const NEW_PLAYLIST_CMD: &str = "newplaylist";
const RENAME_CMD: &str = "rename";
const SORT_CMD: &str = "sort";
const LOGOUT_CMD: &str = "logout";
const REDRAW_CMD: &str = "redraw";
const EXECUTE_CMD: &str = "exec";
const RECONNECT_CMD: &str = "reconnect";
const LIKE_CMD: &str = "like";
const PANEL_CMD: &str = "panel";
const TAB_CMD: &str = "tab";
const SCREEN_CMD: &str = "screen";
const SLEEP_CMD: &str = "sleep";

/// A command as the command line and the key list describe it.
#[derive(Debug)]
pub struct CommandInfo {
    pub name: &'static str,
    /// Its arguments, `<required>` and `[optional]`; empty if it takes none.
    pub args: &'static str,
    /// What it does, in a few words.
    pub about: &'static str,
    /// Words its first argument can be, for completion.
    pub values: &'static [&'static str],
}

const fn info(
    name: &'static str,
    args: &'static str,
    about: &'static str,
    values: &'static [&'static str],
) -> CommandInfo {
    CommandInfo {
        name,
        args,
        about,
        values,
    }
}

/// Every command, by what it does, for the command line's suggestions and the key list.
pub const COMMANDS: &[CommandInfo] = &[
    // Playback
    info(TOGGLE_PLAY_CMD, "", "Play / pause", &[]),
    info(STOP_CMD, "", "Stop", &[]),
    info(NEXT_CMD, "", "Next song", &[]),
    info(PREVIOUS_CMD, "", "Previous song", &[]),
    info(
        SEEK,
        "<[+|-]time>",
        "Go to a time, or forward/back (+1m, -30s)",
        &[],
    ),
    info(VOLUME_UP_CMD, "[percent]", "Volume up", &[]),
    info(VOLUME_DOWN_CMD, "[percent]", "Volume down", &[]),
    info(
        REPEAT_CMD,
        "[list|track|none]",
        "Repeat: step through, or set",
        &["list", "track", "none"],
    ),
    info(
        SHUFFLE_CMD,
        "[on|off]",
        "Shuffle: toggle, or set",
        &["on", "off"],
    ),
    info(LIKE_CMD, "", "Like the playing song (or unlike it)", &[]),
    info(
        SLEEP_CMD,
        "[minutes|track|off]",
        "Sleep timer: pause later",
        &["track", "off", "15", "30", "60"],
    ),
    // Queue, library and playlists
    info(PLAY, "", "Play the selected item", &[]),
    info(QUEUE_CMD, "", "Add the selected item to the queue", &[]),
    info(PLAY_NEXT_CMD, "", "Play the selected item next", &[]),
    info(CLEAR_CMD, "", "Clear the queue", &[]),
    info(
        SAVE_CMD,
        "[current|queue]",
        "Save the selected item, the playing song or the queue",
        &["current", "queue"],
    ),
    info(
        ADD,
        "[current]",
        "Add the selected (or playing) song to a playlist",
        &["current"],
    ),
    info(DELETE_CMD, "", "Remove the selected item", &[]),
    info(
        SHIFT_CMD,
        "<up|down> [amount]",
        "Move the selected song up or down",
        &["up", "down"],
    ),
    info(NEW_PLAYLIST_CMD, "<name>", "Make a new playlist", &[]),
    info(RENAME_CMD, "[name]", "Rename the playlist", &[]),
    info(
        SORT_CMD,
        "<key> [asc|desc]",
        "Sort the playlist",
        &["title", "album", "artist", "duration", "added", "original"],
    ),
    info(INSERT_CMD, "[url]", "Open a Spotify link", &[]),
    info(
        UPDATE_LIBRARY_CMD,
        "",
        "Refresh the library from Spotify",
        &[],
    ),
    #[cfg(feature = "share_clipboard")]
    info(
        SHARE_CMD,
        "<selected|current>",
        "Copy a link to the selected or playing item",
        &["selected", "current"],
    ),
    // Getting around
    info(FOCUS, "<screen>", "Go to a screen", ScreenId::VARIANTS),
    info(
        SCREEN_CMD,
        "[next|previous]",
        "Next or previous screen",
        &["next", "previous"],
    ),
    info(
        TAB_CMD,
        "[next|previous]",
        "Next or previous tab",
        &["next", "previous"],
    ),
    info(
        PANEL_CMD,
        "<queue|lyrics>",
        "Show Up next or the lyrics",
        &["queue", "lyrics"],
    ),
    info(
        MOVE_CMD,
        "<direction> [amount]",
        "Move the selection",
        &[
            "up", "down", "left", "right", "top", "bottom", "pageup", "pagedown", "playing",
        ],
    ),
    info(
        OPEN_CMD,
        "<selected|current>",
        "Open the selected item, or the playing song's menu",
        &["selected", "current"],
    ),
    info(
        GOTO_CMD,
        "<album|artist>",
        "Go to the selected song's album or artist",
        &["album", "artist"],
    ),
    info(BACK_CMD, "", "Back", &[]),
    info(SEARCH_CMD, "<text>", "Search Spotify", &[]),
    info(JUMP_CMD, "<text>", "Find in the list, like /", &[]),
    info(JUMP_NEXT_CMD, "", "Next match", &[]),
    info(JUMP_PREVIOUS_CMD, "", "Previous match", &[]),
    info(HELP_CMD, "", "All keys and commands", &[]),
    info(HINTS_CMD, "", "Show keys on screen", &[]),
    info(SETTINGS_CMD, "", "Settings", &[]),
    // Other
    info(QUIT_CMD, "", "Quit", &[]),
    info(LOGOUT_CMD, "", "Log out and quit", &[]),
    info(RELOAD_CONFIG_CMD, "", "Reload the configuration", &[]),
    info(RECONNECT_CMD, "", "Reconnect to Spotify", &[]),
    info(REDRAW_CMD, "", "Redraw the screen", &[]),
    info(EXECUTE_CMD, "<shell command>", "Run a shell command", &[]),
    info(NOOP_CMD, "", "Nothing (to unbind a key)", &[]),
];

/// Other names for commands, as (alias, command).
pub const ALIASES: &[(&str, &str)] = &[
    ("q", QUIT_CMD),
    ("x", QUIT_CMD),
    ("pause", TOGGLE_PLAY_CMD),
    ("toggleplay", TOGGLE_PLAY_CMD),
    ("toggleplayback", TOGGLE_PLAY_CMD),
    ("loop", REPEAT_CMD),
];

/// What the `panel` command shows: the Up next or Lyrics screen.
#[derive(Display, Clone, Copy, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[strum(serialize_all = "lowercase")]
pub enum PanelAction {
    /// Show the queue.
    Queue,
    /// Show the lyrics, or go back to the previous screen if they're already showing.
    Lyrics,
}

/// A main screen, as the `focus` command and the `initial_screen` option name it.
#[derive(
    Display,
    EnumString,
    IntoStaticStr,
    VariantNames,
    Clone,
    Copy,
    Serialize,
    Deserialize,
    Debug,
    PartialEq,
    Eq,
)]
#[strum(serialize_all = "lowercase")]
pub enum ScreenId {
    Home,
    Search,
    Library,
    Queue,
    Lyrics,
    Stats,
}

impl ScreenId {
    /// The screen's name on its tab.
    pub fn label(self) -> &'static str {
        match self {
            Self::Home => "Home",
            Self::Search => "Search",
            Self::Library => "Library",
            Self::Queue => "Up next",
            Self::Lyrics => "Lyrics",
            Self::Stats => "Stats",
        }
    }
}

#[derive(Display, Clone, Serialize, Deserialize, Debug)]
#[strum(serialize_all = "lowercase")]
pub enum TargetMode {
    Current,
    Selected,
}

#[derive(Display, Clone, Serialize, Deserialize, Debug)]
#[strum(serialize_all = "lowercase")]
pub enum MoveMode {
    Up,
    Down,
    Left,
    Right,
    Playing,
}

#[derive(Display, Clone, Serialize, Deserialize, Debug)]
#[strum(serialize_all = "lowercase")]
pub enum MoveAmount {
    Integer(i32),
    Float(f32),
    Extreme,
}

impl Default for MoveAmount {
    fn default() -> Self {
        Self::Integer(1)
    }
}

/// Keys that can be used to sort songs on.
#[derive(Display, Clone, Serialize, Deserialize, Debug)]
#[strum(serialize_all = "lowercase")]
pub enum SortKey {
    Title,
    Duration,
    Artist,
    Album,
    Added,
    /// The playlist's own order, as on Spotify.
    Original,
}

#[derive(Display, Clone, Serialize, Deserialize, Debug)]
#[strum(serialize_all = "lowercase")]
pub enum SortDirection {
    Ascending,
    Descending,
}

#[derive(Display, Clone, Serialize, Deserialize, Debug)]
#[strum(serialize_all = "lowercase")]
pub enum JumpMode {
    Previous,
    Next,
    Query(String),
}

#[derive(Display, Clone, Serialize, Deserialize, Debug)]
#[strum(serialize_all = "lowercase")]
pub enum ShiftMode {
    Up,
    Down,
}

#[derive(Display, Clone, Serialize, Deserialize, Debug)]
#[strum(serialize_all = "lowercase")]
pub enum GotoMode {
    Album,
    Artist,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub enum SeekDirection {
    Relative(i32),
    Absolute(u32),
}

impl fmt::Display for SeekDirection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let repr = match self {
            Self::Absolute(pos) => format!("{pos}"),
            Self::Relative(delta) => {
                format!("{}{}", if delta > &0 { "+" } else { "" }, delta)
            }
        };
        write!(f, "{repr}")
    }
}

/// What the `sleep` command sets the sleep timer to.
#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub enum SleepMode {
    /// Pause at the end of the current track, or turn that off if it's set.
    Toggle,
    /// Pause at the end of the current track.
    EndOfTrack,
    /// Pause after this many seconds.
    After(u64),
    Off,
}

impl fmt::Display for SleepMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Toggle => Ok(()),
            Self::EndOfTrack => write!(f, "track"),
            Self::After(seconds) => write!(f, "{seconds}s"),
            Self::Off => write!(f, "off"),
        }
    }
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub enum InsertSource {
    #[cfg(feature = "share_clipboard")]
    Clipboard,
    Input(SpotifyUrl),
}

impl fmt::Display for InsertSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let repr = match self {
            #[cfg(feature = "share_clipboard")]
            Self::Clipboard => "".into(),
            Self::Input(url) => url.to_string(),
        };
        write!(f, "{repr}")
    }
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub enum Command {
    Quit,
    TogglePlay,
    Stop,
    Previous,
    Next,
    Clear,
    Queue,
    PlayNext,
    Play,
    UpdateLibrary,
    Save,
    SaveCurrent,
    SaveQueue,
    Add,
    AddCurrent,
    Delete,
    Focus(ScreenId),
    Seek(SeekDirection),
    VolumeUp(u16),
    VolumeDown(u16),
    Repeat(Option<RepeatSetting>),
    Shuffle(Option<bool>),
    #[cfg(feature = "share_clipboard")]
    Share(TargetMode),
    Back,
    Open(TargetMode),
    Goto(GotoMode),
    Move(MoveMode, MoveAmount),
    Shift(ShiftMode, Option<i32>),
    Search(String),
    Jump(JumpMode),
    Help,
    /// Show or hide the keys of what's on screen, next to it.
    Hints,
    /// Open the settings page.
    Settings,
    ReloadConfig,
    Noop,
    Insert(InsertSource),
    NewPlaylist(String),
    /// Rename the open playlist, or ask for a name when it's empty.
    Rename(String),
    Sort(SortKey, SortDirection),
    Logout,
    Redraw,
    Execute(String),
    Reconnect,
    /// Save the current track to the library, or remove it if it's saved.
    Like,
    Panel(PanelAction),
    /// Switch to the next (true) or previous (false) tab inside the page.
    Tab(bool),
    /// Switch to the next (true) or previous (false) main screen.
    Screen(bool),
    /// Set the sleep timer, which pauses playback.
    Sleep(SleepMode),
}

impl fmt::Display for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut repr_tokens = vec![self.basename().to_owned()];
        let mut extras_args = match self {
            Self::Focus(screen) => vec![screen.to_string()],
            Self::Seek(direction) => vec![direction.to_string()],
            Self::VolumeUp(amount) => vec![amount.to_string()],
            Self::VolumeDown(amount) => vec![amount.to_string()],
            Self::Repeat(mode) => match mode {
                Some(mode) => vec![mode.to_string()],
                None => vec![],
            },
            Self::Shuffle(on) => match on {
                Some(b) => vec![(if *b { "on" } else { "off" }).into()],
                None => vec![],
            },
            #[cfg(feature = "share_clipboard")]
            Self::Share(mode) => vec![mode.to_string()],
            Self::Open(mode) => vec![mode.to_string()],
            Self::Goto(mode) => vec![mode.to_string()],
            Self::Move(mode, amount) => match (mode, amount) {
                (MoveMode::Playing, _) => vec!["playing".to_string()],
                (MoveMode::Up, MoveAmount::Extreme) => vec!["top".to_string()],
                (MoveMode::Down, MoveAmount::Extreme) => vec!["bottom".to_string()],
                (MoveMode::Left, MoveAmount::Extreme) => vec!["leftmost".to_string()],
                (MoveMode::Right, MoveAmount::Extreme) => vec!["rightmost".to_string()],
                (mode, MoveAmount::Float(amount)) => vec![mode.to_string(), amount.to_string()],
                (mode, MoveAmount::Integer(amount)) => vec![mode.to_string(), amount.to_string()],
            },
            Self::Shift(mode, amount) => vec![mode.to_string(), amount.unwrap_or(1).to_string()],
            Self::Search(term) => vec![term.to_owned()],
            Self::Jump(mode) => match mode {
                JumpMode::Previous | JumpMode::Next => vec![],
                JumpMode::Query(term) => vec![term.to_owned()],
            },
            Self::Insert(source) => vec![source.to_string()],
            Self::NewPlaylist(name) | Self::Rename(name) => vec![name.to_owned()],
            Self::Sort(key, direction) => vec![key.to_string(), direction.to_string()],
            Self::Execute(cmd) => vec![cmd.to_owned()],
            Self::Panel(action) => vec![action.to_string()],
            Self::Sleep(SleepMode::Toggle) => vec![],
            Self::Sleep(mode) => vec![mode.to_string()],
            Self::Tab(next) | Self::Screen(next) => {
                vec![(if *next { "next" } else { "previous" }).into()]
            }
            Self::Quit
            | Self::TogglePlay
            | Self::Stop
            | Self::Previous
            | Self::Next
            | Self::Clear
            | Self::Queue
            | Self::PlayNext
            | Self::Play
            | Self::UpdateLibrary
            | Self::Save
            | Self::SaveCurrent
            | Self::SaveQueue
            | Self::Add
            | Self::AddCurrent
            | Self::Delete
            | Self::Back
            | Self::Help
            | Self::Hints
            | Self::Settings
            | Self::ReloadConfig
            | Self::Noop
            | Self::Logout
            | Self::Reconnect
            | Self::Like
            | Self::Redraw => vec![],
        };
        repr_tokens.append(&mut extras_args);
        write!(f, "{}", repr_tokens.join(" "))
    }
}

impl Command {
    pub fn basename(&self) -> &str {
        match self {
            Self::Quit => QUIT_CMD,
            Self::TogglePlay => TOGGLE_PLAY_CMD,
            Self::Stop => STOP_CMD,
            Self::Previous => PREVIOUS_CMD,
            Self::Next => NEXT_CMD,
            Self::Clear => CLEAR_CMD,
            Self::Queue => QUEUE_CMD,
            Self::PlayNext => PLAY_NEXT_CMD,
            Self::Play => PLAY,
            Self::UpdateLibrary => UPDATE_LIBRARY_CMD,
            Self::Save => SAVE_CMD,
            Self::SaveCurrent => SAVE_CURRENT_CMD,
            Self::SaveQueue => SAVE_QUEUE_CMD,
            Self::Add => ADD,
            Self::AddCurrent => ADD_CURRENT_CMD,
            Self::Delete => DELETE_CMD,
            Self::Focus(_) => FOCUS,
            Self::Seek(_) => SEEK,
            Self::VolumeUp(_) => VOLUME_UP_CMD,
            Self::VolumeDown(_) => VOLUME_DOWN_CMD,
            Self::Repeat(_) => REPEAT_CMD,
            Self::Shuffle(_) => SHUFFLE_CMD,
            #[cfg(feature = "share_clipboard")]
            Self::Share(_) => SHARE_CMD,
            Self::Back => BACK_CMD,
            Self::Open(_) => OPEN_CMD,
            Self::Goto(_) => GOTO_CMD,
            Self::Move(_, _) => MOVE_CMD,
            Self::Shift(_, _) => SHIFT_CMD,
            Self::Search(_) => SEARCH_CMD,
            Self::Jump(JumpMode::Previous) => JUMP_PREVIOUS_CMD,
            Self::Jump(JumpMode::Next) => JUMP_NEXT_CMD,
            Self::Jump(JumpMode::Query(_)) => JUMP_CMD,
            Self::Help => HELP_CMD,
            Self::Hints => HINTS_CMD,
            Self::Settings => SETTINGS_CMD,
            Self::ReloadConfig => RELOAD_CONFIG_CMD,
            Self::Noop => NOOP_CMD,
            Self::Insert(_) => INSERT_CMD,
            Self::NewPlaylist(_) => NEW_PLAYLIST_CMD,
            Self::Rename(_) => RENAME_CMD,
            Self::Sort(_, _) => SORT_CMD,
            Self::Logout => LOGOUT_CMD,
            Self::Redraw => REDRAW_CMD,
            Self::Execute(_) => EXECUTE_CMD,
            Self::Reconnect => RECONNECT_CMD,
            Self::Like => LIKE_CMD,
            Self::Panel(_) => PANEL_CMD,
            Self::Tab(_) => TAB_CMD,
            Self::Screen(_) => SCREEN_CMD,
            Self::Sleep(_) => SLEEP_CMD,
        }
    }

    /// Whether this command may come from the IPC socket. Commands that run shell commands
    /// aren't, so something that can only write to the socket can't run programs as the user.
    pub fn allowed_over_ipc(&self) -> bool {
        !matches!(self, Self::Execute(_))
    }

    /// Whether a vim-style count prefix (e.g. `10j`) may repeat this command.
    /// True for movement, seeking, volume, track skipping and search-jumps. A
    /// binding accepts a count only if all of its commands do.
    pub fn accepts_count(&self) -> bool {
        matches!(
            self,
            Self::Move(_, _)
                | Self::Queue
                | Self::PlayNext
                | Self::Next
                | Self::Previous
                | Self::Seek(_)
                | Self::VolumeUp(_)
                | Self::VolumeDown(_)
                | Self::Jump(_)
        )
    }

    /// Returns a single command equivalent to executing `self` `count` times.
    ///
    /// This optimization is used when repetition simply scales a single
    /// parameter. It allows a count-prefixed binding to act once rather than
    /// looping, which prevents redundant side effects (such as re-reading the
    /// async player position on every seek, or sending an individual volume
    /// update per step).
    ///
    /// Returns `None` if the command cannot be scaled and must be executed
    /// sequentially (e.g., movement, jumps, queueing, or track skips). In these
    /// cases, the caller is responsible for looping the execution.
    ///
    /// Expects `count` within `MAX_COUNT`; larger values truncate when cast to
    /// the parameter's type. The sole caller passes a `MAX_COUNT`-clamped count.
    pub fn scaled(&self, count: usize) -> Option<Self> {
        match self {
            Self::Seek(SeekDirection::Relative(delta)) => Some(Self::Seek(
                SeekDirection::Relative(delta.saturating_mul(count as i32)),
            )),
            Self::VolumeUp(amount) => Some(Self::VolumeUp(amount.saturating_mul(count as u16))),
            Self::VolumeDown(amount) => Some(Self::VolumeDown(amount.saturating_mul(count as u16))),
            _ => None,
        }
    }
}

fn handle_aliases(input: &str) -> &str {
    match ALIASES.iter().find(|(alias, _)| *alias == input) {
        Some((_, command)) => command,
        None => input,
    }
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub enum CommandParseError {
    NoSuchCommand {
        cmd: String,
    },
    InsufficientArgs {
        cmd: String,
        hint: Option<String>,
    },
    BadEnumArg {
        arg: String,
        accept: Vec<String>,
        optional: bool,
    },
    ArgParseError {
        arg: String,
        err: String,
    },
}

impl fmt::Display for CommandParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let formatted = match self {
            Self::NoSuchCommand { cmd } => format!("No such command \"{cmd}\""),
            Self::InsufficientArgs { cmd, hint } => {
                if let Some(hint_str) = hint {
                    format!("\"{cmd}\" requires additional arguments: {hint_str}")
                } else {
                    format!("\"{cmd}\" requires additional arguments")
                }
            }
            Self::BadEnumArg {
                arg,
                accept,
                optional,
            } => {
                let accept = accept.join("|");
                if *optional {
                    format!("Argument \"{arg}\" should be one of {accept} or be omitted")
                } else {
                    format!("Argument \"{arg}\" should be one of {accept}")
                }
            }
            Self::ArgParseError { arg, err } => format!("Error with argument \"{arg}\": {err}"),
        };
        write!(f, "{formatted}")
    }
}

pub fn parse(input: &str) -> Result<Vec<Command>, CommandParseError> {
    let mut command_inputs = vec!["".to_string()];
    let mut command_idx = 0;
    enum ParseState {
        Normal,
        SeparatorEncountered,
    }
    let mut parse_state = ParseState::Normal;
    for c in input.chars() {
        let is_separator = c == ';';
        match parse_state {
            ParseState::Normal if is_separator => parse_state = ParseState::SeparatorEncountered,
            ParseState::Normal => command_inputs[command_idx].push(c),
            // ";" is escaped using ";;", so if the previous char already was a ';' push a ';'.
            ParseState::SeparatorEncountered if is_separator => {
                command_inputs[command_idx].push(c);
                parse_state = ParseState::Normal;
            }
            ParseState::SeparatorEncountered => {
                command_idx += 1;
                command_inputs.push(c.to_string());
                parse_state = ParseState::Normal;
            }
        }
    }

    let mut commands = vec![];
    for command_input in command_inputs {
        let components: Vec<_> = command_input.split_whitespace().collect();

        if let Some((command, args)) = components.split_first() {
            let command = handle_aliases(command);
            use CommandParseError as E;
            let command = match command {
                QUIT_CMD => Command::Quit,
                TOGGLE_PLAY_CMD => Command::TogglePlay,
                STOP_CMD => Command::Stop,
                PREVIOUS_CMD => Command::Previous,
                NEXT_CMD => Command::Next,
                CLEAR_CMD => Command::Clear,
                QUEUE_CMD => Command::Queue,
                PLAY_NEXT_CMD => Command::PlayNext,
                PLAY => Command::Play,
                UPDATE_LIBRARY_CMD => Command::UpdateLibrary,
                ADD => match args.first().cloned() {
                    Some("current") => Ok(Command::AddCurrent),
                    Some(arg) => Err(E::BadEnumArg {
                        arg: arg.into(),
                        accept: vec!["current".into()],
                        optional: true,
                    }),
                    None => Ok(Command::Add),
                }?,
                SAVE_CMD => match args.first().cloned() {
                    Some("queue") => Ok(Command::SaveQueue),
                    Some("current") => Ok(Command::SaveCurrent),
                    Some(arg) => Err(E::BadEnumArg {
                        arg: arg.into(),
                        accept: vec!["queue".into(), "current".into()],
                        optional: true,
                    }),
                    None => Ok(Command::Save),
                }?,
                DELETE_CMD => Command::Delete,
                FOCUS => {
                    let &target = args.first().ok_or(E::InsufficientArgs {
                        cmd: command.into(),
                        hint: Some(ScreenId::VARIANTS.join("|")),
                    })?;
                    Command::Focus(target.parse().map_err(|_| E::BadEnumArg {
                        arg: target.into(),
                        accept: ScreenId::VARIANTS.iter().map(|&s| s.into()).collect(),
                        optional: false,
                    })?)
                }
                SEEK => {
                    if args.is_empty() {
                        return Err(E::InsufficientArgs {
                            cmd: command.into(),
                            hint: Some("a duration".into()),
                        });
                    }
                    let arg = args.join(" ");
                    let first_char = arg.chars().next();
                    let duration_raw = match first_char {
                        Some('+' | '-') => {
                            arg.chars().skip(1).collect::<String>().trim().into()
                            // `trim` is necessary here, otherwise `+1000` -> 1 second, but `+ 1000` -> 1000 seconds
                            // this behaviour is inconsistent and could cause confusion
                        }
                        _ => arg,
                    };
                    let unsigned_millis = match duration_raw.parse() {
                        // accept raw milliseconds
                        Ok(millis) => millis,
                        Err(_) => {
                            crate::utils::parse_duration(&duration_raw) // accept fancy duration
                                .map_err(|err| E::ArgParseError {
                                    arg: duration_raw.clone(),
                                    err,
                                })
                                .and_then(|dur| {
                                    dur.as_millis().try_into().map_err(|_| E::ArgParseError {
                                        arg: duration_raw.clone(),
                                        err: "Duration value too large".into(),
                                    })
                                })?
                        }
                    };
                    let seek_direction = match first_char {
                        // handle i32::MAX < unsigned_millis < u32::MAX gracefully
                        Some('+') => i32::try_from(unsigned_millis).map(SeekDirection::Relative),
                        Some('-') => i32::try_from(unsigned_millis)
                            .map(|millis| SeekDirection::Relative(-millis)),
                        _ => Ok(SeekDirection::Absolute(unsigned_millis)),
                    }
                    .map_err(|_| E::ArgParseError {
                        arg: duration_raw,
                        err: "Duration value too large".into(),
                    })?;
                    Command::Seek(seek_direction)
                }
                VOLUME_UP_CMD => {
                    let amount = match args.first() {
                        Some(&amount_raw) => {
                            amount_raw.parse::<u16>().map_err(|err| E::ArgParseError {
                                arg: amount_raw.into(),
                                err: err.to_string(),
                            })?
                        }
                        None => 1,
                    };
                    Command::VolumeUp(amount)
                }
                VOLUME_DOWN_CMD => {
                    let amount = match args.first() {
                        Some(&amount_raw) => {
                            amount_raw.parse::<u16>().map_err(|err| E::ArgParseError {
                                arg: amount_raw.into(),
                                err: err.to_string(),
                            })?
                        }
                        None => 1,
                    };
                    Command::VolumeDown(amount)
                }
                REPEAT_CMD => {
                    let mode = match args.first().cloned() {
                        Some("list" | "playlist" | "queue") => {
                            Ok(Some(RepeatSetting::RepeatPlaylist))
                        }
                        Some("track" | "once" | "single") => Ok(Some(RepeatSetting::RepeatTrack)),
                        Some("none" | "off") => Ok(Some(RepeatSetting::None)),
                        Some(arg) => Err(E::BadEnumArg {
                            arg: arg.into(),
                            accept: vec![
                                "list".into(),
                                "playlist".into(),
                                "queue".into(),
                                "track".into(),
                                "once".into(),
                                "single".into(),
                                "none".into(),
                                "off".into(),
                            ],
                            optional: true,
                        }),
                        None => Ok(None),
                    }?;
                    Command::Repeat(mode)
                }
                SHUFFLE_CMD => {
                    let switch = match args.first().cloned() {
                        Some("on") => Ok(Some(true)),
                        Some("off") => Ok(Some(false)),
                        Some(arg) => Err(E::BadEnumArg {
                            arg: arg.into(),
                            accept: vec!["on".into(), "off".into()],
                            optional: true,
                        }),
                        None => Ok(None),
                    }?;
                    Command::Shuffle(switch)
                }
                #[cfg(feature = "share_clipboard")]
                SHARE_CMD => {
                    let &target_mode_raw = args.first().ok_or(E::InsufficientArgs {
                        cmd: command.into(),
                        hint: Some("selected|current".into()),
                    })?;
                    let target_mode = match target_mode_raw {
                        "selected" => Ok(TargetMode::Selected),
                        "current" => Ok(TargetMode::Current),
                        _ => Err(E::BadEnumArg {
                            arg: target_mode_raw.into(),
                            accept: vec!["selected".into(), "current".into()],
                            optional: false,
                        }),
                    }?;
                    Command::Share(target_mode)
                }
                BACK_CMD => Command::Back,
                OPEN_CMD => {
                    let &target_mode_raw = args.first().ok_or(E::InsufficientArgs {
                        cmd: command.into(),
                        hint: Some("selected|current".into()),
                    })?;
                    let target_mode = match target_mode_raw {
                        "selected" => Ok(TargetMode::Selected),
                        "current" => Ok(TargetMode::Current),
                        _ => Err(E::BadEnumArg {
                            arg: target_mode_raw.into(),
                            accept: vec!["selected".into(), "current".into()],
                            optional: false,
                        }),
                    }?;
                    Command::Open(target_mode)
                }
                GOTO_CMD => {
                    let &goto_mode_raw = args.first().ok_or(E::InsufficientArgs {
                        cmd: command.into(),
                        hint: Some("album|artist".into()),
                    })?;
                    let goto_mode = match goto_mode_raw {
                        "album" => Ok(GotoMode::Album),
                        "artist" => Ok(GotoMode::Artist),
                        _ => Err(E::BadEnumArg {
                            arg: goto_mode_raw.into(),
                            accept: vec!["album".into(), "artist".into()],
                            optional: false,
                        }),
                    }?;
                    Command::Goto(goto_mode)
                }
                MOVE_CMD => {
                    let &move_mode_raw = args.first().ok_or(E::InsufficientArgs {
                        cmd: command.into(),
                        hint: Some("a direction".into()),
                    })?;
                    let move_mode = {
                        use MoveMode as M;
                        match move_mode_raw {
                            "playing" => Ok(M::Playing),
                            "top" | "pageup" | "up" => Ok(M::Up),
                            "bottom" | "pagedown" | "down" => Ok(M::Down),
                            "leftmost" | "pageleft" | "left" => Ok(M::Left),
                            "rightmost" | "pageright" | "right" => Ok(M::Right),
                            _ => Err(E::BadEnumArg {
                                arg: move_mode_raw.into(),
                                accept: vec![
                                    "playing".into(),
                                    "top".into(),
                                    "bottom".into(),
                                    "leftmost".into(),
                                    "rightmost".into(),
                                    "pageup".into(),
                                    "pagedown".into(),
                                    "pageleft".into(),
                                    "pageright".into(),
                                    "up".into(),
                                    "down".into(),
                                    "left".into(),
                                    "right".into(),
                                ],
                                optional: false,
                            }),
                        }?
                    };
                    let move_amount = match move_mode_raw {
                        "playing" => Ok(MoveAmount::default()),
                        "top" | "bottom" | "leftmost" | "rightmost" => Ok(MoveAmount::Extreme),
                        "pageup" | "pagedown" | "pageleft" | "pageright" => {
                            let amount = match args.get(1) {
                                Some(&amount_raw) => amount_raw
                                    .parse::<f32>()
                                    .map(MoveAmount::Float)
                                    .map_err(|err| E::ArgParseError {
                                        arg: amount_raw.into(),
                                        err: err.to_string(),
                                    })?,
                                None => MoveAmount::default(),
                            };
                            Ok(amount)
                        }
                        "up" | "down" | "left" | "right" => {
                            let amount = match args.get(1) {
                                Some(&amount_raw) => amount_raw
                                    .parse::<i32>()
                                    .map(MoveAmount::Integer)
                                    .map_err(|err| E::ArgParseError {
                                        arg: amount_raw.into(),
                                        err: err.to_string(),
                                    })?,
                                None => MoveAmount::default(),
                            };
                            Ok(amount)
                        }
                        _ => unreachable!(), // already guarded when determining MoveMode
                    }?;
                    Command::Move(move_mode, move_amount)
                }
                SHIFT_CMD => {
                    let &shift_dir_raw = args.first().ok_or(E::InsufficientArgs {
                        cmd: command.into(),
                        hint: Some("up|down".into()),
                    })?;
                    let shift_dir = match shift_dir_raw {
                        "up" => Ok(ShiftMode::Up),
                        "down" => Ok(ShiftMode::Down),
                        _ => Err(E::BadEnumArg {
                            arg: shift_dir_raw.into(),
                            accept: vec!["up".into(), "down".into()],
                            optional: false,
                        }),
                    }?;
                    let amount = match args.get(1) {
                        Some(&amount_raw) => {
                            let amount =
                                amount_raw.parse::<i32>().map_err(|err| E::ArgParseError {
                                    arg: amount_raw.into(),
                                    err: err.to_string(),
                                })?;
                            Some(amount)
                        }
                        None => None,
                    };
                    Command::Shift(shift_dir, amount)
                }
                SEARCH_CMD => Command::Search(args.join(" ")),
                JUMP_CMD => Command::Jump(JumpMode::Query(args.join(" "))),
                JUMP_NEXT_CMD => Command::Jump(JumpMode::Next),
                JUMP_PREVIOUS_CMD => Command::Jump(JumpMode::Previous),
                HELP_CMD => Command::Help,
                HINTS_CMD => Command::Hints,
                SETTINGS_CMD => Command::Settings,
                RELOAD_CONFIG_CMD => Command::ReloadConfig,
                NOOP_CMD => Command::Noop,
                INSERT_CMD => {
                    let insert_source = match args.first().cloned() {
                        #[cfg(feature = "share_clipboard")]
                        Some("") | None => Ok(InsertSource::Clipboard),
                        // if clipboard feature is disabled and args is empty
                        #[cfg(not(feature = "share_clipboard"))]
                        None => Err(E::InsufficientArgs {
                            cmd: command.into(),
                            hint: Some("a Spotify URL".into()),
                        }),
                        Some(url) => SpotifyUrl::from_url(url).map(InsertSource::Input).ok_or(
                            E::ArgParseError {
                                arg: url.into(),
                                err: "Invalid Spotify URL".into(),
                            },
                        ),
                    }?;
                    Command::Insert(insert_source)
                }
                NEW_PLAYLIST_CMD => {
                    if !args.is_empty() {
                        Ok(Command::NewPlaylist(args.join(" ")))
                    } else {
                        Err(E::InsufficientArgs {
                            cmd: command.into(),
                            hint: Some("a name".into()),
                        })
                    }?
                }
                RENAME_CMD => Command::Rename(args.join(" ")),
                SORT_CMD => {
                    let &key_raw = args.first().ok_or(E::InsufficientArgs {
                        cmd: command.into(),
                        hint: Some("a sort key".into()),
                    })?;
                    let key = match key_raw {
                        "title" => Ok(SortKey::Title),
                        "duration" => Ok(SortKey::Duration),
                        "album" => Ok(SortKey::Album),
                        "added" => Ok(SortKey::Added),
                        "artist" => Ok(SortKey::Artist),
                        "original" => Ok(SortKey::Original),
                        _ => Err(E::BadEnumArg {
                            arg: key_raw.into(),
                            accept: vec![
                                "title".into(),
                                "duration".into(),
                                "album".into(),
                                "added".into(),
                                "artist".into(),
                                "original".into(),
                            ],
                            optional: false,
                        }),
                    }?;
                    let direction = match args.get(1).copied() {
                        Some("a" | "asc" | "ascending") => Ok(SortDirection::Ascending),
                        Some("d" | "desc" | "descending") => Ok(SortDirection::Descending),
                        Some(direction_raw) => Err(E::BadEnumArg {
                            arg: direction_raw.into(),
                            accept: vec![
                                "a".into(),
                                "asc".into(),
                                "ascending".into(),
                                "d".into(),
                                "desc".into(),
                                "descending".into(),
                            ],
                            optional: true,
                        }),
                        None => Ok(SortDirection::Ascending),
                    }?;
                    Command::Sort(key, direction)
                }
                LOGOUT_CMD => Command::Logout,
                REDRAW_CMD => Command::Redraw,
                EXECUTE_CMD => Command::Execute(args.join(" ")),
                RECONNECT_CMD => Command::Reconnect,
                LIKE_CMD => Command::Like,
                SLEEP_CMD => Command::Sleep(match args.first().copied() {
                    None => SleepMode::Toggle,
                    Some("track") => SleepMode::EndOfTrack,
                    Some("off") => SleepMode::Off,
                    Some(_) => {
                        let arg = args.join(" ");
                        // A plain number is minutes.
                        let seconds = match arg.parse::<u64>() {
                            Ok(minutes) => Ok(minutes.saturating_mul(60)),
                            Err(_) => crate::utils::parse_duration(&arg)
                                .map(|duration| duration.as_secs())
                                .map_err(|err| E::ArgParseError {
                                    arg: arg.clone(),
                                    err,
                                }),
                        }?;
                        if seconds == 0 {
                            return Err(E::ArgParseError {
                                arg,
                                err: "the sleep timer needs a duration of a second or more".into(),
                            });
                        }
                        SleepMode::After(seconds)
                    }
                }),
                SCREEN_CMD => Command::Screen(match args.first().copied() {
                    Some("next") | None => true,
                    Some("previous" | "prev") => false,
                    Some(arg) => {
                        return Err(E::BadEnumArg {
                            arg: arg.into(),
                            accept: vec!["next".into(), "previous".into()],
                            optional: true,
                        });
                    }
                }),
                TAB_CMD => Command::Tab(match args.first().copied() {
                    Some("next") | None => true,
                    Some("previous" | "prev") => false,
                    Some(arg) => {
                        return Err(E::BadEnumArg {
                            arg: arg.into(),
                            accept: vec!["next".into(), "previous".into()],
                            optional: true,
                        });
                    }
                }),
                PANEL_CMD => Command::Panel(match args.first().copied() {
                    Some("queue") => PanelAction::Queue,
                    Some("lyrics") => PanelAction::Lyrics,
                    Some(arg) => {
                        return Err(E::BadEnumArg {
                            arg: arg.into(),
                            accept: vec!["queue".into(), "lyrics".into()],
                            optional: false,
                        });
                    }
                    None => {
                        return Err(E::InsufficientArgs {
                            cmd: command.into(),
                            hint: Some("queue|lyrics".into()),
                        });
                    }
                }),
                _ => {
                    return Err(E::NoSuchCommand {
                        cmd: command.into(),
                    });
                }
            };
            commands.push(command);
        };
    }
    Ok(commands)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_described_command_parses() {
        for command in COMMANDS {
            if !command.args.starts_with('<') {
                assert!(parse(command.name).is_ok(), "{}", command.name);
            }
            for value in command.values {
                let line = format!("{} {value}", command.name);
                assert!(parse(&line).is_ok(), "{line}");
            }
        }
        for (alias, _) in ALIASES {
            assert!(parse(alias).is_ok(), "{alias}");
        }
    }

    #[test]
    fn shell_commands_are_refused_over_ipc() {
        let commands = parse("exec touch /tmp/x; next").unwrap();
        let allowed: Vec<bool> = commands.iter().map(Command::allowed_over_ipc).collect();
        assert_eq!(allowed, [false, true]);
    }

    #[test]
    fn focus_takes_a_screen() {
        assert!(matches!(
            parse("focus lyrics").as_deref(),
            Ok([Command::Focus(ScreenId::Lyrics)])
        ));
        assert!(matches!(
            parse("focus nowhere").as_deref(),
            Err(CommandParseError::BadEnumArg { .. })
        ));
        assert!(matches!(
            parse("focus").as_deref(),
            Err(CommandParseError::InsufficientArgs { .. })
        ));
    }

    #[test]
    fn rename_takes_an_optional_name() {
        assert!(matches!(
            parse("rename Road trip").as_deref(),
            Ok([Command::Rename(name)]) if name == "Road trip"
        ));
        assert!(matches!(
            parse("rename").as_deref(),
            Ok([Command::Rename(name)]) if name.is_empty()
        ));
    }

    #[test]
    fn sort_takes_original() {
        assert!(matches!(
            parse("sort original").as_deref(),
            Ok([Command::Sort(SortKey::Original, SortDirection::Ascending)])
        ));
    }

    #[test]
    fn sleep_takes_a_duration_track_or_off() {
        let sleep = |input: &str| match parse(input).as_deref() {
            Ok([Command::Sleep(mode)]) => Some(*mode),
            _ => None,
        };
        assert_eq!(sleep("sleep"), Some(SleepMode::Toggle));
        assert_eq!(sleep("sleep track"), Some(SleepMode::EndOfTrack));
        assert_eq!(sleep("sleep off"), Some(SleepMode::Off));
        assert_eq!(sleep("sleep 30"), Some(SleepMode::After(1800)));
        assert_eq!(sleep("sleep 1h 30m"), Some(SleepMode::After(5400)));
        assert_eq!(sleep("sleep 90s"), Some(SleepMode::After(90)));
        assert_eq!(sleep("sleep 0"), None);
        assert_eq!(sleep("sleep soon"), None);
        for mode in [
            SleepMode::Toggle,
            SleepMode::EndOfTrack,
            SleepMode::Off,
            SleepMode::After(90),
        ] {
            assert_eq!(sleep(&Command::Sleep(mode).to_string()), Some(mode));
        }
    }

    #[test]
    fn eligible_commands_accept_count() {
        let eligible = [
            Command::Move(MoveMode::Down, MoveAmount::default()),
            Command::Move(MoveMode::Up, MoveAmount::Extreme),
            Command::Queue,
            Command::PlayNext,
            Command::Next,
            Command::Previous,
            Command::Seek(SeekDirection::Relative(1000)),
            Command::VolumeUp(1),
            Command::VolumeDown(5),
            Command::Jump(JumpMode::Next),
            Command::Jump(JumpMode::Previous),
        ];
        for cmd in eligible {
            assert!(cmd.accepts_count(), "{cmd:?} should accept a count");
        }
    }

    #[test]
    fn one_shot_and_menu_commands_ignore_count() {
        let ineligible = [
            Command::Open(TargetMode::Selected),
            Command::Goto(GotoMode::Album),
            Command::Play,
            Command::TogglePlay,
            Command::Stop,
            Command::Repeat(None),
            Command::Shuffle(None),
            Command::Help,
            Command::Back,
            Command::Delete,
            Command::Quit,
            Command::Focus(ScreenId::Queue),
            Command::Shift(ShiftMode::Up, None),
        ];
        for cmd in ineligible {
            assert!(!cmd.accepts_count(), "{cmd:?} should ignore a count");
        }
    }

    #[test]
    fn binding_accepts_count_only_if_all_commands_do() {
        let space = [
            Command::Queue,
            Command::Move(MoveMode::Down, MoveAmount::default()),
        ];
        let dot = [
            Command::PlayNext,
            Command::Move(MoveMode::Down, MoveAmount::default()),
        ];
        let open = [Command::Open(TargetMode::Selected)];
        assert!(space.iter().all(Command::accepts_count));
        assert!(dot.iter().all(Command::accepts_count));
        assert!(!open.iter().all(Command::accepts_count));
    }

    #[test]
    fn scaled_multiplies_seek_and_volume() {
        // `Command` has no `PartialEq`, so match on the collapsed variants.
        assert!(matches!(
            Command::Seek(SeekDirection::Relative(-1000)).scaled(3),
            Some(Command::Seek(SeekDirection::Relative(-3000)))
        ));
        assert!(matches!(
            Command::VolumeUp(5).scaled(4),
            Some(Command::VolumeUp(20))
        ));
        assert!(matches!(
            Command::VolumeDown(2).scaled(3),
            Some(Command::VolumeDown(6))
        ));
    }

    #[test]
    fn scaled_saturates_instead_of_overflowing() {
        assert!(matches!(
            Command::VolumeUp(u16::MAX).scaled(9999),
            Some(Command::VolumeUp(u16::MAX))
        ));
        assert!(matches!(
            Command::Seek(SeekDirection::Relative(i32::MIN)).scaled(9999),
            Some(Command::Seek(SeekDirection::Relative(i32::MIN)))
        ));
    }

    #[test]
    fn scaled_is_none_for_repeat_by_execution_commands() {
        // These skip/move commands are collapsed elsewhere or must genuinely
        // repeat, so they have no scaled form.
        assert!(Command::Next.scaled(3).is_none());
        assert!(Command::Previous.scaled(3).is_none());
        assert!(Command::Queue.scaled(3).is_none());
        assert!(
            Command::Move(MoveMode::Down, MoveAmount::default())
                .scaled(3)
                .is_none()
        );
        assert!(
            Command::Seek(SeekDirection::Absolute(1000))
                .scaled(3)
                .is_none()
        );
    }
}
