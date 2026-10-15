//! The home screen, like Spotify's: rows of cards for what you've been listening to.
//!
//! * Jump back in: albums, playlists and artists played from recently.
//! * Recently played: tracks.
//! * Your top tracks and Your top artists, of the last four weeks.
//!
//! Arrow keys (or hjkl) move between cards, Enter opens an album, playlist or artist and plays a
//! track (with the rest of its row after it). Cards can be clicked too.

use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::{Duration, Instant};

use cursive::event::{Callback, Event, EventResult, MouseButton, MouseEvent};
use cursive::theme::{ColorStyle, Effect};
use cursive::traits::View;
use cursive::{Cursive, Printer, Vec2};
use log::error;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::command::{Command, MoveAmount, MoveMode, TargetMode};
use crate::commands::CommandResult;
use crate::library::Library;
use crate::model::album::Album;
use crate::model::artist::Artist;
use crate::model::playable::Playable;
use crate::model::playlist::Playlist;
use crate::model::track::Track;
use crate::queue::Queue;
use crate::spotify::Spotify;
use crate::spotify_api::PlayContext;
use crate::traits::{ListItem, ViewExt};
#[cfg(feature = "cover")]
use crate::ui::cover::{
    Area, HOME_CARD_IMAGE_ID, ImageLayer, ImageRequest, kitty_supported, terminal_font_size,
};
use crate::ui::layout::Layout;

/// Cards per row at most.
const MAX_CARDS: usize = 20;
/// Rows of a card's cover (when covers can be shown).
const COVER_ROWS: usize = 6;
/// Narrowest card, and the gap between cards.
const MIN_CARD_WIDTH: usize = 16;
const CARD_GAP: usize = 2;
/// The home screen is reloaded when shown again after this long.
const REFRESH_AFTER: Duration = Duration::from_secs(10 * 60);

#[derive(Clone)]
enum CardItem {
    Album(Album),
    Playlist(Playlist),
    Artist(Artist),
    Track(Track),
}

impl CardItem {
    fn as_listitem(&self) -> Box<dyn ListItem> {
        match self {
            Self::Album(album) => album.as_listitem(),
            Self::Playlist(playlist) => playlist.as_listitem(),
            Self::Artist(artist) => artist.as_listitem(),
            Self::Track(track) => track.as_listitem(),
        }
    }
}

#[derive(Clone)]
struct Card {
    item: CardItem,
    title: String,
    subtitle: String,
    image: Option<String>,
}

impl Card {
    fn album(album: Album) -> Self {
        Self {
            title: album.title.clone(),
            subtitle: format!("Album • {}", album.artists.join(", ")),
            image: album.cover_url.clone(),
            item: CardItem::Album(album),
        }
    }

    fn playlist(playlist: Playlist) -> Self {
        Self {
            title: playlist.name.clone(),
            subtitle: match &playlist.owner_name {
                Some(owner) => format!("Playlist • {owner}"),
                None => "Playlist".into(),
            },
            image: playlist.cover_url.clone(),
            item: CardItem::Playlist(playlist),
        }
    }

    fn artist(artist: Artist, image: Option<String>) -> Self {
        Self {
            title: artist.name.clone(),
            subtitle: "Artist".into(),
            image,
            item: CardItem::Artist(artist),
        }
    }

    fn track(track: Track) -> Self {
        Self {
            title: track.title.clone(),
            subtitle: track.artists.join(", "),
            image: track.cover_url.clone(),
            item: CardItem::Track(track),
        }
    }
}

struct Section {
    title: &'static str,
    cards: Vec<Card>,
}

enum HomeData {
    Loading,
    Loaded(Vec<Section>),
}

/// Load the home screen's sections from Spotify. Slow (several requests): run in the background.
fn load(spotify: &Spotify, library: &Library) -> Vec<Section> {
    let api = &spotify.api;
    let plays = api.recently_played().unwrap_or_else(|()| {
        error!("couldn't load recently played tracks");
        Vec::new()
    });

    // Jump back in: what recent tracks were played from, falling back to their album.
    let mut seen = HashSet::new();
    let mut jump_back_in = Vec::new();
    for play in &plays {
        if jump_back_in.len() >= MAX_CARDS {
            break;
        }
        let (kind, id) = match &play.context {
            Some((kind, id)) => (*kind, id.clone()),
            None => match &play.album.id {
                Some(id) => (PlayContext::Album, id.clone()),
                None => continue,
            },
        };
        if !seen.insert((kind, id.clone())) {
            continue;
        }
        let card = match kind {
            PlayContext::Album if play.album.id.as_ref() == Some(&id) => {
                Some(Card::album(play.album.clone()))
            }
            PlayContext::Album => api.album(&id).ok().map(|a| Card::album((&a).into())),
            PlayContext::Playlist => library
                .playlists
                .read()
                .unwrap()
                .iter()
                .find(|p| p.id == id)
                .cloned()
                .or_else(|| api.playlist(&id).ok().map(|p| (&p).into()))
                .map(Card::playlist),
            PlayContext::Artist => api.artist(&id).ok().map(|a| {
                let image = a.images.first().map(|i| i.url.clone());
                Card::artist((&a).into(), image)
            }),
        };
        jump_back_in.extend(card);
    }

    let mut seen = HashSet::new();
    let recent_tracks = plays
        .into_iter()
        .filter(|play| seen.insert(play.track.id.clone()))
        .take(MAX_CARDS)
        .map(|play| Card::track(play.track))
        .collect();

    let top_tracks = api
        .top_tracks(MAX_CARDS as u32)
        .unwrap_or_default()
        .into_iter()
        .map(Card::track)
        .collect();
    let top_artists = api
        .top_artists(MAX_CARDS as u32)
        .unwrap_or_default()
        .into_iter()
        .map(|(artist, image)| Card::artist(artist, image))
        .collect();

    [
        ("Jump back in", jump_back_in),
        ("Recently played", recent_tracks),
        ("Your top tracks", top_tracks),
        ("Your top artists", top_artists),
    ]
    .into_iter()
    .filter(|(_, cards)| !cards.is_empty())
    .map(|(title, cards)| Section { title, cards })
    .collect()
}

pub struct HomeView {
    queue: Arc<Queue>,
    library: Arc<Library>,
    data: Arc<RwLock<HomeData>>,
    loaded_at: RwLock<Option<Instant>>,
    /// Selected section and card.
    section: usize,
    card: usize,
    /// First section shown, and the first card shown in each section.
    first_section: AtomicUsize,
    first_cards: RwLock<Vec<usize>>,
    /// Where each card was drawn last (area within the view, section and card), for clicks.
    card_areas: RwLock<Vec<(Vec2, Vec2, usize, usize)>>,
    #[cfg(feature = "cover")]
    images: ImageLayer,
}

impl HomeView {
    pub fn new(queue: Arc<Queue>, library: Arc<Library>) -> Self {
        Self {
            queue,
            library,
            data: Arc::new(RwLock::new(HomeData::Loading)),
            loaded_at: RwLock::new(None),
            section: 0,
            card: 0,
            first_section: AtomicUsize::new(0),
            first_cards: RwLock::new(Vec::new()),
            card_areas: RwLock::new(Vec::new()),
            #[cfg(feature = "cover")]
            images: ImageLayer::default(),
        }
    }

    /// Load the sections in the background if they haven't been, or are getting old.
    fn refresh_if_needed(&self) {
        let mut loaded_at = self.loaded_at.write().unwrap();
        if loaded_at.is_some_and(|at| at.elapsed() < REFRESH_AFTER) {
            return;
        }
        *loaded_at = Some(Instant::now());
        let data = self.data.clone();
        let library = self.library.clone();
        let spotify = self.queue.get_spotify();
        thread::spawn(move || {
            let sections = load(&spotify, &library);
            *data.write().unwrap() = HomeData::Loaded(sections);
            library.trigger_redraw();
        });
    }

    fn with_sections<R>(&self, f: impl FnOnce(&[Section]) -> R) -> R {
        match &*self.data.read().unwrap() {
            HomeData::Loaded(sections) => f(sections),
            HomeData::Loading => f(&[]),
        }
    }

    fn selected_card(&self) -> Option<(Card, Vec<Card>)> {
        self.with_sections(|sections| {
            let section = sections.get(self.section)?;
            Some((section.cards.get(self.card)?.clone(), section.cards.clone()))
        })
    }

    /// Enter on a card: open albums, playlists and artists; play tracks, followed by the rest of
    /// the row.
    fn activate(&self) -> Option<Box<dyn ViewExt>> {
        let (card, row) = self.selected_card()?;
        if let CardItem::Track(_) = card.item {
            let tracks: Vec<Playable> = row
                .iter()
                .filter_map(|c| match &c.item {
                    CardItem::Track(t) => Some(Playable::Track(t.clone())),
                    _ => None,
                })
                .collect();
            let title = self.with_sections(|s| s.get(self.section).map(|s| s.title));
            self.queue.clear();
            let first = self.queue.append_next_from(&tracks, title);
            self.queue.play(first + self.card, true, false);
            None
        } else {
            card.item
                .as_listitem()
                .open(self.queue.clone(), self.library.clone())
        }
    }

    /// Width in cells of a card's cover, or 0 if covers can't be shown.
    fn cover_width(&self) -> usize {
        #[cfg(feature = "cover")]
        if kitty_supported() {
            let font = terminal_font_size();
            return (COVER_ROWS * font.y).div_ceil(font.x.max(1));
        }
        0
    }

    fn cover_rows(&self) -> usize {
        if self.cover_width() > 0 {
            COVER_ROWS
        } else {
            0
        }
    }

    /// Rows a section takes: its title, the covers, card title and subtitle, a blank row.
    fn section_rows(&self) -> usize {
        1 + self.cover_rows() + 2 + 1
    }

    fn card_width(&self) -> usize {
        self.cover_width().max(MIN_CARD_WIDTH)
    }
}

/// Cut `text` to at most `width` columns, ending with "…" if it was cut.
fn truncate(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_string();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars() {
        let w = c.width().unwrap_or(0);
        if used + w + 1 > width {
            break;
        }
        out.push(c);
        used += w;
    }
    if width > 0 {
        out.push('…');
    }
    out
}

/// The first item to show of a list scrolled to `first`, so that `selected` is among the
/// `visible` shown.
fn scroll_to(first: usize, selected: usize, visible: usize) -> usize {
    let visible = visible.max(1);
    if selected < first {
        selected
    } else if selected >= first + visible {
        selected + 1 - visible
    } else {
        first
    }
}

impl View for HomeView {
    fn draw(&self, printer: &Printer<'_, '_>) {
        self.refresh_if_needed();
        let size = printer.size;
        let x = 2;
        let secondary = ColorStyle::secondary();
        let mut card_areas = Vec::new();
        #[cfg(feature = "cover")]
        let mut images = Vec::new();

        self.with_sections(|sections| {
            if sections.is_empty() {
                let text = match &*self.data.read().unwrap() {
                    HomeData::Loading => "Loading…",
                    HomeData::Loaded(_) => "Nothing here yet: play some music first",
                };
                printer.with_color(secondary, |printer| printer.print((x, 1), text));
                return;
            }

            let section_rows = self.section_rows();
            let visible_sections = (size.y / section_rows).max(1);
            let first_section = scroll_to(
                self.first_section.load(Ordering::Relaxed),
                self.section,
                visible_sections,
            );
            self.first_section.store(first_section, Ordering::Relaxed);

            let card_width = self.card_width();
            let cover_width = self.cover_width();
            let cover_rows = self.cover_rows();
            let visible_cards = (size.x.saturating_sub(x) + CARD_GAP) / (card_width + CARD_GAP);
            let mut first_cards = self.first_cards.write().unwrap();
            first_cards.resize(sections.len(), 0);

            for (s, section) in sections.iter().enumerate().skip(first_section) {
                let top = (s - first_section) * section_rows;
                if top + section_rows > size.y + 1 {
                    break;
                }
                printer.with_color(ColorStyle::title_primary(), |printer| {
                    printer.with_effect(Effect::Bold, |printer| {
                        printer.print((x, top), section.title);
                    });
                });

                if s == self.section {
                    first_cards[s] = scroll_to(first_cards[s], self.card, visible_cards);
                }
                for (c, card) in section
                    .cards
                    .iter()
                    .enumerate()
                    .skip(first_cards[s])
                    .take(visible_cards)
                {
                    let card_x = x + (c - first_cards[s]) * (card_width + CARD_GAP);
                    let cover_top = top + 1;
                    #[cfg(feature = "cover")]
                    if cover_rows > 0
                        && let Some(url) = &card.image
                    {
                        images.push(ImageRequest {
                            image_id: HOME_CARD_IMAGE_ID + images.len() as u32,
                            url: url.clone(),
                            offset: printer.offset + Vec2::new(card_x, cover_top),
                            cells: Vec2::new(cover_width, cover_rows),
                        });
                    }
                    let _ = cover_width;

                    let text_top = cover_top + cover_rows;
                    let selected = printer.focused && s == self.section && c == self.card;
                    let (title_style, subtitle_style) = if selected {
                        (ColorStyle::highlight(), ColorStyle::highlight())
                    } else {
                        (ColorStyle::primary(), secondary)
                    };
                    if selected {
                        printer.with_color(ColorStyle::highlight(), |printer| {
                            for row in text_top..text_top + 2 {
                                printer.print_hline((card_x, row), card_width, " ");
                            }
                        });
                    }
                    printer.with_color(title_style, |printer| {
                        printer.with_effect(Effect::Bold, |printer| {
                            printer.print((card_x, text_top), &truncate(&card.title, card_width));
                        });
                    });
                    printer.with_color(subtitle_style, |printer| {
                        printer.print(
                            (card_x, text_top + 1),
                            &truncate(&card.subtitle, card_width),
                        );
                    });
                    card_areas.push((
                        Vec2::new(card_x, cover_top),
                        Vec2::new(card_width, cover_rows + 2),
                        s,
                        c,
                    ));
                }
            }
        });

        *self.card_areas.write().unwrap() = card_areas;
        #[cfg(feature = "cover")]
        self.images.set(images);
    }

    fn required_size(&mut self, constraint: Vec2) -> Vec2 {
        constraint
    }

    fn take_focus(
        &mut self,
        _source: cursive::direction::Direction,
    ) -> Result<EventResult, cursive::view::CannotFocus> {
        Ok(EventResult::consumed())
    }

    fn on_event(&mut self, event: Event) -> EventResult {
        let Event::Mouse {
            offset,
            position,
            event,
        } = event
        else {
            return EventResult::Ignored;
        };
        let Some(position) = position.checked_sub(offset) else {
            return EventResult::Ignored;
        };
        match event {
            MouseEvent::Press(MouseButton::Left) => {
                let clicked = self.card_areas.read().unwrap().iter().find_map(
                    |&(card_offset, card_size, s, c)| {
                        let inside = position.fits_in_rect(card_offset, card_size);
                        inside.then_some((s, c))
                    },
                );
                if let Some((section, card)) = clicked {
                    self.section = section;
                    self.card = card;
                    if let Some(view) = self.activate() {
                        return EventResult::Consumed(Some(Callback::from_fn_once(
                            move |s: &mut Cursive| {
                                s.call_on_name("main", move |l: &mut Layout| l.push_view(view));
                            },
                        )));
                    }
                }
            }
            MouseEvent::WheelUp => self.section = self.section.saturating_sub(1),
            MouseEvent::WheelDown => {
                let count = self.with_sections(<[Section]>::len);
                self.section = (self.section + 1).min(count.saturating_sub(1));
            }
            _ => return EventResult::Ignored,
        }
        self.card = self.card.min(
            self.with_sections(|s| s.get(self.section).map_or(0, |s| s.cards.len()))
                .saturating_sub(1),
        );
        EventResult::consumed()
    }
}

impl ViewExt for HomeView {
    fn title(&self) -> String {
        "Home".into()
    }

    fn title_sub(&self) -> String {
        match &self.library.display_name {
            Some(name) => format!("Hi, {name}"),
            None => String::new(),
        }
    }

    fn on_leave(&self) {
        #[cfg(feature = "cover")]
        {
            self.images.set(Vec::new());
            self.images.render_to_terminal(&[], terminal_font_size());
        }
    }

    #[cfg(feature = "cover")]
    fn render_images(&self, overlays: &[Area]) {
        self.images
            .render_to_terminal(overlays, terminal_font_size());
    }

    fn on_command(&mut self, _s: &mut Cursive, cmd: &Command) -> Result<CommandResult, String> {
        let (sections, cards) = self.with_sections(|s| {
            (
                s.len(),
                s.get(self.section).map_or(0, |section| section.cards.len()),
            )
        });
        let last_section = sections.saturating_sub(1);
        let step = |amount: &MoveAmount| match amount {
            MoveAmount::Integer(n) => n.unsigned_abs() as usize,
            MoveAmount::Float(_) => 1,
            MoveAmount::Extreme => usize::MAX / 2,
        };
        match cmd {
            Command::Move(MoveMode::Up, amount) => {
                self.section = self.section.saturating_sub(step(amount));
            }
            Command::Move(MoveMode::Down, amount) => {
                self.section = self.section.saturating_add(step(amount)).min(last_section);
            }
            Command::Move(MoveMode::Left, amount) => {
                self.card = self.card.saturating_sub(step(amount));
            }
            Command::Move(MoveMode::Right, amount) => {
                self.card = self
                    .card
                    .saturating_add(step(amount))
                    .min(cards.saturating_sub(1));
            }
            Command::Play | Command::Open(TargetMode::Selected) => {
                return Ok(match self.activate() {
                    Some(view) => CommandResult::View(view),
                    None => CommandResult::Consumed(None),
                });
            }
            Command::Queue | Command::PlayNext => {
                if let Some((card, _)) = self.selected_card() {
                    let mut item = card.item.as_listitem();
                    if let Command::Queue = cmd {
                        item.queue(&self.queue);
                    } else {
                        item.play_next(&self.queue);
                    }
                }
            }
            _ => return Ok(CommandResult::Ignored),
        }
        // Keep the card within the (possibly shorter) row.
        let cards = self.with_sections(|s| s.get(self.section).map_or(0, |s| s.cards.len()));
        self.card = self.card.min(cards.saturating_sub(1));
        Ok(CommandResult::Consumed(None))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scroll_to_keeps_selection_visible() {
        assert_eq!(scroll_to(0, 2, 5), 0);
        assert_eq!(scroll_to(0, 7, 5), 3);
        assert_eq!(scroll_to(4, 2, 5), 2);
        assert_eq!(scroll_to(3, 7, 5), 3);
        assert_eq!(scroll_to(0, 3, 0), 3);
    }
}
