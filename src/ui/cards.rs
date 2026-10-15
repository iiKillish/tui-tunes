//! Cards with covers, like Spotify's home screen and library: albums, playlists, artists and
//! tracks shown as a cover with a title and subtitle under it.
//!
//! [CardGrid] lays cards out either as rows (one per section, scrolling sideways, as on the home
//! screen) or as one wrapped grid (as in the library). Arrow keys or hjkl move between cards,
//! Enter opens albums, playlists and artists and plays tracks, and cards can be clicked.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};

use cursive::event::{Callback, Event, EventResult, MouseButton, MouseEvent};
use cursive::theme::{ColorStyle, ColorType, Effect, PaletteColor};
use cursive::traits::View;
use cursive::view::Margins;
use cursive::views::Dialog;
use cursive::{Cursive, Printer, Vec2};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::command::{Command, JumpMode, MoveAmount, MoveMode, TargetMode};
use crate::commands::CommandResult;
use crate::library::Library;
use crate::model::album::Album;
use crate::model::artist::Artist;
use crate::model::playable::Playable;
use crate::model::playlist::Playlist;
use crate::model::track::Track;
use crate::queue::Queue;
use crate::traits::{ListItem, ViewExt};
use crate::ui::contextmenu::ContextMenu;
#[cfg(feature = "cover")]
use crate::ui::cover::{kitty_supported, request_frame_image, terminal_font_size};
use crate::ui::layout::Layout;
use crate::ui::listview::ListView;
use crate::ui::modal::Modal;
use crate::ui::pageheader::PageHeader;

/// Rows of a card's cover (when covers can be shown).
const COVER_ROWS: usize = 6;
/// Narrowest card, and the gap between cards.
const MIN_CARD_WIDTH: usize = 16;
const CARD_GAP: usize = 2;
/// Left margin.
const MARGIN: usize = 2;

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
}

impl Card {
    pub fn album(album: Album) -> Self {
        Self {
            title: album.title.clone(),
            subtitle: format!("Album • {}", album.artists.join(", ")),
            image: album.cover_url.clone(),
            item: CardItem::Album(album),
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
        }
    }

    pub fn artist(artist: Artist) -> Self {
        Self {
            title: artist.name.clone(),
            subtitle: "Artist".into(),
            // Looked up when the card is drawn (see draw_card), so only for cards on screen.
            image: artist.known_image_url(),
            item: CardItem::Artist(artist),
        }
    }

    pub fn track(track: Track) -> Self {
        Self {
            title: track.title.clone(),
            subtitle: track.artists.join(", "),
            image: track.cover_url.clone(),
            item: CardItem::Track(track),
        }
    }

    pub fn liked_songs(count: usize) -> Self {
        Self {
            item: CardItem::LikedSongs,
            title: "Liked Songs".into(),
            subtitle: format!("Playlist • {count} songs"),
            image: None,
        }
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

    /// Shown in place of a missing cover.
    fn placeholder(&self) -> &str {
        match self.item {
            CardItem::LikedSongs => "♥",
            CardItem::Artist(_) => "♫",
            _ => "♪",
        }
    }
}

#[derive(Clone)]
pub struct Section {
    /// Shown above the section's cards, unless empty.
    pub title: String,
    pub cards: Vec<Card>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CardLayout {
    /// One row per section, each scrolling sideways.
    Rows,
    /// The first section's cards wrapped into as many rows as needed, scrolling down.
    Grid,
}

/// Where the cards come from: called on every draw (and command), so the cards stay up to date
/// with their source. `None` while loading.
pub type CardSource = Box<dyn Fn() -> Option<Vec<Section>> + Send + Sync>;

pub struct CardGrid {
    queue: Arc<Queue>,
    library: Arc<Library>,
    layout: CardLayout,
    source: CardSource,
    /// Shown when there are no cards.
    empty_message: String,
    /// Selected section and card.
    section: usize,
    card: usize,
    /// Rows mode: first section shown, and the first card shown in each section. Grid mode: the
    /// first row of cards shown.
    first_section: AtomicUsize,
    first_cards: RwLock<Vec<usize>>,
    first_row: AtomicUsize,
    /// Cards per row in grid mode, as drawn last.
    columns: AtomicUsize,
    /// Where each card was drawn last (area within the view, section and card), for clicks.
    card_areas: RwLock<Vec<(Vec2, Vec2, usize, usize)>>,
    /// Matches of the last `/` search (section, card), and which one is selected.
    jump_matches: Vec<(usize, usize)>,
    jump_index: usize,
}

impl CardGrid {
    pub fn new(
        queue: Arc<Queue>,
        library: Arc<Library>,
        layout: CardLayout,
        source: CardSource,
    ) -> Self {
        Self {
            queue,
            library,
            layout,
            source,
            empty_message: "Nothing here yet".into(),
            section: 0,
            card: 0,
            first_section: AtomicUsize::new(0),
            first_cards: RwLock::new(Vec::new()),
            first_row: AtomicUsize::new(0),
            columns: AtomicUsize::new(1),
            card_areas: RwLock::new(Vec::new()),
            jump_matches: Vec::new(),
            jump_index: 0,
        }
    }

    pub fn empty_message(mut self, message: impl Into<String>) -> Self {
        self.empty_message = message.into();
        self
    }

    fn sections(&self) -> Option<Vec<Section>> {
        (self.source)()
    }

    fn section_len(&self, sections: &[Section], section: usize) -> usize {
        sections.get(section).map_or(0, |s| s.cards.len())
    }

    /// Select `section`, at the same place on screen as the card selected now (the second card
    /// shown stays the second card shown). Each row keeps its own sideways scroll.
    fn select_section(&mut self, sections: &[Section], section: usize) {
        if section == self.section {
            return;
        }
        let card = {
            let first_cards = self.first_cards.read().unwrap();
            let first = |s: usize| first_cards.get(s).copied().unwrap_or(0);
            first(section) + self.card.saturating_sub(first(self.section))
        };
        self.section = section;
        self.card = card.min(self.section_len(sections, section).saturating_sub(1));
    }

    /// Enter on a card: open albums, playlists and artists; play tracks, followed by the rest of
    /// their section.
    fn activate(&self, sections: &[Section]) -> Option<Box<dyn ViewExt>> {
        let section = sections.get(self.section)?;
        let card = section.cards.get(self.card)?;
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
                self.queue.clear();
                let first = self.queue.append_next_from(&tracks, source);
                self.queue.play(first + position, true, false);
                None
            }
            CardItem::LikedSongs => Some(Box::new(LikedSongsView::new(
                self.queue.clone(),
                self.library.clone(),
            ))),
            _ => card
                .as_listitem()?
                .open(self.queue.clone(), self.library.clone()),
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

    fn card_width(&self) -> usize {
        self.cover_width().max(MIN_CARD_WIDTH)
    }

    /// Rows of one card: the cover, title and subtitle, a blank row.
    fn card_rows(&self) -> usize {
        self.cover_rows() + 3
    }

    fn columns_for(&self, width: usize) -> usize {
        ((width.saturating_sub(MARGIN) + CARD_GAP) / (self.card_width() + CARD_GAP)).max(1)
    }

    /// Draw `card` with its top-left corner at `at`; `selected` draws its text highlighted.
    fn draw_card(&self, printer: &Printer<'_, '_>, card: &Card, at: Vec2, selected: bool) {
        let card_width = self.card_width();
        let cover_rows = self.cover_rows();
        let cover_width = self.cover_width();
        if cover_rows > 0 {
            let image = card.image.clone().or_else(|| match &card.item {
                CardItem::Artist(artist) => artist.image_url(),
                _ => None,
            });
            match &image {
                #[cfg(feature = "cover")]
                Some(url) => request_frame_image(
                    url.clone(),
                    printer.offset + at,
                    Vec2::new(cover_width, cover_rows),
                ),
                _ => {
                    // A plain square with a symbol where the cover would be.
                    let style = ColorStyle::new(
                        ColorType::Palette(PaletteColor::Secondary),
                        ColorType::Palette(PaletteColor::HighlightInactive),
                    );
                    printer.with_color(style, |printer| {
                        for row in 0..cover_rows {
                            printer.print_hline(at + (0, row), cover_width, " ");
                        }
                        printer.print(
                            at + (cover_width / 2, cover_rows / 2 - 1),
                            card.placeholder(),
                        );
                    });
                }
            }
        }

        let text_top = at.y + cover_rows;
        let (title_style, subtitle_style) = if selected {
            (ColorStyle::highlight(), ColorStyle::highlight())
        } else {
            (ColorStyle::primary(), ColorStyle::secondary())
        };
        if selected {
            printer.with_color(ColorStyle::highlight(), |printer| {
                for row in text_top..text_top + 2 {
                    printer.print_hline((at.x, row), card_width, " ");
                }
            });
        }
        printer.with_color(title_style, |printer| {
            printer.with_effect(Effect::Bold, |printer| {
                printer.print((at.x, text_top), &truncate(&card.title, card_width));
            });
        });
        printer.with_color(subtitle_style, |printer| {
            printer.print((at.x, text_top + 1), &truncate(&card.subtitle, card_width));
        });
    }

    fn draw_section_title(&self, printer: &Printer<'_, '_>, title: &str, y: usize) {
        printer.with_color(ColorStyle::title_primary(), |printer| {
            printer.with_effect(Effect::Bold, |printer| {
                printer.print((MARGIN, y), title);
            });
        });
    }

    fn draw_rows(&self, printer: &Printer<'_, '_>, sections: &[Section]) -> Vec<CardArea> {
        let size = printer.size;
        let mut areas = Vec::new();
        let section_rows = 1 + self.card_rows();
        let visible_sections = (size.y / section_rows).max(1);
        let first_section = scroll_to(
            self.first_section.load(Ordering::Relaxed),
            self.section,
            visible_sections,
        );
        self.first_section.store(first_section, Ordering::Relaxed);

        let card_width = self.card_width();
        let visible_cards = self.columns_for(size.x);
        let mut first_cards = self.first_cards.write().unwrap();
        first_cards.resize(sections.len(), 0);

        for (s, section) in sections.iter().enumerate().skip(first_section) {
            let top = (s - first_section) * section_rows;
            if top + section_rows > size.y + 1 {
                break;
            }
            self.draw_section_title(printer, &section.title, top);
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
                let at = Vec2::new(
                    MARGIN + (c - first_cards[s]) * (card_width + CARD_GAP),
                    top + 1,
                );
                let selected = printer.focused && s == self.section && c == self.card;
                self.draw_card(printer, card, at, selected);
                areas.push((at, Vec2::new(card_width, self.card_rows() - 1), s, c));
            }
        }
        areas
    }

    fn draw_grid(&self, printer: &Printer<'_, '_>, sections: &[Section]) -> Vec<CardArea> {
        let size = printer.size;
        let mut areas = Vec::new();
        let Some(section) = sections.first() else {
            return areas;
        };
        let mut top = 0;
        if !section.title.is_empty() {
            self.draw_section_title(printer, &section.title, 0);
            top = 2;
        }
        let columns = self.columns_for(size.x);
        self.columns.store(columns, Ordering::Relaxed);
        let card_rows = self.card_rows();
        let visible_rows = (size.y.saturating_sub(top) / card_rows).max(1);
        let first_row = scroll_to(
            self.first_row.load(Ordering::Relaxed),
            self.card / columns,
            visible_rows,
        );
        self.first_row.store(first_row, Ordering::Relaxed);

        let card_width = self.card_width();
        for (c, card) in section
            .cards
            .iter()
            .enumerate()
            .skip(first_row * columns)
            .take(visible_rows * columns)
        {
            let row = c / columns - first_row;
            let at = Vec2::new(
                MARGIN + (c % columns) * (card_width + CARD_GAP),
                top + row * card_rows,
            );
            let selected = printer.focused && self.section == 0 && c == self.card;
            self.draw_card(printer, card, at, selected);
            areas.push((at, Vec2::new(card_width, card_rows - 1), 0, c));
        }
        areas
    }

    /// Ask before deleting the selected playlist, like the playlists list does.
    fn delete_dialog(&self, card: &Card) -> Option<Modal<Dialog>> {
        let CardItem::Playlist(playlist) = &card.item else {
            return None;
        };
        let library = self.library.clone();
        let id = playlist.id.clone();
        let dialog = Dialog::text("Are you sure you want to delete this playlist?")
            .padding(Margins::lrtb(1, 1, 1, 0))
            .title("Delete playlist")
            .dismiss_button("No")
            .button("Yes", move |s: &mut Cursive| {
                library.delete_playlist(&id);
                s.pop_layer();
            });
        Some(Modal::new(dialog))
    }

    /// Move the selection; returns false when it's already at the edge in that direction.
    fn move_selection(
        &mut self,
        sections: &[Section],
        mode: &MoveMode,
        amount: &MoveAmount,
    ) -> bool {
        let step = match amount {
            MoveAmount::Integer(n) => n.unsigned_abs() as usize,
            MoveAmount::Float(_) => 3,
            MoveAmount::Extreme => usize::MAX / 2,
        };
        let len = self.section_len(sections, self.section);
        let last_section = sections.len().saturating_sub(1);
        let columns = self.columns.load(Ordering::Relaxed).max(1);
        match (self.layout, mode) {
            (_, MoveMode::Left) if self.card == 0 => return false,
            (_, MoveMode::Right) if self.card + 1 >= len => return false,
            (_, MoveMode::Left) => self.card = self.card.saturating_sub(step),
            (_, MoveMode::Right) => self.card = (self.card + step).min(len.saturating_sub(1)),
            (CardLayout::Rows, MoveMode::Up) => {
                self.select_section(sections, self.section.saturating_sub(step))
            }
            (CardLayout::Rows, MoveMode::Down) => {
                self.select_section(sections, (self.section + step).min(last_section))
            }
            (CardLayout::Grid, MoveMode::Up) => {
                self.card = self.card.saturating_sub(step.saturating_mul(columns));
            }
            (CardLayout::Grid, MoveMode::Down) => {
                let target = self.card.saturating_add(step.saturating_mul(columns));
                // Into the last, partly filled row too.
                if target < len || self.card / columns < (len.saturating_sub(1)) / columns {
                    self.card = target.min(len.saturating_sub(1));
                }
            }
            (_, MoveMode::Playing) => {}
        }
        true
    }

    fn jump_to_match(&mut self) {
        if let Some(&(section, card)) = self.jump_matches.get(self.jump_index) {
            self.section = section;
            self.card = card;
        }
    }
}

type CardArea = (Vec2, Vec2, usize, usize);

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

impl View for CardGrid {
    fn draw(&self, printer: &Printer<'_, '_>) {
        let areas = match self.sections() {
            None => {
                printer.with_color(ColorStyle::secondary(), |printer| {
                    printer.print((MARGIN, 1), "Loading…");
                });
                Vec::new()
            }
            Some(sections) if sections.iter().all(|s| s.cards.is_empty()) => {
                printer.with_color(ColorStyle::secondary(), |printer| {
                    printer.print((MARGIN, 1), &self.empty_message);
                });
                Vec::new()
            }
            Some(sections) => match self.layout {
                CardLayout::Rows => self.draw_rows(printer, &sections),
                CardLayout::Grid => self.draw_grid(printer, &sections),
            },
        };
        *self.card_areas.write().unwrap() = areas;
    }

    fn layout(&mut self, size: Vec2) {
        self.columns
            .store(self.columns_for(size.x), Ordering::Relaxed);
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
        let sections = self.sections().unwrap_or_default();
        match event {
            MouseEvent::Press(MouseButton::Left) => {
                let clicked = self
                    .card_areas
                    .read()
                    .unwrap()
                    .iter()
                    .find(|(at, size, _, _)| position.fits_in_rect(*at, *size))
                    .map(|&(_, _, s, c)| (s, c));
                if let Some((section, card)) = clicked {
                    self.section = section;
                    self.card = card;
                    if let Some(view) = self.activate(&sections) {
                        return EventResult::Consumed(Some(Callback::from_fn_once(
                            move |s: &mut Cursive| {
                                s.call_on_name("main", move |l: &mut Layout| l.push_view(view));
                            },
                        )));
                    }
                }
            }
            MouseEvent::WheelUp => {
                self.move_selection(&sections, &MoveMode::Up, &MoveAmount::Integer(1));
            }
            MouseEvent::WheelDown => {
                self.move_selection(&sections, &MoveMode::Down, &MoveAmount::Integer(1));
            }
            _ => return EventResult::Ignored,
        }
        EventResult::consumed()
    }
}

impl ViewExt for CardGrid {
    fn handles_horizontal_moves(&self) -> bool {
        true
    }

    fn on_command(&mut self, s: &mut Cursive, cmd: &Command) -> Result<CommandResult, String> {
        let sections = self.sections().unwrap_or_default();
        let selected = sections
            .get(self.section)
            .and_then(|section| section.cards.get(self.card))
            .cloned();
        match cmd {
            Command::Move(mode, amount) => {
                return Ok(if self.move_selection(&sections, mode, amount) {
                    CommandResult::Consumed(None)
                } else {
                    CommandResult::Ignored
                });
            }
            Command::Play => {
                return Ok(match self.activate(&sections) {
                    Some(view) => CommandResult::View(view),
                    None => CommandResult::Consumed(None),
                });
            }
            Command::Open(TargetMode::Selected) => {
                // Tracks get their context menu, like in lists.
                if let Some(card) = &selected
                    && let CardItem::Track(track) = &card.item
                {
                    let menu = ContextMenu::new(
                        &*track.as_listitem(),
                        self.queue.clone(),
                        self.library.clone(),
                    );
                    return Ok(CommandResult::Modal(Box::new(menu)));
                }
                return Ok(match self.activate(&sections) {
                    Some(view) => CommandResult::View(view),
                    None => CommandResult::Consumed(None),
                });
            }
            Command::Queue | Command::PlayNext | Command::Save => {
                if let Some(mut item) = selected.as_ref().and_then(Card::as_listitem) {
                    match cmd {
                        Command::Queue => item.queue(&self.queue),
                        Command::PlayNext => item.play_next(&self.queue),
                        _ => item.save(&self.library),
                    }
                }
            }
            Command::Delete => {
                if let Some(dialog) = selected.as_ref().and_then(|c| self.delete_dialog(c)) {
                    s.add_layer(dialog);
                }
            }
            Command::Jump(mode) => {
                match mode {
                    JumpMode::Query(query) => {
                        let query = query.to_lowercase();
                        self.jump_matches = sections
                            .iter()
                            .enumerate()
                            .flat_map(|(s, section)| {
                                section.cards.iter().enumerate().filter_map({
                                    let query = &query;
                                    move |(c, card)| {
                                        let text = format!("{} {}", card.title, card.subtitle);
                                        text.to_lowercase().contains(query).then_some((s, c))
                                    }
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
                self.jump_to_match();
            }
            _ => return Ok(CommandResult::Ignored),
        }
        Ok(CommandResult::Consumed(None))
    }
}

/// The user's saved tracks as a page, opened from the Liked Songs card.
pub struct LikedSongsView {
    list: ListView<Track>,
    library: Arc<Library>,
}

impl LikedSongsView {
    pub fn new(queue: Arc<Queue>, library: Arc<Library>) -> Self {
        Self {
            list: ListView::new(library.tracks.clone(), queue, library.clone())
                .with_source("Liked Songs"),
            library,
        }
    }
}

impl cursive::view::ViewWrapper for LikedSongsView {
    cursive::wrap_impl!(self.list: ListView<Track>);
}

impl ViewExt for LikedSongsView {
    fn title(&self) -> String {
        "Liked Songs".into()
    }

    fn header(&self) -> Option<PageHeader> {
        let tracks = self.library.tracks.read().unwrap();
        Some(PageHeader {
            kind: "Playlist".into(),
            title: "Liked Songs".into(),
            details: vec![PageHeader::songs_and_length(
                tracks.iter().map(|t| t.duration),
            )],
            cover_url: None,
        })
    }

    fn on_command(&mut self, s: &mut Cursive, cmd: &Command) -> Result<CommandResult, String> {
        self.list.on_command(s, cmd)
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

    #[test]
    fn test_truncate() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello world", 6), "hello…");
    }
}
