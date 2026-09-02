//! The bar at a window's edge that says where in the buffer it is.
//!
//! Not a scroll bar to drag — the wheel and the keys do that — but the
//! indication one gives: a thin mark whose place and length are the
//! window's place and size in its buffer. It shows while the window moves
//! and fades once the window has been still for a moment, so a page that
//! is being read has nothing at its edge and a page that is being scrolled
//! says how far there is to go.

use maxgus_core::WindowId;
use maxgus_core::render::ScrollPosition;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// How long the bar stays after the window last moved, before it fades.
const HOLD: Duration = Duration::from_millis(800);
/// How long the fade takes.
const FADE: Duration = Duration::from_millis(350);

/// What each window was last seen doing: where it was, and when it last
/// moved.
#[derive(Debug, Default)]
pub struct Bars {
    seen: HashMap<WindowId, (f32, Instant)>,
    /// Whether the last frame drew any bar. If it did, another frame is
    /// owed: to fade it further, or to draw the edge without it.
    showing: bool,
}

/// A bar to draw: where, and how strongly.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bar {
    pub position: ScrollPosition,
    /// One while the window is moving, falling to nought as it fades.
    pub opacity: f32,
}

impl Bars {
    pub fn new() -> Bars {
        Bars::default()
    }

    /// Takes in where every window is now, and says which bars to draw. A
    /// window that has moved since it was last seen — or is seen for the
    /// first time — starts its hold over; one that has not goes on fading.
    /// Windows that have gone are forgotten.
    pub fn observe(&mut self, positions: &[ScrollPosition], now: Instant) -> Vec<Bar> {
        let mut bars = Vec::new();
        let mut kept = HashMap::with_capacity(positions.len());
        for position in positions {
            let moved_at = match self.seen.get(&position.window) {
                Some(&(above, at)) if above == position.above => at,
                _ => now,
            };
            kept.insert(position.window, (position.above, moved_at));
            let opacity = opacity(now.saturating_duration_since(moved_at));
            if opacity > 0.0 {
                bars.push(Bar {
                    position: *position,
                    opacity,
                });
            }
        }
        self.seen = kept;
        self.showing = !bars.is_empty();
        bars
    }

    /// Whether the bars owe a frame: one was drawn last time, so it must be
    /// drawn fainter, or its place drawn clear once it has gone.
    pub fn is_fading(&self) -> bool {
        self.showing
    }
}

/// How strongly a bar shows `since` the window last moved.
fn opacity(since: Duration) -> f32 {
    if since <= HOLD {
        return 1.0;
    }
    let fading = since - HOLD;
    (1.0 - fading.as_secs_f32() / FADE.as_secs_f32()).max(0.0)
}

/// Where a bar goes, in pixels from the grid's corner: a `thickness`-wide
/// strip a little in from the right edge of the window's text, as long as
/// the window's share of the buffer and never shorter than a couple of
/// lines, so a window on a huge buffer still has a mark that can be seen.
pub fn geometry(
    position: &ScrollPosition,
    cell: (f32, f32),
    thickness: f32,
) -> ([f32; 2], [f32; 2]) {
    let (width, height) = cell;
    let area = position.area;
    let top = area.y as f32 * height;
    let room = area.height as f32 * height;
    let least = height * 2.0;
    let length = (position.shown * room).max(least).min(room);
    // Placed by how far down the buffer the window is, so the bar reaches
    // the bottom exactly when the window does.
    let travel = room - length;
    let progress = match position.shown >= 1.0 {
        true => 0.0,
        false => (position.above / (1.0 - position.shown)).clamp(0.0, 1.0),
    };
    let x = (area.x + area.width) as f32 * width - thickness - 2.0;
    ([x, top + travel * progress], [thickness, length])
}

#[cfg(test)]
mod tests {
    use super::*;
    use maxgus_tui::Rect;

    fn at(window: u64, above: f32) -> ScrollPosition {
        ScrollPosition {
            window: WindowId(window),
            area: Rect::new(0, 0, 80, 20),
            above,
            shown: 0.25,
        }
    }

    #[test]
    fn a_window_that_moves_shows_its_bar_and_a_still_one_lets_it_fade() {
        let mut bars = Bars::new();
        let start = Instant::now();
        let first = bars.observe(&[at(1, 0.0)], start);
        assert_eq!(first.len(), 1, "a window first seen shows where it is");
        assert_eq!(first[0].opacity, 1.0);
        // Still, past the hold and into the fade.
        let mid = start + HOLD + FADE / 2;
        let fading = bars.observe(&[at(1, 0.0)], mid);
        assert!(
            fading[0].opacity > 0.3 && fading[0].opacity < 0.7,
            "half way through the fade: {}",
            fading[0].opacity
        );
        assert!(bars.is_fading());
        // Gone — but only once a frame has been drawn without it.
        let after = start + HOLD + FADE * 2;
        assert!(bars.observe(&[at(1, 0.0)], after).is_empty());
        assert!(!bars.is_fading(), "nothing owes a frame any more");
        // A move brings it straight back.
        let moved = bars.observe(&[at(1, 0.1)], after);
        assert_eq!(moved[0].opacity, 1.0);
    }

    #[test]
    fn each_window_fades_on_its_own() {
        let mut bars = Bars::new();
        let start = Instant::now();
        bars.observe(&[at(1, 0.0), at(2, 0.0)], start);
        let later = start + HOLD + FADE / 2;
        let shown = bars.observe(&[at(1, 0.0), at(2, 0.5)], later);
        assert_eq!(shown.len(), 2);
        assert!(shown[0].opacity < 1.0, "the still window fades");
        assert_eq!(shown[1].opacity, 1.0, "the moved one does not");
        // A window that is closed is forgotten rather than kept for ever.
        bars.observe(&[at(2, 0.5)], later);
        assert_eq!(bars.seen.len(), 1);
    }

    #[test]
    fn the_bar_sits_inside_the_right_edge_and_reaches_the_bottom_with_the_window() {
        let cell = (10.0, 20.0);
        let top = at(1, 0.0);
        let (position, size) = geometry(&top, cell, 4.0);
        assert_eq!(position, [800.0 - 4.0 - 2.0, 0.0]);
        assert_eq!(
            size,
            [4.0, 100.0],
            "a quarter of the buffer is a quarter of the window"
        );
        // Three quarters above it is the bottom of the buffer.
        let bottom = ScrollPosition { above: 0.75, ..top };
        let (position, size) = geometry(&bottom, cell, 4.0);
        assert_eq!(
            position[1] + size[1],
            400.0,
            "the bar ends where the window does"
        );
        // A window showing a sliver of a huge buffer still gets a bar that
        // can be seen.
        let sliver = ScrollPosition {
            shown: 0.001,
            ..top
        };
        assert_eq!(geometry(&sliver, cell, 4.0).1[1], 40.0);
    }
}
