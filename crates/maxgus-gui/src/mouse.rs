//! What a pointer means to an editor made of cells.
//!
//! The window has pixels and the editor has a grid, so every one of these is a
//! division. Kept apart from the event loop because that is where the mistakes
//! are — an off-by-one here puts the cursor on the wrong character — and this
//! way they can be checked without a window.

use crate::font::CellMetrics;

/// The cell under a pointer, clamped into a grid of `columns` by `rows`
/// whose top left corner is `origin` pixels into the window.
pub fn cell_at(
    x: f64,
    y: f64,
    origin: [f32; 2],
    metrics: CellMetrics,
    columns: u16,
    rows: u16,
    scroll: f32,
) -> (u16, u16) {
    let column = ((x as f32 - origin[0]) / metrics.width).floor().max(0.0) as u32;
    let row = ((y as f32 - origin[1] + scroll) / metrics.height)
        .floor()
        .max(0.0) as u32;
    (
        column.min(columns.saturating_sub(1) as u32) as u16,
        row.min(rows.saturating_sub(1) as u32) as u16,
    )
}

/// How far a wheel event moves the view, in pixels.
///
/// A mouse reports whole notches and a touchpad reports pixels. A notch is
/// `mouse-wheel-lines` lines — three by default, which is what every other
/// program does with one; a touchpad already said how far it moved and is
/// taken at its word.
pub fn wheel_pixels(
    delta: winit::event::MouseScrollDelta,
    line_height: f32,
    per_notch: usize,
) -> f32 {
    use winit::event::MouseScrollDelta;
    match delta {
        MouseScrollDelta::LineDelta(_, lines) => -lines * line_height * per_notch as f32,
        MouseScrollDelta::PixelDelta(position) => -position.y as f32,
    }
}

/// How many steps the wheel zooms the text with control held: one a notch
/// for a mouse, and for a touchpad one each time the fingers have gone
/// this many pixels, carried in `so_far` between events. Up is larger.
pub fn zoom_steps(delta: winit::event::MouseScrollDelta, so_far: &mut f32) -> i32 {
    use winit::event::MouseScrollDelta;
    /// A touchpad's worth of a notch: a modest swipe, not a twitch.
    const SWIPE: f32 = 40.0;
    match delta {
        MouseScrollDelta::LineDelta(_, lines) => lines.round() as i32,
        MouseScrollDelta::PixelDelta(position) => {
            *so_far += position.y as f32;
            let steps = (*so_far / SWIPE).trunc();
            *so_far -= steps * SWIPE;
            steps as i32
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::event::MouseScrollDelta;

    const METRICS: CellMetrics = CellMetrics {
        width: 10.0,
        height: 20.0,
        ascent: 15.0,
    };

    #[test]
    fn a_pointer_lands_on_the_cell_it_is_over() {
        assert_eq!(cell_at(0.0, 0.0, [0.0, 0.0], METRICS, 80, 24, 0.0), (0, 0));
        assert_eq!(cell_at(9.9, 19.9, [0.0, 0.0], METRICS, 80, 24, 0.0), (0, 0));
        assert_eq!(
            cell_at(10.0, 20.0, [0.0, 0.0], METRICS, 80, 24, 0.0),
            (1, 1)
        );
        assert_eq!(
            cell_at(35.0, 51.0, [0.0, 0.0], METRICS, 80, 24, 0.0),
            (3, 2)
        );
    }

    #[test]
    fn a_pointer_outside_the_grid_is_clamped_into_it() {
        assert_eq!(
            cell_at(-5.0, -5.0, [0.0, 0.0], METRICS, 80, 24, 0.0),
            (0, 0)
        );
        assert_eq!(
            cell_at(10_000.0, 10_000.0, [0.0, 0.0], METRICS, 80, 24, 0.0),
            (79, 23)
        );
    }

    #[test]
    fn a_scrolled_view_shifts_which_row_is_under_the_pointer() {
        // The text is drawn shifted up by half a line, so the pointer is
        // half a line further down the buffer than the window.
        assert_eq!(cell_at(0.0, 15.0, [0.0, 0.0], METRICS, 80, 24, 0.0), (0, 0));
        assert_eq!(
            cell_at(0.0, 15.0, [0.0, 0.0], METRICS, 80, 24, 10.0),
            (0, 1)
        );
    }

    #[test]
    fn padding_moves_the_grid_in_from_the_window_edge() {
        // Eight pixels of margin: the pointer at (8, 8) is on the first
        // cell, and what is in the margin counts as the nearest cell.
        let origin = [8.0, 8.0];
        assert_eq!(cell_at(8.0, 8.0, origin, METRICS, 80, 24, 0.0), (0, 0));
        assert_eq!(cell_at(3.0, 3.0, origin, METRICS, 80, 24, 0.0), (0, 0));
        assert_eq!(cell_at(18.0, 28.0, origin, METRICS, 80, 24, 0.0), (1, 1));
        assert_eq!(cell_at(17.9, 27.9, origin, METRICS, 80, 24, 0.0), (0, 0));
    }

    #[test]
    fn a_wheel_notch_is_three_lines_unless_told_otherwise() {
        let pixels = wheel_pixels(MouseScrollDelta::LineDelta(0.0, -1.0), 20.0, 3);
        assert_eq!(pixels, 60.0, "one notch down is three lines down");
        let up = wheel_pixels(MouseScrollDelta::LineDelta(0.0, 1.0), 20.0, 3);
        assert_eq!(up, -60.0);
        // `mouse-wheel-lines` is how far a notch goes.
        let further = wheel_pixels(MouseScrollDelta::LineDelta(0.0, -1.0), 20.0, 8);
        assert_eq!(further, 160.0, "eight lines a notch is eight lines");
    }

    #[test]
    fn a_notch_with_control_held_is_one_step_of_zoom() {
        let mut carried = 0.0;
        let up = zoom_steps(MouseScrollDelta::LineDelta(0.0, 1.0), &mut carried);
        assert_eq!(up, 1, "up is larger");
        let down = zoom_steps(MouseScrollDelta::LineDelta(0.0, -1.0), &mut carried);
        assert_eq!(down, -1);
        assert_eq!(carried, 0.0, "a notch carries nothing over");
    }

    #[test]
    fn a_touchpad_zooms_a_step_per_swipe_and_carries_the_rest() {
        let mut carried = 0.0;
        let small = MouseScrollDelta::PixelDelta((0.0, 15.0).into());
        assert_eq!(zoom_steps(small, &mut carried), 0, "a twitch is not a step");
        assert_eq!(zoom_steps(small, &mut carried), 0);
        assert_eq!(zoom_steps(small, &mut carried), 1, "three add up to one");
        assert_eq!(carried, 5.0, "the pixels past the step wait for the next");
        let back = MouseScrollDelta::PixelDelta((0.0, -85.0).into());
        assert_eq!(zoom_steps(back, &mut carried), -2, "a long swipe is two");
    }

    #[test]
    fn a_touchpad_reports_the_pixels_it_moved() {
        // It already said how far the fingers went; multiplying that by a
        // line count would make a touchpad unusable.
        let delta = MouseScrollDelta::PixelDelta((0.0, -7.5).into());
        assert_eq!(wheel_pixels(delta, 20.0, 3), 7.5);
        assert_eq!(wheel_pixels(delta, 20.0, 12), 7.5);
    }
}
