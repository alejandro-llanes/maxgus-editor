//! Smooth scrolling.
//!
//! A terminal can only scroll by whole lines: it has no way to draw half of
//! one. A window can, so scrolling here is a pixel offset that the renderer
//! shifts the text by, and only when it passes a whole line does the window's
//! `top_line` change. The wheel's own deltas are accumulated into the same
//! offset, which is what makes a touchpad feel continuous rather than notched.
//!
//! The offset also *eases*: a wheel notch asks for a jump, and the text takes
//! a few frames to get there rather than teleporting. That is the difference
//! between a window that scrolls and one that flickers.

/// How far the text is from where it is being asked to be, in pixels.
#[derive(Debug, Clone, Copy, Default)]
pub struct Scroll {
    /// Where the view is, in pixels from the top of the buffer's first
    /// visible line. Positive means the text is drawn shifted upwards.
    offset: f32,
    /// Where it is heading. Equal to `offset` when the animation has settled.
    target: f32,
}

impl Scroll {
    /// How much of the remaining distance is covered each frame.
    ///
    /// Chosen for 60Hz: a notch is most of the way there in about four frames
    /// and settled in ten, which reads as movement rather than as a jump or
    /// as a slide.
    const EASING: f32 = 0.35;

    /// Below this the animation is over: a fraction of a pixel is not worth a
    /// frame, and floating point would otherwise never quite arrive.
    const SETTLED: f32 = 0.5;

    pub fn new() -> Scroll {
        Scroll::default()
    }

    /// Asks to move `pixels` further down the buffer.
    pub fn nudge(&mut self, pixels: f32) {
        self.target += pixels;
    }

    /// Stops where it is, forgetting any journey it was making.
    ///
    /// What a keyboard motion does: `C-v` and a click both set the view
    /// outright, and letting the wheel's animation continue over the top of
    /// that would drag the text away from where it was just put.
    pub fn settle(&mut self) {
        self.offset = 0.0;
        self.target = 0.0;
    }

    /// Advances the animation one frame and returns the whole lines that have
    /// been crossed, which is what the window's `top_line` moves by.
    ///
    /// The remainder stays in `offset` as the sub-line shift the renderer
    /// draws with, so a line that is half scrolled is drawn half scrolled.
    pub fn step(&mut self, line_height: f32) -> isize {
        let distance = self.target - self.offset;
        if distance.abs() < Scroll::SETTLED {
            self.offset = self.target;
        } else {
            self.offset += distance * Scroll::EASING;
        }
        let lines = (self.offset / line_height).trunc();
        if lines != 0.0 {
            self.offset -= lines * line_height;
            self.target -= lines * line_height;
        }
        lines as isize
    }

    /// The sub-line shift to draw with, in pixels.
    pub fn pixels(&self) -> f32 {
        self.offset
    }

    /// The whole lines still owed, for an animation that is being cut short.
    ///
    /// A keystroke in the middle of one ends it, and ending it by throwing
    /// away the distance left would leave the view short of where the wheel
    /// asked for it.
    pub fn remaining(&self, line_height: f32) -> isize {
        ((self.target - self.offset) / line_height).round() as isize
    }

    /// True while there is still movement owed, which is what tells the event
    /// loop to ask for another frame.
    pub fn is_moving(&self) -> bool {
        (self.target - self.offset).abs() >= Scroll::SETTLED
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINE: f32 = 20.0;

    #[test]
    fn a_still_view_asks_for_no_frames() {
        let mut scroll = Scroll::new();
        assert!(!scroll.is_moving());
        assert_eq!(scroll.step(LINE), 0);
        assert_eq!(scroll.pixels(), 0.0);
    }

    #[test]
    fn a_nudge_arrives_over_several_frames() {
        let mut scroll = Scroll::new();
        scroll.nudge(3.0 * LINE);
        let mut lines = 0;
        let mut frames = 0;
        while scroll.is_moving() {
            lines += scroll.step(LINE);
            frames += 1;
            assert!(frames < 100, "the animation never settled");
        }
        lines += scroll.step(LINE);
        assert_eq!(lines, 3, "it did not arrive where it was sent");
        assert!(frames > 1, "it arrived in one frame, which is a jump");
        assert!(frames < 30, "it took {frames} frames, which is a crawl");
    }

    #[test]
    fn part_of_a_line_is_kept_to_be_drawn_with() {
        // The point of the whole thing: half a line scrolled is half a line
        // drawn, not nothing drawn and then a jump.
        let mut scroll = Scroll::new();
        scroll.nudge(LINE / 2.0);
        scroll.step(LINE);
        assert!(
            scroll.pixels() > 0.0 && scroll.pixels() < LINE,
            "no sub-line offset: {}",
            scroll.pixels()
        );
    }

    #[test]
    fn scrolling_up_crosses_lines_the_other_way() {
        let mut scroll = Scroll::new();
        scroll.nudge(-2.0 * LINE);
        let mut lines = 0;
        for _ in 0..60 {
            lines += scroll.step(LINE);
        }
        assert_eq!(lines, -2);
    }

    #[test]
    fn many_small_deltas_add_up_to_a_line() {
        // A touchpad sends a stream of a few pixels at a time; none of them
        // is a line on its own and together they have to be.
        let mut scroll = Scroll::new();
        let mut lines = 0;
        for _ in 0..10 {
            scroll.nudge(LINE / 10.0);
            lines += scroll.step(LINE);
        }
        for _ in 0..40 {
            lines += scroll.step(LINE);
        }
        assert_eq!(lines, 1, "ten tenths of a line is one line");
    }

    #[test]
    fn what_is_left_of_a_journey_can_be_taken_in_one_step() {
        // A keystroke in the middle of an animation ends it, and ending it
        // by dropping the distance left would leave the view short of where
        // the wheel asked for it.
        let mut scroll = Scroll::new();
        scroll.nudge(3.0 * LINE);
        let mut crossed = scroll.step(LINE);
        assert!(crossed < 3, "it arrived in one frame, so there is no test");
        crossed += scroll.remaining(LINE);
        assert_eq!(crossed, 3, "the rest of the journey was lost");
    }

    #[test]
    fn a_settled_view_owes_nothing() {
        let scroll = Scroll::new();
        assert_eq!(scroll.remaining(LINE), 0);
    }

    #[test]
    fn settling_forgets_where_it_was_going() {
        let mut scroll = Scroll::new();
        scroll.nudge(10.0 * LINE);
        scroll.step(LINE);
        scroll.settle();
        assert!(!scroll.is_moving());
        assert_eq!(scroll.pixels(), 0.0);
        assert_eq!(scroll.step(LINE), 0, "it kept going after being stopped");
    }
}
