//! An artist: their top tracks under a header with their picture, and their discography as
//! Spotify's apps group it (Albums, Singles and EPs, Compilations, Appears On), each a row of
//! covers, newest first. The discography is loaded in the background the first time its tab is
//! shown, a page per group, and more of a group as its row is scrolled towards the end.

use std::sync::{Arc, Mutex, RwLock};
use std::thread;

use ratatui::Frame;
use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

use crate::command::{Command, MoveMode};
use crate::model::artist::Artist;
use crate::model::discography::{Discography, ReleaseGroup, ReleasePage};
use crate::model::track::Track;
use crate::traits::OpenTarget;
use crate::tui::header::{HeaderAction, PageHeader};
use crate::tui::hints;
use crate::tui::page::{Ctx, Outcome, Page};
use crate::tui::style::{Styles, put};
use crate::tui::widgets::cards::{Card, CardGrid, CardLayout, Section};
use crate::tui::widgets::list::ItemList;
use crate::tui::widgets::tabs::draw_tab_row;

/// Loads the page of an artist's releases in a group that starts at an offset.
pub type FetchReleases = Arc<dyn Fn(ReleaseGroup, usize) -> Option<ReleasePage> + Send + Sync>;

/// How close to the end of a row the selection gets before more of it is loaded.
const LOAD_AHEAD: usize = 10;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    TopTracks,
    Discography,
}

const TABS: [(Tab, &str); 2] = [
    (Tab::TopTracks, "Top tracks"),
    (Tab::Discography, "Discography"),
];

pub struct ArtistPage {
    artist: Artist,
    top_tracks: ItemList<Track>,
    discography: Arc<Mutex<Discography>>,
    releases: CardGrid,
    /// `None` for artists without an ID, who have no discography to load.
    fetch: Option<FetchReleases>,
    /// Whether the discography has started loading.
    loading_started: bool,
    tab: Tab,
    tab_rects: Vec<Rect>,
}

impl ArtistPage {
    pub fn new(artist: Artist, ctx: &Ctx) -> Self {
        let tracks: Arc<RwLock<Vec<Track>>> = Arc::default();
        let mut fetch: Option<FetchReleases> = None;
        if let Some(id) = artist.id.clone() {
            let tracks = tracks.clone();
            let spotify = ctx.spotify.clone();
            let events = ctx.events.clone();
            let top_id = id.clone();
            thread::spawn(move || {
                if let Ok(top) = spotify.api.artist_top_tracks(&top_id) {
                    tracks.write().unwrap().extend(top);
                    events.trigger();
                }
            });
            let api = ctx.spotify.api.clone();
            fetch = Some(Arc::new(move |group, offset| {
                api.artist_releases(&id, group, u32::try_from(offset).ok()?)
            }));
        }
        Self::build(artist, tracks, fetch)
    }

    /// An artist page whose discography comes from `fetch`, and without top tracks.
    #[cfg(test)]
    pub fn new_for_test(artist: Artist, fetch: FetchReleases) -> Self {
        Self::build(artist, Arc::default(), Some(fetch))
    }

    fn build(
        artist: Artist,
        tracks: Arc<RwLock<Vec<Track>>>,
        fetch: Option<FetchReleases>,
    ) -> Self {
        let source = format!("{} top tracks", artist.name);
        let discography: Arc<Mutex<Discography>> = Arc::default();
        let releases = Self::release_grid(&artist, discography.clone());
        Self {
            top_tracks: ItemList::new(tracks)
                .numbered()
                .with_source(&source)
                .played_from(OpenTarget::Artist(artist.clone())),
            artist,
            discography,
            releases,
            fetch,
            loading_started: false,
            tab: Tab::TopTracks,
            tab_rects: Vec::new(),
        }
    }

    /// A row of covers per group of releases, leaving out the groups without any. "Loading…"
    /// until the first group with releases is in.
    fn release_grid(artist: &Artist, discography: Arc<Mutex<Discography>>) -> CardGrid {
        let source = Box::new(move || {
            let discography = discography.lock().unwrap();
            let shown = discography.shown();
            if shown.is_empty() && !discography.is_done() {
                return None;
            }
            Some(
                shown
                    .into_iter()
                    .map(|(group, releases)| {
                        let cards = releases
                            .iter()
                            .map(|release| {
                                let mut card = Card::album(release.album.clone());
                                card.subtitle = release.subtitle(group);
                                card
                            })
                            .collect();
                        Section::new(group.title(), cards)
                    })
                    .collect(),
            )
        });
        CardGrid::new(
            format!("{} discography", artist.name),
            CardLayout::Rows,
            source,
        )
    }

    /// Load the next page of `group` on a thread of its own, unless one is on its way already.
    fn load(&self, group: ReleaseGroup) {
        let Some(fetch) = self.fetch.clone() else {
            return;
        };
        let offset = {
            let mut discography = self.discography.lock().unwrap();
            let releases = discography.group_mut(group);
            if releases.loading || !releases.has_more() {
                return;
            }
            releases.loading = true;
            releases.listed
        };
        let discography = self.discography.clone();
        thread::spawn(move || {
            let page = fetch(group, offset);
            Self::loaded(&discography, group, page);
        });
    }

    /// File a page of `group` that was loaded (`None`: it couldn't be).
    fn loaded(discography: &Mutex<Discography>, group: ReleaseGroup, page: Option<ReleasePage>) {
        {
            let mut discography = discography.lock().unwrap();
            let releases = discography.group_mut(group);
            releases.loading = false;
            match page {
                Some(page) => releases.add(page),
                None => releases.failed = true,
            }
        }
        crate::tui::request_redraw();
    }

    /// Start loading the discography: the first page of each group, one after the other so the
    /// albums come first.
    fn start_loading(&mut self) {
        if self.loading_started {
            return;
        }
        self.loading_started = true;
        let Some(fetch) = self.fetch.clone() else {
            return;
        };
        {
            let mut discography = self.discography.lock().unwrap();
            for group in ReleaseGroup::ALL {
                discography.group_mut(group).loading = true;
            }
        }
        let discography = self.discography.clone();
        thread::spawn(move || {
            for group in ReleaseGroup::ALL {
                let page = fetch(group, 0);
                Self::loaded(&discography, group, page);
            }
        });
    }

    /// Load more of the selected row when the selection nears its end.
    fn load_more_if_needed(&self) {
        let (section, card) = self.releases.selection();
        let group = {
            let discography = self.discography.lock().unwrap();
            let Some(&(group, releases)) = discography.shown().get(section) else {
                return;
            };
            if card + LOAD_AHEAD < releases.len() {
                return;
            }
            group
        };
        self.load(group);
    }

    /// What's said in place of the discography when there's none to show.
    fn no_releases_message(&self) -> Option<&'static str> {
        if self.fetch.is_none() {
            return Some("No releases");
        }
        let discography = self.discography.lock().unwrap();
        if !discography.is_done() || !discography.shown().is_empty() {
            return None;
        }
        Some(if discography.failed() {
            "Couldn't load the releases"
        } else {
            "No releases"
        })
    }

    fn switch(&mut self, tab: Tab) {
        self.tab = tab;
        if tab == Tab::Discography {
            self.start_loading();
        }
    }
}

impl Page for ArtistPage {
    fn title(&self) -> String {
        self.artist.name.clone()
    }

    fn header(&self, ctx: &Ctx) -> Option<PageHeader> {
        Some(PageHeader {
            kind: "Artist".into(),
            title: self.artist.name.clone(),
            details: Vec::new(),
            cover_url: self.artist.image_url(),
            playable: true,
            liked: Some(ctx.library.is_followed_artist(&self.artist)),
        })
    }

    fn header_action(&mut self, action: HeaderAction, ctx: &Ctx) -> Outcome {
        match action {
            HeaderAction::Play | HeaderAction::Shuffle => {
                ctx.queue.set_shuffle(action == HeaderAction::Shuffle);
                self.top_tracks.play_all(ctx);
                Outcome::Consumed
            }
            HeaderAction::Like => {
                if ctx.library.is_followed_artist(&self.artist) {
                    ctx.library.unfollow_artist(&self.artist);
                } else {
                    ctx.library.follow_artist(&self.artist);
                }
                Outcome::Consumed
            }
        }
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect, ctx: &Ctx, focused: bool) {
        self.tab_rects.clear();
        if area.is_empty() {
            return;
        }
        // The tab row and a blank row under it, as in tabbed pages; in very short areas only the
        // tab row, and in a single row only the page.
        let tab_rows = match area.height {
            1 => 0,
            2 => 1,
            _ => 2,
        };
        if tab_rows > 0 {
            let labels: Vec<String> = TABS.iter().map(|(_, label)| (*label).into()).collect();
            let current = TABS.iter().position(|(t, _)| *t == self.tab).unwrap_or(0);
            let row = Rect::new(area.x, area.y, area.width, 1);
            self.tab_rects = draw_tab_row(frame, row, &labels, current, ctx);
            if focused {
                hints::add_row(row, vec![Command::Tab(false), Command::Tab(true)]);
            }
        }
        let inner = Rect::new(
            area.x,
            area.y + tab_rows,
            area.width,
            area.height - tab_rows,
        );
        match self.tab {
            Tab::TopTracks => self.top_tracks.draw(frame, inner, ctx, focused),
            Tab::Discography => {
                if let Some(message) = self.no_releases_message() {
                    put(
                        frame.buffer_mut(),
                        inner.x + 2,
                        inner.y,
                        message,
                        usize::from(inner.width.saturating_sub(2)),
                        Styles(&ctx.palette).secondary(),
                    );
                    return;
                }
                self.releases.draw(frame, inner, ctx, focused);
                self.load_more_if_needed();
            }
        }
    }

    fn command(&mut self, cmd: &Command, ctx: &Ctx) -> Outcome {
        match (cmd, self.tab) {
            (Command::Tab(next), _) => {
                self.switch(if *next {
                    Tab::Discography
                } else {
                    Tab::TopTracks
                });
                Outcome::Consumed
            }
            // Left and right switch tabs from the list; in the discography they move between
            // covers.
            (Command::Move(mode @ (MoveMode::Left | MoveMode::Right), _), Tab::TopTracks) => {
                if matches!(mode, MoveMode::Right) {
                    self.switch(Tab::Discography);
                }
                Outcome::Consumed
            }
            (_, Tab::TopTracks) => self.top_tracks.command(cmd, ctx),
            (_, Tab::Discography) => self.releases.command(cmd, ctx),
        }
    }

    fn mouse(&mut self, event: MouseEvent, ctx: &Ctx) -> Outcome {
        if let MouseEventKind::Down(MouseButton::Left) = event.kind
            && let Some(i) = self
                .tab_rects
                .iter()
                .position(|r| r.contains((event.column, event.row).into()))
        {
            self.switch(TABS[i].0);
            return Outcome::Consumed;
        }
        match self.tab {
            Tab::TopTracks => self.top_tracks.mouse(event, ctx),
            Tab::Discography => self.releases.mouse(event, ctx),
        }
    }

    fn handles_horizontal_moves(&self) -> bool {
        self.tab == Tab::Discography
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::KeyModifiers;

    use super::*;
    use crate::command::MoveAmount;
    use crate::model::discography::Release;
    use crate::model::discography::tests::release;
    use crate::tui::images;

    fn artist() -> Artist {
        Artist {
            id: None,
            name: "Some Artist".into(),
            url: None,
            tracks: None,
            is_followed: false,
        }
    }

    /// Releases by group, all on one page each; a group missing from `groups` fails to load.
    fn fetch_from(groups: Vec<(ReleaseGroup, Vec<Release>)>) -> FetchReleases {
        Arc::new(move |group, offset| {
            let releases = groups.iter().find(|(g, _)| *g == group)?.1.clone();
            let total = releases.len();
            Some(ReleasePage {
                releases: releases.into_iter().skip(offset).collect(),
                total,
            })
        })
    }

    fn discography() -> FetchReleases {
        fetch_from(vec![
            (
                ReleaseGroup::Albums,
                vec![
                    release("Newest Album", "2024-03-01", "album"),
                    // Its clean version.
                    release("Newest Album", "2024-03-01", "album"),
                    release("First Album", "2019-10-18", "album"),
                ],
            ),
            (ReleaseGroup::Singles, Vec::new()),
            (
                ReleaseGroup::Compilations,
                vec![release("The Best Of", "2022", "compilation")],
            ),
            (
                ReleaseGroup::AppearsOn,
                vec![release("Summer Hits", "2021-06-01", "compilation")],
            ),
        ])
    }

    fn text(terminal: &Terminal<TestBackend>) -> String {
        let buffer = terminal.backend().buffer();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
                    + "\n"
            })
            .collect()
    }

    fn draw(page: &mut ArtistPage, terminal: &mut Terminal<TestBackend>, ctx: &Ctx) {
        terminal
            .draw(|frame| page.draw(frame, frame.area(), ctx, true))
            .unwrap();
    }

    /// Draw until the discography has loaded (it loads on a thread of its own).
    fn draw_loaded(page: &mut ArtistPage, terminal: &mut Terminal<TestBackend>, ctx: &Ctx) {
        let start = Instant::now();
        loop {
            draw(page, terminal, ctx);
            let busy = {
                let discography = page.discography.lock().unwrap();
                !discography.is_done()
                    || ReleaseGroup::ALL
                        .iter()
                        .any(|&g| discography.group(g).loading)
            };
            if !busy || start.elapsed() > Duration::from_secs(5) {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        draw(page, terminal, ctx);
    }

    fn click(column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    /// Where `needle` was drawn: its column and row.
    fn find(terminal: &Terminal<TestBackend>, needle: &str) -> Option<(u16, u16)> {
        text(terminal).lines().enumerate().find_map(|(row, line)| {
            let byte = line.find(needle)?;
            Some((line[..byte].chars().count() as u16, row as u16))
        })
    }

    #[test]
    fn test_discography_shows_each_group_with_releases() {
        images::force_supported(false);
        let ctx = crate::tui::tests::app().ctx;
        let mut page = ArtistPage::new_for_test(artist(), discography());
        let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
        draw(&mut page, &mut terminal, &ctx);
        let shown = text(&terminal);
        assert!(
            shown.contains("Top tracks") && shown.contains("Discography"),
            "{shown}"
        );
        assert!(!page.loading_started, "loaded only once the tab is shown");

        page.command(&Command::Tab(true), &ctx);
        draw_loaded(&mut page, &mut terminal, &ctx);
        let shown = text(&terminal);
        for expected in [
            "Albums",
            "Newest Album",
            "2024 • Album",
            "First Album",
            "Compilations",
            "2022 • Compilat",
            "Appears On",
            "Summer Hits",
            "2021 • Some Art",
        ] {
            assert!(shown.contains(expected), "{expected:?} missing:\n{shown}");
        }
        // No singles: no row for them.
        assert!(!shown.contains("Singles and EPs"), "{shown}");
        // The clean version isn't shown again.
        assert_eq!(shown.matches("Newest Album").count(), 1, "{shown}");
        // Newest first.
        let newest = find(&terminal, "Newest Album").unwrap();
        let first = find(&terminal, "First Album").unwrap();
        assert!(newest.0 < first.0 && newest.1 == first.1, "{shown}");
    }

    #[test]
    fn test_clicking_or_entering_a_release_opens_it() {
        images::force_supported(false);
        let ctx = crate::tui::tests::app().ctx;
        let mut page = ArtistPage::new_for_test(artist(), discography());
        let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
        page.command(&Command::Tab(true), &ctx);
        draw_loaded(&mut page, &mut terminal, &ctx);
        let (x, y) = find(&terminal, "The Best Of").expect("the compilation");
        match page.mouse(click(x, y), &ctx) {
            Outcome::Open(opened) => assert_eq!(opened.title(), "The Best Of"),
            _ => panic!("clicking a release didn't open it"),
        }
        // Enter on the selected release: from the compilation clicked, up to the albums and on
        // to the second one.
        page.command(&Command::Move(MoveMode::Up, MoveAmount::Integer(1)), &ctx);
        page.command(
            &Command::Move(MoveMode::Right, MoveAmount::Integer(1)),
            &ctx,
        );
        match page.command(&Command::Play, &ctx) {
            Outcome::Open(opened) => assert_eq!(opened.title(), "First Album"),
            _ => panic!("Enter on a release didn't open it"),
        }
        // Clicking the tab goes back to the top tracks.
        let (x, y) = find(&terminal, "Top tracks").unwrap();
        page.mouse(click(x, y), &ctx);
        assert!(page.tab == Tab::TopTracks);
    }

    #[test]
    fn test_no_releases_or_failures_say_so() {
        images::force_supported(false);
        let ctx = crate::tui::tests::app().ctx;
        let mut terminal = Terminal::new(TestBackend::new(80, 20)).unwrap();
        let none = fetch_from(ReleaseGroup::ALL.iter().map(|&g| (g, Vec::new())).collect());
        let failing = fetch_from(Vec::new());
        for (fetch, message) in [
            (none, "No releases"),
            (failing, "Couldn't load the releases"),
        ] {
            let mut page = ArtistPage::new_for_test(artist(), fetch);
            page.command(&Command::Tab(true), &ctx);
            draw_loaded(&mut page, &mut terminal, &ctx);
            let shown = text(&terminal);
            assert!(shown.contains(message), "{message:?} missing:\n{shown}");
            // The top tracks are still there to go back to.
            page.command(&Command::Tab(false), &ctx);
            draw(&mut page, &mut terminal, &ctx);
            assert!(!text(&terminal).contains(message));
        }
        // Some groups failing: the others are shown.
        let some = fetch_from(vec![(
            ReleaseGroup::Singles,
            vec![release("A Single", "2023", "single")],
        )]);
        let mut page = ArtistPage::new_for_test(artist(), some);
        page.command(&Command::Tab(true), &ctx);
        draw_loaded(&mut page, &mut terminal, &ctx);
        let shown = text(&terminal);
        assert!(
            shown.contains("Singles and EPs") && shown.contains("2023 • Single"),
            "{shown}"
        );
    }

    #[test]
    fn test_more_releases_load_as_a_row_is_scrolled() {
        images::force_supported(false);
        let ctx = crate::tui::tests::app().ctx;
        let albums: Vec<Release> = (0..120)
            .map(|n| release(&format!("Album {n}"), &format!("{}", 2200 - n), "album"))
            .collect();
        let asked = Arc::new(Mutex::new(Vec::new()));
        let fetch: FetchReleases = {
            let asked = asked.clone();
            Arc::new(move |group, offset| {
                asked.lock().unwrap().push((group, offset));
                if group != ReleaseGroup::Albums {
                    return Some(ReleasePage {
                        releases: Vec::new(),
                        total: 0,
                    });
                }
                Some(ReleasePage {
                    releases: albums.iter().skip(offset).take(50).cloned().collect(),
                    total: albums.len(),
                })
            })
        };
        let mut page = ArtistPage::new_for_test(artist(), fetch);
        let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
        page.command(&Command::Tab(true), &ctx);
        draw_loaded(&mut page, &mut terminal, &ctx);
        let loaded = |page: &ArtistPage| page.discography.lock().unwrap().shown()[0].1.len();
        assert_eq!(loaded(&page), 50);
        for _ in 0..3 {
            page.command(&Command::Move(MoveMode::Right, MoveAmount::Extreme), &ctx);
            draw_loaded(&mut page, &mut terminal, &ctx);
        }
        assert_eq!(loaded(&page), 120);
        let albums_asked: Vec<usize> = asked
            .lock()
            .unwrap()
            .iter()
            .filter(|(g, _)| *g == ReleaseGroup::Albums)
            .map(|(_, offset)| *offset)
            .collect();
        assert_eq!(albums_asked, [0, 50, 100]);
    }

    #[test]
    fn test_tiny_sizes_dont_panic() {
        let ctx = crate::tui::tests::app().ctx;
        for images_on in [false, true] {
            images::force_supported(images_on);
            for (width, height) in [(1, 1), (2, 1), (5, 2), (10, 3), (20, 5), (30, 7), (80, 4)] {
                let mut page = ArtistPage::new_for_test(artist(), discography());
                let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                draw(&mut page, &mut terminal, &ctx);
                page.command(&Command::Tab(true), &ctx);
                draw_loaded(&mut page, &mut terminal, &ctx);
                for cmd in [
                    Command::Move(MoveMode::Down, MoveAmount::Integer(3)),
                    Command::Move(MoveMode::Right, MoveAmount::Extreme),
                    Command::Move(MoveMode::Up, MoveAmount::Extreme),
                    Command::Play,
                ] {
                    page.command(&cmd, &ctx);
                    draw(&mut page, &mut terminal, &ctx);
                }
                page.mouse(click(0, 0), &ctx);
                page.mouse(click(width - 1, height - 1), &ctx);
                draw(&mut page, &mut terminal, &ctx);
            }
        }
    }
}
