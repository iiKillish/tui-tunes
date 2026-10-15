//! The Up next screen. Placeholder until it's built.

use crate::tui::page::Ctx;
use crate::tui::pages::MessagePage;

pub fn new(_ctx: &Ctx) -> MessagePage {
    MessagePage { title: "Up next".into(), message: "Up next is being built".into() }
}
