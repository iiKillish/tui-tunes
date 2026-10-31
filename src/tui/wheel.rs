//! Mouse wheel notches. Some terminals send several wheel events for one notch (Ghostty sends 3
//! by default, its `mouse-scroll-multiplier`), all at once. That suits views that scroll by rows,
//! and those that move a whole item per event while many are on screen, but with only one or two
//! showing, items would go by unseen: [moves] counts a notch once then.

use std::cell::Cell;
use std::time::{Duration, Instant};

use ratatui::crossterm::event::{MouseEvent, MouseEventKind};

/// Events closer together than this, the same way, are one notch.
const BURST: Duration = Duration::from_millis(20);

thread_local! {
    /// The last wheel event: which way, and when.
    static LAST: Cell<Option<(MouseEventKind, Instant)>> = const { Cell::new(None) };
}

/// Most items on screen for a notch to move just one.
const FEW: usize = 2;

/// Whether wheel `event` should move a view that moves an item per event and has `visible`
/// items on screen: every event when there are many, else only the one starting a notch.
pub fn moves(event: &MouseEvent, visible: usize) -> bool {
    let notch = notch_at(event.kind, Instant::now());
    visible > FEW || notch
}

/// Whether an event `kind` way at `now` starts a notch, rather than being one of the events sent
/// with the one before. Each event, whether it starts a notch or not, extends the burst.
fn notch_at(kind: MouseEventKind, now: Instant) -> bool {
    let previous = LAST.replace(Some((kind, now)));
    !previous.is_some_and(|(last, at)| last == kind && now.duration_since(at) < BURST)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_a_burst_the_same_way_is_one_notch() {
        let start = Instant::now();
        let down = MouseEventKind::ScrollDown;
        let at = |ms| start + Duration::from_millis(ms);
        assert!(notch_at(down, at(0)));
        assert!(!notch_at(down, at(1)));
        assert!(!notch_at(down, at(2)));
        // The next notch, after a pause.
        assert!(notch_at(down, at(150)));
        // Turning the wheel back is a notch of its own.
        assert!(notch_at(MouseEventKind::ScrollUp, at(151)));
    }

    #[test]
    fn test_only_views_showing_few_items_move_once_a_notch() {
        let event = MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 0,
            row: 0,
            modifiers: ratatui::crossterm::event::KeyModifiers::NONE,
        };
        let burst = |visible| (0..3).filter(|_| moves(&event, visible)).count();
        assert_eq!(burst(1), 1);
        std::thread::sleep(BURST * 2);
        assert_eq!(burst(2), 1);
        std::thread::sleep(BURST * 2);
        assert_eq!(burst(3), 3);
    }
}
