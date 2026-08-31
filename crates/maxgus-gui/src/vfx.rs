//! What the cursor leaves behind it.
//!
//! Six of them: three that mark where the cursor landed, and three that
//! trail particles along the way it came. They are decoration and they are
//! meant to be — the editor works identically with `cursor-vfx` unset,
//! which is the default, and nothing here runs at all until a name is put
//! in it.
//!
//! The two families are quite different animals. A *highlight* is one shape
//! at the destination that grows and fades over a fixed lifetime. A *trail*
//! spawns particles along the path the cursor travelled, each with its own
//! speed, rotation and lifetime, and then simply lets them run down.
//!
//! Pure, like the rest of the animation here: given a destination and a
//! frame's worth of time it says what to draw, and a test can watch a
//! particle live and die without a GPU.

use crate::quads::{Circle, Frame, Rect};

/// Which effect, if any.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// Nothing at all, which is what the editor does unless asked.
    #[default]
    None,
    /// A filled disc swelling out of where the cursor landed.
    SonicBoom,
    /// A ring doing the same.
    Ripple,
    /// A square outline doing the same.
    Wireframe,
    /// Particles flung along the path, curling as they go.
    Railgun,
    /// Particles thrown backwards from the direction of travel.
    Torpedo,
    /// Particles that fall away upwards, like something shaken loose.
    PixieDust,
}

impl Mode {
    /// The name in the configuration file, or `None` for one nobody wrote.
    pub fn parse(name: &str) -> Option<Mode> {
        Some(match name {
            "" | "none" => Mode::None,
            "sonicboom" => Mode::SonicBoom,
            "ripple" => Mode::Ripple,
            "wireframe" => Mode::Wireframe,
            "railgun" => Mode::Railgun,
            "torpedo" => Mode::Torpedo,
            "pixiedust" => Mode::PixieDust,
            _ => return None,
        })
    }

    /// Every name that can be written, for the error that suggests one.
    pub const NAMES: &'static [&'static str] = &[
        "sonicboom",
        "ripple",
        "wireframe",
        "railgun",
        "torpedo",
        "pixiedust",
    ];

    fn is_highlight(self) -> bool {
        matches!(self, Mode::SonicBoom | Mode::Ripple | Mode::Wireframe)
    }
}

/// The knobs, taken from the configuration once per frame.
#[derive(Debug, Clone, Copy)]
pub struct Settings {
    pub mode: Mode,
    /// How solid the effect is at its strongest, from 0 to 1.
    pub opacity: f32,
    /// How long a trail particle lives, in seconds.
    pub particle_lifetime: f32,
    /// How long a highlight takes to swell and fade, in seconds.
    pub highlight_lifetime: f32,
    /// Particles per cell of distance travelled.
    pub density: f32,
    pub speed: f32,
    /// How far round the arc a railgun's particles are flung.
    pub phase: f32,
    /// How fast a particle's direction turns as it flies.
    pub curl: f32,
}

impl Default for Settings {
    fn default() -> Settings {
        // Converted out of the percentages the configuration takes them
        // as.
        Settings {
            mode: Mode::None,
            opacity: 0.78,
            particle_lifetime: 0.5,
            highlight_lifetime: 0.2,
            density: 0.7,
            speed: 10.0,
            phase: 1.5,
            curl: 1.0,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Particle {
    at: [f32; 2],
    speed: [f32; 2],
    /// Radians per second the speed turns through.
    rotation: f32,
    /// Seconds left before it goes.
    life: f32,
}

/// The effect, and whatever it currently has in flight.
#[derive(Debug, Clone)]
pub struct Vfx {
    mode: Mode,
    /// How far through its life a highlight is, from 0 to 1.
    t: f32,
    centre: [f32; 2],
    particles: Vec<Particle>,
    /// Where the cursor was going last time, so a change means it moved.
    previous: Option<[f32; 2]>,
    /// The fraction of a particle left over from the last spawn, so a slow
    /// drag still emits at the right rate rather than rounding to none.
    remainder: f32,
    rng: Rng,
}

impl Default for Vfx {
    fn default() -> Vfx {
        Vfx::new()
    }
}

impl Vfx {
    pub fn new() -> Vfx {
        Vfx {
            mode: Mode::None,
            t: 1.0,
            centre: [0.0, 0.0],
            particles: Vec::new(),
            previous: None,
            remainder: 0.0,
            rng: Rng::new(),
        }
    }

    /// True while there is something to draw, which is what asks the event
    /// loop for another frame.
    pub fn is_running(&self) -> bool {
        !self.particles.is_empty() || (self.mode.is_highlight() && self.t < 1.0)
    }

    /// Advances by a frame. `centre` is the middle of the cell the cursor is
    /// heading for, and `cell` is how big one is.
    pub fn step(
        &mut self,
        elapsed: std::time::Duration,
        centre: [f32; 2],
        cell: (f32, f32),
        settings: &Settings,
    ) {
        // A mode changed under it — `load-theme` and friends re-read the
        // configuration — so whatever the last one had in flight goes.
        if settings.mode != self.mode {
            self.mode = settings.mode;
            self.particles.clear();
            self.t = 1.0;
            self.remainder = 0.0;
        }
        if self.mode == Mode::None {
            self.previous = Some(centre);
            return;
        }
        let dt = elapsed.as_secs_f32();
        // The first sighting is not a movement. Trailing particles across
        // the screen because the editor just opened is a firework nobody
        // asked for.
        let was = self.previous;
        let moved = was.is_some_and(|was| was != centre);
        self.previous = Some(centre);

        if self.mode.is_highlight() {
            self.centre = centre;
            if moved {
                self.t = 0.0;
            }
            if settings.highlight_lifetime > 0.0 {
                self.t = (self.t + dt / settings.highlight_lifetime).min(1.0);
            } else {
                self.t = 1.0;
            }
            return;
        }

        // A trail: age what is in flight, then spawn for the ground covered.
        self.particles.retain_mut(|particle| {
            particle.life -= dt;
            if particle.life <= 0.0 {
                return false;
            }
            particle.at[0] += particle.speed[0] * dt;
            particle.at[1] += particle.speed[1] * dt;
            particle.speed = rotate(particle.speed, dt * particle.rotation);
            true
        });
        if let Some(from) = was.filter(|_| moved) {
            self.spawn(from, centre, cell, settings);
        }
    }

    /// Scatters particles along the path from `from` to `centre`.
    ///
    /// `from` is passed in rather than remembered, because what a trail
    /// travelled from is the last place the cursor *was* — and the field
    /// that used to be read here is only kept up to date by the highlights,
    /// so the first trail of a session set out from the top left corner.
    fn spawn(&mut self, from: [f32; 2], centre: [f32; 2], cell: (f32, f32), settings: &Settings) {
        let travel = [centre[0] - from[0], centre[1] - from[1]];
        let far = (travel[0] * travel[0] + travel[1] * travel[1]).sqrt();
        self.centre = centre;
        if far <= f32::EPSILON || cell.1 <= 0.0 {
            return;
        }
        // More of them the further it went, and the fraction left over is
        // kept so a slow drag still emits at the right rate.
        let wanted = (far / cell.1) * settings.density + self.remainder;
        let count = wanted as usize;
        self.remainder = wanted - count as f32;
        // A jump across a large window can ask for hundreds. There is no
        // sense in which the six-hundredth is visible, and a frame spent
        // making them is a frame the cursor is not moving in.
        let count = count.min(256);
        for n in 0..count {
            let t = (n + 1) as f32 / count as f32;
            let speed = match self.mode {
                Mode::Railgun => {
                    let phase = t / std::f32::consts::PI * settings.phase * (far / cell.1);
                    [
                        phase.sin() * 2.0 * settings.speed,
                        phase.cos() * 2.0 * settings.speed,
                    ]
                }
                Mode::Torpedo => {
                    let along = normalize(travel);
                    let away = self.rng.direction();
                    let out = [away[0] - along[0] * 1.5, away[1] - along[1] * 1.5];
                    let out = normalize(out);
                    [out[0] * settings.speed, out[1] * settings.speed]
                }
                // Upwards and outwards, like something shaken loose.
                _ => {
                    let base = self.rng.direction();
                    let out = normalize([base[0] * 0.5, 0.4 + base[1].abs()]);
                    [out[0] * 3.0 * settings.speed, out[1] * 3.0 * settings.speed]
                }
            };
            let at = match self.mode {
                // Strung out evenly along the path it took.
                Mode::Railgun => [from[0] + travel[0] * t, from[1] + travel[1] * t],
                // Scattered along it instead, and half a line lower, which
                // is where the ink is rather than where the cell starts.
                _ => {
                    let along = self.rng.next_f32();
                    [
                        from[0] + travel[0] * along,
                        from[1] + travel[1] * along + cell.1 * 0.5,
                    ]
                }
            };
            let rotation = match self.mode {
                Mode::Railgun => std::f32::consts::PI * settings.curl,
                _ => (self.rng.next_f32() - 0.5) * std::f32::consts::FRAC_PI_2 * settings.curl,
            };
            self.particles.push(Particle {
                at,
                speed,
                rotation,
                // Staggered, so the trail thins out behind rather than
                // vanishing all at once.
                life: t * settings.particle_lifetime,
            });
        }
    }

    /// Draws whatever is in flight. `colour` is the cursor's own.
    pub fn draw(&self, frame: &mut Frame, colour: [f32; 4], cell: (f32, f32), settings: &Settings) {
        let faded = |alpha: f32| [colour[0], colour[1], colour[2], alpha.clamp(0.0, 1.0)];
        match self.mode {
            Mode::None => {}
            Mode::SonicBoom | Mode::Ripple | Mode::Wireframe => {
                if self.t >= 1.0 {
                    return;
                }
                // Fading out as it swells, quadratically, so it is gone
                // before it is large enough to be in the way.
                let alpha = settings.opacity * (1.0 - self.t * self.t);
                let radius = self.t * 3.0 * cell.1;
                match self.mode {
                    Mode::SonicBoom => frame.circles.push(Circle {
                        center: self.centre,
                        radius: radius * 0.5,
                        thickness: 0.0,
                        color: faded(alpha),
                    }),
                    Mode::Ripple => frame.circles.push(Circle {
                        center: self.centre,
                        radius: radius * 0.5,
                        thickness: cell.1 * 0.2,
                        color: faded(alpha),
                    }),
                    // A square has no shader of its own; four thin bars
                    // are the same picture and cost nothing to add.
                    _ => outline(frame, self.centre, radius, cell.1 * 0.2, faded(alpha)),
                }
            }
            Mode::Railgun | Mode::Torpedo | Mode::PixieDust => {
                for particle in &self.particles {
                    let left = match settings.particle_lifetime > 0.0 {
                        true => particle.life / settings.particle_lifetime,
                        false => 0.0,
                    };
                    let alpha = settings.opacity * left;
                    match self.mode {
                        Mode::PixieDust => {
                            let size = cell.0 * 0.2;
                            frame.rects.push(Rect {
                                position: [
                                    particle.at[0] - size * 0.5,
                                    particle.at[1] - size * 0.5,
                                ],
                                size: [size, size],
                                color: faded(alpha),
                            });
                        }
                        // Shrinking as they die, which reads as distance.
                        _ => frame.circles.push(Circle {
                            center: particle.at,
                            radius: cell.0 * 0.25 * left,
                            thickness: cell.1 * 0.2,
                            color: faded(alpha),
                        }),
                    }
                }
            }
        }
    }
}

/// Four bars making a square outline, since there is no stroked rectangle.
fn outline(frame: &mut Frame, centre: [f32; 2], size: f32, thickness: f32, colour: [f32; 4]) {
    let half = size * 0.5;
    let (left, top) = (centre[0] - half, centre[1] - half);
    let bar = |position, size| Rect {
        position,
        size,
        color: colour,
    };
    frame.rects.push(bar([left, top], [size, thickness]));
    frame
        .rects
        .push(bar([left, top + size - thickness], [size, thickness]));
    frame.rects.push(bar([left, top], [thickness, size]));
    frame
        .rects
        .push(bar([left + size - thickness, top], [thickness, size]));
}

fn normalize(v: [f32; 2]) -> [f32; 2] {
    let length = (v[0] * v[0] + v[1] * v[1]).sqrt();
    match length > f32::EPSILON {
        true => [v[0] / length, v[1] / length],
        false => [0.0, 0.0],
    }
}

fn rotate(v: [f32; 2], by: f32) -> [f32; 2] {
    let (sin, cos) = by.sin_cos();
    [v[0] * cos - v[1] * sin, v[0] * sin + v[1] * cos]
}

/// A small PCG generator, so the scatter is the same on every machine.
///
/// Its own rather than a crate's: this wants a handful of numbers a frame
/// and nothing about their quality matters beyond not being visibly a
/// pattern.
#[derive(Debug, Clone)]
struct Rng {
    state: u64,
    inc: u64,
}

impl Rng {
    fn new() -> Rng {
        Rng {
            state: 0x853C_49E6_748F_EA9B,
            inc: (0xDA3E_39CB_94B9_5BDBu64 << 1) | 1,
        }
    }

    fn next(&mut self) -> u32 {
        let was = self.state;
        self.state = was
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(self.inc);
        let rot = (was >> 59) as u32;
        let xsh = (((was >> 18) ^ was) >> 27) as u32;
        xsh.rotate_right(rot)
    }

    fn next_f32(&mut self) -> f32 {
        self.next() as f32 / u32::MAX as f32
    }

    /// A direction, of length one, pointing anywhere.
    fn direction(&mut self) -> [f32; 2] {
        let x = self.next_f32() * 2.0 - 1.0;
        let y = self.next_f32() * 2.0 - 1.0;
        normalize([x, y])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: std::time::Duration = std::time::Duration::from_micros(16_667);
    const CELL: (f32, f32) = (10.0, 20.0);

    fn settings(mode: Mode) -> Settings {
        Settings {
            mode,
            ..Settings::default()
        }
    }

    fn frame(vfx: &Vfx, settings: &Settings) -> Frame {
        let mut frame = Frame::default();
        vfx.draw(&mut frame, [1.0, 1.0, 1.0, 1.0], CELL, settings);
        frame
    }

    #[test]
    fn every_name_in_the_reference_is_one_that_parses() {
        for name in Mode::NAMES {
            assert!(Mode::parse(name).is_some(), "`{name}` does not parse");
            assert_ne!(Mode::parse(name), Some(Mode::None), "`{name}` is nothing");
        }
        assert_eq!(Mode::parse(""), Some(Mode::None));
        assert_eq!(Mode::parse("fireworks"), None);
    }

    #[test]
    fn nothing_happens_when_nothing_was_asked_for() {
        // The default, and the thing most people will run: it has to cost
        // nothing and draw nothing.
        let settings = settings(Mode::None);
        let mut vfx = Vfx::new();
        for n in 0..30 {
            vfx.step(FRAME, [n as f32 * 40.0, 0.0], CELL, &settings);
        }
        assert!(!vfx.is_running());
        let frame = frame(&vfx, &settings);
        assert!(frame.circles.is_empty() && frame.rects.is_empty());
    }

    #[test]
    fn the_editor_opening_is_not_a_movement() {
        // The cursor's first sighting is not a journey from the top left,
        // and trailing particles across the screen for it would be the
        // first thing anyone sees.
        for mode in [Mode::Railgun, Mode::Torpedo, Mode::PixieDust] {
            let settings = settings(mode);
            let mut vfx = Vfx::new();
            vfx.step(FRAME, [900.0, 500.0], CELL, &settings);
            assert!(
                !vfx.is_running(),
                "{mode:?} let off a trail on the first frame"
            );
        }
    }

    #[test]
    fn a_trail_is_left_behind_a_moving_cursor_and_then_clears() {
        for mode in [Mode::Railgun, Mode::Torpedo, Mode::PixieDust] {
            let settings = settings(mode);
            let mut vfx = Vfx::new();
            vfx.step(FRAME, [0.0, 0.0], CELL, &settings);
            vfx.step(FRAME, [400.0, 0.0], CELL, &settings);
            assert!(vfx.is_running(), "{mode:?} left nothing behind");
            let drawn = frame(&vfx, &settings);
            assert!(
                !drawn.circles.is_empty() || !drawn.rects.is_empty(),
                "{mode:?} has particles and draws none"
            );

            // And they go out, rather than piling up forever.
            let mut frames = 0;
            while vfx.is_running() {
                vfx.step(FRAME, [400.0, 0.0], CELL, &settings);
                frames += 1;
                assert!(frames < 600, "{mode:?} particles never died");
            }
            assert!(frame(&vfx, &settings).circles.is_empty());
        }
    }

    #[test]
    fn a_trail_sets_out_from_where_the_cursor_was() {
        // Not from wherever the field happened to hold, which for a trail
        // was the origin: the first movement of a session laid a streak of
        // particles from the top left corner of the window to the cursor.
        let settings = settings(Mode::Railgun);
        let mut vfx = Vfx::new();
        vfx.step(FRAME, [600.0, 400.0], CELL, &settings);
        vfx.step(FRAME, [640.0, 400.0], CELL, &settings);
        assert!(!vfx.particles.is_empty(), "nothing was spawned");
        for particle in &vfx.particles {
            assert!(
                particle.at[0] >= 590.0 && particle.at[1] >= 390.0,
                "a particle was laid at {:?}, back towards the origin",
                particle.at
            );
        }
    }

    #[test]
    fn a_longer_journey_leaves_more_behind_it() {
        // The density setting is per cell travelled, so it has to be.
        let settings = settings(Mode::Railgun);
        let count = |to: f32| {
            let mut vfx = Vfx::new();
            vfx.step(FRAME, [0.0, 0.0], CELL, &settings);
            vfx.step(FRAME, [to, 0.0], CELL, &settings);
            vfx.particles.len()
        };
        assert!(count(400.0) > count(100.0), "distance made no difference");
    }

    #[test]
    fn a_highlight_swells_out_of_where_the_cursor_landed_and_fades() {
        for mode in [Mode::SonicBoom, Mode::Ripple, Mode::Wireframe] {
            let settings = settings(mode);
            let mut vfx = Vfx::new();
            vfx.step(FRAME, [0.0, 0.0], CELL, &settings);
            vfx.step(FRAME, [400.0, 200.0], CELL, &settings);
            assert!(vfx.is_running(), "{mode:?} did not start");

            let size = |vfx: &Vfx| {
                let drawn = frame(vfx, &settings);
                match mode {
                    Mode::Wireframe => drawn.rects.first().map(|r| r.size[0]).unwrap_or(0.0),
                    _ => drawn.circles.first().map(|c| c.radius).unwrap_or(0.0),
                }
            };
            let alpha = |vfx: &Vfx| {
                let drawn = frame(vfx, &settings);
                match mode {
                    Mode::Wireframe => drawn.rects.first().map(|r| r.color[3]).unwrap_or(0.0),
                    _ => drawn.circles.first().map(|c| c.color[3]).unwrap_or(0.0),
                }
            };
            let (was_size, was_alpha) = (size(&vfx), alpha(&vfx));
            for _ in 0..4 {
                vfx.step(FRAME, [400.0, 200.0], CELL, &settings);
            }
            assert!(size(&vfx) > was_size, "{mode:?} did not swell");
            assert!(alpha(&vfx) < was_alpha, "{mode:?} did not fade");

            // And it ends.
            let mut frames = 0;
            while vfx.is_running() {
                vfx.step(FRAME, [400.0, 200.0], CELL, &settings);
                frames += 1;
                assert!(frames < 600, "{mode:?} never finished");
            }
            assert!(frame(&vfx, &settings).circles.is_empty());
            assert!(frame(&vfx, &settings).rects.is_empty());
        }
    }

    #[test]
    fn a_highlight_starts_again_where_the_cursor_next_lands() {
        let settings = settings(Mode::SonicBoom);
        let mut vfx = Vfx::new();
        vfx.step(FRAME, [0.0, 0.0], CELL, &settings);
        vfx.step(FRAME, [100.0, 0.0], CELL, &settings);
        for _ in 0..4 {
            vfx.step(FRAME, [100.0, 0.0], CELL, &settings);
        }
        let grown = frame(&vfx, &settings).circles[0].radius;
        vfx.step(FRAME, [500.0, 300.0], CELL, &settings);
        let restarted = frame(&vfx, &settings).circles[0];
        assert!(restarted.radius < grown, "it kept growing from the old one");
        assert_eq!(restarted.center, [500.0, 300.0], "it stayed where it was");
    }

    #[test]
    fn a_jump_across_a_large_window_does_not_ask_for_thousands() {
        // A frame spent making particles is a frame the cursor is not
        // moving in, and nobody can see the six-hundredth one.
        let mut settings = settings(Mode::PixieDust);
        settings.density = 20.0;
        let mut vfx = Vfx::new();
        vfx.step(FRAME, [0.0, 0.0], CELL, &settings);
        vfx.step(FRAME, [8000.0, 4000.0], CELL, &settings);
        assert!(
            vfx.particles.len() <= 256,
            "{} particles from one jump",
            vfx.particles.len()
        );
    }

    #[test]
    fn changing_the_mode_clears_what_the_last_one_had_in_flight() {
        let mut vfx = Vfx::new();
        let railgun = settings(Mode::Railgun);
        vfx.step(FRAME, [0.0, 0.0], CELL, &railgun);
        vfx.step(FRAME, [400.0, 0.0], CELL, &railgun);
        assert!(vfx.is_running());
        let none = settings(Mode::None);
        vfx.step(FRAME, [400.0, 0.0], CELL, &none);
        assert!(!vfx.is_running(), "the old effect kept running");
    }

    #[test]
    fn the_generator_is_not_visibly_a_pattern() {
        // It only has to scatter. But a generator that returns the same
        // number every time would make every particle fly the same way,
        // which is the failure that would look deliberate.
        let mut rng = Rng::new();
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..64 {
            let v = rng.next_f32();
            assert!((0.0..=1.0).contains(&v), "{v} is not a fraction");
            seen.insert((v * 1000.0) as u32);
        }
        assert!(seen.len() > 50, "only {} distinct values in 64", seen.len());
    }
}
