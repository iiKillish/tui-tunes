use std::time::Duration;

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, TryIter, unbounded};

use crate::queue::QueueEvent;
use crate::spotify::PlayerEvent;

/// Events that can be sent to and handled by the main event loop (the one drawing the TUI).
pub enum Event {
    Player(PlayerEvent),
    Queue(QueueEvent),
    SessionDied,
    IpcInput(String),
    /// Terminal input (keys, mouse, resizes), read on a thread of its own.
    Input(ratatui::crossterm::event::Event),
    /// Something changed that should be shown: redraw.
    Redraw,
}

/// Manager that can be used to send and receive messages across threads.
#[derive(Clone)]
pub struct EventManager {
    tx: Sender<Event>,
    rx: Receiver<Event>,
}

impl EventManager {
    /// Create an EventManager whose events are never handled, for tests.
    #[cfg(test)]
    pub fn new_for_test() -> Self {
        Self::new()
    }

    pub fn new() -> Self {
        let (tx, rx) = unbounded();
        Self { tx, rx }
    }

    /// Return a non-blocking iterator over the messages awaiting handling.
    pub fn msg_iter(&self) -> TryIter<'_, Event> {
        self.rx.try_iter()
    }

    /// Wait up to `timeout` for the next event.
    pub fn recv_timeout(&self, timeout: Duration) -> Option<Event> {
        match self.rx.recv_timeout(timeout) {
            Ok(event) => Some(event),
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => None,
        }
    }

    /// Send a new event to be handled.
    pub fn send(&self, event: Event) {
        let _ = self.tx.send(event);
    }

    /// Ask the main loop to redraw.
    pub fn trigger(&self) {
        let _ = self.tx.send(Event::Redraw);
    }
}
