//! The Lyrics screen: the current track's lyrics, big and centered, following the song (see
//! [LyricsView]).

use ratatui::Frame;
use ratatui::crossterm::event::MouseEvent;
use ratatui::layout::Rect;

use crate::command::Command;
use crate::tui::page::{Ctx, Outcome, Page};
use crate::tui::widgets::lyrics::LyricsView;

/// Fewest columns kept free either side of the lyrics.
const MIN_MARGIN: u16 = 4;
/// Widest the lyrics get; on wider windows the margins grow.
const MAX_TEXT_WIDTH: u16 = 80;

pub struct LyricsPage {
    view: LyricsView,
}

pub fn new(_ctx: &Ctx) -> LyricsPage {
    LyricsPage {
        view: LyricsView::new().spacious(),
    }
}

/// Columns to leave either side of the lyrics in `width` columns.
fn margin(width: u16) -> u16 {
    MIN_MARGIN.max(width.saturating_sub(MAX_TEXT_WIDTH) / 2)
}

impl Page for LyricsPage {
    fn title(&self) -> String {
        "Lyrics".into()
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect, ctx: &Ctx, _focused: bool) {
        // A blank row under the title row.
        let area = Rect {
            y: area.y + area.height.min(1),
            height: area.height.saturating_sub(1),
            ..area
        };
        self.view.draw(frame, area, ctx, margin(area.width));
    }

    fn command(&mut self, cmd: &Command, ctx: &Ctx) -> Outcome {
        self.view.command(cmd, ctx)
    }

    fn mouse(&mut self, event: MouseEvent, ctx: &Ctx) -> Outcome {
        self.view.mouse(event, ctx)
    }

    fn on_show(&mut self, _ctx: &Ctx) {
        self.view.follow();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_margin() {
        assert_eq!(margin(40), MIN_MARGIN);
        assert_eq!(margin(80), MIN_MARGIN);
        assert_eq!(margin(120), 20);
    }
}
