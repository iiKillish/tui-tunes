use std::sync::{Arc, RwLock};

use ratatui::Frame;
use ratatui::crossterm::event::MouseEvent;
use ratatui::layout::Rect;

use crate::command::Command;
use crate::config::SortingOrder;
use crate::model::playable::Playable;
use crate::model::playlist::Playlist;
use crate::tui::header::{HeaderAction, PageHeader};
use crate::tui::page::{Ctx, Outcome, Page};
use crate::tui::style::songs_and_length;
use crate::tui::widgets::list::ItemList;

/// A playlist: its tracks, numbered, under a header. `d` removes a track, `:sort` sorts.
pub struct PlaylistPage {
    playlist: Playlist,
    list: ItemList<Playable>,
}

impl PlaylistPage {
    pub fn new(playlist: Playlist, ctx: &Ctx) -> Self {
        let mut playlist = playlist;
        playlist.load_tracks(&ctx.spotify);
        if let Some(order) = ctx.library.cfg.state().playlist_orders.get(&playlist.id) {
            playlist.sort(&order.key, &order.direction);
        }
        let list = Self::list(&playlist);
        Self { playlist, list }
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
        self.list.draw(frame, area, ctx, focused);
    }

    fn command(&mut self, cmd: &Command, ctx: &Ctx) -> Outcome {
        match cmd {
            Command::Delete => {
                let index = self.list.selected();
                if self
                    .playlist
                    .delete_track(index, ctx.spotify.clone(), &ctx.library)
                {
                    self.list.remove(index);
                    Outcome::Consumed
                } else {
                    Outcome::Error("Could not delete the track".into())
                }
            }
            Command::Sort(key, direction) => {
                ctx.library.cfg.with_state_mut(|state| {
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
