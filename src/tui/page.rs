//! What screens, pages and overlays implement, and what they get to work with.
//!
//! * A [Page] fills the content area: the top-level screens (Home, Search, Library, Up next,
//!   Lyrics) and the pages opened on top of them (an album, a playlist, an artist…).
//! * An [Overlay] is drawn over everything: menus, dialogs, the help screen.
//!
//! Keys are turned into [Command]s by the keybindings (see [crate::tui::keys]); each command goes
//! to the top overlay, else the current page, and if that ignores it, to the app (switching
//! screens, going back) and finally to the global actions (play/pause, volume…). Pages draw with
//! ratatui and ask for covers with [crate::tui::images].

use std::sync::Arc;

use ratatui::Frame;
use ratatui::crossterm::event::{KeyEvent, MouseEvent};
use ratatui::layout::Rect;

use crate::command::Command;
use crate::config::Config;
use crate::events::EventManager;
use crate::library::Library;
use crate::queue::Queue;
use crate::spotify::Spotify;
use crate::theme::Palette;
use crate::tui::header::{HeaderAction, PageHeader};

/// Shared handles every page can use. Cheap to clone.
#[derive(Clone)]
pub struct Ctx {
    pub queue: Arc<Queue>,
    pub library: Arc<Library>,
    pub spotify: Spotify,
    pub config: Arc<Config>,
    pub events: EventManager,
    pub palette: Palette,
}

/// What handling a command, key or click led to.
pub enum Outcome {
    /// Not for this page: let the app and the global actions have it.
    Ignored,
    /// Done.
    Consumed,
    /// Show a page on top of the current screen (back returns to this one).
    Open(Box<dyn Page>),
    /// Show a menu or dialog over everything.
    Overlay(Box<dyn Overlay>),
    /// Close the overlay that returned it.
    Close,
    /// Close the overlay that returned it and show another in its place.
    Replace(Box<dyn Overlay>),
    /// A message for the status line.
    Message(String),
    /// An error for the status line.
    Error(String),
    /// Run these commands as if their keys were pressed (e.g. a menu entry).
    Run(Vec<Command>),
    /// The settings changed (and were loaded): use them (keys, library tabs, colors), and say
    /// what changed, if anything.
    SettingsChanged(Option<String>),
}

impl Outcome {
    pub fn is_ignored(&self) -> bool {
        matches!(self, Self::Ignored)
    }
}

pub trait Page {
    /// Name shown in the title row (and as "< name" on pages opened on top of it).
    fn title(&self) -> String;

    /// Text at the right of the title row, e.g. "12 songs".
    fn subtitle(&self) -> String {
        String::new()
    }

    /// A Spotify-style header (cover, name, details, buttons) drawn above the page by the app,
    /// instead of the title row's title.
    fn header(&self, _ctx: &Ctx) -> Option<PageHeader> {
        None
    }

    /// A header button was clicked.
    fn header_action(&mut self, _action: HeaderAction, _ctx: &Ctx) -> Outcome {
        Outcome::Ignored
    }

    /// Draw into `area`. `focused` is false while an overlay has the keyboard. Remember `area`
    /// (or what was drawn where) for [Page::mouse].
    fn draw(&mut self, frame: &mut Frame, area: Rect, ctx: &Ctx, focused: bool);

    /// Handle a command from a keybinding or the command line.
    fn command(&mut self, cmd: &Command, ctx: &Ctx) -> Outcome;

    /// A mouse event, in terminal coordinates. Only called for events inside the page's area.
    fn mouse(&mut self, _event: MouseEvent, _ctx: &Ctx) -> Outcome {
        Outcome::Ignored
    }

    /// Whether key presses are typing (a focused search field) rather than keybindings. While
    /// true, keys go to [Page::key] first, and only those it ignores to the keybindings.
    fn captures_text(&self) -> bool {
        false
    }

    /// A raw key press, while [Page::captures_text].
    fn key(&mut self, _key: KeyEvent, _ctx: &Ctx) -> Outcome {
        Outcome::Ignored
    }

    /// Called each time the page is shown again (switched to, or returned to with back).
    fn on_show(&mut self, _ctx: &Ctx) {}

    /// Whether left/right moves are for the page itself (a grid of cards) rather than switching
    /// its parent's tabs.
    fn handles_horizontal_moves(&self) -> bool {
        false
    }
}

pub trait Overlay {
    /// Draw over `screen` (the whole terminal) and return the area covered, so covers under it
    /// can be hidden (terminal images are always drawn over text).
    fn draw(&mut self, frame: &mut Frame, screen: Rect, ctx: &Ctx) -> Rect;

    /// Handle a command. `Command::Back` should close it.
    fn command(&mut self, cmd: &Command, ctx: &Ctx) -> Outcome;

    /// Whether key presses are typing (a text field), see [Page::captures_text].
    fn captures_text(&self) -> bool {
        false
    }

    /// A raw key press, while [Overlay::captures_text].
    fn key(&mut self, _key: KeyEvent, _ctx: &Ctx) -> Outcome {
        Outcome::Ignored
    }

    /// A mouse event anywhere (clicking outside usually closes it).
    fn mouse(&mut self, _event: MouseEvent, _ctx: &Ctx) -> Outcome {
        Outcome::Consumed
    }
}
