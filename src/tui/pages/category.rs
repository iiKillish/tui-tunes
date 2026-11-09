use std::sync::atomic::Ordering;

use ratatui::Frame;
use ratatui::crossterm::event::MouseEvent;
use ratatui::layout::Rect;

use crate::command::Command;
use crate::model::category::Category;
use crate::model::playlist::Playlist;
use crate::tui::header::PageHeader;
use crate::tui::page::{Ctx, Outcome, Page};
use crate::tui::pages::ListPage;
use crate::tui::style::{Styles, put};
use crate::tui::widgets::list::ItemList;

/// Shown for a category without playlists. Spotify stopped giving them to apps registered since
/// 27 November 2024, which every ttunes user's own client ID is, so that's the likely reason
/// (the request fails); the categories themselves still load.
const NO_PLAYLISTS: &str =
    "No playlists: Spotify stopped giving apps the playlists of browse categories in late 2024";

/// A browse category: its playlists, loaded in the background.
pub struct CategoryPage(ListPage<Playlist>);

impl CategoryPage {
    pub fn new_category(category: Category, ctx: &Ctx) -> Self {
        let spotify = ctx.spotify.clone();
        let id = category.id.clone();
        Self(
            ListPage::new(category.name.clone(), ItemList::new(Default::default()))
                .load_in_background(move || spotify.api.category_playlists(&id)),
        )
    }

    /// Whether the playlists finished loading and there were none.
    fn loaded_empty(&self) -> bool {
        self.0.list.is_empty()
            && self
                .0
                .loading
                .as_ref()
                .is_some_and(|loading| !loading.load(Ordering::Relaxed))
    }
}

impl Page for CategoryPage {
    fn title(&self) -> String {
        self.0.title()
    }

    fn header(&self, ctx: &Ctx) -> Option<PageHeader> {
        self.0.header(ctx)
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect, ctx: &Ctx, focused: bool) {
        if self.loaded_empty() {
            put(
                frame.buffer_mut(),
                area.x + 2,
                area.y,
                NO_PLAYLISTS,
                usize::from(area.width.saturating_sub(2)),
                Styles(&ctx.palette).secondary(),
            );
            return;
        }
        self.0.draw(frame, area, ctx, focused);
    }

    fn command(&mut self, cmd: &Command, ctx: &Ctx) -> Outcome {
        self.0.command(cmd, ctx)
    }

    fn mouse(&mut self, event: MouseEvent, ctx: &Ctx) -> Outcome {
        self.0.mouse(event, ctx)
    }
}
