//! A critically damped spring, which is how every animation here moves.
//!
//! The obvious way to ease something towards a destination is to move it a
//! fixed fraction of the remaining distance each frame — `1 - exp(-dt/tau)`.
//! That is what this used to do, and it is wrong in a way that is easier to
//! feel than to see: an exponential decay is *fastest at the very first
//! frame* and slower every frame after. Things do not start at full speed.
//! The eye reads it as a snap followed by a crawl, and the crawl is the part
//! it notices.
//!
//! A critically damped spring starts at rest, accelerates, and settles
//! without overshooting — which is how something being moved actually
//! moves. It also has a *velocity*, so a second push while the first is
//! still arriving adds to it rather than restarting it: three notches of a
//! wheel in quick succession build up instead of stuttering.
//!
//! Neovide reached the same conclusion and this is its arithmetic, down to
//! the choice of `omega`. `position` is the distance still to travel, not
//! where anything is: it decays to zero, and whatever is being animated is
//! its destination minus this.

/// The distance something still has to travel, and how fast it is going.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Spring {
    /// How far there is left to go. Zero when it has arrived.
    pub position: f32,
    velocity: f32,
}

impl Spring {
    /// Closer than this and it has arrived. A quarter of a pixel is below
    /// anything the eye can follow, and an analytic decay never reaches
    /// zero on its own.
    const SETTLED: f32 = 0.25;

    /// How many time constants of the decay `length` is asked to cover.
    ///
    /// Neovide's choice, kept because the feel is the thing being copied.
    /// What it buys is a constant: whatever the distance and whatever the
    /// duration, **nine tenths of the way is covered by `length`**. The
    /// sliver after that takes about twice as long again, and happens
    /// below a quarter of a pixel where there is nothing to see.
    ///
    /// So a setting in milliseconds names when the movement is over as far
    /// as anyone watching is concerned, which is what a setting should
    /// name. It does not name when the arithmetic stops.
    const CONSTANTS: f32 = 4.0;

    pub fn new() -> Spring {
        Spring::default()
    }

    /// Adds `distance` to the journey, keeping whatever speed it had.
    ///
    /// Which is the point of a spring over an ease: a second wheel notch
    /// while the first is still arriving makes the text go further and
    /// faster, rather than starting the first journey again from rest.
    pub fn push(&mut self, distance: f32) {
        self.position += distance;
    }

    /// Sets the distance outright, forgetting any speed.
    pub fn set(&mut self, distance: f32) {
        self.position = distance;
        self.velocity = 0.0;
    }

    pub fn reset(&mut self) {
        self.position = 0.0;
        self.velocity = 0.0;
    }

    pub fn is_moving(&self) -> bool {
        self.position.abs() >= Spring::SETTLED
    }

    /// Advances by `dt`, and says whether there is still ground to cover.
    ///
    /// `length` is how long the whole journey should take, in seconds. A
    /// journey shorter than one frame is over, which is also how a setting
    /// of zero turns the animation off.
    pub fn advance(&mut self, dt: std::time::Duration, length: f32) -> bool {
        let dt = dt.as_secs_f32();
        if length <= dt {
            self.reset();
            return false;
        }
        if self.position == 0.0 && self.velocity == 0.0 {
            return false;
        }
        // Critically damped: the fastest approach that does not overshoot.
        let omega = Spring::CONSTANTS / length;
        // The analytic solution, with the initial conditions taken from
        // where it is and how fast it is going.
        let a = self.position;
        let b = self.position * omega + self.velocity;
        let decay = (-omega * dt).exp();
        self.position = (a + b * dt) * decay;
        self.velocity = decay * (-a * omega - b * dt * omega + b);
        if self.position.abs() < Spring::SETTLED {
            self.reset();
            return false;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: std::time::Duration = std::time::Duration::from_micros(16_667);

    fn run(spring: &mut Spring, length: f32) -> std::time::Duration {
        let mut spent = std::time::Duration::ZERO;
        while spring.advance(FRAME, length) {
            spent += FRAME;
            assert!(spent.as_secs_f32() < 10.0, "it never settled");
        }
        spent
    }

    #[test]
    fn nine_tenths_of_the_way_is_covered_by_the_time_it_was_given() {
        // What the setting actually promises, and it is the same promise
        // whatever the distance and whatever the duration — which is the
        // useful thing about this particular `omega`. Asserting a hard
        // arrival time instead would be asserting something untrue: the
        // last sliver takes about twice as long again, sub-pixel, where
        // there is nothing to see.
        for length in [0.05f32, 0.15, 0.3, 1.0] {
            for distance in [20.0f32, 100.0, 400.0, 2000.0] {
                let mut spring = Spring::new();
                spring.push(distance);
                let mut spent = std::time::Duration::ZERO;
                while spent.as_secs_f32() < length {
                    spring.advance(FRAME, length);
                    spent += FRAME;
                }
                let covered = 1.0 - spring.position.abs() / distance;
                assert!(
                    covered > 0.85,
                    "{length}s covered only {:.0}% of {distance}px",
                    covered * 100.0
                );
            }
        }
    }

    #[test]
    fn it_comes_to_a_full_stop_soon_after() {
        // And it does stop: a spring that never settles is a window that
        // never sleeps.
        for length in [0.05f32, 0.15, 0.3, 1.0] {
            let mut spring = Spring::new();
            spring.push(2000.0);
            let took = run(&mut spring, length).as_secs_f32();
            assert!(
                took <= length * 3.5,
                "{length}s of spring was still going after {took}s"
            );
            assert_eq!(spring.position, 0.0);
        }
    }

    #[test]
    fn it_starts_slowly_rather_than_at_full_speed() {
        // The whole reason this replaced an exponential ease. An ease covers
        // more ground in its first frame than in any other, which reads as a
        // snap and then a crawl; a spring starts at rest.
        let mut spring = Spring::new();
        spring.push(400.0);
        let mut covered = Vec::new();
        for _ in 0..4 {
            let before = spring.position;
            spring.advance(FRAME, 0.3);
            covered.push(before - spring.position);
        }
        assert!(
            covered[1] > covered[0],
            "the second frame covered less than the first: {covered:?}"
        );
        assert!(
            covered[2] > covered[1],
            "it was already slowing down by the third frame: {covered:?}"
        );
    }

    #[test]
    fn it_never_overshoots() {
        // Critically damped rather than merely damped: an overshoot in a
        // scroll is the text going too far and coming back, which is worse
        // than not animating at all.
        let mut spring = Spring::new();
        spring.push(400.0);
        for _ in 0..200 {
            spring.advance(FRAME, 0.3);
            assert!(
                spring.position >= -0.5,
                "it went past the end: {}",
                spring.position
            );
        }
    }

    #[test]
    fn a_second_push_adds_to_the_first_rather_than_restarting_it() {
        // Three notches of a wheel in quick succession should go further and
        // faster, not stutter.
        let mut one = Spring::new();
        one.push(100.0);
        for _ in 0..3 {
            one.advance(FRAME, 0.3);
        }

        let mut two = Spring::new();
        two.push(100.0);
        for _ in 0..3 {
            two.advance(FRAME, 0.3);
        }
        two.push(100.0);
        let before = two.position;
        two.advance(FRAME, 0.3);
        let moved_together = before - two.position;

        let before = one.position;
        one.advance(FRAME, 0.3);
        let moved_alone = before - one.position;
        assert!(
            moved_together > moved_alone,
            "the second push did not add speed: {moved_together} against {moved_alone}"
        );
    }

    #[test]
    fn the_same_length_is_the_same_speed_at_any_frame_rate() {
        let took = |frame: std::time::Duration| {
            let mut spring = Spring::new();
            spring.push(400.0);
            let mut spent = std::time::Duration::ZERO;
            while spring.advance(frame, 0.3) && spent.as_secs_f32() < 10.0 {
                spent += frame;
            }
            spent.as_millis() as i64
        };
        let sixty = took(std::time::Duration::from_micros(16_667));
        let one_forty_four = took(std::time::Duration::from_micros(6_944));
        assert!(
            (sixty - one_forty_four).abs() < 25,
            "60Hz took {sixty}ms and 144Hz took {one_forty_four}ms"
        );
    }

    #[test]
    fn a_journey_shorter_than_a_frame_is_over() {
        // Which is how a setting of zero turns the animation off.
        let mut spring = Spring::new();
        spring.push(400.0);
        assert!(!spring.advance(FRAME, 0.0));
        assert_eq!(spring.position, 0.0);
    }

    #[test]
    fn a_spring_at_rest_asks_for_no_frames() {
        let mut spring = Spring::new();
        assert!(!spring.is_moving());
        assert!(!spring.advance(FRAME, 0.3));
    }

    #[test]
    fn setting_the_distance_forgets_the_speed() {
        // What a resize does: where it was coming from was measured in cells
        // that are a different size now.
        let mut spring = Spring::new();
        spring.push(400.0);
        spring.advance(FRAME, 0.3);
        spring.set(10.0);
        assert_eq!(spring.position, 10.0);
        let before = spring.position;
        spring.advance(FRAME, 0.3);
        let moved = before - spring.position;
        // Starting from rest again, so barely anything in the first frame.
        assert!(moved < 2.0, "it kept the speed it had: moved {moved}");
    }
}
