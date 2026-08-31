//! The cursor, and how it gets where it is going.
//!
//! A terminal's cursor is wherever the terminal decides to put it, and it
//! gets there between one frame and the next: there is no position between
//! two cells for it to be in. A window has every position between them, so
//! the block slides, and the eye follows it instead of having to find it
//! again on the other side of the screen.
//!
//! The smear is what makes that work over a long jump. Each of the four
//! corners has its own pair of [`crate::spring::Spring`]s and its own time
//! to get there — the ones at the back are given longer — so the block
//! stretches out behind itself while it travels and gathers back into a
//! cell when it arrives. How far the back lags is `cursor-trail`.
//!
//! A hop of a cell or two is not smeared at all. That is the common case —
//! a key typed, a character rubbed out — and animating it over the same
//! duration as a jump across the screen makes ordinary typing look like it
//! is lagging behind the keyboard. `cursor-short-animation-ms` is how long
//! those get, and it is much shorter.
//!
//! This is the arithmetic alone — no GPU, no window — because that is the
//! part that can be wrong in a way nobody sees until they are watching a
//! cursor behave oddly and cannot say why.

use crate::spring::Spring;

/// Where a cell is on the screen, in pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cell {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Cell {
    /// The corners, in the order the renderer's unit quad has them: top
    /// left, top right, bottom left, bottom right.
    fn corners(&self) -> [[f32; 2]; 4] {
        let (l, t) = (self.x, self.y);
        let (r, b) = (self.x + self.width, self.y + self.height);
        [[l, t], [r, t], [l, b], [r, b]]
    }
}

/// One corner of the block, and how it is getting where it is going.
///
/// Two springs, because a corner moves in two directions independently and
/// a single spring on the distance would drag it along a straight line — the
/// smear is exactly the corners *not* travelling together.
#[derive(Debug, Clone, Copy, Default)]
struct Corner {
    x: Spring,
    y: Spring,
    /// How long this corner has to get there. Not the same for all four:
    /// the ones at the back are given longer, which is the smear.
    length: f32,
}

/// A block that slides to where point went.
#[derive(Debug, Clone, Copy)]
pub struct Cursor {
    /// Where the block is going, in the order the unit quad has its corners:
    /// top left, top right, bottom left, bottom right.
    destination: [[f32; 2]; 4],
    /// How far each corner still is from its share of the destination.
    corners: [Corner; 4],
    /// False until the cursor has been put somewhere once. The first frame
    /// has nowhere to come from, and sliding in from the origin is a cursor
    /// that flies across the screen when the editor opens.
    placed: bool,
}

impl Default for Cursor {
    fn default() -> Cursor {
        Cursor::new()
    }
}

impl Cursor {
    pub fn new() -> Cursor {
        Cursor {
            destination: [[0.0; 2]; 4],
            corners: [Corner::default(); 4],
            placed: false,
        }
    }

    /// Says where point is now. The block starts making its way there.
    ///
    /// `settle` is `cursor-animation-ms`, `short` is
    /// `cursor-short-animation-ms` and `trail` is `cursor-trail` in percent.
    /// They are wanted here rather than in [`Cursor::step`] because how long
    /// a corner has is decided when the journey starts: it depends on how
    /// far the block is going and on which way, and both are known now and
    /// not later.
    pub fn go_to(&mut self, cell: Cell, settle: usize, short: usize, trail: usize) {
        let was = self.destination;
        self.destination = cell.corners();
        if !self.placed {
            self.placed = true;
            self.snap();
            return;
        }
        if was == self.destination {
            return;
        }
        let travel = direction(centre(&self.destination), centre(&was));
        let middle = centre(&self.destination);
        // A short hop is the common case — a key typed, a character deleted
        // — and smearing it makes ordinary typing look like it is lagging.
        // treated as its own, much quicker animation.
        let hop = distance(centre(&self.destination), centre(&was));
        let short_hop = hop <= cell.width * 2.001
            && (centre(&self.destination)[1] - centre(&was)[1]).abs() < 0.001;
        let settle = match short_hop {
            true => settle.min(short) as f32 / 1000.0,
            false => settle as f32 / 1000.0,
        };
        let trail = (trail.min(95) as f32) / 100.0;
        // The leading corners are given less time than the setting and the
        // trailing ones all of it, so the setting names when the *last* of
        // the block arrives — which is when the cursor has arrived.
        let leading = settle * (1.0 - trail);
        for (n, corner) in self.corners.iter_mut().enumerate() {
            let to = self.destination[n];
            let side = direction(to, middle);
            let lead = (side[0] * travel[0] + side[1] * travel[1]).clamp(-1.0, 1.0);
            // `lead` is +1 for a corner at the front and -1 at the back.
            let towards_front = (lead + 1.0) / 2.0;
            corner.length = leading + (settle - leading) * (1.0 - towards_front);
            // How far this corner has left to go, measured from where it
            // actually is — which is the *old* destination less whatever it
            // had left of the last journey, not the new one.
            let at = [was[n][0] - corner.x.position, was[n][1] - corner.y.position];
            // Assigned rather than set, so a cursor sent somewhere new
            // while still arriving keeps the speed it had instead of
            // stopping dead and starting again.
            corner.x.position = to[0] - at[0];
            corner.y.position = to[1] - at[1];
        }
    }

    /// Puts the block where it is going, at once.
    ///
    /// For the things that are not a cursor moving: the window being
    /// resized, the font being reloaded, a frame drawn after the editor was
    /// left alone for a minute. Sliding across those is an animation of
    /// something that did not happen.
    pub fn snap(&mut self) {
        for corner in &mut self.corners {
            corner.x.reset();
            corner.y.reset();
        }
    }

    /// True while the block still has ground to cover, which is what asks
    /// the event loop for another frame.
    pub fn is_moving(&self) -> bool {
        self.corners
            .iter()
            .any(|corner| corner.x.is_moving() || corner.y.is_moving())
    }

    /// The four corners as they are now, for the renderer to draw.
    pub fn corners(&self) -> [[f32; 2]; 4] {
        let mut out = self.destination;
        for (at, corner) in out.iter_mut().zip(&self.corners) {
            at[0] -= corner.x.position;
            at[1] -= corner.y.position;
        }
        out
    }

    /// Where the block is heading, which is where point actually is.
    pub fn destination(&self) -> [[f32; 2]; 4] {
        self.destination
    }

    /// Advances by however long the last frame took.
    ///
    /// How long each corner has was settled when the journey started, so
    /// this takes no settings: it only runs the springs down.
    pub fn step(&mut self, elapsed: std::time::Duration) {
        for corner in &mut self.corners {
            let length = corner.length;
            corner.x.advance(elapsed, length);
            corner.y.advance(elapsed, length);
        }
    }
}

fn centre(corners: &[[f32; 2]; 4]) -> [f32; 2] {
    let x = corners.iter().map(|c| c[0]).sum::<f32>() / 4.0;
    let y = corners.iter().map(|c| c[1]).sum::<f32>() / 4.0;
    [x, y]
}

/// How far apart two points are.
fn distance(to: [f32; 2], from: [f32; 2]) -> f32 {
    let (dx, dy) = (to[0] - from[0], to[1] - from[1]);
    (dx * dx + dy * dy).sqrt()
}

/// `to - from`, as a unit vector, or nothing when they are the same point.
fn direction(to: [f32; 2], from: [f32; 2]) -> [f32; 2] {
    let (dx, dy) = (to[0] - from[0], to[1] - from[1]);
    let length = (dx * dx + dy * dy).sqrt();
    match length > f32::EPSILON {
        true => [dx / length, dy / length],
        false => [0.0, 0.0],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: std::time::Duration = std::time::Duration::from_micros(16_667);
    const SETTLE: usize = 90;
    const TRAIL: usize = 70;
    /// Short enough that only a hop of a cell or two takes it.
    const SHORT: usize = 40;

    fn cell(x: f32, y: f32) -> Cell {
        Cell {
            x,
            y,
            width: 10.0,
            height: 20.0,
        }
    }

    fn width(cursor: &Cursor) -> f32 {
        let c = cursor.corners();
        (c[1][0] - c[0][0]).abs()
    }

    fn height(cursor: &Cursor) -> f32 {
        let c = cursor.corners();
        (c[2][1] - c[0][1]).abs()
    }

    #[test]
    fn the_first_placement_does_not_fly_in_from_the_corner() {
        // Sliding from the origin is what a cursor that was never anywhere
        // does, and it is the first thing anyone would see on opening the
        // editor.
        let mut cursor = Cursor::new();
        cursor.go_to(cell(300.0, 400.0), SETTLE, SHORT, TRAIL);
        assert!(!cursor.is_moving());
        assert_eq!(cursor.corners()[0], [300.0, 400.0]);
    }

    #[test]
    fn it_arrives_where_it_was_sent() {
        let mut cursor = Cursor::new();
        cursor.go_to(cell(0.0, 0.0), SETTLE, SHORT, TRAIL);
        cursor.go_to(cell(100.0, 60.0), SETTLE, SHORT, TRAIL);
        let mut frames = 0;
        while cursor.is_moving() {
            cursor.step(FRAME);
            frames += 1;
            assert!(frames < 300, "it never settled");
        }
        assert_eq!(cursor.corners(), cell(100.0, 60.0).corners());
        assert!(frames > 1, "it arrived in one frame, which is a jump");
    }

    #[test]
    fn the_setting_is_how_long_the_slide_takes() {
        // As with the scroll: a spring covers nine tenths of the way in the
        // time it was given and the sliver after that is sub-pixel, so what
        // is worth asserting is that the setting is obeyed in proportion
        // and that it does stop.
        for settle in [60usize, 150, 400] {
            let mut cursor = Cursor::new();
            cursor.go_to(cell(0.0, 0.0), settle, SHORT, TRAIL);
            cursor.go_to(cell(400.0, 0.0), settle, SHORT, TRAIL);
            let mut spent = std::time::Duration::ZERO;
            while spent.as_millis() < settle as u128 {
                cursor.step(FRAME);
                spent += FRAME;
            }
            let covered = cursor.corners()[0][0] / 400.0;
            assert!(
                covered > 0.7,
                "{settle}ms moved the cursor only {:.0}% of the way",
                covered * 100.0
            );
            while cursor.is_moving() {
                cursor.step(FRAME);
                spent += FRAME;
                assert!(spent.as_millis() < 4_000, "it never settled");
            }
            assert!(
                spent.as_millis() <= settle as u128 * 4,
                "{settle}ms of cursor was still going after {}ms",
                spent.as_millis()
            );
        }
    }

    #[test]
    fn the_same_setting_is_the_same_speed_at_any_frame_rate() {
        let ran_for = |frame: std::time::Duration| {
            let mut cursor = Cursor::new();
            cursor.go_to(cell(0.0, 0.0), SETTLE, SHORT, TRAIL);
            cursor.go_to(cell(400.0, 0.0), SETTLE, SHORT, TRAIL);
            let mut spent = std::time::Duration::ZERO;
            while cursor.is_moving() && spent.as_millis() < 4_000 {
                cursor.step(frame);
                spent += frame;
            }
            spent.as_millis() as i64
        };
        let sixty = ran_for(std::time::Duration::from_micros(16_667));
        let one_forty_four = ran_for(std::time::Duration::from_micros(6_944));
        assert!(
            (sixty - one_forty_four).abs() < 30,
            "60Hz took {sixty}ms and 144Hz took {one_forty_four}ms"
        );
    }

    #[test]
    fn the_block_smears_along_the_way_and_gathers_at_the_end() {
        // The whole point of the trail: the back of the block is given less
        // of the distance each frame, so it stretches out behind itself.
        let mut cursor = Cursor::new();
        cursor.go_to(cell(0.0, 0.0), SETTLE, SHORT, TRAIL);
        cursor.go_to(cell(400.0, 0.0), SETTLE, SHORT, TRAIL);
        cursor.step(FRAME);
        cursor.step(FRAME);
        assert!(
            width(&cursor) > 10.0 * 2.0,
            "the block did not stretch: it is {} wide",
            width(&cursor)
        );
        while cursor.is_moving() {
            cursor.step(FRAME);
        }
        assert_eq!(width(&cursor), 10.0, "it never gathered back into a cell");
    }

    #[test]
    fn a_cursor_with_no_trail_keeps_its_shape() {
        // `cursor-trail=0` is how someone asks for a block that slides
        // rather than smears.
        let mut cursor = Cursor::new();
        cursor.go_to(cell(0.0, 0.0), SETTLE, SHORT, 0);
        cursor.go_to(cell(400.0, 0.0), SETTLE, SHORT, 0);
        for _ in 0..6 {
            cursor.step(FRAME);
            assert!(
                (width(&cursor) - 10.0).abs() < 0.01,
                "a trail-less cursor stretched to {}",
                width(&cursor)
            );
        }
    }

    #[test]
    fn it_smears_the_way_it_is_going_and_not_the_other_way() {
        // Moving down should stretch it vertically, not sideways: the
        // corners at the back are the ones behind the direction of travel,
        // and which those are depends on where it is going.
        let mut cursor = Cursor::new();
        cursor.go_to(cell(0.0, 0.0), SETTLE, SHORT, TRAIL);
        cursor.go_to(cell(0.0, 400.0), SETTLE, SHORT, TRAIL);
        cursor.step(FRAME);
        cursor.step(FRAME);
        assert!(
            height(&cursor) > 20.0 * 2.0,
            "it did not stretch downwards: {} tall",
            height(&cursor)
        );
        assert!(
            (width(&cursor) - 10.0).abs() < 0.5,
            "it stretched sideways going down: {} wide",
            width(&cursor)
        );
    }

    #[test]
    fn turning_the_animation_off_puts_it_straight_there() {
        // `cursor-animation-ms=0`.
        let mut cursor = Cursor::new();
        cursor.go_to(cell(0.0, 0.0), 0, 0, TRAIL);
        cursor.go_to(cell(400.0, 200.0), 0, 0, TRAIL);
        cursor.step(FRAME);
        assert!(!cursor.is_moving());
        assert_eq!(cursor.corners(), cell(400.0, 200.0).corners());
    }

    #[test]
    fn a_cursor_that_has_not_moved_asks_for_no_frames() {
        let mut cursor = Cursor::new();
        cursor.go_to(cell(50.0, 50.0), SETTLE, SHORT, TRAIL);
        assert!(!cursor.is_moving());
        cursor.step(FRAME);
        assert!(!cursor.is_moving());
        assert_eq!(cursor.corners(), cell(50.0, 50.0).corners());
    }

    #[test]
    fn typing_does_not_smear() {
        // The common case, and the one that made this feel like it lagged:
        // a hop of a cell gets the short animation, which is over in a
        // frame or two, not the one meant for crossing the screen.
        let mut cursor = Cursor::new();
        cursor.go_to(cell(0.0, 0.0), SETTLE, SHORT, TRAIL);
        cursor.go_to(cell(10.0, 0.0), SETTLE, SHORT, TRAIL);
        let mut frames = 0;
        while cursor.is_moving() {
            cursor.step(FRAME);
            assert!(
                width(&cursor) < 10.0 * 1.6,
                "one cell of typing smeared to {}",
                width(&cursor)
            );
            frames += 1;
            assert!(frames < 40, "a one-cell hop never settled");
        }
        assert!(frames <= 8, "a one-cell hop took {frames} frames");
    }

    /// How many frames the block takes to get from the origin to `to`.
    fn frames_to(to: Cell) -> usize {
        let mut cursor = Cursor::new();
        cursor.go_to(cell(0.0, 0.0), SETTLE, SHORT, TRAIL);
        cursor.go_to(to, SETTLE, SHORT, TRAIL);
        let mut frames = 0;
        while cursor.is_moving() {
            cursor.step(FRAME);
            frames += 1;
            assert!(frames < 500, "it never settled");
        }
        frames
    }

    #[test]
    fn a_jump_further_than_a_hop_still_takes_its_time() {
        // The short animation is for hops, and three cells is not one: the
        // rule has to have an edge or it is just a shorter animation. Said
        // as a comparison rather than as a frame count, because the counts
        // are whatever the settings happen to be and the *difference*
        // between them is the thing being claimed.
        let hop = frames_to(cell(10.0, 0.0));
        let jump = frames_to(cell(30.0, 0.0));
        assert!(
            jump > hop,
            "three cells took {jump} frames and one took {hop}"
        );
    }

    #[test]
    fn a_hop_down_a_line_is_not_a_hop() {
        // Moving down a line is a whole cell height and reads as a jump,
        // however few columns it also moved.
        let hop = frames_to(cell(10.0, 0.0));
        let down = frames_to(cell(0.0, 20.0));
        assert!(
            down > hop,
            "a line down took {down} frames and a hop across took {hop}"
        );
    }

    #[test]
    fn snapping_ends_a_slide_where_it_was_going() {
        // What a resize does: the cells are a different size now and where
        // the block was coming from was measured in the old ones.
        let mut cursor = Cursor::new();
        cursor.go_to(cell(0.0, 0.0), SETTLE, SHORT, TRAIL);
        cursor.go_to(cell(400.0, 0.0), SETTLE, SHORT, TRAIL);
        cursor.step(FRAME);
        cursor.snap();
        assert!(!cursor.is_moving());
        assert_eq!(cursor.corners(), cell(400.0, 0.0).corners());
    }
}
