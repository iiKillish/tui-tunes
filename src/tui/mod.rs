//! The terminal user interface, built with ratatui. See [app] for how it's put together and
//! [page] for what screens and overlays implement.

pub mod actions;
pub mod app;
pub mod completion;
pub mod header;
pub mod hints;
#[cfg(feature = "cover")]
pub mod icons;
pub mod images;
pub mod keys;
pub mod nav;
pub mod overlays;
pub mod page;
pub mod pages;
pub mod pointer;
pub mod splash;
pub mod style;
pub mod widgets;

#[cfg(test)]
mod tests;

use std::io::{self, Stdout};
use std::sync::OnceLock;

use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event as InputEvent, KeyCode, KeyEventKind,
    KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};

use crate::events::{Event, EventManager};
use crate::tui::app::App;

static EVENTS: OnceLock<EventManager> = OnceLock::new();

/// Make [request_redraw] work; called once at startup.
pub fn set_event_manager(events: EventManager) {
    let _ = EVENTS.set(events);
}

/// Ask for a redraw from any thread (e.g. when a cover finished downloading).
pub fn request_redraw() {
    if let Some(events) = EVENTS.get() {
        events.trigger();
    }
}

/// An error from work in the background, shown as a message on the next frame.
static BACKGROUND_ERROR: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// Show `message` as an error from any thread, e.g. when a change couldn't be made on Spotify.
pub fn report_error(message: String) {
    // Tests run side by side: one's error would show in another's screen.
    if cfg!(test) {
        return;
    }
    *BACKGROUND_ERROR.lock().unwrap() = Some(message);
    request_redraw();
}

/// An item found by work in the background, opened on the next frame.
static BACKGROUND_OPEN: std::sync::Mutex<Option<Box<dyn crate::traits::ListItem>>> =
    std::sync::Mutex::new(None);

/// Open `item` (its page, or its menu for tracks and episodes) from any thread, e.g. once a link
/// has been looked up.
pub fn open_later(item: Box<dyn crate::traits::ListItem>) {
    // As with errors: not in another test's screen.
    if cfg!(test) {
        return;
    }
    *BACKGROUND_OPEN.lock().unwrap() = Some(item);
    request_redraw();
}

/// The item [open_later] left, if any.
pub fn take_background_open() -> Option<Box<dyn crate::traits::ListItem>> {
    BACKGROUND_OPEN.lock().unwrap().take()
}

/// The error [report_error] left, if any.
pub fn take_background_error() -> Option<String> {
    BACKGROUND_ERROR.lock().unwrap().take()
}

/// The terminal, set up for the UI: raw mode, the alternate screen, mouse reporting. Keys and
/// mouse events are read on a thread and arrive as [Event::Input].
pub struct Terminal {
    terminal: ratatui::Terminal<CrosstermBackend<Stdout>>,
}

impl Terminal {
    pub fn start() -> io::Result<Self> {
        enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
        // Terminals with the Kitty keyboard protocol report Ctrl and Shift on their own (held
        // together, they show the key hints) and Alt as Alt where a Mac would type a character.
        // Shifted keys come as the character typed, as without it.
        if ratatui::crossterm::terminal::supports_keyboard_enhancement().unwrap_or(false) {
            execute!(
                io::stdout(),
                PushKeyboardEnhancementFlags(
                    KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                        | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
                        | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES
                        | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
                )
            )?;
            KEYBOARD_ENHANCED.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        pointer::enable();
        images::start();
        // Restore the terminal if something panics, so the panic message can be read.
        let hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = restore();
            hook(info);
        }));
        let terminal = ratatui::Terminal::new(CrosstermBackend::new(io::stdout()))?;

        if let Some(events) = EVENTS.get().cloned() {
            std::thread::Builder::new()
                .name("input".into())
                .spawn(move || {
                    while let Ok(input) = event::read() {
                        events.send(Event::Input(input));
                    }
                })?;
        }
        Ok(Self { terminal })
    }

    pub fn stop(&mut self) -> io::Result<()> {
        images::clear();
        restore()?;
        self.terminal.show_cursor()
    }

    /// Draw the app, then put its images on the terminal.
    pub fn draw(&mut self, app: &mut App) -> io::Result<()> {
        if std::mem::take(&mut app.clear) {
            self.terminal.clear()?;
            images::reset();
        }
        self.terminal.draw(|frame| app.draw(frame))?;
        images::render(app.overlay_areas());
        // Things may have moved under the mouse.
        pointer::update();
        Ok(())
    }

    pub fn input(&mut self, input: InputEvent, app: &mut App) {
        match input {
            // Not `if let` in a match guard, which older Rust (see rust-version) doesn't have.
            InputEvent::Key(key) => match key.code {
                KeyCode::Modifier(code) => app.modifier_key(code, key.kind),
                _ if key.kind != KeyEventKind::Release => app.key(key),
                _ => {}
            },
            InputEvent::Mouse(mouse) => {
                pointer::hover((mouse.column, mouse.row).into());
                app.mouse(mouse);
            }
            // Terminals move images around when they reflow: place them again.
            InputEvent::Resize(..) => images::reset(),
            _ => {}
        }
    }
}

/// Whether the keyboard protocol was turned on, so it's turned off again on the way out.
static KEYBOARD_ENHANCED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn restore() -> io::Result<()> {
    if KEYBOARD_ENHANCED.swap(false, std::sync::atomic::Ordering::Relaxed) {
        let _ = execute!(io::stdout(), PopKeyboardEnhancementFlags);
    }
    pointer::reset();
    images::stop();
    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture)
}
