//! The Search screen. Placeholder until it's built.

use crate::tui::page::Ctx;
use crate::tui::pages::MessagePage;

pub fn new(_ctx: &Ctx) -> MessagePage {
    MessagePage { title: "Search".into(), message: "Search is being built".into() }
}
