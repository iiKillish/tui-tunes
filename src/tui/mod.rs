//! The terminal user interface, built with ratatui. See [app] for how it's put together and
//! [page] for what screens and overlays implement.

pub mod actions;
pub mod app;
pub mod header;
#[cfg(feature = "cover")]
pub mod icons;
pub mod images;
pub mod keys;
pub mod nav;
pub mod overlays;
pub mod page;
pub mod pages;
pub mod pointer;
pub mod style;
pub mod widgets;

#[cfg(test)]
mod tests;

use std::io::{self, Stdout};
use std::sync::OnceLock;

use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event as InputEvent, KeyEventKind,
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

/// The terminal, set up for the UI: raw mode, the alternate screen, mouse reporting. Keys and
/// mouse events are read on a thread and arrive as [Event::Input].
pub struct Terminal {
    terminal: ratatui::Terminal<CrosstermBackend<Stdout>>,
}

impl Terminal {
    pub fn start() -> io::Result<Self> {
        enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
        pointer::enable();
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
            InputEvent::Key(key) if key.kind != KeyEventKind::Release => app.key(key),
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

fn restore() -> io::Result<()> {
    pointer::reset();
    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture)
}
