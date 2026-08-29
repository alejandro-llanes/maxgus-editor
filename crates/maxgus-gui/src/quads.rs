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
    /// The colour a face's foreground or background resolves to.
    pub fn resolve(&self, color: Option<Color>, default: [f32; 4]) -> [f32; 4] {
        match color {
            None | Some(Color::Default) => default,
            Some(Color::Rgb(r, g, b)) => {
                [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0]
            }
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
        [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0]
    }
}

/// Builds the frame for a drawn surface.
///
/// `scroll` shifts every row by that many pixels, which is what makes the
/// scrolling smooth: the grid is still whole cells, and the whole grid slides.
pub fn build(
    surface: &Surface,
    fonts: &mut Fonts,
    palette: &Palette,
    scroll: f32,
    cursor: Option<(u16, u16)>,
) -> Frame {
    let metrics = fonts.metrics();
    let mut frame = Frame::default();
    let size = surface.size();
    for y in 0..size.height {
        for x in 0..size.width {
            let Some(cell) = surface.get(x, y) else {
                continue;
            };
            if cell.continuation {
                continue;
            }
            let on_cursor = cursor == Some((x, y));
            push_cell(
                &mut frame, cell, x, y, metrics, palette, fonts, scroll, on_cursor,
            );
        }
    }
    frame
}

#[allow(clippy::too_many_arguments)]
fn push_cell(
    frame: &mut Frame,
    cell: &maxgus_tui::Cell,
    x: u16,
    y: u16,
    metrics: CellMetrics,
    palette: &Palette,
    fonts: &mut Fonts,
    scroll: f32,
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
    let top = y as f32 * metrics.height - scroll;

    // The background, always: a cell that shares the window's colour still
    // has to cover whatever the last frame left there.
    frame.rects.push(Rect {
        position: [left, top],
        size: [metrics.width, metrics.height],
        color: background,
    });

    if cell.ch != ' '
        && let Some(glyph) = fonts.glyph(
            cell.ch,
            Style::of(
                face.attributes.bold.unwrap_or(false),
                face.attributes.italic.unwrap_or(false),
            ),
        )
    {
        frame.sprites.push(Sprite {
            position: [left + glyph.left, top + metrics.ascent + glyph.top],
            size: [glyph.width as f32, glyph.height as f32],
            source: [glyph.x as f32, glyph.y as f32],
            source_size: [glyph.width as f32, glyph.height as f32],
            color: foreground,
        });
    }

    // Underlines are a rectangle under the baseline rather than a glyph.
    if face.attributes.underline.unwrap_or(false) {
        frame.rects.push(Rect {
            position: [left, top + metrics.ascent + 1.0],
            size: [metrics.width, 1.0_f32.max(metrics.height / 16.0)],
            color: foreground,
        });
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
            [1.0, 0.0, 128.0 / 255.0, 1.0]
        );
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
        let frame = build(&surface, &mut fonts, &palette(), 0.0, None);
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
        let frame = build(&surface, &mut fonts, &palette(), 0.0, None);
        for (n, rect) in frame.rects.iter().enumerate() {
            assert_eq!(rect.position[0], n as f32 * metrics.width);
            assert_eq!(rect.position[1], 0.0);
        }
    }

    #[test]
    fn scrolling_shifts_every_row_by_the_same_pixels() {
        let Some(mut fonts) = fonts() else {
            eprintln!("skipping: no monospace font is installed");
            return;
        };
        let surface = surface_of("abc", Face::default());
        let still = build(&surface, &mut fonts, &palette(), 0.0, None);
        let moved = build(&surface, &mut fonts, &palette(), 7.0, None);
        for (a, b) in still.rects.iter().zip(&moved.rects) {
            assert_eq!(b.position[1], a.position[1] - 7.0);
        }
        for (a, b) in still.sprites.iter().zip(&moved.sprites) {
            assert_eq!(b.position[1], a.position[1] - 7.0);
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
        let frame = build(&surface, &mut fonts, &palette(), 0.0, Some((1, 0)));
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
        let frame = build(&surface, &mut fonts, &palette(), 0.0, None);
        assert_eq!(frame.rects.len(), 2, "a background and a rule");
        let rule = frame.rects[1];
        assert!(
            rule.position[1] > fonts.metrics().ascent,
            "the rule is above the baseline"
        );
        assert!(rule.size[1] >= 1.0, "a rule with no thickness");
    }
}
