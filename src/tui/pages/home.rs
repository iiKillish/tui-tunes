//! The Home screen (design H3), like Spotify's: what you've been listening to.
//!
//! * At the top, a featured "Pick up where you left off" card: the first album, playlist or
//!   artist played from recently, with a ▶ Play button.
//! * Filter pills: All · Albums · Playlists · Artists · Songs (`h`/`l` or a click switch).
//! * One list of rows (2-row cover, title, "Album • Artist"): Jump back in (albums, playlists and
//!   artists played from recently), your top artists, recently played songs and your top songs.
//!
//! Everything is loaded from Spotify in the background, and again when the screen is shown after
//! [REFRESH_AFTER].

use std::cmp::min;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::{Duration, Instant};

use log::error;
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};
use ratatui::style::Style;

use crate::command::{Command, GotoMode, MoveAmount, MoveMode, TargetMode};
use crate::library::Library;
use crate::model::album::Album;
use crate::model::artist::Artist;
use crate::model::playable::Playable;
use crate::model::playlist::Playlist;
use crate::model::track::Track;
use crate::spotify::Spotify;
use crate::spotify_api::{PlayContext, RecentPlay, WebApi};
use crate::traits::{ListItem, OpenTarget};
use crate::tui::images;
use crate::tui::nav;
use crate::tui::page::{Ctx, Outcome, Page};
use crate::tui::pointer;
use crate::tui::style::{SELECTION_MARKER, Styles, bold, fill, put, scroll_to};

/// Items per kind loaded at most.
const MAX_ITEMS: usize = 20;
/// Home is reloaded when shown again after this long.
const REFRESH_AFTER: Duration = Duration::from_secs(10 * 60);
/// Rows of the featured card, with and without a cover.
const FEATURED_ROWS: u16 = 7;
const FEATURED_ROWS_TEXT: u16 = 5;
/// Rows per list item: title, subtitle, and a blank row.
const ITEM_ROWS: u16 = 3;

/// How many list items fit whole in `height` rows (the blank row after the last one isn't
/// needed).
fn items_fitting(height: u16) -> usize {
    usize::from((height + 1) / ITEM_ROWS)
}

/// The item drawn at `offset` rows into a list of `height` rows that starts at item `first`,
/// out of `len`; `None` for the blank rows between items and the space below the last one.
fn item_at(offset: u16, height: u16, first: usize, len: usize) -> Option<usize> {
    let slot = usize::from(offset / ITEM_ROWS);
    let index = first + slot;
    (offset % ITEM_ROWS != ITEM_ROWS - 1 && slot < items_fitting(height) && index < len)
        .then_some(index)
}
/// Left margin.
const MARGIN: u16 = 2;
/// Label of the featured card.
const FEATURED_LABEL: &str = "Pick up where you left off";
const PLAY_BUTTON: &str = " ▶ Play ";
/// Narrowest list that shows covers before its items.
const MIN_WIDTH_FOR_THUMBNAILS: u16 = 32;
/// Narrowest text beside the featured card's cover.
const FEATURED_MIN_TEXT_WIDTH: u16 = 24;

/// Something shown on Home.
#[derive(Clone)]
enum HomeItem {
    Album(Album),
    Playlist(Playlist),
    Artist(Artist),
    Song(Track),
}

impl HomeItem {
    fn kind(&self) -> Filter {
        match self {
            Self::Album(_) => Filter::Albums,
            Self::Playlist(_) => Filter::Playlists,
            Self::Artist(_) => Filter::Artists,
            Self::Song(_) => Filter::Songs,
        }
    }

    /// What tells two items apart, for leaving out duplicates.
    fn key(&self) -> (Filter, String) {
        let id = match self {
            Self::Album(album) => album.id.clone().unwrap_or_else(|| album.title.clone()),
            Self::Playlist(playlist) => playlist.id.clone(),
            Self::Artist(artist) => artist.id.clone().unwrap_or_else(|| artist.name.clone()),
            Self::Song(track) => track.id.clone().unwrap_or_else(|| track.uri.clone()),
        };
        (self.kind(), id)
    }

    fn title(&self) -> String {
        match self {
            Self::Album(album) => album.title.clone(),
            Self::Playlist(playlist) => playlist.name.clone(),
            Self::Artist(artist) => artist.name.clone(),
            Self::Song(track) => track.title.clone(),
        }
    }

    /// "Album • Green Day", "Playlist • iiKillish", "Artist", "Song • Paramore".
    fn subtitle(&self) -> String {
        match self {
            Self::Album(album) => format!("Album • {}", album.artists.join(", ")),
            Self::Playlist(playlist) => match &playlist.owner_name {
                Some(owner) => format!("Playlist • {owner}"),
                None => "Playlist".into(),
            },
            Self::Artist(_) => "Artist".into(),
            Self::Song(track) => format!("Song • {}", track.artists.join(", ")),
        }
    }

    /// The cover, or the artist's picture (looked up in the background the first time, so only
    /// for items on screen).
    fn image_url(&self) -> Option<String> {
        match self {
            Self::Album(album) => album.cover_url.clone(),
            Self::Playlist(playlist) => playlist.cover_url.clone(),
            Self::Artist(artist) => artist.image_url(),
            Self::Song(track) => track.cover_url.clone(),
        }
    }

    fn as_listitem(&self) -> Box<dyn ListItem> {
        match self {
            Self::Album(album) => album.as_listitem(),
            Self::Playlist(playlist) => playlist.as_listitem(),
            Self::Artist(artist) => artist.as_listitem(),
            Self::Song(track) => track.as_listitem(),
        }
    }
}

/// The filter pills. Also the kinds of [HomeItem].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Filter {
    All,
    Albums,
    Playlists,
    Artists,
    Songs,
}

impl Filter {
    const ALL: [Self; 5] = [
        Self::All,
        Self::Albums,
        Self::Playlists,
        Self::Artists,
        Self::Songs,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Albums => "Albums",
            Self::Playlists => "Playlists",
            Self::Artists => "Artists",
            Self::Songs => "Songs",
        }
    }

    /// Shown when nothing matches.
    fn empty_message(self) -> &'static str {
        match self {
            Self::All => "Nothing here yet: play some music first",
            Self::Albums => "No albums played recently",
            Self::Playlists => "No playlists played recently",
            Self::Artists => "No artists here yet",
            Self::Songs => "No songs here yet",
        }
    }

    fn position(self) -> usize {
        Self::ALL.iter().position(|f| *f == self).unwrap_or(0)
    }
}

/// What Home shows, as loaded from Spotify.
#[derive(Clone, Default)]
struct Sections {
    /// Albums, playlists and artists played from recently, most recent first.
    jump_back_in: Vec<HomeItem>,
    top_artists: Vec<Artist>,
    /// Recently played songs, most recent first, each once.
    recent: Vec<Track>,
    top_tracks: Vec<Track>,
}

impl Sections {
    fn is_empty(&self) -> bool {
        self.jump_back_in.is_empty()
            && self.top_artists.is_empty()
            && self.recent.is_empty()
            && self.top_tracks.is_empty()
    }

    /// The featured card's item.
    fn featured(&self) -> Option<&HomeItem> {
        self.jump_back_in.first()
    }

    /// The list's rows for `filter`, each item once. With `skip_featured`, the featured item is
    /// left out of All (it's right above).
    fn rows(&self, filter: Filter, skip_featured: bool) -> Vec<HomeItem> {
        let skip = if skip_featured { 1 } else { 0 };
        let all = self
            .jump_back_in
            .iter()
            .skip(if filter == Filter::All { skip } else { 0 })
            .cloned()
            .chain(self.top_artists.iter().cloned().map(HomeItem::Artist))
            .chain(self.recent.iter().cloned().map(HomeItem::Song))
            .chain(self.top_tracks.iter().cloned().map(HomeItem::Song));
        let mut seen = HashSet::new();
        if skip_featured
            && filter == Filter::All
            && let Some(featured) = self.featured()
        {
            seen.insert(featured.key());
        }
        all.filter(|item| filter == Filter::All || item.kind() == filter)
            .filter(|item| seen.insert(item.key()))
            .collect()
    }
}

/// Load what Home shows from Spotify. Slow (several requests): run in the background.
fn load(spotify: &Spotify, library: &Library) -> Sections {
    let api = &spotify.api;
    let plays = api.recently_played().unwrap_or_else(|()| {
        error!("couldn't load recently played tracks");
        Vec::new()
    });

    // Jump back in: what recent tracks were played from, or their album when that's unknown or
    // can't be loaded (Spotify's own mixes, like Daily Mix playlists, come through the session).
    let mut loaded: HashMap<(PlayContext, String), Option<HomeItem>> = HashMap::new();
    let mut shown = HashSet::new();
    let mut jump_back_in = Vec::new();
    for play in &plays {
        if jump_back_in.len() >= MAX_ITEMS {
            break;
        }
        let context_item = play.context.as_ref().and_then(|(kind, id)| {
            loaded
                .entry((*kind, id.clone()))
                .or_insert_with(|| load_context(api, library, play, *kind, id))
                .clone()
                .map(|item| ((*kind, id.clone()), item))
        });
        let item = context_item.or_else(|| {
            let id = play.album.id.clone()?;
            Some((
                (PlayContext::Album, id),
                HomeItem::Album(play.album.clone()),
            ))
        });
        if let Some((key, item)) = item
            && shown.insert(key)
        {
            jump_back_in.push(item);
        }
    }

    let mut seen = HashSet::new();
    let recent = plays
        .into_iter()
        .filter(|play| seen.insert(play.track.id.clone()))
        .take(MAX_ITEMS)
        .map(|play| play.track)
        .collect();
    let top_tracks = api.top_tracks(MAX_ITEMS as u32).unwrap_or_default();
    let top_artists = api
        .top_artists(MAX_ITEMS as u32)
        .unwrap_or_default()
        .into_iter()
        .map(|(artist, _)| artist)
        .collect();

    Sections {
        jump_back_in,
        top_artists,
        recent,
        top_tracks,
    }
}

/// The album, playlist or artist `play` was played from, if it can be loaded.
fn load_context(
    api: &WebApi,
    library: &Library,
    play: &RecentPlay,
    kind: PlayContext,
    id: &str,
) -> Option<HomeItem> {
    match kind {
        PlayContext::Album if play.album.id.as_deref() == Some(id) => {
            Some(HomeItem::Album(play.album.clone()))
        }
        PlayContext::Album => api.album(id).ok().map(|a| HomeItem::Album((&a).into())),
        PlayContext::Playlist => library
            .playlists
            .read()
            .unwrap()
            .iter()
            .find(|p| p.id == id)
            .cloned()
            .or_else(|| api.any_playlist(id))
            .map(HomeItem::Playlist),
        PlayContext::Artist => api.artist(id).ok().map(|a| HomeItem::Artist((&a).into())),
    }
}

/// What's selected.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Selection {
    Featured,
    Row(usize),
}

/// Where things were drawn last, for the mouse and page sizes.
#[derive(Default)]
struct Regions {
    featured: Option<Rect>,
    play_button: Option<Rect>,
    pills: Vec<(Rect, Filter)>,
    list: Rect,
}

pub struct HomePage {
    sections: Arc<RwLock<Option<Sections>>>,
    loaded_at: Option<Instant>,
    filter: Filter,
    selected: Selection,
    /// First list item shown.
    first: usize,
    regions: Regions,
    display_name: Option<String>,
}

pub fn new(ctx: &Ctx) -> HomePage {
    HomePage {
        sections: Arc::new(RwLock::new(None)),
        loaded_at: None,
        filter: Filter::All,
        selected: Selection::Featured,
        first: 0,
        regions: Regions::default(),
        display_name: ctx.library.display_name.clone(),
    }
}

/// Home already loaded with these items, for tests: albums, playlists and artists to jump back
/// into (the first album featured), the artists as top artists too, and recent songs.
#[cfg(test)]
pub fn new_for_test(
    ctx: &Ctx,
    albums: Vec<Album>,
    playlists: Vec<Playlist>,
    artists: Vec<Artist>,
    recent: Vec<Track>,
) -> HomePage {
    let jump_back_in = (albums.into_iter().map(HomeItem::Album))
        .chain(playlists.into_iter().map(HomeItem::Playlist))
        .chain(artists.iter().cloned().map(HomeItem::Artist))
        .collect();
    let mut page = new(ctx);
    page.loaded_at = Some(Instant::now());
    *page.sections.write().unwrap() = Some(Sections {
        jump_back_in,
        top_artists: artists,
        recent,
        top_tracks: Vec::new(),
    });
    page
}

impl HomePage {
    /// Load in the background if not loaded yet, or loaded a while ago.
    fn refresh_if_needed(&mut self, ctx: &Ctx) {
        if self
            .loaded_at
            .is_some_and(|at| at.elapsed() < REFRESH_AFTER)
        {
            return;
        }
        self.loaded_at = Some(Instant::now());
        let sections = self.sections.clone();
        let library = ctx.library.clone();
        let spotify = ctx.spotify.clone();
        let events = ctx.events.clone();
        thread::spawn(move || {
            let loaded = load(&spotify, &library);
            *sections.write().unwrap() = Some(loaded);
            events.trigger();
        });
    }

    /// Whether the featured card is shown (it is left out when there's no room).
    fn featured_shown(&self) -> bool {
        self.regions.featured.is_some()
    }

    /// The featured item, if it's shown.
    fn featured(&self) -> Option<HomeItem> {
        if !self.featured_shown() {
            return None;
        }
        self.sections
            .read()
            .unwrap()
            .as_ref()
            .and_then(|s| s.featured().cloned())
    }

    /// The list's rows for the current filter.
    fn rows(&self) -> Vec<HomeItem> {
        self.sections
            .read()
            .unwrap()
            .as_ref()
            .map(|s| s.rows(self.filter, self.featured_shown()))
            .unwrap_or_default()
    }

    fn selected_item(&self) -> Option<HomeItem> {
        match self.selected {
            Selection::Featured => self.featured(),
            Selection::Row(index) => self.rows().get(index).cloned(),
        }
    }

    /// List items that fit in the list's area (the last one's blank row may be cut off).
    fn visible_items(&self) -> usize {
        items_fitting(self.regions.list.height).max(1)
    }

    /// Selectable positions: the featured card (0, when shown), then the rows.
    fn position(&self) -> usize {
        match self.selected {
            Selection::Featured => 0,
            Selection::Row(index) => index + usize::from(self.featured_shown()),
        }
    }

    fn select_position(&mut self, position: usize, len: usize) {
        let featured = usize::from(self.featured_shown());
        let last = (len + featured).saturating_sub(1);
        let position = min(position, last);
        self.selected = if featured == 1 && position == 0 {
            Selection::Featured
        } else {
            Selection::Row(position - featured)
        };
    }

    fn set_filter(&mut self, filter: Filter) {
        if filter == self.filter {
            return;
        }
        self.filter = filter;
        self.first = 0;
        if self.selected != Selection::Featured {
            self.selected = Selection::Row(0);
        }
    }

    /// Open the item: its page, or for songs, a menu.
    fn open(&self, item: &HomeItem, ctx: &Ctx) -> Outcome {
        let listitem = item.as_listitem();
        match listitem.target() {
            Some(target) => Outcome::Open(nav::open_target(target, ctx)),
            None => Outcome::Overlay(nav::context_menu(listitem, ctx)),
        }
    }

    /// Play the item: an album, playlist or artist by itself, a song with the songs after it in
    /// the list following in the queue.
    fn play(&self, item: &HomeItem, ctx: &Ctx) {
        ctx.queue.clear();
        if let (HomeItem::Song(_), Selection::Row(index)) = (item, self.selected) {
            let rows = self.rows();
            let tracks: Vec<Playable> = rows
                .iter()
                .filter_map(|row| match row {
                    HomeItem::Song(track) => Some(Playable::Track(track.clone())),
                    _ => None,
                })
                .collect();
            let song_index = rows[..min(index, rows.len())]
                .iter()
                .filter(|row| matches!(row, HomeItem::Song(_)))
                .count();
            let first = ctx.queue.append_next_from(&tracks, Some("Home"));
            ctx.queue.play(first + song_index, true, false);
        } else {
            item.as_listitem().play(&ctx.queue);
        }
    }

    /// Enter, or a click: open albums, playlists and artists, play songs.
    fn activate(&self, ctx: &Ctx) -> Outcome {
        match self.selected_item() {
            Some(item @ HomeItem::Song(_)) => {
                self.play(&item, ctx);
                Outcome::Consumed
            }
            Some(item) => self.open(&item, ctx),
            None => Outcome::Consumed,
        }
    }

    fn draw_featured(
        &mut self,
        buf: &mut Buffer,
        area: Rect,
        item: &HomeItem,
        ctx: &Ctx,
        focused: bool,
    ) {
        let styles = Styles(&ctx.palette);
        let selected = self.selected == Selection::Featured;
        let bg = if selected {
            let style = if focused {
                styles.selected()
            } else {
                styles.selected_inactive()
            };
            fill(buf, area, style);
            style.bg.unwrap_or(ctx.palette.background)
        } else {
            ctx.palette.background
        };
        if selected && focused {
            for y in area.top()..area.bottom() {
                put(buf, area.x, y, SELECTION_MARKER, 1, styles.marker());
            }
        }

        let mut x = area.x + MARGIN;
        let cover = images::square_width(area.height);
        // Narrow windows leave the cover out rather than squeeze the text.
        if images::supported() && area.width >= MARGIN + cover + 2 + FEATURED_MIN_TEXT_WIDTH {
            if let Some(url) = item.image_url() {
                images::cover(&url, Rect::new(x, area.y, cover, area.height));
            }
            x += cover + 2;
        }
        let room = usize::from(area.right().saturating_sub(x + 1));
        // Text centered in the card: label, title, subtitle, then the button.
        let top = area.y + area.height.saturating_sub(FEATURED_ROWS_TEXT) / 2;
        put(buf, x, top, FEATURED_LABEL, room, styles.secondary().bg(bg));
        put(
            buf,
            x,
            top + 1,
            &item.title(),
            room,
            bold(styles.primary().bg(bg)),
        );
        put(
            buf,
            x,
            top + 2,
            &item.subtitle(),
            room,
            styles.secondary().bg(bg),
        );
        let written = put(buf, x, top + 4, PLAY_BUTTON, room, styles.button());
        self.regions.play_button = Some(Rect::new(x, top + 4, written as u16, 1));
    }

    fn draw_pills(&mut self, buf: &mut Buffer, area: Rect, ctx: &Ctx) {
        let styles = Styles(&ctx.palette);
        let mut x = area.x + MARGIN;
        for filter in Filter::ALL {
            let label = format!(" {} ", filter.label());
            let style = if filter == self.filter {
                styles.button()
            } else {
                styles.secondary()
            };
            let written = put(
                buf,
                x,
                area.y,
                &label,
                usize::from(area.right().saturating_sub(x)),
                style,
            );
            self.regions
                .pills
                .push((Rect::new(x, area.y, written as u16, 1), filter));
            x += written as u16 + 1;
        }
    }

    fn draw_rows(
        &mut self,
        buf: &mut Buffer,
        area: Rect,
        rows: &[HomeItem],
        ctx: &Ctx,
        focused: bool,
    ) {
        let styles = Styles(&ctx.palette);
        if rows.is_empty() {
            put(
                buf,
                area.x + MARGIN,
                area.y,
                self.filter.empty_message(),
                usize::from(area.width),
                styles.secondary(),
            );
            return;
        }
        let visible = self.visible_items();
        if let Selection::Row(index) = self.selected {
            self.first = scroll_to(self.first, index, visible);
        }
        self.first = min(self.first, rows.len().saturating_sub(visible));

        let thumb = if images::supported() && area.width >= MIN_WIDTH_FOR_THUMBNAILS {
            images::square_width(2)
        } else {
            0
        };
        let has_queue = ctx.queue.get_current_index().is_some();
        for (row, item) in rows
            .iter()
            .enumerate()
            .skip(self.first)
            .take(items_fitting(area.height))
        {
            let y = area.y + (row - self.first) as u16 * ITEM_ROWS;
            let lines = Rect::new(area.x, y, area.width, 2).intersection(area);
            pointer::clickable(lines);
            let selected = self.selected == Selection::Row(row);
            let style = if selected {
                if focused {
                    styles.selected()
                } else {
                    styles.selected_inactive()
                }
            } else {
                styles.primary()
            };
            let bg = style.bg.unwrap_or(ctx.palette.background);
            fill(buf, lines, style);
            if selected && focused {
                for y in lines.top()..lines.bottom() {
                    put(buf, area.x, y, SELECTION_MARKER, 1, styles.marker());
                }
            }

            let mut x = area.x + MARGIN;
            if thumb > 0 {
                if lines.height == 2
                    && let Some(url) = item.image_url()
                {
                    images::cover(&url, Rect::new(x, y, thumb, 2));
                }
                x += thumb + 2;
            }
            let room = usize::from(area.right().saturating_sub(x + 1));
            let playing = has_queue
                && matches!(item, HomeItem::Song(_))
                && item.as_listitem().is_playing(&ctx.queue);
            let title_style: Style = if playing {
                styles.accent().bg(bg)
            } else {
                style
            };
            put(buf, x, y, &item.title(), room, bold(title_style));
            if lines.height == 2 {
                put(
                    buf,
                    x,
                    y + 1,
                    &item.subtitle(),
                    room,
                    styles.secondary().bg(bg),
                );
            }
        }
    }

    /// Scroll the list by `delta` items, keeping the selection on screen.
    fn scroll(&mut self, delta: isize) {
        let len = self.rows().len();
        let visible = self.visible_items();
        self.first = self
            .first
            .saturating_add_signed(delta)
            .min(len.saturating_sub(visible));
        if let Selection::Row(index) = self.selected {
            let last = self.first + visible.saturating_sub(1);
            self.selected = Selection::Row(index.clamp(self.first, last.max(self.first)));
        }
    }
}

impl Page for HomePage {
    fn title(&self) -> String {
        "Home".into()
    }

    fn subtitle(&self) -> String {
        match &self.display_name {
            Some(name) => format!("Hi, {name}"),
            None => String::new(),
        }
    }

    fn on_show(&mut self, ctx: &Ctx) {
        self.refresh_if_needed(ctx);
    }

    fn handles_horizontal_moves(&self) -> bool {
        true
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect, ctx: &Ctx, focused: bool) {
        self.refresh_if_needed(ctx);
        self.regions = Regions::default();
        let styles = Styles(&ctx.palette);
        let buf = frame.buffer_mut();
        let sections = self.sections.read().unwrap().clone();
        let Some(sections) = sections else {
            put(
                buf,
                area.x + MARGIN,
                area.y + 1,
                "Loading…",
                usize::from(area.width),
                styles.secondary(),
            );
            return;
        };
        if sections.is_empty() {
            put(
                buf,
                area.x + MARGIN,
                area.y + 1,
                Filter::All.empty_message(),
                usize::from(area.width),
                styles.secondary(),
            );
            return;
        }

        let mut y = area.y;
        let card_rows = if images::supported() {
            FEATURED_ROWS
        } else {
            FEATURED_ROWS_TEXT
        };
        if let Some(featured) = sections.featured()
            && area.height >= card_rows + 1 + 2 + 2 * ITEM_ROWS
        {
            let card = Rect::new(area.x, y, area.width, card_rows);
            pointer::clickable(card);
            self.regions.featured = Some(card);
            self.draw_featured(buf, card, featured, ctx, focused);
            y += card_rows + 1;
        } else if self.selected == Selection::Featured {
            self.selected = Selection::Row(0);
        }

        if y < area.bottom() {
            self.draw_pills(buf, Rect::new(area.x, y, area.width, 1), ctx);
            for (pill, _) in &self.regions.pills {
                pointer::clickable(*pill);
            }
            y += 2;
        }

        let rows = sections.rows(self.filter, self.featured_shown());
        if let Selection::Row(index) = self.selected {
            if rows.is_empty() && self.featured_shown() {
                self.selected = Selection::Featured;
            } else {
                self.selected = Selection::Row(min(index, rows.len().saturating_sub(1)));
            }
        }
        let list = Rect::new(area.x, y, area.width, area.bottom().saturating_sub(y));
        self.regions.list = list;
        if !list.is_empty() {
            self.draw_rows(buf, list, &rows, ctx, focused);
        }
    }

    fn command(&mut self, cmd: &Command, ctx: &Ctx) -> Outcome {
        let step = |amount: &MoveAmount, page: usize| match amount {
            MoveAmount::Integer(n) => n.unsigned_abs() as usize,
            MoveAmount::Float(scale) => (page as f32 * scale) as usize,
            MoveAmount::Extreme => usize::MAX,
        };
        match cmd {
            Command::Move(MoveMode::Up, amount) => {
                let len = self.rows().len();
                let position = self
                    .position()
                    .saturating_sub(step(amount, self.visible_items()));
                self.select_position(position, len);
            }
            Command::Move(MoveMode::Down, amount) => {
                let len = self.rows().len();
                let position = self
                    .position()
                    .saturating_add(step(amount, self.visible_items()));
                self.select_position(position, len);
            }
            Command::Move(MoveMode::Left, _) => {
                let position = self.filter.position().saturating_sub(1);
                self.set_filter(Filter::ALL[position]);
            }
            Command::Move(MoveMode::Right, _) => {
                let position = min(self.filter.position() + 1, Filter::ALL.len() - 1);
                self.set_filter(Filter::ALL[position]);
            }
            Command::Play => return self.activate(ctx),
            Command::Queue | Command::PlayNext => {
                if let Some(item) = self.selected_item() {
                    nav::queue_item(item.as_listitem(), matches!(cmd, Command::PlayNext), ctx);
                }
            }
            Command::Save => {
                if let Some(item) = self.selected_item() {
                    item.as_listitem().save(&ctx.library);
                }
            }
            Command::Add => {
                if let Some(HomeItem::Song(track)) = self.selected_item() {
                    return Outcome::Overlay(nav::add_to_playlist(vec![track], ctx));
                }
            }
            Command::Open(TargetMode::Selected) => {
                if let Some(item) = self.selected_item() {
                    return self.open(&item, ctx);
                }
            }
            Command::Goto(mode) => {
                let Some(item) = self.selected_item() else {
                    return Outcome::Consumed;
                };
                let item = item.as_listitem();
                return match mode {
                    GotoMode::Album => match item.album(&ctx.queue) {
                        Some(album) => {
                            Outcome::Open(nav::open_target(OpenTarget::Album(album), ctx))
                        }
                        None => Outcome::Consumed,
                    },
                    GotoMode::Artist => match item.artists() {
                        Some(artists) => nav::go_to_artists(artists, ctx),
                        None => Outcome::Consumed,
                    },
                };
            }
            #[cfg(feature = "share_clipboard")]
            Command::Share(TargetMode::Selected) => {
                if let Some(url) = self
                    .selected_item()
                    .and_then(|i| i.as_listitem().share_url())
                {
                    let _ = crate::sharing::write_share(url);
                    return Outcome::Message("Link copied".into());
                }
            }
            _ => return Outcome::Ignored,
        }
        Outcome::Consumed
    }

    fn mouse(&mut self, event: MouseEvent, ctx: &Ctx) -> Outcome {
        let at = Position::new(event.column, event.row);
        match event.kind {
            MouseEventKind::ScrollDown => self.scroll(1),
            MouseEventKind::ScrollUp => self.scroll(-1),
            MouseEventKind::Down(button @ (MouseButton::Left | MouseButton::Right)) => {
                if self.regions.play_button.is_some_and(|r| r.contains(at)) {
                    self.selected = Selection::Featured;
                    if let Some(item) = self.featured() {
                        self.play(&item, ctx);
                    }
                    return Outcome::Consumed;
                }
                if let Some((_, filter)) = self.regions.pills.iter().find(|(r, _)| r.contains(at)) {
                    self.set_filter(*filter);
                    return Outcome::Consumed;
                }
                if self.regions.featured.is_some_and(|r| r.contains(at)) {
                    self.selected = Selection::Featured;
                } else if self.regions.list.contains(at) {
                    let list = self.regions.list;
                    let Some(index) = item_at(
                        event.row - list.y,
                        list.height,
                        self.first,
                        self.rows().len(),
                    ) else {
                        return Outcome::Consumed;
                    };
                    self.selected = Selection::Row(index);
                } else {
                    return Outcome::Consumed;
                }
                if button == MouseButton::Right {
                    if let Some(item) = self.selected_item() {
                        return Outcome::Overlay(nav::context_menu(item.as_listitem(), ctx));
                    }
                    return Outcome::Consumed;
                }
                return self.activate(ctx);
            }
            _ => return Outcome::Ignored,
        }
        Outcome::Consumed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn album(id: &str) -> HomeItem {
        let album = serde_json::json!({
            "id": id, "title": id, "artists": [], "artist_ids": [], "year": "",
        });
        HomeItem::Album(serde_json::from_value(album).unwrap())
    }

    fn artist(id: &str) -> Artist {
        Artist::new(id.into(), id.into())
    }

    fn track(id: &str) -> Track {
        let track = serde_json::json!({
            "id": id, "uri": format!("spotify:track:{id}"), "title": id, "track_number": 1,
            "disc_number": 1, "duration": 1000, "artists": [], "artist_ids": [],
            "album_artists": [], "url": "", "list_index": 0, "is_local": false,
        });
        serde_json::from_value(track).unwrap()
    }

    #[test]
    fn clicks_only_find_items_that_are_drawn() {
        // 10 rows fit three items (rows 0-1, 3-4, 6-7); row 9 is below the last one.
        assert_eq!(items_fitting(10), 3);
        assert_eq!(item_at(0, 10, 0, 20), Some(0));
        assert_eq!(item_at(1, 10, 0, 20), Some(0));
        assert_eq!(item_at(2, 10, 0, 20), None, "the blank row between items");
        assert_eq!(item_at(7, 10, 5, 20), Some(7));
        assert_eq!(item_at(9, 10, 5, 20), None, "below the last item drawn");
        assert_eq!(item_at(3, 10, 0, 1), None, "past the end of the list");
    }

    fn sections() -> Sections {
        Sections {
            jump_back_in: vec![album("a1"), HomeItem::Artist(artist("x")), album("a2")],
            top_artists: vec![artist("x"), artist("y")],
            recent: vec![track("t1"), track("t2")],
            top_tracks: vec![track("t2"), track("t3")],
        }
    }

    fn titles(items: &[HomeItem]) -> Vec<String> {
        items.iter().map(HomeItem::title).collect()
    }

    #[test]
    fn test_all_is_deduplicated_and_skips_featured() {
        let rows = sections().rows(Filter::All, true);
        assert_eq!(titles(&rows), ["x", "a2", "y", "t1", "t2", "t3"]);
        let rows = sections().rows(Filter::All, false);
        assert_eq!(titles(&rows), ["a1", "x", "a2", "y", "t1", "t2", "t3"]);
    }

    #[test]
    fn test_filters_show_one_kind() {
        let sections = sections();
        assert_eq!(titles(&sections.rows(Filter::Albums, true)), ["a1", "a2"]);
        assert_eq!(titles(&sections.rows(Filter::Artists, true)), ["x", "y"]);
        assert_eq!(
            titles(&sections.rows(Filter::Songs, true)),
            ["t1", "t2", "t3"]
        );
        assert!(sections.rows(Filter::Playlists, true).is_empty());
    }

    #[test]
    fn test_subtitles() {
        let mut track = track("t");
        track.artists = vec!["Paramore".into()];
        assert_eq!(HomeItem::Song(track).subtitle(), "Song • Paramore");
        assert_eq!(HomeItem::Artist(artist("x")).subtitle(), "Artist");
    }
}
