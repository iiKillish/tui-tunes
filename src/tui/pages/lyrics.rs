//! The Lyrics screen. Placeholder until it's built.

use crate::tui::page::Ctx;
use crate::tui::pages::MessagePage;

pub fn new(_ctx: &Ctx) -> MessagePage {
    MessagePage {
        title: "Lyrics".into(),
        message: "Lyrics are being built".into(),
    }
}
