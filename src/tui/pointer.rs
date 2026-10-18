//! The mouse pointer's shape: a hand over things that can be clicked and a text cursor over text
//! fields, set with the OSC 22 escape sequence and CSS cursor names (Kitty, Ghostty, foot; other
//! terminals ignore it).
//!
//! Widgets mark their clickable areas while drawing, the same areas their `mouse` handlers check;
//! the marks start over with each frame. When the mouse moves, or a redraw moves things under
//! it, the shape is worked out from the marks and sent only if it changed.

use std::cell::{Cell, RefCell};
use std::io::{self, Write};

use ratatui::layout::{Position, Rect};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Shape {
    #[default]
    Default,
    /// Something that can be clicked: a hand.
    Pointer,
    /// A text field: an I-beam.
    Text,
}

impl Shape {
    /// The name OSC 22 knows it by (CSS cursor names).
    fn name(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Pointer => "pointer",
            Self::Text => "text",
        }
    }
}

thread_local! {
    /// Areas marked this frame, later marks on top.
    static AREAS: RefCell<Vec<(Rect, Shape)>> = const { RefCell::new(Vec::new()) };
    /// Where the mouse was last seen.
    static MOUSE: Cell<Option<Position>> = const { Cell::new(None) };
    /// The shape sent last.
    static SHOWN: Cell<Shape> = const { Cell::new(Shape::Default) };
    /// Whether to send shapes: only to a real terminal, never in tests.
    static ENABLED: Cell<bool> = const { Cell::new(false) };
}

/// Send shapes to the terminal from now on; called when the UI starts.
pub fn enable() {
    ENABLED.set(true);
}

/// Forget the marks, at the start of a frame, or under an overlay, which takes the clicks.
pub fn clear() {
    AREAS.with_borrow_mut(Vec::clear);
}

/// Mark `area` as clickable.
pub fn clickable(area: Rect) {
    mark(area, Shape::Pointer);
}

/// Mark `area` as a text field.
pub fn text(area: Rect) {
    mark(area, Shape::Text);
}

fn mark(area: Rect, shape: Shape) {
    if !area.is_empty() {
        AREAS.with_borrow_mut(|areas| areas.push((area, shape)));
    }
}

/// The shape for the mouse at `at`: the topmost mark under it.
pub fn shape_at(at: Position) -> Shape {
    AREAS.with_borrow(|areas| {
        areas
            .iter()
            .rev()
            .find(|(area, _)| area.contains(at))
            .map_or(Shape::Default, |(_, shape)| *shape)
    })
}

/// The mouse moved to `at`.
pub fn hover(at: Position) {
    MOUSE.set(Some(at));
    update();
}

/// Bring the shape up to date after a redraw, which may have moved things under the mouse.
pub fn update() {
    if let Some(at) = MOUSE.get() {
        show(shape_at(at));
    }
}

/// Put the default pointer back, when the UI stops.
pub fn reset() {
    show(Shape::Default);
    MOUSE.set(None);
}

fn show(shape: Shape) {
    if !ENABLED.get() || SHOWN.get() == shape {
        return;
    }
    SHOWN.set(shape);
    let mut stdout = io::stdout();
    let _ = write!(stdout, "\x1b]22;{}\x1b\\", shape.name());
    let _ = stdout.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_topmost_mark_under_the_mouse_decides() {
        clear();
        clickable(Rect::new(0, 0, 10, 5));
        text(Rect::new(2, 1, 4, 1));
        mark(Rect::new(0, 9, 0, 0), Shape::Pointer);
        assert_eq!(shape_at(Position::new(0, 0)), Shape::Pointer);
        assert_eq!(shape_at(Position::new(3, 1)), Shape::Text);
        assert_eq!(shape_at(Position::new(10, 0)), Shape::Default);
        assert_eq!(shape_at(Position::new(0, 9)), Shape::Default);
        clear();
        assert_eq!(shape_at(Position::new(0, 0)), Shape::Default);
    }
}
