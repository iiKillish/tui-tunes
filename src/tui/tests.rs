//! Smoke tests: the whole app against a disconnected Spotify and an in-memory terminal. They
//! check that every screen draws, and that keys, commands and clicks don't panic.

use std::sync::Arc;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};

use crate::command::{Command, MoveAmount, MoveMode};
use crate::config::Config;
use crate::events::EventManager;
use crate::library::Library;
use crate::queue::Queue;
use crate::spotify::Spotify;
use crate::tui::app::App;
use crate::tui::page::Ctx;

fn app() -> App {
    let config = Config::new_for_test();
    let events = EventManager::new_for_test();
    let spotify = Spotify::new_for_test(config.clone(), events.clone());
    let library = Library::new_for_test(events.clone(), spotify.clone(), config.clone());
    let queue = Arc::new(Queue::new(spotify.clone(), config.clone(), library.clone()));
    let palette = config.build_theme();
    App::new(Ctx {
        queue,
        library,
        spotify,
        config,
        events,
        palette,
    })
}

fn screen_text(terminal: &Terminal<TestBackend>) -> String {
    let buffer = terminal.backend().buffer();
    let mut text = String::new();
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            text.push_str(buffer[(x, y)].symbol());
        }
        text.push('\n');
    }
    text
}

fn key(app: &mut App, code: KeyCode) {
    app.key(KeyEvent::new(code, KeyModifiers::NONE));
}

#[test]
fn test_every_screen_draws_portrait_and_wide() {
    for (width, height) in [(75, 60), (200, 50), (60, 20)] {
        let mut app = app();
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        for screen in ["home", "search", "library", "queue", "lyrics"] {
            app.run(&Command::Focus(screen.into()));
            terminal.draw(|frame| app.draw(frame)).unwrap();
        }
        let text = screen_text(&terminal);
        assert!(
            text.contains("Home") && text.contains("Library"),
            "tabs missing:\n{text}"
        );
        // Tiny windows get a bar too narrow for the title.
        if width >= 75 {
            assert!(text.contains("Nothing playing"), "player missing:\n{text}");
        }
    }
}

#[test]
fn test_keys_commands_and_clicks_dont_panic() {
    let mut app = app();
    let mut terminal = Terminal::new(TestBackend::new(75, 60)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    for screen in ["home", "search", "library", "queue", "lyrics"] {
        app.run(&Command::Focus(screen.into()));
        for cmd in [
            Command::Move(MoveMode::Down, MoveAmount::Integer(3)),
            Command::Move(MoveMode::Right, MoveAmount::Integer(1)),
            Command::Move(MoveMode::Up, MoveAmount::Extreme),
            Command::Tab(true),
            Command::Play,
            Command::Back,
        ] {
            app.run(&cmd);
            terminal.draw(|frame| app.draw(frame)).unwrap();
        }
        for (column, row) in [(5, 30), (40, 40), (10, 59), (70, 2)] {
            app.mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column,
                row,
                modifiers: KeyModifiers::NONE,
            });
            terminal.draw(|frame| app.draw(frame)).unwrap();
        }
    }
    // The command line, a count prefix and help.
    key(&mut app, KeyCode::Char(':'));
    for c in "hel".chars() {
        key(&mut app, KeyCode::Char(c));
    }
    key(&mut app, KeyCode::Tab);
    key(&mut app, KeyCode::Enter);
    terminal.draw(|frame| app.draw(frame)).unwrap();
    assert!(screen_text(&terminal).contains("Keys"), "help didn't open");
    key(&mut app, KeyCode::Esc);
    key(&mut app, KeyCode::Char('3'));
    key(&mut app, KeyCode::Char('j'));
    terminal.draw(|frame| app.draw(frame)).unwrap();
}

/// Prints the portrait layout: `cargo test --features cover print_layout -- --ignored --nocapture`.
#[test]
#[ignore]
fn print_layout() {
    let mut app = app();
    let mut terminal = Terminal::new(TestBackend::new(75, 60)).unwrap();
    app.run(&Command::Focus("library".into()));
    terminal.draw(|frame| app.draw(frame)).unwrap();
    println!("{}", screen_text(&terminal));
}
