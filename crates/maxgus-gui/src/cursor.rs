//! The cursor, and how it gets where it is going.
//!
//! A terminal's cursor is wherever the terminal decides to put it, and it
//! gets there between one frame and the next: there is no position between
//! two cells for it to be in. A window has every position between them, so
//! the block slides, and the eye follows it instead of having to find it
//! again on the other side of the screen.
//!
//! The smear is what makes that work over a long jump. The four corners are
//! animated separately and the ones at the back are given less of the
//! distance each frame, so the block stretches out behind itself while it
//! travels and gathers back into a cell when it arrives. Neovide does this
//! and calls the amount `cursor_trail_size`; the name here is `cursor-trail`
//! and it is the same idea.
//!
//! This is the arithmetic alone — no GPU, no window — because that is the
//! part that can be wrong in a way nobody sees until they are watching a
//! cursor behave oddly and cannot say why.

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

/// A block that slides to where point went.
#[derive(Debug, Clone, Copy)]
pub struct Cursor {
    corners: [[f32; 2]; 4],
    destination: [[f32; 2]; 4],
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
    /// Closer than this to its destination and it has arrived. Half a pixel
    /// is not a frame's worth of movement, and an exponential ease would
    /// otherwise never quite finish.
    const SETTLED: f32 = 0.5;

    /// How many time constants of the ease count as arrived, so that
    /// `cursor-animation-ms` names the whole slide and means it.
    const CONSTANTS: f32 = 4.0;

    pub fn new() -> Cursor {
        Cursor {
            corners: [[0.0; 2]; 4],
            destination: [[0.0; 2]; 4],
            placed: false,
        }
    }

    /// Says where point is now. The block starts making its way there.
    pub fn go_to(&mut self, cell: Cell) {
        self.destination = cell.corners();
        if !self.placed {
            self.corners = self.destination;
            self.placed = true;
        }
    }

    /// Puts the block where it is going, at once.
    ///
    /// For the things that are not a cursor moving: the window being
    /// resized, the font being reloaded, a frame drawn after the editor was
    /// left alone for a minute. Sliding across those is an animation of
    /// something that did not happen.
    pub fn snap(&mut self) {
        self.corners = self.destination;
    }

    /// True while the block still has ground to cover, which is what asks
    /// the event loop for another frame.
    pub fn is_moving(&self) -> bool {
        self.corners.iter().zip(&self.destination).any(|(at, to)| {
            (to[0] - at[0]).abs() >= Cursor::SETTLED || (to[1] - at[1]).abs() >= Cursor::SETTLED
        })
    }

    /// The four corners as they are now, for the renderer to draw.
    pub fn corners(&self) -> [[f32; 2]; 4] {
        self.corners
    }

    /// Advances by however long the last frame took.
    ///
    /// `settle` is `cursor-animation-ms` and `trail` is `cursor-trail` in
    /// percent. Advancing by real time rather than by a frame means the
    /// same setting is the same speed on a 60Hz display and on a 144Hz one.
    pub fn step(&mut self, elapsed: std::time::Duration, settle: usize, trail: usize) {
        if settle == 0 {
            self.snap();
            return;
        }
        if !self.is_moving() {
            self.snap();
            return;
        }
        let tau = settle as f32 / Cursor::CONSTANTS / 1000.0;
        let trail = (trail.min(95) as f32) / 100.0;
        let travel = direction(centre(&self.destination), centre(&self.corners));
        let middle = centre(&self.corners);
        for (corner, to) in self.corners.iter_mut().zip(&self.destination) {
            // A corner at the back of the block is given less of the
            // distance, so the block stretches while it travels. Which
            // corners those are depends on where it is going, which is why
            // this cannot be baked into the corner order.
            let side = direction([corner[0] - middle[0], corner[1] - middle[1]], [0.0, 0.0]);
            let lead = side[0] * travel[0] + side[1] * travel[1];
            // Scaled so the *slowest* corner is the one that takes
            // `cursor-animation-ms`, because that corner is the one still
            // arriving and the setting says how long arriving takes. The
            // leading corners are then quicker than the setting rather than
            // the trailing ones being slower than it, which is also what
            // makes the smear: the front shoots ahead and the back drags.
            let speed = (1.0 - trail * (-lead).max(0.0)) / (1.0 - trail);
            let covered = 1.0 - (-elapsed.as_secs_f32() * speed / tau).exp();
            let covered = covered.clamp(0.0, 1.0);
            corner[0] += (to[0] - corner[0]) * covered;
            corner[1] += (to[1] - corner[1]) * covered;
        }
        if !self.is_moving() {
            self.snap();
        }
    }
}

fn centre(corners: &[[f32; 2]; 4]) -> [f32; 2] {
    let x = corners.iter().map(|c| c[0]).sum::<f32>() / 4.0;
    let y = corners.iter().map(|c| c[1]).sum::<f32>() / 4.0;
    [x, y]
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
        cursor.go_to(cell(300.0, 400.0));
        assert!(!cursor.is_moving());
        assert_eq!(cursor.corners()[0], [300.0, 400.0]);
    }

    #[test]
    fn it_arrives_where_it_was_sent() {
        let mut cursor = Cursor::new();
        cursor.go_to(cell(0.0, 0.0));
        cursor.go_to(cell(100.0, 60.0));
        let mut frames = 0;
        while cursor.is_moving() {
            cursor.step(FRAME, SETTLE, TRAIL);
            frames += 1;
            assert!(frames < 300, "it never settled");
        }
        assert_eq!(cursor.corners(), cell(100.0, 60.0).corners());
        assert!(frames > 1, "it arrived in one frame, which is a jump");
    }

    #[test]
    fn the_setting_is_how_long_the_slide_takes() {
        for settle in [40usize, 90, 300] {
            let mut cursor = Cursor::new();
            cursor.go_to(cell(0.0, 0.0));
            cursor.go_to(cell(400.0, 0.0));
            let mut spent = std::time::Duration::ZERO;
            while cursor.is_moving() {
                cursor.step(FRAME, settle, TRAIL);
                spent += FRAME;
                assert!(spent.as_millis() < 4_000, "it never settled");
            }
            let took = spent.as_millis() as usize;
            assert!(
                took >= settle / 2 && took <= settle * 2,
                "{settle}ms of cursor took {took}ms"
            );
        }
    }

    #[test]
    fn the_same_setting_is_the_same_speed_at_any_frame_rate() {
        let ran_for = |frame: std::time::Duration| {
            let mut cursor = Cursor::new();
            cursor.go_to(cell(0.0, 0.0));
            cursor.go_to(cell(400.0, 0.0));
            let mut spent = std::time::Duration::ZERO;
            while cursor.is_moving() && spent.as_millis() < 4_000 {
                cursor.step(frame, SETTLE, TRAIL);
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
        cursor.go_to(cell(0.0, 0.0));
        cursor.go_to(cell(400.0, 0.0));
        cursor.step(FRAME, SETTLE, TRAIL);
        cursor.step(FRAME, SETTLE, TRAIL);
        assert!(
            width(&cursor) > 10.0 * 2.0,
            "the block did not stretch: it is {} wide",
            width(&cursor)
        );
        while cursor.is_moving() {
            cursor.step(FRAME, SETTLE, TRAIL);
        }
        assert_eq!(width(&cursor), 10.0, "it never gathered back into a cell");
    }

    #[test]
    fn a_cursor_with_no_trail_keeps_its_shape() {
        // `cursor-trail=0` is how someone asks for a block that slides
        // rather than smears.
        let mut cursor = Cursor::new();
        cursor.go_to(cell(0.0, 0.0));
        cursor.go_to(cell(400.0, 0.0));
        for _ in 0..6 {
            cursor.step(FRAME, SETTLE, 0);
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
        cursor.go_to(cell(0.0, 0.0));
        cursor.go_to(cell(0.0, 400.0));
        cursor.step(FRAME, SETTLE, TRAIL);
        cursor.step(FRAME, SETTLE, TRAIL);
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
        cursor.go_to(cell(0.0, 0.0));
        cursor.go_to(cell(400.0, 200.0));
        cursor.step(FRAME, 0, TRAIL);
        assert!(!cursor.is_moving());
        assert_eq!(cursor.corners(), cell(400.0, 200.0).corners());
    }

    #[test]
    fn a_cursor_that_has_not_moved_asks_for_no_frames() {
        let mut cursor = Cursor::new();
        cursor.go_to(cell(50.0, 50.0));
        assert!(!cursor.is_moving());
        cursor.step(FRAME, SETTLE, TRAIL);
        assert!(!cursor.is_moving());
        assert_eq!(cursor.corners(), cell(50.0, 50.0).corners());
    }

    #[test]
    fn snapping_ends_a_slide_where_it_was_going() {
        // What a resize does: the cells are a different size now and where
        // the block was coming from was measured in the old ones.
        let mut cursor = Cursor::new();
        cursor.go_to(cell(0.0, 0.0));
        cursor.go_to(cell(400.0, 0.0));
        cursor.step(FRAME, SETTLE, TRAIL);
        cursor.snap();
        assert!(!cursor.is_moving());
        assert_eq!(cursor.corners(), cell(400.0, 0.0).corners());
    }
}
