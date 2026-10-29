use std::sync::{Arc, RwLock};
use std::thread;

use ratatui::Frame;
use ratatui::crossterm::event::MouseEvent;
use ratatui::layout::Rect;

use crate::command::Command;
use crate::model::artist::Artist;
use crate::model::track::Track;
use crate::traits::OpenTarget;
use crate::tui::header::{HeaderAction, PageHeader};
use crate::tui::page::{Ctx, Outcome, Page};
use crate::tui::widgets::list::ItemList;

/// An artist: their top tracks under a header with their picture.
pub struct ArtistPage {
    artist: Artist,
    top_tracks: ItemList<Track>,
}

impl ArtistPage {
    pub fn new(artist: Artist, ctx: &Ctx) -> Self {
        let tracks: Arc<RwLock<Vec<Track>>> = Arc::default();
        if let Some(id) = artist.id.clone() {
            let tracks = tracks.clone();
            let spotify = ctx.spotify.clone();
            let events = ctx.events.clone();
            thread::spawn(move || {
                if let Ok(top) = spotify.api.artist_top_tracks(&id) {
                    tracks.write().unwrap().extend(top);
                    events.trigger();
                }
            });
        }
        let source = format!("{} top tracks", artist.name);
        Self {
            top_tracks: ItemList::new(tracks)
                .numbered()
                .with_source(&source)
                .played_from(OpenTarget::Artist(artist.clone())),
            artist,
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
            details: vec!["Top tracks".into()],
            cover_url: self.artist.image_url(),
            playable: true,
            liked: Some(ctx.library.is_followed_artist(&self.artist)),
        })
    }

    fn header_action(&mut self, action: HeaderAction, ctx: &Ctx) -> Outcome {
        match action {
            HeaderAction::Play | HeaderAction::Shuffle => {
                ctx.queue.set_shuffle(action == HeaderAction::Shuffle);
                self.top_tracks.select(0);
                self.top_tracks.command(&Command::Play, ctx)
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
        self.top_tracks.draw(frame, area, ctx, focused);
    }

    fn command(&mut self, cmd: &Command, ctx: &Ctx) -> Outcome {
        self.top_tracks.command(cmd, ctx)
    }

    fn mouse(&mut self, event: MouseEvent, ctx: &Ctx) -> Outcome {
        self.top_tracks.mouse(event, ctx)
    }
}
