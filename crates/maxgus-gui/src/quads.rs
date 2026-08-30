//! Turning a drawn surface into what the GPU draws.
//!
//! The editor's redisplay produces a grid of cells; this turns that grid into
//! two lists of instanced quads — one of solid rectangles for the backgrounds,
//! one of textured rectangles for the glyphs. Both are plain data, so what
//! goes to the GPU can be checked without one.

use crate::font::{CellMetrics, Fonts, Style};
use maxgus_faces::{Color, Face};
use maxgus_tui::Surface;

/// One rectangle of solid colour.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Rect {
    /// Position and size in pixels, from the top left of the window.
    pub position: [f32; 2],
    pub size: [f32; 2],
    pub color: [f32; 4],
}

/// One rectangle sampled from the glyph atlas.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Sprite {
    pub position: [f32; 2],
    pub size: [f32; 2],
    /// The glyph's place in the atlas, in pixels.
    pub source: [f32; 2],
    pub source_size: [f32; 2],
    pub color: [f32; 4],
}

/// Everything one frame draws.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Frame {
    pub rects: Vec<Rect>,
    pub sprites: Vec<Sprite>,
}

/// One colour component, from what a theme writes to what the GPU wants.
///
/// A theme's `#1d1f21` is an sRGB byte, which is how every colour anyone
/// types is meant. The window's surface is an sRGB format, which means the
/// GPU encodes what the shader writes on the way out — so what the shader
/// writes has to be linear, or the encoding happens twice and every colour
/// comes out pale. `#1d1f21` gamma-encoded twice is `#5e6164`, which is
/// what a dark theme looked like in this window for a while: grey.
pub fn linear(component: u8) -> f32 {
    let value = component as f32 / 255.0;
    match value <= 0.04045 {
        true => value / 12.92,
        false => ((value + 0.055) / 1.055).powf(2.4),
    }
}

/// A theme's colour, as the GPU wants it.
pub fn linear_rgb(r: u8, g: u8, b: u8) -> [f32; 4] {
    [linear(r), linear(g), linear(b), 1.0]
}

/// The colours a theme's `Default` resolves to, which a window has to choose
/// for itself: there is no terminal underneath to inherit them from.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub foreground: [f32; 4],
    pub background: [f32; 4],
    /// The sixteen ANSI colours, for a theme that names them by index.
    pub ansi: [[f32; 4]; 16],
}

impl Palette {
    /// The palette a theme implies.
    ///
    /// Rebuilt for every frame rather than captured when the window opens:
    /// `load-theme` changes the theme underneath, and a palette taken once
    /// at startup is why the window used to keep the colours it was born
    /// with however many themes were loaded over the top of them.
    pub fn of(theme: &maxgus_faces::Theme) -> Palette {
        let default = theme.resolve("default");
        let plain = |color: Option<Color>, fallback: [f32; 4]| match color {
            Some(Color::Rgb(r, g, b)) => linear_rgb(r, g, b),
            _ => fallback,
        };
        let mut ansi = [[0.0, 0.0, 0.0, 1.0]; 16];
        for (index, slot) in ansi.iter_mut().enumerate() {
            let (r, g, b) = maxgus_faces::xterm_palette_rgb(index as u8);
            *slot = linear_rgb(r, g, b);
        }
        Palette {
            foreground: plain(default.foreground, linear_rgb(217, 222, 230)),
            background: plain(default.background, linear_rgb(23, 26, 31)),
            ansi,
        }
    }

    /// The colour a face's foreground or background resolves to.
    pub fn resolve(&self, color: Option<Color>, default: [f32; 4]) -> [f32; 4] {
        match color {
            None | Some(Color::Default) => default,
            Some(Color::Rgb(r, g, b)) => linear_rgb(r, g, b),
            Some(Color::Indexed(index)) => self.indexed(index),
        }
    }

    /// The xterm palette. The first sixteen come from the window's own
    /// palette, which a theme may have redefined; the rest are xterm's, read
    /// from the faces crate rather than written out a second time here.
    fn indexed(&self, index: u8) -> [f32; 4] {
        if index < 16 {
            return self.ansi[index as usize];
        }
        let (r, g, b) = maxgus_faces::xterm_palette_rgb(index);
        linear_rgb(r, g, b)
    }
}

/// One window's text sliding, which is what smooth scrolling is.
///
/// Only the text area of the window being scrolled moves. Shifting the whole
/// surface — which this did for a while — drags the mode line, the echo area,
/// the file tree and every other window up and down with it, and a wheel
/// notch then makes the entire editor judder.
#[derive(Debug, Clone, PartialEq)]
pub struct Shift {
    /// The window's text area, in cells: its mode line is not in it.
    pub area: maxgus_tui::Rect,
    /// How far up the text in it is drawn, in pixels. Negative draws it
    /// lower, which is what scrolling towards the top of the buffer does.
    pub pixels: f32,
    /// The line sliding into the gap the shift opens, and which cell row it
    /// belongs at, counted from the top of the area — so `area.height` for
    /// the line arriving at the bottom and `-1` for the one at the top.
    ///
    /// The editor draws the lines that fit in the window and no others, so
    /// this one has to be fetched separately. Without it the gap is left as
    /// background, which is right only at the ends of a buffer.
    pub incoming: Option<(i32, Vec<maxgus_tui::Cell>)>,
}

impl Shift {
    /// The pixels the area covers, which is what its cells are clipped to.
    fn band(&self, metrics: CellMetrics) -> (f32, f32) {
        (
            self.area.y as f32 * metrics.height,
            (self.area.y + self.area.height) as f32 * metrics.height,
        )
    }

    fn holds(&self, x: u16, y: u16) -> bool {
        x >= self.area.x
            && x < self.area.x + self.area.width
            && y >= self.area.y
            && y < self.area.y + self.area.height
    }
}

/// Builds the frame for a drawn surface.
pub fn build(
    surface: &Surface,
    fonts: &mut Fonts,
    palette: &Palette,
    shift: Option<&Shift>,
    cursor: Option<(u16, u16)>,
) -> Frame {
    let metrics = fonts.metrics();
    let mut frame = Frame::default();
    let size = surface.size();
    // The gap the shift opens has to be covered before the cells go over it,
    // or the frame before this one shows through it.
    if let Some(shift) = shift
        && shift.pixels != 0.0
    {
        let (top, bottom) = shift.band(metrics);
        frame.rects.push(Rect {
            position: [shift.area.x as f32 * metrics.width, top],
            size: [shift.area.width as f32 * metrics.width, bottom - top],
            color: palette.background,
        });
    }
    for y in 0..size.height {
        for x in 0..size.width {
            let Some(cell) = surface.get(x, y) else {
                continue;
            };
            if cell.continuation {
                continue;
            }
            let on_cursor = cursor == Some((x, y));
            let moves = shift.filter(|shift| shift.holds(x, y));
            let top = y as f32 * metrics.height - moves.map(|s| s.pixels).unwrap_or(0.0);
            let band = moves.map(|s| s.band(metrics));
            push_cell(
                &mut frame, cell, x, top, metrics, palette, fonts, band, on_cursor,
            );
        }
    }
    // And the line sliding in, which the editor did not draw because it is
    // not in the window yet.
    if let Some(shift) = shift
        && let Some((row, cells)) = shift.incoming.as_ref()
    {
        let band = shift.band(metrics);
        let top = (shift.area.y as i32 + row) as f32 * metrics.height - shift.pixels;
        for (n, cell) in cells.iter().enumerate() {
            if cell.continuation {
                continue;
            }
            let x = shift.area.x + n as u16;
            if x >= shift.area.x + shift.area.width {
                break;
            }
            push_cell(
                &mut frame,
                cell,
                x,
                top,
                metrics,
                palette,
                fonts,
                Some(band),
                false,
            );
        }
    }
    frame
}

/// Cuts a rectangle down to a band of pixels, or drops it if it is wholly
/// outside one. Returns where it now starts, how tall it now is, and how much
/// came off the top — which a glyph needs, to know where in the atlas it now
/// starts reading.
fn clipped(top: f32, height: f32, band: Option<(f32, f32)>) -> Option<(f32, f32, f32)> {
    let Some((first, last)) = band else {
        return Some((top, height, 0.0));
    };
    let bottom = top + height;
    if bottom <= first || top >= last {
        return None;
    }
    let cut = top.max(first);
    Some((cut, bottom.min(last) - cut, cut - top))
}

#[allow(clippy::too_many_arguments)]
fn push_cell(
    frame: &mut Frame,
    cell: &maxgus_tui::Cell,
    x: u16,
    top: f32,
    metrics: CellMetrics,
    palette: &Palette,
    fonts: &mut Fonts,
    band: Option<(f32, f32)>,
    on_cursor: bool,
) {
    let face: &Face = &cell.face;
    let reverse = face.attributes.reverse.unwrap_or(false) ^ on_cursor;
    let (mut foreground, mut background) = (
        palette.resolve(face.foreground, palette.foreground),
        palette.resolve(face.background, palette.background),
    );
    if reverse {
        std::mem::swap(&mut foreground, &mut background);
    }
    let left = x as f32 * metrics.width;

    // The background, always: a cell that shares the window's colour still
    // has to cover whatever the last frame left there.
    if let Some((top, height, _)) = clipped(top, metrics.height, band) {
        frame.rects.push(Rect {
            position: [left, top],
            size: [metrics.width, height],
            color: background,
        });
    }

    if cell.ch != ' '
        && let Some(glyph) = fonts.glyph(
            cell.ch,
            Style::of(
                face.attributes.bold.unwrap_or(false),
                face.attributes.italic.unwrap_or(false),
            ),
        )
    {
        let ink = top + metrics.ascent + glyph.top;
        // A glyph cut in half reads a shorter part of the atlas, or the half
        // that is left would be squashed into it rather than cropped.
        if let Some((ink, height, cut)) = clipped(ink, glyph.height as f32, band) {
            frame.sprites.push(Sprite {
                position: [left + glyph.left, ink],
                size: [glyph.width as f32, height],
                source: [glyph.x as f32, glyph.y as f32 + cut],
                source_size: [glyph.width as f32, height],
                color: foreground,
            });
        }
    }

    // Underlines are a rectangle under the baseline rather than a glyph.
    if face.attributes.underline.unwrap_or(false) {
        let rule = top + metrics.ascent + 1.0;
        let thickness = 1.0_f32.max(metrics.height / 16.0);
        if let Some((rule, height, _)) = clipped(rule, thickness, band) {
            frame.rects.push(Rect {
                position: [left, rule],
                size: [metrics.width, height],
                color: foreground,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use maxgus_faces::Attributes;
    use maxgus_tui::{Cell, Size};

    fn palette() -> Palette {
        Palette {
            foreground: [1.0, 1.0, 1.0, 1.0],
            background: [0.0, 0.0, 0.0, 1.0],
            ansi: [[0.5, 0.5, 0.5, 1.0]; 16],
        }
    }

    fn fonts() -> Option<Fonts> {
        Fonts::load("this-font-does-not-exist", 16.0).ok()
    }

    fn surface_of(text: &str, face: Face) -> Surface {
        let mut surface = Surface::new(Size::new(text.chars().count() as u16, 1));
        for (x, ch) in text.chars().enumerate() {
            surface.set(x as u16, 0, Cell::new(ch, face));
        }
        surface
    }

    #[test]
    fn a_colour_is_resolved_from_the_face_or_the_window() {
        let palette = palette();
        assert_eq!(
            palette.resolve(None, palette.foreground),
            palette.foreground
        );
        assert_eq!(
            palette.resolve(Some(Color::Default), palette.background),
            palette.background
        );
        assert_eq!(
            palette.resolve(Some(Color::Rgb(255, 0, 128)), palette.foreground),
            linear_rgb(255, 0, 128)
        );
    }

    #[test]
    fn a_colour_reaches_the_gpu_as_light_rather_than_as_a_byte() {
        // The window's surface is an sRGB format: the GPU encodes what the
        // shader writes, so what the shader writes must already be linear.
        // Handing it the byte would encode it twice and wash it out.
        assert_eq!(linear(0), 0.0, "black is black either way");
        assert_eq!(linear(255), 1.0, "white is white either way");
        let half = linear(128);
        assert!(
            (half - 0.2158).abs() < 0.001,
            "mid grey should be {half} of the light, not half of it"
        );
        // The theme's own background, and what it used to come out as.
        let dark = linear_rgb(0x1d, 0x1f, 0x21);
        assert!(
            dark[0] < 0.02,
            "a dark theme reached the window as {dark:?}, which is grey"
        );
    }

    /// The encoding this undoes, so the test above is checking a round trip
    /// rather than a number somebody typed.
    fn encode(linear: f32) -> f32 {
        match linear <= 0.003_130_8 {
            true => linear * 12.92,
            false => 1.055 * linear.powf(1.0 / 2.4) - 0.055,
        }
    }

    #[test]
    fn what_the_gpu_encodes_is_what_the_theme_wrote() {
        for byte in [0u8, 1, 17, 29, 128, 200, 254, 255] {
            let there_and_back = encode(linear(byte)) * 255.0;
            assert!(
                (there_and_back - byte as f32).abs() < 0.5,
                "{byte} came back as {there_and_back}"
            );
        }
    }

    #[test]
    fn the_palette_cube_and_ramp_are_the_xterm_ones() {
        let palette = palette();
        // 16 is the bottom of the cube: black.
        assert_eq!(palette.indexed(16), [0.0, 0.0, 0.0, 1.0]);
        // 231 is the top: white.
        assert_eq!(palette.indexed(231), [1.0, 1.0, 1.0, 1.0]);
        // The ramp climbs.
        assert!(palette.indexed(232)[0] < palette.indexed(255)[0]);
    }

    #[test]
    fn every_cell_gets_a_background_whether_or_not_it_has_ink() {
        let Some(mut fonts) = fonts() else {
            eprintln!("skipping: no monospace font is installed");
            return;
        };
        let surface = surface_of("a b", Face::default());
        let frame = build(&surface, &mut fonts, &palette(), None, None);
        assert_eq!(frame.rects.len(), 3, "one background per cell");
        assert_eq!(frame.sprites.len(), 2, "the space has no glyph");
    }

    #[test]
    fn cells_are_laid_out_along_the_grid() {
        let Some(mut fonts) = fonts() else {
            eprintln!("skipping: no monospace font is installed");
            return;
        };
        let metrics = fonts.metrics();
        let surface = surface_of("abc", Face::default());
        let frame = build(&surface, &mut fonts, &palette(), None, None);
        for (n, rect) in frame.rects.iter().enumerate() {
            assert_eq!(rect.position[0], n as f32 * metrics.width);
            assert_eq!(rect.position[1], 0.0);
        }
    }

    /// A surface with `rows` rows of `text`, which stands in for a window,
    /// its mode line and the echo area under it.
    fn stack(text: &str, rows: u16) -> Surface {
        let mut surface = Surface::new(Size::new(text.chars().count() as u16, rows));
        for y in 0..rows {
            for (x, ch) in text.chars().enumerate() {
                surface.set(x as u16, y, Cell::new(ch, Face::default()));
            }
        }
        surface
    }

    #[test]
    fn only_the_scrolling_window_moves() {
        // The bug this is here for: the shift was applied to the whole
        // surface, so a wheel notch slid the mode line, the echo area, the
        // file tree and every other window up and down with the text, and
        // the entire editor juddered.
        let Some(mut fonts) = fonts() else {
            eprintln!("skipping: no monospace font is installed");
            return;
        };
        let metrics = fonts.metrics();
        let surface = stack("abc", 4);
        let shift = Shift {
            // Three rows of text; the fourth is a mode line and does not move.
            area: maxgus_tui::Rect::new(0, 0, 3, 3),
            pixels: 5.0,
            incoming: None,
        };
        let still = build(&surface, &mut fonts, &palette(), None, None);
        let moved = build(&surface, &mut fonts, &palette(), Some(&shift), None);

        let row_of = |frame: &Frame, y: u16| -> Vec<f32> {
            frame
                .sprites
                .iter()
                .filter(|s| {
                    let expected = y as f32 * metrics.height + metrics.ascent;
                    (s.position[1] - expected).abs() < metrics.height
                })
                .map(|s| s.position[1])
                .collect()
        };
        for y in 0..3 {
            let before = row_of(&still, y);
            assert!(!before.is_empty(), "row {y} has no ink to move");
        }
        // Every glyph left in the last row sits exactly where it did.
        let outside: Vec<f32> = moved
            .sprites
            .iter()
            .map(|s| s.position[1])
            .filter(|y| *y >= 3.0 * metrics.height)
            .collect();
        let unmoved: Vec<f32> = still
            .sprites
            .iter()
            .map(|s| s.position[1])
            .filter(|y| *y >= 3.0 * metrics.height)
            .collect();
        assert_eq!(outside, unmoved, "the mode line moved with the text");
    }

    #[test]
    fn the_scrolling_window_moves_by_the_pixels_it_was_given() {
        let Some(mut fonts) = fonts() else {
            eprintln!("skipping: no monospace font is installed");
            return;
        };
        let surface = stack("abc", 4);
        let shift = Shift {
            area: maxgus_tui::Rect::new(0, 0, 3, 3),
            pixels: 5.0,
            incoming: None,
        };
        let still = build(&surface, &mut fonts, &palette(), None, None);
        let moved = build(&surface, &mut fonts, &palette(), Some(&shift), None);
        // The first row's glyphs, which are inside the area either way.
        let first = |frame: &Frame| frame.sprites[0].position[1];
        assert_eq!(first(&moved), first(&still) - 5.0);
    }

    #[test]
    fn nothing_is_drawn_outside_the_window_that_is_scrolling() {
        // A line shifted up by a fraction would otherwise spill over the
        // window above it, or over its own mode line.
        let Some(mut fonts) = fonts() else {
            eprintln!("skipping: no monospace font is installed");
            return;
        };
        let metrics = fonts.metrics();
        let surface = stack("abc", 4);
        // A window that starts one row down, so there is something above it
        // to spill into as well as below.
        let area = maxgus_tui::Rect::new(0, 1, 3, 2);
        let band = (
            metrics.height,
            (area.y + area.height) as f32 * metrics.height,
        );
        for pixels in [-9.0, -1.0, 1.0, 9.0] {
            let shift = Shift {
                area,
                pixels,
                incoming: None,
            };
            let frame = build(&surface, &mut fonts, &palette(), Some(&shift), None);
            for rect in &frame.rects {
                let (top, bottom) = (rect.position[1], rect.position[1] + rect.size[1]);
                // Rows outside the area are drawn where they always were.
                if bottom <= band.0 || top >= band.1 {
                    continue;
                }
                assert!(
                    top >= band.0 - 0.01 && bottom <= band.1 + 0.01,
                    "a rectangle at {top}..{bottom} spilled out of {band:?} \
                     at {pixels} pixels"
                );
            }
            for sprite in &frame.sprites {
                let (top, bottom) = (sprite.position[1], sprite.position[1] + sprite.size[1]);
                if bottom <= band.0 || top >= band.1 {
                    continue;
                }
                assert!(
                    top >= band.0 - 0.01 && bottom <= band.1 + 0.01,
                    "a glyph at {top}..{bottom} spilled out of {band:?}"
                );
            }
        }
    }

    #[test]
    fn the_line_sliding_in_fills_the_gap_the_shift_opens() {
        // Without it the bottom of the window is a band of background that
        // grows and snaps back every line, which is the flicker.
        let Some(mut fonts) = fonts() else {
            eprintln!("skipping: no monospace font is installed");
            return;
        };
        let metrics = fonts.metrics();
        let surface = stack("abc", 4);
        let area = maxgus_tui::Rect::new(0, 0, 3, 3);
        let arriving: Vec<Cell> = "xyz"
            .chars()
            .map(|ch| Cell::new(ch, Face::default()))
            .collect();
        let shift = Shift {
            area,
            pixels: metrics.height - 2.0,
            incoming: Some((area.height as i32, arriving)),
        };
        let frame = build(&surface, &mut fonts, &palette(), Some(&shift), None);
        let gap = (
            area.height as f32 * metrics.height - shift.pixels,
            area.height as f32 * metrics.height,
        );
        let filled = frame
            .rects
            .iter()
            .filter(|r| r.position[1] >= gap.0 - 0.01 && r.position[1] < gap.1)
            .count();
        assert!(
            filled >= 3,
            "the arriving line did not reach the gap {gap:?}: {:#?}",
            frame.rects
        );
    }

    #[test]
    fn a_glyph_cut_in_half_reads_half_the_atlas() {
        // Squashing it into the space left instead would make the text
        // shorter and shorter as it slid off the edge.
        let Some(mut fonts) = fonts() else {
            eprintln!("skipping: no monospace font is installed");
            return;
        };
        let metrics = fonts.metrics();
        let surface = stack("abc", 2);
        let whole = build(&surface, &mut fonts, &palette(), None, None);
        let shift = Shift {
            area: maxgus_tui::Rect::new(0, 0, 3, 1),
            pixels: metrics.height / 2.0,
            incoming: None,
        };
        let cut = build(&surface, &mut fonts, &palette(), Some(&shift), None);
        let first = cut.sprites.first().expect("some ink survived the cut");
        let before = whole.sprites[0];
        if first.size[1] < before.size[1] {
            assert_eq!(
                first.source[1] - before.source[1],
                before.size[1] - first.size[1],
                "the glyph was squashed rather than cropped"
            );
        }
    }

    #[test]
    fn the_cursor_cell_is_drawn_the_other_way_round() {
        let Some(mut fonts) = fonts() else {
            eprintln!("skipping: no monospace font is installed");
            return;
        };
        let face = Face {
            foreground: Some(Color::Rgb(255, 255, 255)),
            background: Some(Color::Rgb(0, 0, 0)),
            attributes: Attributes::default(),
        };
        let surface = surface_of("ab", face);
        let frame = build(&surface, &mut fonts, &palette(), None, Some((1, 0)));
        assert_eq!(frame.rects[0].color, [0.0, 0.0, 0.0, 1.0], "the plain cell");
        assert_eq!(
            frame.rects[1].color,
            [1.0, 1.0, 1.0, 1.0],
            "the cursor cell keeps its own colours the wrong way round"
        );
        assert_eq!(frame.sprites[1].color, [0.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn an_underline_is_a_rectangle_under_the_baseline() {
        let Some(mut fonts) = fonts() else {
            eprintln!("skipping: no monospace font is installed");
            return;
        };
        let attributes = Attributes {
            underline: Some(true),
            ..Default::default()
        };
        let face = Face {
            attributes,
            ..Default::default()
        };
        let surface = surface_of("a", face);
        let frame = build(&surface, &mut fonts, &palette(), None, None);
        assert_eq!(frame.rects.len(), 2, "a background and a rule");
        let rule = frame.rects[1];
        assert!(
            rule.position[1] > fonts.metrics().ascent,
            "the rule is above the baseline"
        );
        assert!(rule.size[1] >= 1.0, "a rule with no thickness");
    }
}
