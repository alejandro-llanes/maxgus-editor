//! What a pointer means to an editor made of cells.
//!
//! The window has pixels and the editor has a grid, so every one of these is a
//! division. Kept apart from the event loop because that is where the mistakes
//! are — an off-by-one here puts the cursor on the wrong character — and this
//! way they can be checked without a window.

use crate::font::CellMetrics;

/// The cell under a pointer, clamped into a grid of `columns` by `rows`.
pub fn cell_at(
    x: f64,
    y: f64,
    metrics: CellMetrics,
    columns: u16,
    rows: u16,
    scroll: f32,
) -> (u16, u16) {
    let column = (x as f32 / metrics.width).floor().max(0.0) as u32;
    let row = ((y as f32 + scroll) / metrics.height).floor().max(0.0) as u32;
    (
        column.min(columns.saturating_sub(1) as u32) as u16,
        row.min(rows.saturating_sub(1) as u32) as u16,
    )
}

/// How far a wheel event moves the view, in pixels.
///
/// A mouse reports whole notches and a touchpad reports pixels; a notch is
/// three lines, which is what every other application does with one.
pub fn wheel_pixels(delta: winit::event::MouseScrollDelta, line_height: f32) -> f32 {
    use winit::event::MouseScrollDelta;
    match delta {
        MouseScrollDelta::LineDelta(_, lines) => -lines * line_height * 3.0,
        MouseScrollDelta::PixelDelta(position) => -position.y as f32,
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
        assert_eq!(cell_at(0.0, 0.0, METRICS, 80, 24, 0.0), (0, 0));
        assert_eq!(cell_at(9.9, 19.9, METRICS, 80, 24, 0.0), (0, 0));
        assert_eq!(cell_at(10.0, 20.0, METRICS, 80, 24, 0.0), (1, 1));
        assert_eq!(cell_at(35.0, 51.0, METRICS, 80, 24, 0.0), (3, 2));
    }

    #[test]
    fn a_pointer_outside_the_grid_is_clamped_into_it() {
        assert_eq!(cell_at(-5.0, -5.0, METRICS, 80, 24, 0.0), (0, 0));
        assert_eq!(cell_at(10_000.0, 10_000.0, METRICS, 80, 24, 0.0), (79, 23));
    }

    #[test]
    fn a_scrolled_view_shifts_which_row_is_under_the_pointer() {
        // The text is drawn shifted up by half a line, so the pointer is
        // half a line further down the buffer than the window.
        assert_eq!(cell_at(0.0, 15.0, METRICS, 80, 24, 0.0), (0, 0));
        assert_eq!(cell_at(0.0, 15.0, METRICS, 80, 24, 10.0), (0, 1));
    }

    #[test]
    fn a_wheel_notch_is_three_lines() {
        let pixels = wheel_pixels(MouseScrollDelta::LineDelta(0.0, -1.0), 20.0);
        assert_eq!(pixels, 60.0, "one notch down is three lines down");
        let up = wheel_pixels(MouseScrollDelta::LineDelta(0.0, 1.0), 20.0);
        assert_eq!(up, -60.0);
    }

    #[test]
    fn a_touchpad_reports_the_pixels_it_moved() {
        let delta = MouseScrollDelta::PixelDelta((0.0, -7.5).into());
        assert_eq!(wheel_pixels(delta, 20.0), 7.5);
    }
}
