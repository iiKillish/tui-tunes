//! Smoke tests: the whole app against a disconnected Spotify and an in-memory terminal. They
//! check that every screen draws, and that keys, commands and clicks don't panic.

use std::collections::HashMap;
use std::sync::Arc;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::{Position, Rect};

use crate::command::{Command, MoveAmount, MoveMode, ScreenId, TargetMode};
use crate::config::Config;
use crate::events::EventManager;
use crate::library::Library;
use crate::model::album::Album;
use crate::model::artist::Artist;
use crate::model::playable::Playable;
use crate::model::playlist::Playlist;
use crate::model::track::Track;
use crate::queue::Queue;
use crate::spotify::Spotify;
use crate::tui::app::App;
use crate::tui::images;
use crate::tui::page::Ctx;
use crate::tui::pages;

pub(crate) fn app() -> App {
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
        for screen in SCREENS {
            app.run(&Command::Focus(screen));
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
    for screen in SCREENS {
        app.run(&Command::Focus(screen));
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
    for screen in SCREENS {
        app.run(&Command::Focus(screen));
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let text = screen_text(&terminal);
        // The page part only, under the player.
        println!("===== {screen}");
        for line in text.lines().skip(24).take(14) {
            println!("{}", line.trim_end());
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Layout at many sizes, with and without images, over a library with some content.

fn track(n: usize) -> Track {
    serde_json::from_value(serde_json::json!({
        "id": format!("test{n}"),
        "uri": format!("spotify:track:test{n}"),
        "title": format!("Song number {n} with a fairly long title"),
        "track_number": n,
        "disc_number": 1,
        "duration": 180_000 + n * 1000,
        "artists": ["Some Artist", "Another Artist"],
        "artist_ids": ["artist1", "artist2"],
        "album": "An Album Title",
        "album_id": null,
        "album_artists": ["Some Artist"],
        // Never resolves (RFC 2606): covers take their space but never load.
        "cover_url": "https://covers.invalid/track.jpg",
        "url": "",
        "added_at": null,
        "list_index": n,
        "is_local": false,
        "is_playable": true,
    }))
    .unwrap()
}

fn tracks(count: usize) -> Vec<Track> {
    (1..=count).map(track).collect()
}

fn album() -> Album {
    serde_json::from_value(serde_json::json!({
        "id": null,
        "title": "An Album Title That Goes On For A While",
        "artists": ["Some Artist"],
        "artist_ids": ["artist1"],
        "year": "1999",
        "cover_url": "https://covers.invalid/album.jpg",
        "url": null,
        "tracks": tracks(12),
        "added_at": null,
    }))
    .unwrap()
}

fn playlist() -> Playlist {
    let tracks: Vec<Playable> = tracks(30).into_iter().map(Playable::Track).collect();
    Playlist {
        id: "playlist".into(),
        name: "A Playlist".into(),
        owner_id: "someone".into(),
        owner_name: Some("Someone".into()),
        snapshot_id: String::new(),
        num_tracks: tracks.len(),
        tracks: Some(tracks),
        collaborative: false,
        cover_url: Some("https://covers.invalid/playlist.jpg".into()),
    }
}

fn artist() -> Artist {
    Artist {
        id: None,
        name: "Some Artist".into(),
        url: None,
        tracks: None,
        is_followed: false,
    }
}

/// An app whose library and queue have content, playing the first song.
fn full_app() -> App {
    let app = app();
    let ctx = app.ctx.clone();
    ctx.library.tracks.write().unwrap().extend(tracks(40));
    ctx.library.albums.write().unwrap().push(album());
    ctx.library.playlists.write().unwrap().push(playlist());
    ctx.library.artists.write().unwrap().push(artist());
    *ctx.library.is_done.write().unwrap() = true;
    for track in tracks(25) {
        ctx.queue.append(Playable::Track(track));
    }
    ctx.queue.play(0, false, false);
    let mut albums = vec![album()];
    albums.extend((2..=4).map(|n| {
        let mut album = album();
        album.title = format!("Album {n}");
        album.id = Some(format!("album{n}"));
        album
    }));
    let home = pages::home::new_for_test(albums, vec![playlist()], vec![artist()], tracks(10));
    let mut app = app;
    app.set_root(ScreenId::Home, Box::new(home));
    app
}

const SCREENS: [ScreenId; 6] = [
    ScreenId::Home,
    ScreenId::Search,
    ScreenId::Library,
    ScreenId::Queue,
    ScreenId::Lyrics,
    ScreenId::Stats,
];
const PAGES: [&str; 3] = ["album", "playlist", "artist"];
/// The library's tabs after Tracks.
const LIBRARY_GRIDS: [&str; 3] = ["albums", "artists", "playlists"];

/// Draws each screen, and pages opened on Home, on a `width`×`height` terminal. Returns the
/// text of each.
fn draw_all(app: &mut App, width: u16, height: u16) -> Vec<(&'static str, String)> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    let mut shots = Vec::new();
    let mut shot = |app: &mut App, name: &'static str| {
        terminal
            .draw(|frame| app.draw(frame))
            .unwrap_or_else(|e| panic!("{name} at {width}x{height}: {e}"));
        shots.push((name, screen_text(&terminal)));
    };
    for screen in SCREENS {
        app.run(&Command::Focus(screen));
        shot(app, screen.into());
    }
    // The library's card grids.
    app.run(&Command::Focus(ScreenId::Library));
    for tab in LIBRARY_GRIDS {
        app.run(&Command::Tab(true));
        shot(app, tab);
    }
    for _ in LIBRARY_GRIDS {
        app.run(&Command::Tab(false));
    }
    app.run(&Command::Focus(ScreenId::Home));
    let ctx = app.ctx.clone();
    app.open_page(Box::new(pages::album::AlbumPage::new(album(), &ctx)));
    shot(app, "album");
    app.open_page(Box::new(pages::playlist::PlaylistPage::new(
        playlist(),
        &ctx,
    )));
    shot(app, "playlist");
    app.open_page(Box::new(pages::artist::ArtistPage::new(artist(), &ctx)));
    shot(app, "artist");
    for _ in PAGES {
        app.run(&Command::Back);
    }
    shots
}

/// The screens of [full_app] at `width`×`height`, by name ([SCREENS], [LIBRARY_GRIDS], [PAGES]).
fn shots(images_on: bool, width: u16, height: u16) -> HashMap<&'static str, String> {
    images::force_supported(images_on);
    draw_all(&mut full_app(), width, height)
        .into_iter()
        .collect()
}

/// Asserts that `text` contains each of `expected`, showing the screen if not.
#[track_caller]
fn assert_shows(name: &str, text: &str, expected: &[&str]) {
    for e in expected {
        assert!(text.contains(e), "{name}: {e:?} missing from\n{text}");
    }
}

/// The rows of `text` from `from` on.
fn rows_from(text: &str, from: usize) -> String {
    text.lines().skip(from).collect::<Vec<_>>().join("\n")
}

/// The player's controls, unless they're images.
fn controls(images_on: bool) -> &'static [&'static str] {
    if images_on && cfg!(feature = "cover") {
        &[]
    } else {
        &["⇄", "⏮", "▶", "⏭", "↻"]
    }
}

/// Page content each screen shows at any reasonable size.
fn page_content(name: &str) -> &'static [&'static str] {
    match name {
        "home" => &["All", "Album • Some Artist"],
        "search" => &["What do you want to listen to?"],
        "library" => &["Tracks", "Song number 1"],
        "queue" => &["Now playing", "Song number 1"],
        "lyrics" => &["Loading lyrics"],
        "stats" => &["Listening", "Top: 4 weeks"],
        "albums" => &["An Album Title", "Album • Some"],
        "artists" => &["Some Artist", "Artist"],
        "playlists" => &["Liked Songs", "A Playlist"],
        "album" | "playlist" => &["Song number 2", "3:02"],
        "artist" => &["‹ ", "Some Artist"],
        _ => unreachable!(),
    }
}

#[test]
fn test_layout_big_windows() {
    for images_on in [false, true] {
        for (width, height) in [(200, 60), (75, 60), (120, 30)] {
            for (name, text) in shots(images_on, width, height) {
                let name = &format!("{name} {width}x{height} images: {images_on}");
                // The hero: the track, the controls, the volume.
                assert_shows(
                    name,
                    &text,
                    &[
                        "Song number 1 with a fairly",
                        "Some Artist, Another Artist",
                        "-3:01",
                        "♪",
                        "Home   Search   Library   Up next   Lyrics",
                    ],
                );
                assert_shows(name, &text, controls(images_on));
                assert_shows(name, &text, page_content(name.split(' ').next().unwrap()));
            }
            // Room for the featured card and the page headers.
            let shots = shots(images_on, width, height);
            assert_shows(
                "home",
                &shots["home"],
                &["Pick up where you left off", "▶ Play"],
            );
            assert_shows(
                "album",
                &shots["album"],
                &[
                    "Album",
                    "Some Artist • 1999 • 12 songs",
                    "⇄ Shuffle",
                    "♥ Liked",
                ],
            );
            assert_shows(
                "playlist",
                &shots["playlist"],
                &["Playlist", "Someone • 30 songs", "♡ Like"],
            );
            assert_shows("artist", &shots["artist"], &["Top tracks", "▶ Play"]);
            assert_shows("queue", &shots["queue"], &["Next up", "Song number 2"]);
        }
    }
}

#[test]
fn test_layout_small_windows() {
    for images_on in [false, true] {
        for (width, height) in [(80, 15), (80, 10), (40, 10), (60, 20)] {
            for (name, text) in shots(images_on, width, height) {
                let name_size = &format!("{name} {width}x{height} images: {images_on}");
                // The bar: the title readable, the controls and the progress.
                assert_shows(name_size, &text, &["Song number 1", "0:00", "-3:01"]);
                assert_shows(name_size, &text, controls(images_on));
                assert_shows(name_size, &text, &["Home   Search   Library"]);
                if height >= 15 || !["queue", "home"].contains(&name) {
                    assert_shows(name_size, &text, page_content(name));
                }
            }
        }
    }
}

#[test]
fn test_layout_tiny_windows() {
    for images_on in [false, true] {
        for (width, height) in [(20, 5), (30, 7), (80, 4), (80, 6)] {
            for (name, text) in shots(images_on, width, height) {
                let name = &format!("{name} {width}x{height} images: {images_on}");
                // The tabs, and the bar's controls, inside its rows at the bottom.
                assert_shows(name, &text, &["Home   Search"]);
                let bar_rows = (height / 3) as usize;
                let above_bar = text
                    .lines()
                    .take(height as usize - bar_rows)
                    .collect::<String>();
                let bar = rows_from(&text, height as usize - bar_rows);
                assert_shows(name, &bar, controls(images_on));
                // (▶ also marks the song playing in lists.)
                for control in controls(images_on).iter().filter(|c| **c != "▶") {
                    assert!(
                        !above_bar.contains(control),
                        "{name}: {control} above the bar\n{text}"
                    );
                }
            }
        }
        // Tabbed pages with a page area 2 rows high still show their tabs.
        let shots = shots(images_on, 80, 6);
        assert_shows("library", &shots["library"], &["Tracks   Albums"]);
    }
}

#[test]
fn test_layout_narrow_windows() {
    for images_on in [false, true] {
        let shots = shots(images_on, 24, 40);
        for (name, text) in &shots {
            let name = &format!("{name} 24x40 images: {images_on}");
            // The hero's controls don't run into each other.
            assert_shows(name, text, controls(images_on));
            for pair in ["⇄⏮", "⏭↻"] {
                assert!(!text.contains(pair), "{name}: controls collide\n{text}");
            }
        }
        // Covers are left out rather than squeeze the text.
        assert_shows("home", &shots["home"], &["Pick up where", "An Album Title"]);
        assert_shows("album", &shots["album"], &["An Album Title", "Some Artist"]);
    }
}

#[test]
fn test_card_grids_in_short_windows() {
    // Too short for covers: the cards are text only, rather than not drawn at all.
    for images_on in [false, true] {
        let shots = shots(images_on, 80, 10);
        for name in LIBRARY_GRIDS {
            assert_shows(name, &shots[name], page_content(name));
        }
    }
}

#[test]
fn test_layout_snapshot() {
    images::force_supported(false);
    let shots: HashMap<_, _> = draw_all(&mut full_app(), 40, 10).into_iter().collect();
    let expected = [
        "  Home   Search   Library   Up next   L…",
        "                                        ",
        "  ‹ Home   An Album Title That Goes O…  ",
        "▌  ▶ Song number 1 with a fair…  ✓ 3:01 ",
        "   2 Song number 2 with a fair…  ✓ 3:02 ",
        "   3 Song number 3 with a fair…  ✓ 3:03 ",
        "   4 Song number 4 with a fair…  ✓ 3:04 ",
        "  Song number 1 with a fairly long…  ♥  ",
        "         ⇄    ⏮    ▶    ⏭    ↻          ",
        "      0:00 ━━━━━━━━━━━━━━━━━ -3:01      ",
    ];
    assert_eq!(
        shots["album"].lines().collect::<Vec<_>>(),
        expected,
        "\n{}",
        shots["album"]
    );
}

#[test]
fn test_no_panic_at_any_size() {
    for images_on in [false, true] {
        images::force_supported(images_on);
        let mut app = full_app();
        for width in (1..=120).step_by(7) {
            for height in (1..=50).step_by(3) {
                draw_all(&mut app, width, height);
            }
        }
        for (width, height) in [(0, 0), (1, 1), (2, 2), (300, 2), (2, 120), (250, 80)] {
            draw_all(&mut app, width, height);
        }
    }
}

#[test]
fn test_overlays_at_any_size() {
    let mut app = full_app();
    app.run(&Command::Focus(ScreenId::Library));
    for width in (0..=60).step_by(3) {
        for height in (0..=24).step_by(2) {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            for open in [
                Command::Help,
                Command::SaveQueue,
                Command::Open(TargetMode::Selected),
            ] {
                app.run(&open);
                terminal
                    .draw(|frame| app.draw(frame))
                    .unwrap_or_else(|e| panic!("{open:?} at {width}x{height}: {e}"));
                let screen = Rect::new(0, 0, width, height);
                let areas = app.overlay_areas();
                assert_eq!(areas.len(), 1, "{open:?} at {width}x{height} didn't open");
                assert!(
                    areas[0].is_empty() || screen.union(areas[0]) == screen,
                    "{open:?} at {width}x{height} is off screen: {:?}",
                    areas[0]
                );
                key(&mut app, KeyCode::Esc);
            }
        }
    }
}

#[test]
fn test_clicks_in_small_windows_dont_panic() {
    for images_on in [false, true] {
        images::force_supported(images_on);
        for (width, height) in [(20, 5), (40, 10), (80, 15)] {
            let mut app = full_app();
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            for screen in SCREENS {
                for row in 0..height {
                    for column in (0..width).step_by(5) {
                        app.run(&Command::Focus(screen));
                        terminal.draw(|frame| app.draw(frame)).unwrap();
                        app.mouse(MouseEvent {
                            kind: MouseEventKind::Down(MouseButton::Left),
                            column,
                            row,
                            modifiers: KeyModifiers::NONE,
                        });
                        terminal.draw(|frame| app.draw(frame)).unwrap();
                        key(&mut app, KeyCode::Esc);
                        app.run(&Command::Back);
                    }
                }
            }
        }
    }
}

/// Prints every screen at a size:
/// `SIZE=80x15 IMAGES=1 cargo test print_size -- --ignored --nocapture`.
#[test]
#[ignore]
fn print_size() {
    let size = std::env::var("SIZE").unwrap_or("80x24".into());
    let (w, h) = size.split_once('x').unwrap();
    images::force_supported(std::env::var("IMAGES").is_ok());
    let mut app = full_app();
    for (name, text) in draw_all(&mut app, w.parse().unwrap(), h.parse().unwrap()) {
        println!("===== {name} {size}");
        for line in text.lines() {
            println!("|{line}|");
        }
    }
}

#[test]
fn test_typing_filters_a_filterable_menu() {
    use crate::tui::overlays::menu::{Entry, Menu};
    use crate::tui::page::{Outcome, Overlay};
    use std::cell::Cell;
    use std::rc::Rc;

    let ctx = app().ctx.clone();
    let picked = Rc::new(Cell::new(""));
    let entries = ["Road trip", "Chill", "Rock classics", "Café del Mar"]
        .into_iter()
        .map(|name| {
            let picked = picked.clone();
            Entry::new(name, move |_| {
                picked.set(name);
                Outcome::Consumed
            })
        })
        .collect();
    let mut menu = Menu::new("Add to playlist", entries).filterable();
    assert!(menu.captures_text());
    let press = |menu: &mut Menu, code| menu.key(KeyEvent::new(code, KeyModifiers::NONE), &ctx);

    for c in "RO".chars() {
        assert!(matches!(
            press(&mut menu, KeyCode::Char(c)),
            Outcome::Consumed
        ));
    }
    // "ro" matches "Road trip" and "Rock classics", ignoring case; Down picks the second.
    menu.command(&Command::Move(MoveMode::Down, MoveAmount::Integer(1)), &ctx);
    menu.command(&Command::Play, &ctx);
    assert_eq!(picked.get(), "Rock classics");

    // Arrows and Enter go on to the key bindings; Esc clears the filter before closing.
    assert!(press(&mut menu, KeyCode::Down).is_ignored());
    assert!(matches!(press(&mut menu, KeyCode::Esc), Outcome::Consumed));
    assert!(press(&mut menu, KeyCode::Esc).is_ignored());

    // Accents don't count either: "cafe" finds "Café del Mar".
    for c in "cafe".chars() {
        press(&mut menu, KeyCode::Char(c));
    }
    menu.command(&Command::Play, &ctx);
    assert_eq!(picked.get(), "Café del Mar");
    press(&mut menu, KeyCode::Esc);

    for (width, height) in [(0, 0), (10, 3), (30, 4), (60, 20)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        for c in ["", "x"] {
            for ch in c.chars() {
                press(&mut menu, KeyCode::Char(ch));
            }
            terminal
                .draw(|frame| {
                    menu.draw(frame, frame.area(), &ctx);
                })
                .unwrap_or_else(|e| panic!("menu at {width}x{height}: {e}"));
        }
        press(&mut menu, KeyCode::Backspace);
    }
}

#[test]
fn test_tab_leaves_the_search_field_and_switches_screens() {
    let mut app = app();
    // Through every screen and back, including Search, whose field has the keyboard when shown.
    for expected in SCREENS.iter().cycle().skip(1).take(SCREENS.len()) {
        key(&mut app, KeyCode::Tab);
        assert_eq!(app.current_screen(), *expected);
    }
    for expected in SCREENS.iter().rev() {
        app.key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT));
        assert_eq!(app.current_screen(), *expected);
    }
}

/// Where `text` was drawn: the column and row of its first character.
fn find(terminal: &Terminal<TestBackend>, text: &str) -> Option<Position> {
    screen_text(terminal)
        .lines()
        .enumerate()
        .find_map(|(row, line)| {
            let byte = line.find(text)?;
            let column = line[..byte].chars().count();
            Some(Position::new(column as u16, row as u16))
        })
}

#[test]
fn test_pointer_is_a_hand_over_clickable_things() {
    use crate::tui::pointer::{Shape, shape_at};

    let mut app = full_app();
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let tab = find(&terminal, "Library").expect("the tab row is drawn");
    assert_eq!(shape_at(tab), Shape::Pointer, "a screen tab");
    assert_eq!(
        shape_at(Position::new(119, 39)),
        Shape::Default,
        "the bottom right corner"
    );

    // The search field is for typing.
    app.run(&Command::Focus(ScreenId::Search));
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let field = find(&terminal, "What do you want to listen to?").expect("the search field");
    assert_eq!(shape_at(field), Shape::Text, "the search field");

    // A menu takes the clicks: its entries are clickable, the tabs under it aren't.
    app.run(&Command::Help);
    terminal.draw(|frame| app.draw(frame)).unwrap();
    assert_eq!(shape_at(tab), Shape::Default, "a tab under the key list");
    let entry = find(&terminal, "Play / pause").expect("a key list entry");
    assert_eq!(shape_at(entry), Shape::Pointer, "a key list entry");
    let filter = find(&terminal, "Type to find a key").expect("the filter field");
    assert_eq!(shape_at(filter), Shape::Text, "the filter field");
}

fn click(column: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }
}

#[test]
fn test_clicks_outside_a_lists_rows_do_nothing() {
    use crate::tui::widgets::list::ItemList;
    use std::sync::RwLock;

    let ctx = app().ctx.clone();
    let mut list = ItemList::new(Arc::new(RwLock::new(tracks(10))));
    let mut terminal = Terminal::new(TestBackend::new(40, 12)).unwrap();
    // The list sits below two rows of something else, like a tab row, and above two more.
    let area = Rect::new(0, 2, 40, 8);
    terminal
        .draw(|frame| list.draw(frame, area, &ctx, true))
        .unwrap();
    list.select(2);
    list.mouse(click(5, 0), &ctx);
    assert_eq!(list.selected(), 2, "a click above the list");
    list.mouse(click(5, 11), &ctx);
    assert_eq!(list.selected(), 2, "a click below the list");
    list.mouse(click(5, 3), &ctx);
    assert_eq!(list.selected(), 1, "a click on the list's second row");
}

#[test]
fn test_clicking_an_empty_menu_row_keeps_the_selection() {
    use crate::tui::overlays::menu::{Entry, Menu};
    use crate::tui::page::Overlay;
    use std::cell::Cell;
    use std::rc::Rc;

    let ctx = app().ctx.clone();
    let picked = Rc::new(Cell::new(""));
    let entries = ["Rock", "Jazz", "Pop"]
        .into_iter()
        .map(|name| {
            let picked = picked.clone();
            Entry::new(name, move |_| {
                picked.set(name);
                crate::tui::page::Outcome::Consumed
            })
        })
        .collect();
    let mut menu = Menu::new("Add to playlist", entries).filterable();
    menu.key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE), &ctx);
    let mut terminal = Terminal::new(TestBackend::new(40, 20)).unwrap();
    terminal
        .draw(|frame| {
            menu.draw(frame, frame.area(), &ctx);
        })
        .unwrap();
    // Only Jazz is left; the rows the other two used are empty.
    let jazz = find(&terminal, "Jazz").expect("the remaining entry");
    menu.mouse(click(jazz.x, jazz.y + 1), &ctx);
    assert_eq!(picked.get(), "", "an empty row picked something");
    menu.command(&Command::Play, &ctx);
    assert_eq!(picked.get(), "Jazz", "the selection moved off Jazz");
}

#[test]
fn test_key_hints_label_whats_on_screen() {
    images::force_supported(false);
    let mut app = app();
    let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    assert!(!screen_text(&terminal).contains("Keys on screen"));

    // `;` shows them: the tabs' F-keys, Space on play/pause, how to see all keys.
    key(&mut app, KeyCode::Char(';'));
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let text = screen_text(&terminal);
    for label in [
        " F1 ",
        " F3 ",
        " Space ",
        "Keys on screen",
        "? for all keys",
    ] {
        assert!(text.contains(label), "{label:?} missing:\n{text}");
    }
    // Black on yellow, and nothing hides the player's icons for them.
    let space = find(&terminal, " Space ").unwrap();
    let cell = &terminal.backend().buffer()[(space.x + 1, space.y)];
    assert_eq!(cell.bg, ratatui::style::Color::Yellow);
    assert!(app.overlay_areas().is_empty());

    // The next key hides them.
    key(&mut app, KeyCode::Down);
    terminal.draw(|frame| app.draw(frame)).unwrap();
    assert!(!screen_text(&terminal).contains("Keys on screen"));

    // Holding Ctrl and Shift shows them until either is let go.
    use ratatui::crossterm::event::{KeyEventKind, ModifierKeyCode};
    app.modifier_key(ModifierKeyCode::LeftControl, KeyEventKind::Press);
    app.modifier_key(ModifierKeyCode::LeftShift, KeyEventKind::Press);
    terminal.draw(|frame| app.draw(frame)).unwrap();
    assert!(screen_text(&terminal).contains("let go to hide"));
    app.modifier_key(ModifierKeyCode::LeftShift, KeyEventKind::Release);
    terminal.draw(|frame| app.draw(frame)).unwrap();
    assert!(!screen_text(&terminal).contains("Keys on screen"));
}

#[test]
fn test_help_groups_keys_and_filters() {
    let mut app = app();
    let mut terminal = Terminal::new(TestBackend::new(90, 45)).unwrap();
    app.run(&Command::Help);
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let text = screen_text(&terminal);
    for expected in ["Playback", "Play / pause", "Space", "Getting around"] {
        assert!(text.contains(expected), "{expected:?} missing:\n{text}");
    }
    for c in "lyrics".chars() {
        key(&mut app, KeyCode::Char(c));
    }
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let text = screen_text(&terminal);
    assert!(text.contains("Show the lyrics"), "{text}");

    // The screens in their order.
    let mut app = self::app();
    app.run(&Command::Help);
    for c in "go to".chars() {
        key(&mut app, KeyCode::Char(c));
    }
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let text = screen_text(&terminal);
    let at = |s: &str| {
        text.find(s)
            .unwrap_or_else(|| panic!("{s:?} missing:\n{text}"))
    };
    assert!(at("Go to Home") < at("Go to Search"));
    assert!(at("Go to Search") < at("Go to Library"));
    assert!(at("Go to Lyrics") < at("Go to Stats"));
    assert!(!text.contains("Play / pause"), "{text}");
}

#[test]
fn test_settings_change_tabs_home_and_keys() {
    use crate::config::LibraryTab;

    let mut app = app();
    let mut terminal = Terminal::new(TestBackend::new(100, 60)).unwrap();
    key(&mut app, KeyCode::Char(','));
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let text = screen_text(&terminal);
    for expected in [
        "Settings",
        "Library tabs",
        "[✓] Podcasts",
        "Pick up where you left off",
    ] {
        assert!(text.contains(expected), "{expected:?} missing:\n{text}");
    }

    // Podcasts is the fifth tab: turn it off.
    for _ in 0..4 {
        key(&mut app, KeyCode::Down);
    }
    key(&mut app, KeyCode::Enter);
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let tabs = app.ctx.config.values().library_tabs.clone().unwrap();
    assert!(!tabs.contains(&LibraryTab::Podcasts), "{tabs:?}");
    assert!(screen_text(&terminal).contains("[ ] Podcasts"));

    // Then the Home card, two rows on.
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Enter);
    assert_eq!(
        app.ctx.config.values().pick_up_where_you_left_off,
        Some(false)
    );

    // Rebind the first key (in Playback): Enter, then the new key.
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Char('y'));
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let bindings = app.ctx.config.values().keybindings.clone().unwrap();
    assert!(bindings.contains_key("y"), "{bindings:?}");
    assert!(screen_text(&terminal).contains("y now does this"));
}

#[test]
fn test_stats_screen_shows_listening_and_tabs() {
    let mut app = app();
    let mut terminal = Terminal::new(TestBackend::new(110, 50)).unwrap();
    key(&mut app, KeyCode::F(6));
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let text = screen_text(&terminal);
    for expected in ["Stats", "Listening", "Top: 4 weeks", "All time", "Library"] {
        assert!(text.contains(expected), "{expected:?} missing:\n{text}");
    }
    // The library in numbers, on the last tab.
    for _ in 0..4 {
        app.run(&Command::Tab(true));
    }
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let text = screen_text(&terminal);
    assert!(text.contains("liked songs"), "{text}");
}

#[test]
fn test_key_list_scrolls_back_to_its_titles() {
    let mut app = app();
    let mut terminal = Terminal::new(TestBackend::new(90, 20)).unwrap();
    app.run(&Command::Help);
    for _ in 0..30 {
        key(&mut app, KeyCode::Down);
        terminal.draw(|frame| app.draw(frame)).unwrap();
    }
    assert!(!screen_text(&terminal).contains("Playback"));
    for _ in 0..30 {
        key(&mut app, KeyCode::Up);
        terminal.draw(|frame| app.draw(frame)).unwrap();
    }
    assert!(screen_text(&terminal).contains("Playback"));

    // The same in the settings, back to the first title.
    app.run(&Command::Back);
    key(&mut app, KeyCode::Char(','));
    for _ in 0..30 {
        key(&mut app, KeyCode::Down);
        terminal.draw(|frame| app.draw(frame)).unwrap();
    }
    for _ in 0..30 {
        key(&mut app, KeyCode::Up);
        terminal.draw(|frame| app.draw(frame)).unwrap();
    }
    assert!(screen_text(&terminal).contains("Library tabs"));
}
