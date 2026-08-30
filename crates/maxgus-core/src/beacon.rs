//! A light that shows where the cursor went.
//!
//! After a jump — a new buffer, a scroll, another window — the cursor can be
//! hard to find. This puts a short bright trail beside it that fades away, so
//! the eye is led to it rather than having to search. It is `beacon` for
//! Emacs, replicated: the same shape, the same timing, and the same settings
//! under the same names.
//!
//! The shape is a run of cells starting at point and running right, coloured
//! from the beacon's own colour at point to the buffer's background at the
//! far end. It is held at full length for a delay, and then eaten one cell at
//! a time from the bright end, so it both shortens and dims until it is gone.

use maxgus_faces::Color;

/// A beacon that is currently showing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Beacon {
    /// The window it is in. A beacon belongs to one window: the others have
    /// their own cursors and would be lit for no reason.
    pub window: crate::window::WindowId,
    /// Where it starts, as a character offset into that window's buffer.
    pub offset: usize,
    /// How long it has been showing.
    pub elapsed: std::time::Duration,
}

impl Beacon {
    pub fn new(window: crate::window::WindowId, offset: usize) -> Beacon {
        Beacon {
            window,
            offset,
            elapsed: std::time::Duration::ZERO,
        }
    }
}

/// How the beacon is drawn and how long it lasts.
///
/// The names are `beacon`'s own, so a setting copied from an Emacs
/// configuration means what it meant there.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shape {
    /// Cells, counting the one point is on.
    pub size: usize,
    /// How long it stays at full length before it starts to fade.
    pub delay: std::time::Duration,
    /// How long the fade takes.
    pub duration: std::time::Duration,
}

impl Shape {
    /// How long a beacon lives altogether.
    pub fn lifetime(&self) -> std::time::Duration {
        self.delay + self.duration
    }

    /// How often a frame is worth drawing: one cell's worth of fade.
    ///
    /// Any faster and frames are drawn that look the same as the last, which
    /// on a terminal is a screenful of escape sequences for nothing.
    pub fn tick(&self) -> std::time::Duration {
        self.duration / self.size.max(1) as u32
    }

    /// How many cells have been eaten by `elapsed`.
    ///
    /// Nothing during the delay, then one per tick, as `beacon--dec` does.
    pub fn consumed(&self, elapsed: std::time::Duration) -> usize {
        if elapsed < self.delay {
            return 0;
        }
        let fading = elapsed - self.delay;
        let ticks = fading.as_secs_f64() / self.tick().as_secs_f64().max(f64::EPSILON);
        (ticks as usize).min(self.size)
    }

    /// True once there is nothing left to draw.
    pub fn is_over(&self, elapsed: std::time::Duration) -> bool {
        self.consumed(elapsed) >= self.size
    }
}

/// The colour the light is, before it is mixed with the background.
///
/// `beacon-color` is either a colour or a number, and the number means a grey
/// chosen against the background: bright on a dark theme, dark on a light
/// one. Written as a number in a configuration it stays right when the theme
/// changes, which is why it is the default.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Light {
    /// A grade from 0 to 1, as `beacon-color` means a number.
    Grade(f32),
    Colour(Color),
}

impl Light {
    /// The colour to start the gradient from, given the buffer's background.
    pub fn against(&self, background: (u8, u8, u8)) -> (u8, u8, u8) {
        match self {
            Light::Colour(colour) => colour.to_rgb().unwrap_or((128, 128, 128)),
            Light::Grade(grade) => {
                let grade = grade.clamp(0.0, 1.0);
                // Beacon's rule: a dark background takes a light beacon, and
                // the other way round.
                let level = match is_dark(background) {
                    true => grade,
                    false => 1.0 - grade,
                };
                let level = (level * 255.0).round().clamp(0.0, 255.0) as u8;
                (level, level, level)
            }
        }
    }
}

/// Whether a background counts as dark, by the same measure beacon uses:
/// which of black or white it is nearer to.
pub fn is_dark(rgb: (u8, u8, u8)) -> bool {
    let (r, g, b) = (rgb.0 as u32, rgb.1 as u32, rgb.2 as u32);
    // Distance to black against distance to white, squared, as
    // `color-distance` compares them.
    let to_black = r * r + g * g + b * b;
    let white = 255u32;
    let to_white = (white - r).pow(2) + (white - g).pow(2) + (white - b).pow(2);
    to_black < to_white
}

/// The colour of one cell of the beacon.
///
/// `index` counts from the cell point is on. `consumed` is how many cells the
/// fade has eaten: the ramp shifts along by that many, so what is left is
/// both shorter and dimmer.
pub fn cell_colour(
    shape: &Shape,
    light: &Light,
    background: (u8, u8, u8),
    index: usize,
    consumed: usize,
) -> Option<(u8, u8, u8)> {
    let remaining = shape.size.checked_sub(consumed)?;
    if index >= remaining {
        return None;
    }
    let from = light.against(background);
    // How far along the gradient this cell sits once the fade has moved it.
    let step = (index + consumed) as f32 / shape.size.max(1) as f32;
    Some(mix(from, background, step))
}

/// `a` and `b` mixed, `t` of the way from one to the other.
fn mix(a: (u8, u8, u8), b: (u8, u8, u8), t: f32) -> (u8, u8, u8) {
    let t = t.clamp(0.0, 1.0);
    let blend = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
    (blend(a.0, b.0), blend(a.1, b.1), blend(a.2, b.2))
}

/// What may set a beacon off, matching `beacon`'s own switches.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Triggers {
    pub on: bool,
    pub buffer_changes: bool,
    pub window_scrolls: bool,
    pub window_changes: bool,
    /// Lines point must move for a beacon, or `0` for never — `beacon`'s
    /// `beacon-blink-when-point-moves-vertically`, which is off by default
    /// because ordinary editing would light it constantly.
    pub point_moves_vertically: usize,
}

/// What the editor looked like, for telling whether anything worth a beacon
/// happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Watch {
    pub buffer: maxgus_text::BufferId,
    pub window: crate::window::WindowId,
    pub top_line: usize,
    pub line: usize,
}

/// Whether the move from `before` to `after` is worth a beacon.
pub fn should_blink(triggers: &Triggers, before: &Watch, after: &Watch) -> bool {
    if !triggers.on {
        return false;
    }
    if triggers.buffer_changes && before.buffer != after.buffer {
        return true;
    }
    if triggers.window_changes && before.window != after.window {
        return true;
    }
    if triggers.window_scrolls && before.top_line != after.top_line {
        return true;
    }
    if triggers.point_moves_vertically > 0
        && before.line.abs_diff(after.line) >= triggers.point_moves_vertically
    {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn shape() -> Shape {
        Shape {
            size: 40,
            delay: Duration::from_millis(300),
            duration: Duration::from_millis(300),
        }
    }

    const DARK: (u8, u8, u8) = (30, 30, 40);
    const LIGHT: (u8, u8, u8) = (250, 250, 245);

    #[test]
    fn a_number_is_a_grey_chosen_against_the_background() {
        // Beacon's own rule: bright on a dark theme, dark on a light one, so
        // one number in a configuration is right either way.
        let half = Light::Grade(0.5);
        assert_eq!(half.against(DARK), (128, 128, 128));
        assert_eq!(half.against(LIGHT), (128, 128, 128));
        let bright = Light::Grade(0.9);
        assert!(
            bright.against(DARK).0 > 200,
            "a high grade on a dark background should be light"
        );
        assert!(
            bright.against(LIGHT).0 < 60,
            "a high grade on a light background should be dark"
        );
    }

    #[test]
    fn a_colour_is_that_colour_whatever_the_background() {
        let red = Light::Colour(Color::Rgb(200, 0, 0));
        assert_eq!(red.against(DARK), (200, 0, 0));
        assert_eq!(red.against(LIGHT), (200, 0, 0));
    }

    #[test]
    fn dark_and_light_are_told_apart_the_way_beacon_tells_them() {
        assert!(is_dark(DARK));
        assert!(!is_dark(LIGHT));
        assert!(is_dark((0, 0, 0)));
        assert!(!is_dark((255, 255, 255)));
    }

    #[test]
    fn the_beacon_is_brightest_at_point_and_fades_along_its_length() {
        let (shape, light) = (shape(), Light::Grade(0.9));
        let head = cell_colour(&shape, &light, DARK, 0, 0).expect("a first cell");
        let middle = cell_colour(&shape, &light, DARK, 20, 0).expect("a middle cell");
        let tail = cell_colour(&shape, &light, DARK, 39, 0).expect("a last cell");
        assert!(head.0 > middle.0, "it does not fade along its length");
        assert!(middle.0 > tail.0);
        assert!(
            tail.0.abs_diff(DARK.0) < 10,
            "the far end should be almost the background: {tail:?}"
        );
    }

    #[test]
    fn nothing_is_drawn_past_the_end_of_it() {
        let (shape, light) = (shape(), Light::Grade(0.5));
        assert!(cell_colour(&shape, &light, DARK, 39, 0).is_some());
        assert_eq!(cell_colour(&shape, &light, DARK, 40, 0), None);
    }

    #[test]
    fn the_fade_shortens_it_from_the_bright_end() {
        // What `beacon--dec` does: a cell goes, and what is left takes the
        // next colour along, so it retracts and dims together.
        let (shape, light) = (shape(), Light::Grade(0.9));
        let full = cell_colour(&shape, &light, DARK, 0, 0).expect("a cell");
        let faded = cell_colour(&shape, &light, DARK, 0, 10).expect("a cell");
        assert!(faded.0 < full.0, "the head did not dim");
        assert!(
            cell_colour(&shape, &light, DARK, 30, 10).is_none(),
            "it did not get shorter"
        );
    }

    #[test]
    fn nothing_happens_during_the_delay_and_then_it_goes() {
        let shape = shape();
        assert_eq!(shape.consumed(Duration::from_millis(0)), 0);
        assert_eq!(
            shape.consumed(Duration::from_millis(299)),
            0,
            "it faded early"
        );
        assert!(
            shape.consumed(Duration::from_millis(310)) > 0,
            "it did not start"
        );
        assert!(!shape.is_over(Duration::from_millis(400)));
        assert!(shape.is_over(shape.lifetime()), "it outlived its lifetime");
    }

    #[test]
    fn the_frame_rate_is_one_cell_of_fade() {
        // Beacon's own: `duration / size` between ticks.
        assert_eq!(shape().tick(), Duration::from_millis(300) / 40);
    }

    #[test]
    fn a_beacon_of_no_size_does_not_divide_by_zero() {
        let none = Shape {
            size: 0,
            delay: Duration::ZERO,
            duration: Duration::from_millis(100),
        };
        assert!(none.is_over(Duration::ZERO));
        assert_eq!(cell_colour(&none, &Light::Grade(0.5), DARK, 0, 0), None);
    }

    // ---- what sets it off ------------------------------------------------

    fn watch(buffer: u64, window: u64, top_line: usize, line: usize) -> Watch {
        Watch {
            buffer: maxgus_text::BufferId(buffer),
            window: crate::window::WindowId(window),
            top_line,
            line,
        }
    }

    fn triggers() -> Triggers {
        Triggers {
            on: true,
            buffer_changes: true,
            window_scrolls: true,
            window_changes: true,
            point_moves_vertically: 0,
        }
    }

    #[test]
    fn a_new_buffer_lights_it() {
        assert!(should_blink(
            &triggers(),
            &watch(1, 1, 0, 0),
            &watch(2, 1, 0, 0)
        ));
    }

    #[test]
    fn a_scroll_lights_it() {
        assert!(should_blink(
            &triggers(),
            &watch(1, 1, 0, 0),
            &watch(1, 1, 30, 30)
        ));
    }

    #[test]
    fn another_window_lights_it() {
        assert!(should_blink(
            &triggers(),
            &watch(1, 1, 0, 0),
            &watch(1, 2, 0, 0)
        ));
    }

    #[test]
    fn typing_in_one_place_does_not() {
        // The common case, and the one that would make it unbearable.
        assert!(!should_blink(
            &triggers(),
            &watch(1, 1, 0, 5),
            &watch(1, 1, 0, 5)
        ));
    }

    #[test]
    fn moving_a_line_does_not_unless_it_was_asked_for() {
        let before = watch(1, 1, 0, 5);
        let after = watch(1, 1, 0, 6);
        assert!(!should_blink(&triggers(), &before, &after));
        let asked = Triggers {
            point_moves_vertically: 1,
            ..triggers()
        };
        assert!(should_blink(&asked, &before, &after));
    }

    #[test]
    fn a_long_jump_lights_it_when_a_distance_was_given() {
        let asked = Triggers {
            point_moves_vertically: 10,
            ..triggers()
        };
        assert!(!should_blink(
            &asked,
            &watch(1, 1, 0, 5),
            &watch(1, 1, 0, 9)
        ));
        assert!(should_blink(
            &asked,
            &watch(1, 1, 0, 5),
            &watch(1, 1, 0, 40)
        ));
    }

    #[test]
    fn each_trigger_can_be_switched_off_on_its_own() {
        let quiet = Triggers {
            buffer_changes: false,
            window_scrolls: false,
            window_changes: false,
            ..triggers()
        };
        assert!(!should_blink(
            &quiet,
            &watch(1, 1, 0, 0),
            &watch(2, 3, 9, 9)
        ));
    }

    #[test]
    fn switching_it_off_switches_everything_off() {
        let off = Triggers {
            on: false,
            point_moves_vertically: 1,
            ..triggers()
        };
        assert!(!should_blink(&off, &watch(1, 1, 0, 0), &watch(2, 2, 9, 9)));
    }
}
