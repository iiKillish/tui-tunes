use ratatui::Frame;
use ratatui::crossterm::event::MouseEvent;
use ratatui::layout::Rect;

use crate::command::Command;
use crate::traits::ListItem;
use crate::tui::page::{Ctx, Outcome, Page};
use crate::tui::style::{Styles, put};
use crate::tui::widgets::list::ItemList;

/// What starts loading a [LoadingList]'s items, run the first time it's drawn.
pub type Loader = Box<dyn FnOnce(&Ctx)>;

/// A titled list whose items arrive in the background (the library, search results): says so
/// while it's still empty, or that there's nothing once loading is done.
pub struct LoadingList<I: ListItem + Clone> {
    title: String,
    pub list: ItemList<I>,
    /// Whether the items are still on their way.
    loading: Box<dyn Fn() -> bool>,
    loader: Option<Loader>,
    loading_message: String,
    empty_message: String,
}

impl<I: ListItem + Clone> LoadingList<I> {
    pub fn new(title: impl Into<String>, list: ItemList<I>, loading: impl Fn() -> bool + 'static) -> Self {
        Self {
            title: title.into(),
            list,
            loading: Box::new(loading),
            loader: None,
            loading_message: "Loading…".into(),
            empty_message: "Nothing here yet".into(),
        }
    }

    /// Start loading with `loader` when the list is first shown, rather than right away.
    pub fn load_when_shown(mut self, loader: impl FnOnce(&Ctx) + 'static) -> Self {
        self.loader = Some(Box::new(loader));
        self
    }

    pub fn loading_message(mut self, message: impl Into<String>) -> Self {
        self.loading_message = message.into();
        self
    }

    pub fn empty_message(mut self, message: impl Into<String>) -> Self {
        self.empty_message = message.into();
        self
    }
}

impl<I: ListItem + Clone> Page for LoadingList<I> {
    fn title(&self) -> String {
        self.title.clone()
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect, ctx: &Ctx, focused: bool) {
        if let Some(loader) = self.loader.take() {
            loader(ctx);
        }
        if self.list.is_empty() {
            let message = if (self.loading)() { &self.loading_message } else { &self.empty_message };
            let styles = Styles(&ctx.palette);
            put(
                frame.buffer_mut(),
                area.x + 2,
                area.y,
                message,
                usize::from(area.width.saturating_sub(2)),
                styles.secondary(),
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
