use std::sync::{Arc, Mutex, RwLock};

use ratatui::Frame;
use ratatui::crossterm::event::MouseEvent;
use ratatui::layout::Rect;

use crate::command::{Command, ShiftMode, SortDirection, SortKey};
use crate::config::SortingOrder;
use crate::model::playable::Playable;
use crate::model::playlist::Playlist;
use crate::tui::header::{HeaderAction, PageHeader};
use crate::tui::nav;
use crate::tui::page::{Ctx, Outcome, Page};
use crate::tui::style::{Styles, put, songs_and_length};
use crate::tui::widgets::list::ItemList;

/// A playlist: its tracks, numbered, under a header. `d` removes a track, `:shift` moves it,
/// `:sort` sorts and `:rename` renames the playlist.
/// The tracks are loaded in the background (it can take seconds), with "Loading…" shown until
/// they're there.
pub struct PlaylistPage {
    playlist: Playlist,
    list: ItemList<Playable>,
    /// The tracks once they've been loaded in the background, until they're put in the list.
    loaded: Arc<Mutex<Option<Vec<Playable>>>>,
}

impl PlaylistPage {
    pub fn new(playlist: Playlist, ctx: &Ctx) -> Self {
        let loaded = Arc::new(Mutex::new(None));
        if playlist.tracks.is_none() {
            let mut loading = playlist.without_tracks();
            let spotify = ctx.spotify.clone();
            let loaded = loaded.clone();
            std::thread::spawn(move || {
                loading.load_tracks(&spotify);
                *loaded.lock().unwrap() = loading.tracks;
                crate::tui::request_redraw();
            });
        }
        let mut page = Self {
            list: Self::list(&playlist),
            playlist,
            loaded,
        };
        page.sort_as_saved(ctx);
        page
    }

    /// Put the tracks in the list if they've arrived since the last look.
    fn take_loaded(&mut self, ctx: &Ctx) {
        let Some(tracks) = self.loaded.lock().unwrap().take() else {
            return;
        };
        self.playlist.tracks = Some(tracks);
        self.sort_as_saved(ctx);
        // The header, with the number of songs, was drawn before they arrived.
        crate::tui::request_redraw();
    }

    /// Sort the tracks in the order saved for this playlist with `sort`, if any, and show them.
    fn sort_as_saved(&mut self, ctx: &Ctx) {
        if self.playlist.tracks.is_none() {
            return;
        }
        if let Some(order) = ctx
            .library
            .cfg
            .state()
            .playlist_orders
            .get(&self.playlist.id)
        {
            self.playlist.sort(&order.key, &order.direction);
        }
        self.list = Self::list(&self.playlist);
    }

    fn list(playlist: &Playlist) -> ItemList<Playable> {
        let tracks = playlist.tracks.clone().unwrap_or_default();
        ItemList::new(Arc::new(RwLock::new(tracks)))
            .numbered()
            .with_source(&playlist.name)
    }

    fn is_own(&self, ctx: &Ctx) -> bool {
        ctx.library.user_id.as_deref() == Some(self.playlist.owner_id.as_str())
    }

    /// Move the selected track up or down `amount` places (negative turns the direction around).
    fn shift(&mut self, mode: &ShiftMode, amount: Option<i32>, ctx: &Ctx) -> Outcome {
        if !nav::can_edit(&self.playlist, ctx) {
            return Outcome::Error("Only your own playlists can be changed".into());
        }
        if ctx
            .library
            .cfg
            .state()
            .playlist_orders
            .contains_key(&self.playlist.id)
        {
            return Outcome::Error("Songs can't be moved while sorted: :sort original".into());
        }
        let (from, len) = (self.list.selected(), self.list.len());
        let amount = amount.unwrap_or(1);
        let up = matches!(mode, ShiftMode::Up) == (amount >= 0);
        let amount = amount.unsigned_abs() as usize;
        let to = if up {
            from.saturating_sub(amount)
        } else {
            (from + amount).min(len.saturating_sub(1))
        };
        if to == from || from >= len {
            return Outcome::Consumed;
        }
        if !self
            .playlist
            .move_track(from, to, &ctx.spotify, &ctx.library)
        {
            return Outcome::Error("Couldn't move the song".into());
        }
        *self.list.content().write().unwrap() = self.playlist.tracks.clone().unwrap_or_default();
        self.list.select(to);
        Outcome::Consumed
    }
}

impl Page for PlaylistPage {
    fn title(&self) -> String {
        self.playlist.name.clone()
    }

    fn header(&self, ctx: &Ctx) -> Option<PageHeader> {
        let mut details = Vec::new();
        if let Some(owner) = &self.playlist.owner_name {
            details.push(owner.clone());
        }
        if let Some(tracks) = &self.playlist.tracks {
            details.push(songs_and_length(tracks.iter().map(Playable::duration)));
        }
        Some(PageHeader {
            kind: if self.playlist.collaborative {
                "Collaborative playlist".into()
            } else {
                "Playlist".into()
            },
            title: self.playlist.name.clone(),
            details,
            cover_url: self.playlist.cover_url.clone(),
            playable: true,
            // Following only makes sense for other people's playlists.
            liked: (!self.is_own(ctx)).then(|| ctx.library.is_followed_playlist(&self.playlist)),
        })
    }

    fn header_action(&mut self, action: HeaderAction, ctx: &Ctx) -> Outcome {
        match action {
            HeaderAction::Play | HeaderAction::Shuffle => {
                ctx.queue.set_shuffle(action == HeaderAction::Shuffle);
                self.list.select(0);
                self.list.command(&Command::Play, ctx)
            }
            HeaderAction::Like => {
                if ctx.library.is_followed_playlist(&self.playlist) {
                    ctx.library.delete_playlist(&self.playlist.id);
                } else {
                    ctx.library.follow_playlist(self.playlist.clone());
                }
                Outcome::Consumed
            }
        }
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect, ctx: &Ctx, focused: bool) {
        self.take_loaded(ctx);
        if self.playlist.tracks.is_none() {
            put(
                frame.buffer_mut(),
                area.x + 2,
                area.y,
                "Loading…",
                usize::from(area.width.saturating_sub(2)),
                Styles(&ctx.palette).secondary(),
            );
            return;
        }
        self.list.draw(frame, area, ctx, focused);
    }

    fn command(&mut self, cmd: &Command, ctx: &Ctx) -> Outcome {
        self.take_loaded(ctx);
        match cmd {
            Command::Delete => {
                let index = self.list.selected();
                if self
                    .playlist
                    .delete_track(index, &ctx.spotify, &ctx.library)
                {
                    self.list.remove(index);
                    Outcome::Consumed
                } else {
                    Outcome::Error("Could not delete the track".into())
                }
            }
            Command::Shift(mode, amount) => self.shift(mode, *amount, ctx),
            Command::Rename(name) => {
                let outcome = nav::rename_playlist(&self.playlist, name, ctx);
                if matches!(outcome, Outcome::Message(_)) {
                    self.playlist.name = name.trim().to_string();
                    self.list = Self::list(&self.playlist);
                }
                outcome
            }
            Command::Sort(key, direction) => {
                ctx.library.cfg.with_state_mut(|state| {
                    // The playlist's own order is the default: nothing to remember.
                    if let (SortKey::Original, SortDirection::Ascending) = (key, direction) {
                        state.playlist_orders.remove(&self.playlist.id);
                        return;
                    }
                    state.playlist_orders.insert(
                        self.playlist.id.clone(),
                        SortingOrder {
                            key: key.clone(),
                            direction: direction.clone(),
                        },
                    );
                });
                self.playlist.sort(key, direction);
                self.list = Self::list(&self.playlist);
                Outcome::Consumed
            }
            _ => self.list.command(cmd, ctx),
        }
    }

    fn mouse(&mut self, event: MouseEvent, ctx: &Ctx) -> Outcome {
        self.list.mouse(event, ctx)
    }
}
