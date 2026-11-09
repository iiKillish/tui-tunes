//! Reordering by dragging, in Up next and the user's own playlists: a row pressed and dragged up
//! or down to move it, with the list showing it where it would go, and scrolling while it's held
//! at the top or bottom edge. A press let go without moving to another row is a click.

use std::time::{Duration, Instant};

use ratatui::layout::Rect;

/// How often a drag held at the top or bottom edge scrolls the list by a row.
const SCROLL_EVERY: Duration = Duration::from_millis(60);

/// A row being pressed, and maybe dragged to another place.
#[derive(Clone, Copy)]
pub struct Drag {
    /// The pressed row's position in the list.
    pub from: usize,
    /// Where it would go if let go now.
    pub to: usize,
    /// The pointer's row on screen.
    pub row: u16,
    start_row: u16,
    /// It moved to another row: letting go moves it rather than clicking it.
    pub moved: bool,
    /// Whether the pressed row was already selected (the click's meaning depends on it).
    pub was_selected: bool,
    last_scroll: Option<Instant>,
}

impl Drag {
    pub fn new(from: usize, row: u16, was_selected: bool) -> Self {
        Self {
            from,
            to: from,
            row,
            start_row: row,
            moved: false,
            was_selected,
            last_scroll: None,
        }
    }

    /// The pointer moved to `row`.
    pub fn move_to(&mut self, row: u16) {
        self.row = row;
        self.moved |= row != self.start_row;
    }

    /// Rows to scroll `area` by as it's drawn: one up or down every so often while the pointer is
    /// at (or past) its top or bottom row, so a row can be dragged further than what's shown.
    pub fn scroll_step(&mut self, area: Rect) -> isize {
        if !self.moved || area.height < 3 {
            return 0;
        }
        let step = if self.row <= area.y {
            -1
        } else if self.row >= area.bottom().saturating_sub(1) {
            1
        } else {
            return 0;
        };
        if self
            .last_scroll
            .is_some_and(|at| at.elapsed() < SCROLL_EVERY)
        {
            return 0;
        }
        self.last_scroll = Some(Instant::now());
        step
    }

    /// The position whose item shows at `position` while the dragged one is shown at
    /// [Self::to], the others closing up behind it.
    pub fn shown_at(&self, position: usize) -> usize {
        let (from, to) = (self.from, self.to);
        if position == to {
            from
        } else if from < to && position >= from && position < to {
            position + 1
        } else if to < from && position > to && position <= from {
            position - 1
        } else {
            position
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dragged_row_shows_where_it_would_go() {
        let mut drag = Drag::new(1, 10, false);
        drag.to = 3;
        let shown: Vec<usize> = (0..5).map(|p| drag.shown_at(p)).collect();
        assert_eq!(shown, [0, 2, 3, 1, 4]);
        drag.to = 0;
        let shown: Vec<usize> = (0..5).map(|p| drag.shown_at(p)).collect();
        assert_eq!(shown, [1, 0, 2, 3, 4]);
    }

    #[test]
    fn a_press_moves_only_once_it_leaves_its_row() {
        let area = Rect::new(0, 5, 20, 10);
        let mut drag = Drag::new(0, 8, true);
        drag.move_to(8);
        assert!(!drag.moved);
        assert_eq!(drag.scroll_step(area), 0);
        drag.move_to(14);
        assert!(drag.moved);
        assert_eq!(drag.scroll_step(area), 1);
        // Not again straight away.
        assert_eq!(drag.scroll_step(area), 0);
    }
}
