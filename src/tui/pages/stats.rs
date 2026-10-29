//! The Stats screen: your listening as ttunes noted it (time listened, streak, when in the day,
//! top songs and artists), your top artists and songs as Spotify counts them, and your library
//! in numbers.

use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use chrono::Local;
use ratatui::Frame;
use ratatui::buffer::{Buffer, Cell};
use ratatui::crossterm::event::{MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use ratatui::style::Color;
use rspotify::model::TimeRange;

use crate::command::{Command, MoveAmount, MoveMode};
use crate::model::track::Track;
use crate::stats::{self, Summary, hours_minutes};
use crate::tui::page::{Ctx, Outcome, Page};
use crate::tui::style::{Styles, bold, put, truncate, width};
use crate::tui::widgets::list::ItemList;
use crate::tui::widgets::tabs::Tabbed;

/// Top songs and artists shown.
const TOP: usize = 10;
/// Top songs and artists asked of Spotify.
const SPOTIFY_TOP: u32 = 50;
/// How long the listening summary is shown before it's worked out again.
const REFRESH: Duration = Duration::from_secs(30);
/// Rows a tab may draw, at most; what's below the screen is scrolled to.
const MAX_ROWS: u16 = 200;
/// Rows scrolled by the mouse wheel, as in lists.
const WHEEL_ROWS: u16 = 3;

pub fn new() -> Tabbed {
    let mut tabbed = Tabbed::new("Stats");
    tabbed.add_tab("Listening", Box::new(Listening::default()));
    tabbed.add_tab("Top: 4 weeks", Box::new(Top::new(TimeRange::ShortTerm)));
    tabbed.add_tab("6 months", Box::new(Top::new(TimeRange::MediumTerm)));
    tabbed.add_tab("All time", Box::new(Top::new(TimeRange::LongTerm)));
    tabbed.add_tab("Library", Box::new(LibraryNumbers::default()));
    tabbed
}

/// Big numbers side by side, each over a word on what it is: (number, word). Returns the rows
/// used.
fn draw_tiles(buf: &mut Buffer, area: Rect, tiles: &[(String, String)], styles: &Styles) -> u16 {
    if area.height < 2 || tiles.is_empty() {
        return 0;
    }
    let widest = tiles
        .iter()
        .map(|(n, w)| width(n).max(width(w)))
        .max()
        .unwrap_or(0) as u16;
    let tile_width = (widest + 4).max(14);
    let per_row = (area.width.saturating_sub(2) / tile_width).max(1) as usize;
    let mut y = area.y;
    for row in tiles.chunks(per_row) {
        if y + 1 >= area.bottom() {
            break;
        }
        for (i, (number, word)) in row.iter().enumerate() {
            let x = area.x + 2 + i as u16 * tile_width;
            let room = usize::from(tile_width - 2);
            put(buf, x, y, number, room, bold(styles.accent()));
            put(buf, x, y + 1, word, room, styles.secondary());
        }
        y += 3;
    }
    y - area.y
}

/// How far a tab taller than the screen is scrolled.
#[derive(Default)]
struct Scroll {
    offset: u16,
    /// As of the last draw: the furthest it scrolls, and the rows shown.
    max: u16,
    height: u16,
}

impl Scroll {
    /// Draw `content` in `area`, from `offset` rows down. `content` draws into a buffer as tall as
    /// it may need ([MAX_ROWS]) and returns the rows it used.
    fn draw(
        &mut self,
        buf: &mut Buffer,
        area: Rect,
        content: impl FnOnce(&mut Buffer, Rect) -> u16,
    ) {
        let tall = Rect::new(area.x, 0, area.width, MAX_ROWS);
        let mut drawn = Buffer::empty(tall);
        let used = content(&mut drawn, tall).min(MAX_ROWS);
        self.height = area.height;
        self.max = used.saturating_sub(area.height);
        self.offset = self.offset.min(self.max);
        for row in 0..area.height.min(used - self.offset) {
            for x in area.left()..area.right() {
                let from = &drawn[(x, self.offset + row)];
                if *from == Cell::EMPTY {
                    continue;
                }
                // Over the page's background, as if drawn there.
                let to = &mut buf[(x, area.y + row)];
                to.set_symbol(from.symbol());
                to.fg = from.fg;
                to.modifier = from.modifier;
                if from.bg != Color::Reset {
                    to.bg = from.bg;
                }
            }
        }
    }

    /// Scroll for up and down moves and the mouse wheel; anything else is ignored.
    fn command(&mut self, cmd: &Command) -> Outcome {
        let rows = |amount: &MoveAmount| match amount {
            MoveAmount::Integer(n) => u16::try_from(n.unsigned_abs()).unwrap_or(u16::MAX),
            MoveAmount::Float(scale) => ((f32::from(self.height) * scale) as u16).max(1),
            MoveAmount::Extreme => u16::MAX,
        };
        match cmd {
            Command::Move(MoveMode::Up, amount) => self.by(-i32::from(rows(amount))),
            Command::Move(MoveMode::Down, amount) => self.by(i32::from(rows(amount))),
            _ => return Outcome::Ignored,
        }
        Outcome::Consumed
    }

    fn mouse(&mut self, event: MouseEvent) -> Outcome {
        match event.kind {
            MouseEventKind::ScrollUp => self.by(-i32::from(WHEEL_ROWS)),
            MouseEventKind::ScrollDown => self.by(i32::from(WHEEL_ROWS)),
            _ => return Outcome::Ignored,
        }
        Outcome::Consumed
    }

    fn by(&mut self, rows: i32) {
        let offset = (i32::from(self.offset) + rows).clamp(0, i32::from(self.max));
        self.offset = offset as u16;
    }
}

/// A section title at `y`.
fn title(buf: &mut Buffer, area: Rect, y: u16, text: &str, styles: &Styles) {
    if y < area.bottom() {
        put(
            buf,
            area.x + 2,
            y,
            text,
            usize::from(area.width),
            bold(styles.accent()),
        );
    }
}

/// What ttunes noted of the listening.
#[derive(Default)]
struct Listening {
    summary: Option<(Summary, Instant)>,
    scroll: Scroll,
}

impl Listening {
    /// Work the summary out again if it's been shown a while.
    fn refresh(&mut self) {
        let stale = self
            .summary
            .as_ref()
            .is_none_or(|(_, at)| at.elapsed() > REFRESH);
        if stale {
            let summary = stats::summarize(&stats::load(), Local::now(), TOP);
            self.summary = Some((summary, Instant::now()));
        }
    }
}

/// Bars for listening by hour, `rows` tall, under a scale of hours.
fn draw_hours(
    buf: &mut Buffer,
    area: Rect,
    y: u16,
    hours: &[Duration; 24],
    styles: &Styles,
) -> u16 {
    const LEVELS: [&str; 8] = ["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];
    let rows: u16 = 4;
    if y + rows + 1 >= area.bottom() {
        return 0;
    }
    let most = hours
        .iter()
        .max()
        .copied()
        .unwrap_or_default()
        .as_secs()
        .max(1);
    // Two columns per hour, where there's room.
    let column = if area.width >= 24 * 3 + 4 { 3 } else { 2 };
    for (hour, time) in hours.iter().enumerate() {
        // Eighths of a cell, at least one for any listening.
        let mut eighths = (time.as_secs() * u64::from(rows) * 8).div_ceil(most) as u16;
        if time.is_zero() {
            eighths = 0;
        }
        let x = area.x + 2 + hour as u16 * column;
        for row in 0..rows {
            let cell_y = y + rows - 1 - row;
            let filled = eighths.saturating_sub(row * 8).min(8);
            let symbol = match filled {
                0 if row == 0 => "·",
                0 => continue,
                n => LEVELS[usize::from(n) - 1],
            };
            let bar = symbol.repeat(usize::from(column - 1));
            put(buf, x, cell_y, &bar, usize::from(column), styles.accent());
        }
        if hour % 6 == 0 {
            put(buf, x, y + rows, &hour.to_string(), 2, styles.secondary());
        }
    }
    rows + 1
}

impl Page for Listening {
    fn title(&self) -> String {
        "Listening".into()
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect, ctx: &Ctx, _focused: bool) {
        let styles = Styles(&ctx.palette);
        self.refresh();
        let summary = &self.summary.as_ref().unwrap().0;
        self.scroll.draw(frame.buffer_mut(), area, |buf, area| {
            draw_listening(buf, area, summary, &styles)
        });
    }

    fn command(&mut self, cmd: &Command, _ctx: &Ctx) -> Outcome {
        self.scroll.command(cmd)
    }

    fn mouse(&mut self, event: MouseEvent, _ctx: &Ctx) -> Outcome {
        self.scroll.mouse(event)
    }

    fn on_show(&mut self, _ctx: &Ctx) {
        self.summary = None;
    }
}

/// The listening summary, from the top of `area`. Returns the rows used.
fn draw_listening(buf: &mut Buffer, area: Rect, summary: &Summary, styles: &Styles) -> u16 {
    if summary.plays == 0 {
        put(
            buf,
            area.x + 2,
            area.y,
            "ttunes keeps your listening history from now on: play some music and come back.",
            usize::from(area.width.saturating_sub(4)),
            styles.secondary(),
        );
        return 1;
    }
    let since = summary
        .since
        .map(|day| format!("since {}", stats::short_date(day)))
        .unwrap_or_default();
    let days = if summary.streak == 1 { "day" } else { "days" };
    let tiles = [
        (hours_minutes(summary.today), "today".to_string()),
        (hours_minutes(summary.week), "last 7 days".to_string()),
        (format!("{} {days}", summary.streak), "streak".to_string()),
        (hours_minutes(summary.total), format!("in all, {since}")),
        (summary.plays.to_string(), "songs played".to_string()),
    ];
    let mut y = area.y + draw_tiles(buf, area, &tiles, styles);

    title(buf, area, y, "When you listen", styles);
    y += 1;
    y += draw_hours(buf, area, y, &summary.hours, styles) + 1;

    // Top songs and artists side by side where there's room, else one under the other.
    let side_by_side = area.width >= 90;
    let half = if side_by_side {
        area.width / 2
    } else {
        area.width
    };
    let songs: Vec<String> = summary
        .top_songs
        .iter()
        .map(|(title, artists, plays)| {
            let times = if *plays == 1 { "play" } else { "plays" };
            format!("{title} · {artists} · {plays} {times}")
        })
        .collect();
    let artists: Vec<String> = summary
        .top_artists
        .iter()
        .map(|(name, time)| format!("{name} · {}", hours_minutes(*time)))
        .collect();
    let columns = [
        ("Top songs, last 30 days", songs),
        ("Top artists, last 30 days", artists),
    ];
    let mut column_y = y;
    for (i, (heading, lines)) in columns.iter().enumerate() {
        let (x, top) = if side_by_side {
            (area.x + i as u16 * half, y)
        } else {
            (area.x, column_y)
        };
        let column = Rect::new(x, top, half, area.bottom().saturating_sub(top));
        title(buf, column, top, heading, styles);
        for (n, line) in lines.iter().enumerate() {
            let row = top + 1 + n as u16;
            if row >= area.bottom() {
                break;
            }
            put(
                buf,
                x + 2,
                row,
                &format!("{:>2}", n + 1),
                2,
                styles.secondary(),
            );
            let text = truncate(line, usize::from(half.saturating_sub(8)));
            put(buf, x + 5, row, &text, usize::from(half), styles.primary());
        }
        column_y = top + lines.len() as u16 + 2;
    }
    let bottom = if side_by_side {
        let longest = columns.iter().map(|(_, lines)| lines.len()).max();
        y + 1 + longest.unwrap_or(0) as u16
    } else {
        column_y.saturating_sub(1)
    };
    bottom - area.y
}

/// The user's top artists and songs over a time range, as Spotify counts them. The songs are a
/// list to play from.
struct Top {
    range: TimeRange,
    artists: Arc<RwLock<Vec<String>>>,
    songs: ItemList<Track>,
    /// Where the artists' names were drawn last, if they were (beside the songs, where there's
    /// room), and how far they're scrolled: the mouse wheel scrolls them there.
    artists_area: Option<Rect>,
    artists_scroll: Scroll,
    /// Loading has started; it's done once this is set.
    started: bool,
    loaded: Arc<RwLock<bool>>,
}

impl Top {
    fn new(range: TimeRange) -> Self {
        let source = match range {
            TimeRange::ShortTerm => "Top songs, 4 weeks",
            TimeRange::MediumTerm => "Top songs, 6 months",
            TimeRange::LongTerm => "Top songs, all time",
        };
        Self {
            range,
            artists: Arc::default(),
            songs: ItemList::new(Arc::default()).numbered().with_source(source),
            artists_area: None,
            artists_scroll: Scroll::default(),
            started: false,
            loaded: Arc::default(),
        }
    }

    /// Ask Spotify, in the background, the first time it's shown.
    fn load(&mut self, ctx: &Ctx) {
        if std::mem::replace(&mut self.started, true) {
            return;
        }
        let api = ctx.spotify.api.clone();
        let range = self.range;
        let artists = self.artists.clone();
        let songs = self.songs.content().clone();
        let loaded = self.loaded.clone();
        std::thread::spawn(move || {
            if let Ok(top) = api.top_artists(range, SPOTIFY_TOP) {
                *artists.write().unwrap() = top.into_iter().map(|(a, _)| a.name).collect();
            }
            if let Ok(top) = api.top_tracks(range, SPOTIFY_TOP) {
                *songs.write().unwrap() = top;
            }
            *loaded.write().unwrap() = true;
            crate::tui::request_redraw();
        });
    }
}

impl Page for Top {
    fn title(&self) -> String {
        "Top".into()
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect, ctx: &Ctx, focused: bool) {
        self.load(ctx);
        let styles = Styles(&ctx.palette);
        let loaded = *self.loaded.read().unwrap();
        if self.songs.is_empty() {
            let text = if loaded {
                "Spotify has no top songs for you over this time yet"
            } else {
                "Loading…"
            };
            put(
                frame.buffer_mut(),
                area.x + 2,
                area.y,
                text,
                usize::from(area.width),
                styles.secondary(),
            );
            return;
        }
        // Artists in a column at the left where there's room; the songs beside them.
        let artists = self.artists.read().unwrap();
        let (artists_area, songs_area) = if area.width >= 100 && !artists.is_empty() {
            let w = 32;
            (
                Some(Rect::new(area.x, area.y, w, area.height)),
                Rect::new(area.x + w, area.y, area.width - w, area.height),
            )
        } else {
            (None, area)
        };
        self.artists_area = None;
        if let Some(column) = artists_area {
            let buf = frame.buffer_mut();
            title(buf, column, column.y, "Top artists", &styles);
            // The names scroll under the title.
            let names = Rect {
                y: column.y + 1,
                height: column.height.saturating_sub(1),
                ..column
            };
            self.artists_area = Some(names);
            self.artists_scroll.draw(buf, names, |buf, names| {
                for (n, name) in artists.iter().enumerate() {
                    let y = names.y + n as u16;
                    put(
                        buf,
                        names.x + 2,
                        y,
                        &format!("{:>2}", n + 1),
                        2,
                        styles.secondary(),
                    );
                    put(
                        buf,
                        names.x + 5,
                        y,
                        name,
                        usize::from(names.width.saturating_sub(7)),
                        styles.primary(),
                    );
                }
                artists.len() as u16
            });
        }
        drop(artists);
        self.songs.draw(frame, songs_area, ctx, focused);
    }

    fn command(&mut self, cmd: &Command, ctx: &Ctx) -> Outcome {
        self.songs.command(cmd, ctx)
    }

    fn mouse(&mut self, event: MouseEvent, ctx: &Ctx) -> Outcome {
        let over_artists = self
            .artists_area
            .is_some_and(|area| area.contains((event.column, event.row).into()));
        if over_artists {
            return self.artists_scroll.mouse(event);
        }
        self.songs.mouse(event, ctx)
    }
}

/// The library in numbers.
#[derive(Default)]
struct LibraryNumbers {
    scroll: Scroll,
}

impl Page for LibraryNumbers {
    fn title(&self) -> String {
        "Library".into()
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect, ctx: &Ctx, _focused: bool) {
        let styles = Styles(&ctx.palette);
        let library = &ctx.library;
        let liked = library.tracks.read().unwrap();
        let liked_time: Duration = liked
            .iter()
            .map(|track| Duration::from_millis(u64::from(track.duration)))
            .sum();
        let artists: std::collections::HashSet<&String> =
            liked.iter().flat_map(|t| t.artists.iter()).collect();
        let tiles = [
            (liked.len().to_string(), "liked songs".to_string()),
            (hours_minutes(liked_time), "of liked songs".to_string()),
            (artists.len().to_string(), "artists in them".to_string()),
            (
                library.albums.read().unwrap().len().to_string(),
                "saved albums".to_string(),
            ),
            (
                library.playlists.read().unwrap().len().to_string(),
                "playlists".to_string(),
            ),
            (
                library.artists.read().unwrap().len().to_string(),
                "followed artists".to_string(),
            ),
            (
                library.shows.read().unwrap().len().to_string(),
                "podcasts".to_string(),
            ),
        ];
        self.scroll.draw(frame.buffer_mut(), area, |buf, area| {
            draw_tiles(buf, area, &tiles, &styles)
        });
    }

    fn command(&mut self, cmd: &Command, _ctx: &Ctx) -> Outcome {
        self.scroll.command(cmd)
    }

    fn mouse(&mut self, event: MouseEvent, _ctx: &Ctx) -> Outcome {
        self.scroll.mouse(event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn test_listening_draws_summary() {
        let now = Local.with_ymd_and_hms(2026, 10, 18, 21, 0, 0).unwrap();
        let plays: Vec<stats::Play> = (0..30)
            .map(|i| stats::Play {
                at: (now - chrono::Duration::hours(i * 5)).timestamp(),
                ms: 200_000,
                uri: format!("spotify:track:{}", i % 4),
                title: ["Misery Business", "Decode", "good 4 u", "Ignorance"][i as usize % 4]
                    .into(),
                artists: vec![
                    ["Paramore", "Paramore", "Olivia Rodrigo", "Paramore"][i as usize % 4].into(),
                ],
            })
            .collect();
        let mut page = Listening {
            summary: Some((stats::summarize(&plays, now, TOP), Instant::now())),
            scroll: Scroll::default(),
        };
        let ctx = crate::tui::tests::app().ctx;
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal
            .draw(|frame| page.draw(frame, frame.area(), &ctx, true))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let text: String = (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
                    + "\n"
            })
            .collect();
        for expected in [
            "today",
            "streak",
            "When you listen",
            "Misery Business",
            "Paramore",
        ] {
            assert!(text.contains(expected), "{expected:?} missing:\n{text}");
        }
    }

    #[test]
    fn test_top_artists_scroll_under_the_pointer() {
        use ratatui::crossterm::event::KeyModifiers;

        let track: Track = serde_json::from_value(serde_json::json!({
            "id": "t", "uri": "spotify:track:t", "title": "A song", "track_number": 1,
            "disc_number": 1, "duration": 1000, "artists": [], "artist_ids": [],
            "album_artists": [], "url": "", "list_index": 0, "is_local": false,
        }))
        .unwrap();
        let mut page = Top::new(TimeRange::ShortTerm);
        page.started = true;
        *page.loaded.write().unwrap() = true;
        *page.artists.write().unwrap() = (1..=30).map(|n| format!("Artist {n}")).collect();
        *page.songs.content().write().unwrap() = vec![track];
        let ctx = crate::tui::tests::app().ctx;
        let mut terminal = Terminal::new(TestBackend::new(110, 10)).unwrap();
        let mut draw = |page: &mut Top| {
            terminal
                .draw(|frame| page.draw(frame, frame.area(), &ctx, true))
                .unwrap();
            let buffer = terminal.backend().buffer();
            (0..buffer.area.height)
                .map(|y| {
                    (0..buffer.area.width)
                        .map(|x| buffer[(x, y)].symbol())
                        .collect::<String>()
                        + "\n"
                })
                .collect::<String>()
        };
        let text = draw(&mut page);
        assert!(
            text.contains("Artist 1 ") && !text.contains("Artist 30"),
            "{text}"
        );
        let wheel = |row| MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 5,
            row,
            modifiers: KeyModifiers::NONE,
        };
        for _ in 0..10 {
            page.mouse(wheel(4), &ctx);
        }
        let text = draw(&mut page);
        assert!(
            text.contains("Top artists")
                && text.contains("Artist 30")
                && !text.contains("Artist 1 "),
            "{text}"
        );
    }

    #[test]
    fn test_listening_scrolls_to_what_doesnt_fit() {
        let now = Local.with_ymd_and_hms(2026, 10, 18, 21, 0, 0).unwrap();
        let plays: Vec<stats::Play> = (0..10)
            .map(|i| stats::Play {
                at: (now - chrono::Duration::hours(i)).timestamp(),
                ms: 200_000,
                uri: format!("spotify:track:{i}"),
                title: format!("Song {i}"),
                artists: vec!["Paramore".into()],
            })
            .collect();
        let mut page = Listening {
            summary: Some((stats::summarize(&plays, now, TOP), Instant::now())),
            scroll: Scroll::default(),
        };
        let ctx = crate::tui::tests::app().ctx;
        let mut terminal = Terminal::new(TestBackend::new(60, 10)).unwrap();
        let mut draw = |page: &mut Listening| {
            terminal
                .draw(|frame| page.draw(frame, frame.area(), &ctx, true))
                .unwrap();
            let buffer = terminal.backend().buffer();
            (0..buffer.area.height)
                .map(|y| {
                    (0..buffer.area.width)
                        .map(|x| buffer[(x, y)].symbol())
                        .collect::<String>()
                        + "\n"
                })
                .collect::<String>()
        };
        let text = draw(&mut page);
        assert!(
            text.contains("today") && !text.contains("Top artists"),
            "{text}"
        );
        let down = Command::Move(MoveMode::Down, MoveAmount::Extreme);
        assert!(!page.command(&down, &ctx).is_ignored());
        let text = draw(&mut page);
        assert!(
            text.contains("Top artists") && text.contains("Paramore") && !text.contains("today"),
            "{text}"
        );
        // And back up a row at a time.
        let up = Command::Move(MoveMode::Up, MoveAmount::default());
        for _ in 0..MAX_ROWS {
            page.command(&up, &ctx);
        }
        assert!(draw(&mut page).contains("today"));
    }
}
