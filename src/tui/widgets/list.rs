//! A scrolling list of items (tracks, albums, playlists, artists, episodes…), one per row: the
//! track lists of playlist and album pages, search results, Liked Songs.
//!
//! Each row shows an optional number, an optional small cover, then the item's text from
//! [ListItem::display_left], [ListItem::display_center] and [ListItem::display_right]. Handles
//! the usual list commands (move, play, queue, play next, save, open, go to album/artist, share,
//! `/` search) and loads more items from Spotify when scrolled to the end.

use std::cmp::min;
use std::sync::{Arc, RwLock};

use ratatui::Frame;
use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

use crate::command::{Command, GotoMode, JumpMode, MoveAmount, MoveMode, TargetMode};
use crate::model::playable::Playable;
use crate::model::track::Track;
use crate::pagination::Pagination;
use crate::traits::ListItem;
use crate::tui::hints;
use crate::tui::images;
use crate::tui::nav;
use crate::tui::page::{Ctx, Outcome};
use crate::tui::pointer;
use crate::tui::style::{SELECTION_MARKER, Styles, bold, fill, put, scroll_to, width};
use crate::utils::search_text;

/// Narrowest list that shows covers before its items.
const MIN_WIDTH_FOR_THUMBNAILS: u16 = 50;

pub struct ItemList<I: ListItem + Clone> {
    content: Arc<RwLock<Vec<I>>>,
    pagination: Pagination<I>,
    selected: usize,
    /// First row shown.
    first: usize,
    /// Where the list was drawn last, for the mouse and page sizes.
    area: Rect,
    /// Show the position (1, 2, 3…) before each item, like an album's track numbers.
    numbered: bool,
    /// Show a small cover before each item that has one.
    thumbnails: bool,
    /// Whether tracks show their album in a column of its own.
    album_column: bool,
    /// Name of what these items are (an album, playlist…), recorded in the queue when playing
    /// from this list so Up next can show "Next from: …".
    source: Option<String>,
    /// The last `/` search and the rows matching it.
    search: String,
    matches: Vec<usize>,
    match_index: usize,
}

impl<I: ListItem + Clone> ItemList<I> {
    pub fn new(content: Arc<RwLock<Vec<I>>>) -> Self {
        Self {
            content,
            pagination: Pagination::default(),
            selected: 0,
            first: 0,
            area: Rect::default(),
            numbered: false,
            thumbnails: true,
            album_column: true,
            source: None,
            search: String::new(),
            matches: Vec::new(),
            match_index: 0,
        }
    }

    pub fn numbered(mut self) -> Self {
        self.numbered = true;
        self
    }

    /// No covers before the items, e.g. for an album's tracks, which all share one.
    pub fn without_thumbnails(mut self) -> Self {
        self.thumbnails = false;
        self
    }

    /// No album column, e.g. on an album's own page.
    pub fn without_album(mut self) -> Self {
        self.album_column = false;
        self
    }

    pub fn with_source(mut self, source: &str) -> Self {
        self.source = Some(source.to_string());
        self
    }

    pub fn pagination(&self) -> &Pagination<I> {
        &self.pagination
    }

    pub fn content(&self) -> &Arc<RwLock<Vec<I>>> {
        &self.content
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn selected_item(&self) -> Option<I> {
        self.content.read().unwrap().get(self.selected).cloned()
    }

    pub fn len(&self) -> usize {
        self.content.read().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn select(&mut self, index: usize) {
        self.selected = min(index, self.len().saturating_sub(1));
    }

    /// Remove the item at `index` (e.g. after deleting it from a playlist).
    pub fn remove(&mut self, index: usize) {
        let mut content = self.content.write().unwrap();
        if index < content.len() {
            content.remove(index);
        }
        if self.selected >= content.len() {
            self.selected = content.len().saturating_sub(1);
        }
    }

    fn can_paginate(&self) -> bool {
        self.pagination
            .max_content()
            .is_some_and(|max| max > self.pagination.loaded_content())
    }

    /// Load the next page if the selection or the view reached the end.
    fn try_paginate(&self, ctx: &Ctx) {
        let len = self.len();
        let visible = usize::from(self.area.height);
        if self.can_paginate() && (self.selected + 1 >= len || self.first + visible >= len) {
            self.pagination.call(&self.content, ctx.library.clone());
        }
    }

    /// Width of the covers before items, or 0 if there are none (or the list is too narrow to
    /// spare the room).
    fn thumbnail_width(&self, content: &[I], list_width: u16) -> u16 {
        if self.thumbnails
            && images::supported()
            && list_width >= MIN_WIDTH_FOR_THUMBNAILS
            && content.iter().take(20).any(|i| i.thumbnail_url().is_some())
        {
            images::square_width(1)
        } else {
            0
        }
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect, ctx: &Ctx, focused: bool) {
        self.area = area;
        let styles = Styles(&ctx.palette);
        let rows = usize::from(area.height);
        let paginator = usize::from(self.can_paginate());
        // Only the rows on screen, copied so the list isn't locked while they're drawn: drawing
        // asks the library things (is this saved?), and the list may be one of the library's.
        let (len, thumb, visible) = {
            let content = self.content.read().unwrap();
            let len = content.len();
            self.selected = min(self.selected, len.saturating_sub(1));
            self.first = scroll_to(self.first, self.selected, rows);
            self.first = min(self.first, (len + paginator).saturating_sub(rows));
            let thumb = self.thumbnail_width(&content, area.width);
            let end = min(len, self.first + rows);
            (len, thumb, content[self.first.min(end)..end].to_vec())
        };

        let number_width = if self.numbered {
            width(&len.to_string()).max(2) + 2
        } else {
            0
        };
        let current = ctx.queue.get_current_index();
        let buf = frame.buffer_mut();

        for row in 0..rows {
            let index = self.first + row;
            let y = area.y + row as u16;
            let line = Rect::new(area.x, y, area.width, 1);
            if index == len && paginator == 1 {
                let max = self.pagination.max_content().unwrap_or(len);
                put(
                    buf,
                    area.x + 1,
                    y,
                    &format!("{} more items, scroll to load", max.saturating_sub(len)),
                    usize::from(area.width),
                    styles.secondary(),
                );
                continue;
            }
            let Some(item) = visible.get(row) else {
                break;
            };
            pointer::clickable(line);
            let playing = item.is_playing(&ctx.queue) && current.is_some();
            let unplayable = item
                .track_ref()
                .is_some_and(|t| t.is_local || t.is_playable == Some(false));
            let style = if index == self.selected {
                if focused {
                    styles.selected()
                } else {
                    styles.selected_inactive()
                }
            } else if playing {
                styles.playing()
            } else if unplayable {
                styles.secondary()
            } else {
                styles.primary()
            };
            fill(buf, line, style);
            if index == self.selected && focused {
                put(buf, area.x, y, SELECTION_MARKER, 1, styles.marker());
                hints::add_row(
                    line,
                    vec![
                        Command::Play,
                        Command::Queue,
                        Command::PlayNext,
                        Command::Open(TargetMode::Selected),
                    ],
                );
            }

            let mut x = area.x + 1;
            if self.numbered {
                let label = if playing {
                    "▶".to_string()
                } else {
                    (index + 1).to_string()
                };
                let number_style = if index == self.selected || playing {
                    style
                } else {
                    styles.secondary()
                };
                let pad = number_width.saturating_sub(width(&label) + 1);
                put(buf, x + pad as u16, y, &label, number_width, number_style);
                x += number_width as u16;
            }
            if thumb > 0 {
                if let Some(url) = item.thumbnail_url() {
                    images::cover(&url, Rect::new(x, y, thumb, 1));
                }
                x += thumb + 1;
            }

            // Tracks (design R2): the title, then the artists dimmed, then the album in its own
            // column (left out on album pages). Other items use their own display text.
            let (left, artists, center) = match item.track_ref() {
                Some(track) => (
                    track.title.clone(),
                    track.artists.join(", "),
                    if self.album_column {
                        track.album.clone().unwrap_or_default()
                    } else {
                        String::new()
                    },
                ),
                None => (
                    item.display_left(&ctx.library),
                    String::new(),
                    item.display_center(&ctx.library),
                ),
            };
            let right = item.display_right(&ctx.library);
            let right_width = width(&right);
            let end = area.right().saturating_sub(1);
            let right_x = end.saturating_sub(right_width as u16);
            put(buf, right_x, y, &right, right_width, style);

            let room = usize::from(right_x.saturating_sub(x + 1));
            let title_style = if playing || index == self.selected {
                bold(style)
            } else {
                style
            };
            let dim_style = if index == self.selected || playing {
                style
            } else {
                styles
                    .secondary()
                    .bg(style.bg.unwrap_or(ctx.palette.background))
            };
            let left_room = if center.is_empty() {
                room
            } else {
                room * 6 / 10
            };
            let written = put(buf, x, y, &left, left_room.saturating_sub(1), title_style);
            if !artists.is_empty() {
                let artists_x = x + written as u16 + 2;
                let artists_room = left_room.saturating_sub(written + 3);
                if artists_room > 3 {
                    put(buf, artists_x, y, &artists, artists_room, dim_style);
                }
            }
            if !center.is_empty() {
                let center_x = x + left_room as u16;
                let center_room = usize::from(right_x.saturating_sub(center_x + 1));
                let center_style = if index == self.selected {
                    style
                } else {
                    styles.secondary()
                };
                put(buf, center_x, y, &center, center_room, center_style);
            }

            // `/` search matches stand out.
            if self.matches.contains(&index) && index != self.selected {
                put(
                    buf,
                    area.x,
                    y,
                    "›",
                    1,
                    styles
                        .accent()
                        .bg(style.bg.unwrap_or(ctx.palette.background)),
                );
            }
        }
    }

    /// Play from the selected item: for lists of tracks, the whole list from there (so the rest
    /// follows in the queue); otherwise the item itself (an album, a playlist…).
    fn play(&self, ctx: &Ctx) {
        let content = self.content.read().unwrap();
        let any = &*content as &dyn std::any::Any;
        let tracks: Option<Vec<Playable>> =
            any.downcast_ref::<Vec<Playable>>().cloned().or_else(|| {
                any.downcast_ref::<Vec<Track>>()
                    .map(|t| t.iter().cloned().map(Playable::Track).collect())
            });
        ctx.queue.clear();
        match tracks {
            Some(tracks) => {
                let index = ctx.queue.append_next_from(&tracks, self.source.as_deref());
                ctx.queue.play(index + self.selected, true, false);
            }
            None => {
                if let Some(item) = content.get(self.selected) {
                    item.clone().play(&ctx.queue);
                }
            }
        }
    }

    /// Open the selected item: its page, or for tracks and episodes, a menu.
    fn open(&self, ctx: &Ctx) -> Outcome {
        let Some(item) = self.selected_item() else {
            return Outcome::Consumed;
        };
        match item.target() {
            Some(target) => Outcome::Open(nav::open_target(target, ctx)),
            None => Outcome::Overlay(nav::context_menu(item.as_listitem(), ctx)),
        }
    }

    fn jump(&mut self, mode: &JumpMode, ctx: &Ctx) {
        match mode {
            JumpMode::Query(query) => {
                self.search = search_text(query);
                let content = self.content.read().unwrap();
                self.matches = content
                    .iter()
                    .enumerate()
                    .filter(|(_, item)| {
                        search_text(&item.display_left(&ctx.library)).contains(&self.search)
                    })
                    .map(|(i, _)| i)
                    .collect();
                self.match_index = 0;
            }
            JumpMode::Next if !self.matches.is_empty() => {
                self.match_index = (self.match_index + 1) % self.matches.len();
            }
            JumpMode::Previous if !self.matches.is_empty() => {
                self.match_index = self
                    .match_index
                    .checked_sub(1)
                    .unwrap_or(self.matches.len() - 1);
            }
            _ => {}
        }
        if let Some(&index) = self.matches.get(self.match_index) {
            self.selected = index;
        }
    }

    pub fn command(&mut self, cmd: &Command, ctx: &Ctx) -> Outcome {
        let page = usize::from(self.area.height).max(1);
        match cmd {
            Command::Move(MoveMode::Up, amount) => {
                let step = match amount {
                    MoveAmount::Integer(n) => n.unsigned_abs() as usize,
                    MoveAmount::Float(scale) => (page as f32 * scale) as usize,
                    MoveAmount::Extreme => usize::MAX,
                };
                self.selected = self.selected.saturating_sub(step);
            }
            Command::Move(MoveMode::Down, amount) => {
                let step = match amount {
                    MoveAmount::Integer(n) => n.unsigned_abs() as usize,
                    MoveAmount::Float(scale) => (page as f32 * scale) as usize,
                    MoveAmount::Extreme => usize::MAX,
                };
                self.select(self.selected.saturating_add(step));
                self.try_paginate(ctx);
            }
            Command::Move(MoveMode::Playing, _) => {
                let content = self.content.read().unwrap();
                match content.iter().position(|i| i.is_playing(&ctx.queue)) {
                    Some(index) => self.selected = index,
                    None if ctx.queue.get_current().is_some() => {
                        return Outcome::Message("The playing track isn't in this list".into());
                    }
                    None => return Outcome::Message("Nothing is playing".into()),
                }
            }
            Command::Play => self.play(ctx),
            Command::PlayNext | Command::Queue => {
                if let Some(item) = self.selected_item() {
                    nav::queue_item(item.as_listitem(), matches!(cmd, Command::PlayNext), ctx);
                }
            }
            Command::Save => {
                if let Some(mut item) = self.selected_item() {
                    item.save(&ctx.library);
                }
            }
            Command::Add => {
                if let Some(track) = self.selected_item().and_then(|i| i.track()) {
                    return Outcome::Overlay(nav::add_to_playlist(vec![track], ctx));
                }
            }
            Command::Open(TargetMode::Selected) => return self.open(ctx),
            Command::Goto(mode) => {
                let Some(item) = self.selected_item() else {
                    return Outcome::Consumed;
                };
                return match mode {
                    GotoMode::Album => match item.album() {
                        Some(album) => Outcome::Open(nav::open_target(
                            crate::traits::OpenTarget::Album(album),
                            ctx,
                        )),
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
                if let Some(url) = self.selected_item().and_then(|i| i.share_url()) {
                    let _ = crate::sharing::write_share(url);
                    return Outcome::Message("Link copied".into());
                }
            }
            Command::Jump(mode) => self.jump(mode, ctx),
            _ => return Outcome::Ignored,
        }
        Outcome::Consumed
    }

    pub fn mouse(&mut self, event: MouseEvent, ctx: &Ctx) -> Outcome {
        match event.kind {
            MouseEventKind::ScrollDown => {
                let len = self.len();
                let rows = usize::from(self.area.height);
                self.first = min(self.first + 3, len.saturating_sub(rows));
                if self.selected < self.first {
                    self.selected = self.first;
                }
                self.try_paginate(ctx);
            }
            MouseEventKind::ScrollUp => {
                self.first = self.first.saturating_sub(3);
                let last_visible = self.first + usize::from(self.area.height).saturating_sub(1);
                if self.selected > last_visible {
                    self.selected = last_visible;
                }
            }
            MouseEventKind::Down(MouseButton::Left) => {
                // Only rows of the list; it may sit below something else, like a tab row.
                if !self.area.contains((event.column, event.row).into()) {
                    return Outcome::Consumed;
                }
                let row = usize::from(event.row - self.area.y);
                let index = self.first + row;
                if index >= self.len() {
                    return Outcome::Consumed;
                }
                // Clicking the selected track plays it; clicking an album or playlist opens it.
                let already = index == self.selected;
                self.selected = index;
                let is_track = self
                    .selected_item()
                    .is_some_and(|i| i.track().is_some() || i.is_playable());
                if is_track {
                    if already {
                        self.play(ctx);
                    }
                } else {
                    return self.open(ctx);
                }
            }
            _ => return Outcome::Ignored,
        }
        Outcome::Consumed
    }
}
