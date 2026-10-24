//! Pages: the top-level screens and the pages opened on top of them.

pub mod album;
pub mod artist;
pub mod category;
pub mod folder;
pub mod home;
pub mod library;
pub mod liked;
pub mod loading;
pub mod lyrics;
pub mod playlist;
pub mod queue;
pub mod search;
pub mod search_results;
pub mod show;
pub mod stats;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use ratatui::Frame;
use ratatui::crossterm::event::MouseEvent;
use ratatui::layout::Rect;

use crate::command::Command;
use crate::pagination::ApiResult;
use crate::traits::ListItem;
use crate::tui::header::PageHeader;
use crate::tui::page::{Ctx, Outcome, Page};
use crate::tui::style::{Styles, put};
use crate::tui::widgets::list::ItemList;

/// A page that's a titled list, optionally with a header. The base for simple pages.
pub struct ListPage<I: ListItem + Clone> {
    pub title: String,
    pub list: ItemList<I>,
    pub header: Option<PageHeader>,
    /// Set while the items load in the background (see [Self::load_in_background]).
    loading: Option<Arc<AtomicBool>>,
}

impl<I: ListItem + Clone> ListPage<I> {
    pub fn new(title: impl Into<String>, list: ItemList<I>) -> Self {
        Self {
            title: title.into(),
            list,
            header: None,
            loading: None,
        }
    }

    /// Fill the list from `fetch` on a thread of its own, saying "Loading…" until it's done:
    /// its first page, then the next ones as the list is scrolled to the end.
    pub fn load_in_background(
        mut self,
        fetch: impl FnOnce() -> ApiResult<I> + Send + 'static,
    ) -> Self {
        let loading = Arc::new(AtomicBool::new(true));
        let content = self.list.content().clone();
        let pagination = self.list.pagination().clone();
        let done = loading.clone();
        std::thread::spawn(move || {
            let result = fetch();
            let first = std::mem::take(&mut *result.items.write().unwrap());
            let (loaded, total) = (first.len(), result.total as usize);
            content.write().unwrap().extend(first);
            pagination.set(
                loaded,
                total,
                Box::new(move |content| {
                    if let Some(items) = result.next() {
                        result.items.write().unwrap().clear();
                        content.write().unwrap().extend(items);
                    }
                }),
            );
            done.store(false, Ordering::Relaxed);
            crate::tui::request_redraw();
        });
        self.loading = Some(loading);
        self
    }

    pub fn with_header(mut self, header: PageHeader) -> Self {
        self.header = Some(header);
        self
    }
}

impl<I: ListItem + Clone> Page for ListPage<I> {
    fn title(&self) -> String {
        self.title.clone()
    }

    fn header(&self, _ctx: &Ctx) -> Option<PageHeader> {
        self.header.clone()
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect, ctx: &Ctx, focused: bool) {
        if let Some(loading) = &self.loading
            && self.list.is_empty()
        {
            let message = if loading.load(Ordering::Relaxed) {
                "Loading…"
            } else {
                "Nothing here"
            };
            put(
                frame.buffer_mut(),
                area.x + 2,
                area.y,
                message,
                usize::from(area.width.saturating_sub(2)),
                Styles(&ctx.palette).secondary(),
            );
            return;
        }
        self.list.draw(frame, area, ctx, focused);
    }

    fn command(&mut self, cmd: &Command, ctx: &Ctx) -> Outcome {
        self.list.command(cmd, ctx)
    }

    fn mouse(&mut self, event: MouseEvent, ctx: &Ctx) -> Outcome {
        self.list.mouse(event, ctx)
    }
}
