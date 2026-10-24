//! Cards with covers, like Spotify's home screen and library: albums, playlists, artists and
//! tracks shown as a cover with a title and subtitle under it.
//!
//! [CardGrid] lays cards out either as rows (one per section, scrolling sideways, as on the
//! search screen's All tab) or as one wrapped grid (as in the library). Arrow keys or hjkl move
//! between cards, Enter opens albums, playlists and artists and plays tracks, and cards can be
//! clicked. Pages showing one should say yes to [Page::handles_horizontal_moves] (CardGrid
//! itself does), so left/right move between cards instead of switching tabs.

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};
use ratatui::style::Style;

use crate::command::{Command, GotoMode, JumpMode, MoveAmount, MoveMode, TargetMode};
use crate::library::Library;
use crate::model::album::Album;
use crate::model::artist::Artist;
use crate::model::playable::Playable;
use crate::model::playlist::Playlist;
use crate::model::track::Track;
use crate::traits::{ListItem, OpenTarget};
use crate::tui::hints;
use crate::tui::images;
use crate::tui::nav;
use crate::tui::overlays::menu::Menu;
use crate::tui::page::{Ctx, Outcome, Page};
use crate::tui::pages::liked::LikedPage;
use crate::tui::pointer;
use crate::tui::style::{SELECTION_MARKER, Styles, bold, fill, put, scroll_to};
use crate::utils::search_text;

/// Rows of a card's cover (when covers can be shown).
pub const COVER_ROWS: u16 = 6;
/// Smallest covers, in short windows.
const MIN_COVER_ROWS: u16 = 3;
/// Narrowest card.
const MIN_CARD_WIDTH: u16 = 16;
/// Columns between cards.
const CARD_GAP: u16 = 2;
/// Left margin.
const MARGIN: u16 = 2;

#[derive(Clone)]
pub enum CardItem {
    Album(Album),
    Playlist(Playlist),
    Artist(Artist),
    Track(Track),
    /// The user's saved tracks, shown like a playlist.
    LikedSongs,
}

#[derive(Clone)]
pub struct Card {
    pub item: CardItem,
    pub title: String,
    pub subtitle: String,
    pub image: Option<String>,
    /// An artist from the library, made without their saved tracks; see [Self::resolved].
    library_artist: bool,
}

impl Card {
    pub fn album(album: Album) -> Self {
        Self {
            title: album.title.clone(),
            subtitle: format!("Album • {}", album.artists.join(", ")),
            image: album.cover_url.clone(),
            item: CardItem::Album(album),
            library_artist: false,
        }
    }

    pub fn playlist(playlist: Playlist) -> Self {
        Self {
            title: playlist.name.clone(),
            subtitle: match &playlist.owner_name {
                Some(owner) => format!("Playlist • {owner}"),
                None => "Playlist".into(),
            },
            image: playlist.cover_url.clone(),
            item: CardItem::Playlist(playlist),
            library_artist: false,
        }
    }

    pub fn artist(artist: Artist) -> Self {
        Self {
            title: artist.name.clone(),
            subtitle: "Artist".into(),
            // Looked up when the card is drawn, so only for cards on screen.
            image: artist.known_image_url(),
            item: CardItem::Artist(artist),
            library_artist: false,
        }
    }

    pub fn track(track: Track) -> Self {
        Self {
            title: track.title.clone(),
            subtitle: track.artists.join(", "),
            image: track.cover_url.clone(),
            item: CardItem::Track(track),
            library_artist: false,
        }
    }

    pub fn liked_songs(count: usize) -> Self {
        Self {
            item: CardItem::LikedSongs,
            title: "Liked Songs".into(),
            subtitle: format!("Playlist • {count} songs"),
            image: None,
            library_artist: false,
        }
    }

    /// An artist from the library, without the saved tracks it comes with: library grids make
    /// their cards on every draw. They're put back by [Self::resolved].
    pub fn library_artist(artist: &Artist) -> Self {
        let mut card = Self::artist(Artist {
            id: artist.id.clone(),
            name: artist.name.clone(),
            url: artist.url.clone(),
            tracks: None,
            is_followed: artist.is_followed,
        });
        card.library_artist = true;
        card
    }

    fn as_listitem(&self) -> Option<Box<dyn ListItem>> {
        Some(match &self.item {
            CardItem::Album(album) => album.as_listitem(),
            CardItem::Playlist(playlist) => playlist.as_listitem(),
            CardItem::Artist(artist) => artist.as_listitem(),
            CardItem::Track(track) => track.as_listitem(),
            CardItem::LikedSongs => return None,
        })
    }

    /// The card with its album, playlist or library artist as stored in the library, tracks
    /// included: library grids make their cards without them (see [crate::tui::pages::library]),
    /// so they're put back before acting on the card instead of being loaded from Spotify again.
    fn resolved(&self, library: &Library) -> Self {
        let mut card = self.clone();
        match &mut card.item {
            CardItem::Playlist(playlist) if playlist.tracks.is_none() => {
                let playlists = library.playlists.read().unwrap();
                if let Some(full) = playlists.iter().find(|p| p.id == playlist.id) {
                    *playlist = full.clone();
                }
            }
            CardItem::Artist(artist) if self.library_artist => {
                let artists = library.artists.read().unwrap();
                if let Some(full) = artists.iter().find(|a| a.id == artist.id) {
                    *artist = full.clone();
                }
            }
            CardItem::Album(album) if album.tracks.is_none() && album.id.is_some() => {
                let albums = library.albums.read().unwrap();
                if let Some(full) = albums.iter().find(|a| a.id == album.id) {
                    *album = full.clone();
                }
            }
            _ => {}
        }
        card
    }

    /// Shown in place of a missing cover.
    fn placeholder(&self) -> &'static str {
        match self.item {
            CardItem::LikedSongs => "♥",
            CardItem::Artist(_) => "♫",
            _ => "♪",
        }
    }

    /// The cover to show: the card's own, or for artists, their picture (looked up the first
    /// time it's asked for).
    fn cover_url(&self) -> Option<String> {
        self.image.clone().or_else(|| match &self.item {
            CardItem::Artist(artist) => artist.image_url(),
            _ => None,
        })
    }
}

#[derive(Clone)]
pub struct Section {
    /// Shown above the section's cards, unless empty.
    pub title: String,
    pub cards: Vec<Card>,
}

impl Section {
    pub fn new(title: impl Into<String>, cards: Vec<Card>) -> Self {
        Self {
            title: title.into(),
            cards,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CardLayout {
    /// One row per section, each scrolling sideways.
    Rows,
    /// The first section's cards wrapped into as many rows as needed, scrolling down.
    Grid,
}

/// Where the cards come from: called on every draw and command, so the cards stay up to date
/// with their source. `None` while loading.
pub type CardSource = Box<dyn Fn() -> Option<Vec<Section>>>;

pub struct CardGrid {
    title: String,
    layout: CardLayout,
    source: CardSource,
    /// Shown while the source returns `None`, and when there are no cards.
    loading_message: String,
    empty_message: String,
    /// Selected section and card.
    section: usize,
    card: usize,
    /// Rows mode: the first section shown, and the first card shown in each section.
    first_section: usize,
    first_cards: Vec<usize>,
    /// Grid mode: the first row of cards shown.
    first_row: usize,
    /// As drawn last: cards per row, rows of cards (grid) or sections (rows) that fit.
    columns: usize,
    visible: usize,
    /// Rows of the covers as drawn last (they shrink to fit two rows of cards).
    cover: u16,
    /// Where each card was drawn last, with its section and index, for clicks.
    card_areas: Vec<(Rect, usize, usize)>,
    /// Matches of the last `/` search (section, card), and which one is selected.
    jump_matches: Vec<(usize, usize)>,
    jump_index: usize,
}

impl CardGrid {
    pub fn new(title: impl Into<String>, layout: CardLayout, source: CardSource) -> Self {
        Self {
            title: title.into(),
            layout,
            source,
            loading_message: "Loading…".into(),
            empty_message: "Nothing here yet".into(),
            section: 0,
            card: 0,
            first_section: 0,
            first_cards: Vec::new(),
            first_row: 0,
            columns: 1,
            visible: 1,
            cover: COVER_ROWS,
            card_areas: Vec::new(),
            jump_matches: Vec::new(),
            jump_index: 0,
        }
    }

    pub fn loading_message(mut self, message: impl Into<String>) -> Self {
        self.loading_message = message.into();
        self
    }

    pub fn empty_message(mut self, message: impl Into<String>) -> Self {
        self.empty_message = message.into();
        self
    }

    fn sections(&self) -> Vec<Section> {
        let mut sections = (self.source)().unwrap_or_default();
        if self.layout == CardLayout::Grid {
            sections.truncate(1);
        }
        sections
    }

    /// Keep the selection on an existing card after the cards changed.
    fn clamp(&mut self, sections: &[Section]) {
        self.section = self.section.min(sections.len().saturating_sub(1));
        self.card = self
            .card
            .min(section_len(sections, self.section).saturating_sub(1));
    }

    fn selected<'a>(&self, sections: &'a [Section]) -> Option<&'a Card> {
        sections.get(self.section)?.cards.get(self.card)
    }

    /// Width in cells of a cover `cover` rows tall (0: no covers).
    fn cover_width(cover: u16) -> u16 {
        if cover > 0 {
            images::square_width(cover)
        } else {
            0
        }
    }

    fn card_width(cover: u16) -> u16 {
        Self::cover_width(cover).max(MIN_CARD_WIDTH)
    }

    /// Rows of one card and the blank row under it: the cover, title and subtitle, a gap.
    fn card_rows(cover: u16) -> u16 {
        cover + 3
    }

    /// Rows of the covers in an area `height` tall: [COVER_ROWS], smaller (down to
    /// [MIN_COVER_ROWS]) when that would leave room for only one row of cards, or 0 without
    /// images or when not even one row of cards with the smallest covers fits.
    fn fit_cover_rows(height: u16, layout: CardLayout) -> u16 {
        if !images::supported() {
            return 0;
        }
        // Rows besides the covers: of one row of cards (title and subtitle, after a section
        // title in rows mode), and of two rows of cards with the gaps between them.
        let (one, two) = match layout {
            CardLayout::Rows => (1 + 2, 2 * (1 + 3)),
            CardLayout::Grid => (2, 2 * 3 + 2),
        };
        if height < one + MIN_COVER_ROWS {
            return 0;
        }
        (height.saturating_sub(two) / 2).clamp(MIN_COVER_ROWS, COVER_ROWS)
    }

    /// Draw `card` with its top-left corner at (`x`, `y`).
    #[allow(clippy::too_many_arguments)]
    fn draw_card(
        buf: &mut Buffer,
        cover: u16,
        card: &Card,
        x: u16,
        y: u16,
        selected: bool,
        focused: bool,
        ctx: &Ctx,
    ) {
        let styles = Styles(&ctx.palette);
        let card_width = Self::card_width(cover);
        let cover_rows = cover;
        if cover_rows > 0 {
            let cover = Rect::new(x, y, Self::cover_width(cover), cover_rows);
            match card.cover_url() {
                Some(url) => images::cover(&url, cover),
                None => {
                    // A plain square with a symbol where the cover would be.
                    let style = Style::new()
                        .fg(ctx.palette.secondary)
                        .bg(ctx.palette.highlight_inactive_bg);
                    fill(buf, cover, style);
                    put(
                        buf,
                        x + cover.width / 2,
                        y + (cover_rows - 1) / 2,
                        card.placeholder(),
                        1,
                        style,
                    );
                }
            }
        }

        let text_y = y + cover_rows;
        let (title_style, subtitle_style) = if selected {
            let style = if focused {
                styles.selected()
            } else {
                styles.selected_inactive()
            };
            // S2: the darker background behind the text, the marker just left of the card.
            fill(
                buf,
                Rect::new(x.saturating_sub(1), text_y, card_width + 1, 2),
                style,
            );
            if focused {
                for row in text_y..text_y + 2 {
                    put(
                        buf,
                        x.saturating_sub(1),
                        row,
                        SELECTION_MARKER,
                        1,
                        styles.marker(),
                    );
                }
            }
            (bold(style), styles.secondary().bg(ctx.palette.statusbar_bg))
        } else {
            (bold(styles.primary()), styles.secondary())
        };
        let room = usize::from(card_width);
        put(buf, x, text_y, &card.title, room, title_style);
        put(buf, x, text_y + 1, &card.subtitle, room, subtitle_style);
    }

    fn draw_section_title(buf: &mut Buffer, area: Rect, title: &str, y: u16, ctx: &Ctx) {
        let styles = Styles(&ctx.palette);
        put(
            buf,
            area.x + MARGIN,
            y,
            title,
            usize::from(area.width.saturating_sub(MARGIN)),
            bold(styles.accent()),
        );
    }

    fn draw_rows(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        sections: &[Section],
        ctx: &Ctx,
        focused: bool,
    ) {
        let buf = frame.buffer_mut();
        let cover = Self::fit_cover_rows(area.height, CardLayout::Rows);
        self.cover = cover;
        let card_rows = Self::card_rows(cover);
        // A title row, the cards; the last section's blank row may be cut off.
        let section_rows = 1 + card_rows;
        self.visible = usize::from((area.height + 1) / section_rows).max(1);
        self.first_section = scroll_to(self.first_section, self.section, self.visible);
        self.columns = columns_for(area.width, Self::card_width(cover));
        self.first_cards.resize(sections.len(), 0);

        for (s, section) in sections.iter().enumerate().skip(self.first_section) {
            let top = area.y + (s - self.first_section) as u16 * section_rows;
            if top + section_rows - 1 > area.bottom() {
                break;
            }
            Self::draw_section_title(buf, area, &section.title, top, ctx);
            if s == self.section {
                self.first_cards[s] = scroll_to(self.first_cards[s], self.card, self.columns);
            }
            self.first_cards[s] = self.first_cards[s].min(section.cards.len().saturating_sub(1));
            let first = self.first_cards[s];
            for (c, card) in section
                .cards
                .iter()
                .enumerate()
                .skip(first)
                .take(self.columns)
            {
                let x = area.x + MARGIN + (c - first) as u16 * (Self::card_width(cover) + CARD_GAP);
                let selected = s == self.section && c == self.card;
                Self::draw_card(buf, cover, card, x, top + 1, selected, focused, ctx);
                self.card_areas.push((
                    Rect::new(x, top + 1, Self::card_width(cover), card_rows - 1),
                    s,
                    c,
                ));
            }
        }
    }

    fn draw_grid(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        sections: &[Section],
        ctx: &Ctx,
        focused: bool,
    ) {
        let buf = frame.buffer_mut();
        let Some(section) = sections.first() else {
            return;
        };
        let mut top = area.y;
        if !section.title.is_empty() {
            Self::draw_section_title(buf, area, &section.title, top, ctx);
            top += 2;
        }
        let cover = Self::fit_cover_rows(area.bottom().saturating_sub(top), CardLayout::Grid);
        self.cover = cover;
        let card_rows = Self::card_rows(cover);
        let card_width = Self::card_width(cover);
        self.columns = columns_for(area.width, card_width);
        self.visible = usize::from((area.bottom().saturating_sub(top) + 1) / card_rows).max(1);
        self.first_row = scroll_to(self.first_row, self.card / self.columns, self.visible);

        for (c, card) in section
            .cards
            .iter()
            .enumerate()
            .skip(self.first_row * self.columns)
            .take(self.visible * self.columns)
        {
            let row = (c / self.columns - self.first_row) as u16;
            let x = area.x + MARGIN + (c % self.columns) as u16 * (card_width + CARD_GAP);
            let y = top + row * card_rows;
            if y + card_rows - 1 > area.bottom() {
                break;
            }
            Self::draw_card(buf, cover, card, x, y, c == self.card, focused, ctx);
            self.card_areas
                .push((Rect::new(x, y, card_width, card_rows - 1), 0, c));
        }
    }

    /// Enter on a card: open albums, playlists, artists and Liked Songs; play tracks, followed by
    /// the rest of their section's tracks.
    fn activate(&self, sections: &[Section], ctx: &Ctx) -> Outcome {
        let Some(section) = sections.get(self.section) else {
            return Outcome::Consumed;
        };
        let Some(card) = section.cards.get(self.card) else {
            return Outcome::Consumed;
        };
        let card = &card.resolved(&ctx.library);
        match &card.item {
            CardItem::Track(_) => {
                let tracks: Vec<Playable> = section
                    .cards
                    .iter()
                    .filter_map(|c| match &c.item {
                        CardItem::Track(t) => Some(Playable::Track(t.clone())),
                        _ => None,
                    })
                    .collect();
                let position = section.cards[..self.card]
                    .iter()
                    .filter(|c| matches!(c.item, CardItem::Track(_)))
                    .count();
                let source = (!section.title.is_empty()).then_some(section.title.as_str());
                ctx.queue.clear();
                let first = ctx.queue.append_next_from(&tracks, source);
                ctx.queue.play(first + position, true, false);
                Outcome::Consumed
            }
            CardItem::LikedSongs => Outcome::Open(Box::new(LikedPage::new(ctx))),
            _ => match card.as_listitem().and_then(|item| item.target()) {
                Some(target) => Outcome::Open(nav::open_target(target, ctx)),
                None => Outcome::Consumed,
            },
        }
    }

    /// Ask before deleting the selected playlist (it's removed from the library).
    fn delete(card: &Card, ctx: &Ctx) -> Outcome {
        let CardItem::Playlist(playlist) = &card.item else {
            return Outcome::Ignored;
        };
        if !ctx.library.is_followed_playlist(playlist) {
            return Outcome::Error("Only playlists in your library can be deleted".into());
        }
        let id = playlist.id.clone();
        Outcome::Overlay(Box::new(Menu::confirm(
            format!("Delete the playlist {}?", playlist.name),
            move |ctx| {
                ctx.library.delete_playlist(&id);
                Outcome::Consumed
            },
        )))
    }

    /// Move the selection. At the edges it stays put (left/right never leave the grid).
    fn move_selection(&mut self, sections: &[Section], mode: &MoveMode, amount: &MoveAmount) {
        let step = match amount {
            MoveAmount::Integer(n) => n.unsigned_abs() as usize,
            MoveAmount::Float(scale) => ((self.visible as f32 * scale) as usize).max(1),
            MoveAmount::Extreme => usize::MAX / 2,
        }
        .max(1);
        let len = section_len(sections, self.section);
        match (self.layout, mode) {
            (_, MoveMode::Playing) => {}
            (CardLayout::Rows, MoveMode::Left | MoveMode::Right) => {
                self.card = move_in_row(self.card, len, step, matches!(mode, MoveMode::Right));
            }
            (CardLayout::Rows, MoveMode::Up | MoveMode::Down) => {
                let last = sections.len().saturating_sub(1);
                let section = if matches!(mode, MoveMode::Down) {
                    self.section.saturating_add(step).min(last)
                } else {
                    self.section.saturating_sub(step)
                };
                if section != self.section {
                    // Same place on screen: each row keeps its own sideways scroll.
                    let first = |s: usize| self.first_cards.get(s).copied().unwrap_or(0);
                    self.card = carry_column(
                        self.card,
                        first(self.section),
                        first(section),
                        section_len(sections, section),
                    );
                    self.section = section;
                }
            }
            (CardLayout::Grid, mode) => {
                self.card = grid_move(self.card, len, self.columns.max(1), mode, step);
            }
        }
    }

    fn jump(&mut self, sections: &[Section], mode: &JumpMode) {
        match mode {
            JumpMode::Query(query) => {
                let query = search_text(query);
                self.jump_matches = sections
                    .iter()
                    .enumerate()
                    .flat_map(|(s, section)| {
                        let query = &query;
                        section
                            .cards
                            .iter()
                            .enumerate()
                            .filter_map(move |(c, card)| {
                                let text = format!("{} {}", card.title, card.subtitle);
                                search_text(&text).contains(query).then_some((s, c))
                            })
                    })
                    .collect();
                self.jump_index = 0;
            }
            JumpMode::Next if !self.jump_matches.is_empty() => {
                self.jump_index = (self.jump_index + 1) % self.jump_matches.len();
            }
            JumpMode::Previous if !self.jump_matches.is_empty() => {
                self.jump_index = self
                    .jump_index
                    .checked_sub(1)
                    .unwrap_or(self.jump_matches.len() - 1);
            }
            _ => {}
        }
        if let Some(&(section, card)) = self.jump_matches.get(self.jump_index) {
            self.section = section;
            self.card = card;
        }
    }
}

impl Page for CardGrid {
    fn title(&self) -> String {
        self.title.clone()
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect, ctx: &Ctx, focused: bool) {
        self.card_areas.clear();
        let styles = Styles(&ctx.palette);
        let message = |frame: &mut Frame, text: &str| {
            put(
                frame.buffer_mut(),
                area.x + MARGIN,
                area.y,
                text,
                usize::from(area.width.saturating_sub(MARGIN)),
                styles.secondary(),
            );
        };
        let Some(mut sections) = (self.source)() else {
            message(frame, &self.loading_message);
            return;
        };
        if self.layout == CardLayout::Grid {
            sections.truncate(1);
        }
        if sections.iter().all(|s| s.cards.is_empty()) {
            message(frame, &self.empty_message);
            return;
        }
        self.clamp(&sections);
        match self.layout {
            CardLayout::Rows => self.draw_rows(frame, area, &sections, ctx, focused),
            CardLayout::Grid => self.draw_grid(frame, area, &sections, ctx, focused),
        }
        for &(card, s, c) in &self.card_areas {
            pointer::clickable(card);
            // The selected card's keys on its title and subtitle, under the cover (a cover would
            // be drawn over them).
            if focused && (s, c) == (self.section, self.card) {
                let text_rows = card.bottom().saturating_sub(2).max(card.y)..card.bottom();
                for (row, command) in text_rows.zip([Command::Play, Command::Queue]) {
                    hints::add(Rect::new(card.x, row, card.width, 1), command);
                }
            }
        }
    }

    fn command(&mut self, cmd: &Command, ctx: &Ctx) -> Outcome {
        let sections = self.sections();
        self.clamp(&sections);
        let selected = self
            .selected(&sections)
            .map(|card| card.resolved(&ctx.library));
        match cmd {
            Command::Move(mode, amount) => self.move_selection(&sections, mode, amount),
            Command::Play => return self.activate(&sections, ctx),
            Command::Open(TargetMode::Selected) => {
                // Tracks get their menu, like in lists.
                if let Some(Card {
                    item: CardItem::Track(track),
                    ..
                }) = &selected
                {
                    return Outcome::Overlay(nav::context_menu(track.as_listitem(), ctx));
                }
                return self.activate(&sections, ctx);
            }
            Command::Queue | Command::PlayNext | Command::Save => {
                if let Some(mut item) = selected.as_ref().and_then(Card::as_listitem) {
                    match cmd {
                        Command::Queue => nav::queue_item(item, false, ctx),
                        Command::PlayNext => nav::queue_item(item, true, ctx),
                        _ => item.save(&ctx.library),
                    }
                }
            }
            Command::Add => {
                if let Some(Card {
                    item: CardItem::Track(track),
                    ..
                }) = &selected
                {
                    return nav::pick_playlist_for(vec![track.clone()], ctx);
                }
            }
            Command::Delete => {
                return match &selected {
                    Some(card) => Self::delete(card, ctx),
                    None => Outcome::Consumed,
                };
            }
            Command::Rename(name) => {
                return match &selected {
                    Some(Card {
                        item: CardItem::Playlist(playlist),
                        ..
                    }) => nav::rename_playlist(playlist, name, ctx),
                    _ => Outcome::Ignored,
                };
            }
            Command::Goto(mode) => {
                let Some(item) = selected.as_ref().and_then(Card::as_listitem) else {
                    return Outcome::Consumed;
                };
                return match mode {
                    GotoMode::Album => match item.album() {
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
                if let Some(url) = selected
                    .as_ref()
                    .and_then(Card::as_listitem)
                    .and_then(|i| i.share_url())
                {
                    let _ = crate::sharing::write_share(url);
                    return Outcome::Message("Link copied".into());
                }
            }
            Command::Jump(mode) => self.jump(&sections, mode),
            _ => return Outcome::Ignored,
        }
        Outcome::Consumed
    }

    fn mouse(&mut self, event: MouseEvent, ctx: &Ctx) -> Outcome {
        let sections = self.sections();
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let at = Position::new(event.column, event.row);
                if let Some(&(_, section, card)) = self
                    .card_areas
                    .iter()
                    .find(|(rect, _, _)| rect.contains(at))
                {
                    self.section = section;
                    self.card = card;
                    return self.activate(&sections, ctx);
                }
            }
            MouseEventKind::ScrollDown => {
                self.move_selection(&sections, &MoveMode::Down, &MoveAmount::Integer(1));
            }
            MouseEventKind::ScrollUp => {
                self.move_selection(&sections, &MoveMode::Up, &MoveAmount::Integer(1));
            }
            _ => return Outcome::Ignored,
        }
        Outcome::Consumed
    }

    fn handles_horizontal_moves(&self) -> bool {
        true
    }
}

fn section_len(sections: &[Section], section: usize) -> usize {
    sections.get(section).map_or(0, |s| s.cards.len())
}

/// Cards of `card_width` that fit side by side in `width` columns (at least one).
fn columns_for(width: u16, card_width: u16) -> usize {
    usize::from((width.saturating_sub(MARGIN) + CARD_GAP) / (card_width + CARD_GAP)).max(1)
}

/// Move `step` cards sideways in a row of `len` cards, stopping at its ends.
fn move_in_row(card: usize, len: usize, step: usize, right: bool) -> usize {
    if right {
        card.saturating_add(step).min(len.saturating_sub(1))
    } else {
        card.saturating_sub(step)
    }
}

/// The card to select in another row (rows mode) so the selection stays in the same column on
/// screen: the row scrolled to `first_from` with `card` selected, going to a row of `len_to`
/// cards scrolled to `first_to`.
fn carry_column(card: usize, first_from: usize, first_to: usize, len_to: usize) -> usize {
    (first_to + card.saturating_sub(first_from)).min(len_to.saturating_sub(1))
}

/// Move `step` cards (left/right) or rows (up/down) in a grid of `len` cards wrapped into rows of
/// `columns`. Left and right go on into the row above or below (h at the start of a row goes to
/// the end of the row above); up and down keep the column, ending on the last card when the last
/// row is shorter.
fn grid_move(card: usize, len: usize, columns: usize, mode: &MoveMode, step: usize) -> usize {
    let last = len.saturating_sub(1);
    let columns = columns.max(1);
    let (row, column) = (card / columns, card % columns);
    match mode {
        MoveMode::Left => card.saturating_sub(step),
        MoveMode::Right => card.saturating_add(step).min(last),
        MoveMode::Up => row.saturating_sub(step) * columns + column,
        MoveMode::Down => {
            let row = row.saturating_add(step).min(last / columns);
            (row * columns + column).min(last)
        }
        MoveMode::Playing => card,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_columns_for() {
        assert_eq!(columns_for(80, 16), 4);
        assert_eq!(columns_for(10, 16), 1);
        assert_eq!(columns_for(2 + 16 + 2 + 16, 16), 2);
    }

    #[test]
    fn test_move_in_row_stops_at_edges() {
        assert_eq!(move_in_row(0, 5, 1, false), 0);
        assert_eq!(move_in_row(4, 5, 1, true), 4);
        assert_eq!(move_in_row(2, 5, 1, true), 3);
        assert_eq!(move_in_row(2, 5, 10, false), 0);
    }

    #[test]
    fn test_carry_column() {
        // Second card on screen stays the second card on screen.
        assert_eq!(carry_column(5, 4, 0, 10), 1);
        assert_eq!(carry_column(1, 0, 3, 10), 4);
        // Shorter row: its last card.
        assert_eq!(carry_column(6, 0, 0, 3), 2);
    }

    #[test]
    fn test_grid_move() {
        // 10 cards in rows of 4: 0-3, 4-7, 8-9.
        assert_eq!(grid_move(4, 10, 4, &MoveMode::Left, 1), 3);
        assert_eq!(grid_move(3, 10, 4, &MoveMode::Right, 1), 4);
        assert_eq!(grid_move(9, 10, 4, &MoveMode::Right, 1), 9);
        assert_eq!(grid_move(0, 10, 4, &MoveMode::Left, 1), 0);
        assert_eq!(grid_move(5, 10, 4, &MoveMode::Up, 1), 1);
        assert_eq!(grid_move(1, 10, 4, &MoveMode::Up, 1), 1);
        assert_eq!(grid_move(1, 10, 4, &MoveMode::Down, 1), 5);
        // Into the shorter last row.
        assert_eq!(grid_move(7, 10, 4, &MoveMode::Down, 1), 9);
        assert_eq!(grid_move(9, 10, 4, &MoveMode::Down, 1), 9);
        assert_eq!(grid_move(2, 10, 4, &MoveMode::Down, usize::MAX / 2), 9);
        assert_eq!(grid_move(9, 10, 4, &MoveMode::Up, usize::MAX / 2), 1);
    }
}
