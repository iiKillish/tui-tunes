//! The Home screen (design H3). Placeholder until it's built.

use crate::tui::page::Ctx;
use crate::tui::pages::MessagePage;

pub fn new(_ctx: &Ctx) -> MessagePage {
    MessagePage { title: "Home".into(), message: "Home is being built".into() }
}
