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

/// One quadrilateral of solid colour, given as its four corners.
///
/// A [`Rect`] is upright by construction and the cursor is not: the smear it
/// leaves while it travels is a block whose corners have got out of step with
/// each other, which is a shape no position-and-size can describe.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Quad {
    /// The corners in the order the unit quad has them: top left, top right,
    /// bottom left, bottom right.
    pub top_left: [f32; 2],
    pub top_right: [f32; 2],
    pub bottom_left: [f32; 2],
    pub bottom_right: [f32; 2],
    pub color: [f32; 4],
}

/// A disc, filled or as a ring.
///
/// What the cursor's particle effects are made of. Drawn as geometry with
/// the edge worked out in the shader rather than sampled from a texture, so
/// it stays smooth however large a sonic boom gets.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Circle {
    pub center: [f32; 2],
    pub radius: f32,
    /// Zero or less fills it; above zero draws a ring that thick.
    pub thickness: f32,
    pub color: [f32; 4],
}

/// Everything one frame draws.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Frame {
    pub rects: Vec<Rect>,
    /// Drawn over the backgrounds and under the glyphs, so text on top of
    /// the cursor stays text.
    pub quads: Vec<Quad>,
    pub sprites: Vec<Sprite>,
    /// The cursor's effects, drawn last so they are over the text they are
    /// trailing away from.
    pub circles: Vec<Circle>,
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
    /// What the cursor block is filled with while it is travelling.
    ///
    /// The `cursor` face reversed is what a cell under the cursor is drawn
    /// with, so the block's colour is that face's *background* — the same
    /// colour the cell will have when the block arrives on it, which is what
    /// makes the landing invisible.
    pub cursor: [f32; 4],
    /// The line between windows side by side: the `vertical-border` face's
    /// foreground.
    pub divider: [f32; 4],
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
        let foreground = plain(default.foreground, linear_rgb(217, 222, 230));
        let background = plain(default.background, linear_rgb(23, 26, 31));
        let cursor = theme.resolve("cursor");
        // A theme without the face gets the mode line's background, which
        // is the colour the seam already has on its last row.
        let divider = theme
            .resolve("vertical-border")
            .foreground
            .or(theme.resolve("mode-line").background);
        Palette {
            foreground,
            background,
            ansi,
            divider: match divider {
                Some(Color::Rgb(r, g, b)) => linear_rgb(r, g, b),
                Some(Color::Indexed(index)) => {
                    let (r, g, b) = maxgus_faces::xterm_palette_rgb(index);
                    linear_rgb(r, g, b)
                }
                _ => foreground,
            },
            cursor: match cursor.background {
                Some(Color::Rgb(r, g, b)) => linear_rgb(r, g, b),
                Some(Color::Indexed(index)) => {
                    let (r, g, b) = maxgus_faces::xterm_palette_rgb(index);
                    linear_rgb(r, g, b)
                }
                // A theme that leaves it to the terminal has no answer to
                // give, and the text's own colour is what a block cursor
                // has always been.
                _ => foreground,
            },
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
    /// The lines sliding into the gap the shift opens, top to bottom, and
    /// which cell row the first of them belongs at, counted from the top of
    /// the area — so `area.height` for lines arriving at the bottom and
    /// `-n` for the `n` arriving at the top.
    ///
    /// The editor draws the lines that fit in the window and no others, so
    /// these have to be fetched separately. Without them the gap is left as
    /// background, which is right only at the ends of a buffer. There is
    /// more than one because a command can move the view several lines and
    /// `scroll-animation-far-lines` slides the last few of them.
    pub incoming: Option<(i32, Vec<Vec<maxgus_tui::Cell>>)>,
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

/// Everything about a frame that is not the cells themselves.
///
/// A struct rather than eight arguments because it had become eight
/// arguments, and because the last two only make sense together: a window
/// that blurs what is behind a popup draws the frame twice, once for the
/// backdrop and once for the whole thing, and these say which of the two
/// is being asked for.
#[derive(Clone, Copy)]
pub struct Look<'a> {
    pub palette: &'a Palette,
    pub shift: Option<&'a Shift>,
    /// The cell to draw the other way round, when the cursor is resting.
    pub cursor: Option<(u16, u16)>,
    /// The block's four corners, when it is not.
    pub smear: Option<[[f32; 2]; 4]>,
    /// Where the font may join characters into ligatures: the text of the
    /// windows showing code. Nowhere when it is empty. `->` in a help page
    /// or `--color` on a shell line means the two characters it is made
    /// of, and a ligature there is the font contradicting the text.
    pub ligatures: &'a [maxgus_tui::Rect],
    /// The windows with another beside them: a line is drawn down the
    /// right edge of each, so the two do not run into one another.
    pub dividers: &'a [maxgus_tui::Rect],
    /// What floats over the windows. A divider stops where a popup covers
    /// it, since the popup is over the window, not under it.
    pub floating: &'a [maxgus_tui::Rect],
    /// Only the cells inside these, or all of them when it is empty. What
    /// the backdrop pass uses: a blur only shows within a hand's breadth of
    /// the popup it is behind, so the rest of the screen need not be drawn
    /// a second time to be blurred and thrown away.
    pub only: &'a [maxgus_tui::Rect],
    /// Cells inside these get their background at `opacity` rather than
    /// solid, so whatever was blurred underneath shows through them.
    pub translucent: &'a [maxgus_tui::Rect],
    pub opacity: f32,
}

impl<'a> Look<'a> {
    /// The plain thing: the whole frame, opaque.
    pub fn new(palette: &'a Palette) -> Look<'a> {
        Look {
            palette,
            shift: None,
            cursor: None,
            smear: None,
            ligatures: &[],
            dividers: &[],
            floating: &[],
            only: &[],
            translucent: &[],
            opacity: 1.0,
        }
    }
}

/// Whether any of `areas` holds this cell. An empty list holds everything,
/// which is what makes `only` optional without being an `Option`.
fn within(areas: &[maxgus_tui::Rect], x: u16, y: u16) -> bool {
    areas.is_empty() || covered(areas, x, y)
}

/// Whether any of `areas` holds this cell; none of an empty list does.
fn covered(areas: &[maxgus_tui::Rect], x: u16, y: u16) -> bool {
    areas.iter().any(|area| {
        x >= area.x && x < area.x + area.width && y >= area.y && y < area.y + area.height
    })
}

/// Builds the frame for a drawn surface.
pub fn build(surface: &Surface, fonts: &mut Fonts, look: &Look) -> Frame {
    let Look {
        palette,
        shift,
        cursor,
        smear,
        ligatures,
        dividers,
        floating,
        only,
        translucent,
        opacity,
    } = *look;
    let metrics = fonts.metrics();
    let mut frame = Frame::default();
    // The block on its way somewhere. `cursor` is the cell to draw the other
    // way round, and while the block is in transit there is no such cell:
    // the two together would be a cursor in two places at once.
    if let Some(corners) = smear {
        frame.quads.push(Quad {
            top_left: corners[0],
            top_right: corners[1],
            bottom_left: corners[2],
            bottom_right: corners[3],
            color: palette.cursor,
        });
    }
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
    let width = size.width as usize;
    for y in 0..size.height {
        // The font has its say a row at a time, because a ligature is a
        // property of the characters beside each other and not of any one
        // of them.
        let row = y as usize * width;
        let glyphs = row_glyphs(&surface.cells()[row..row + width], fonts, |x| {
            covered(ligatures, x as u16, y)
        });
        for x in 0..size.width {
            let Some(cell) = surface.get(x, y) else {
                continue;
            };
            if cell.continuation {
                continue;
            }
            if !within(only, x, y) {
                continue;
            }
            let on_cursor = cursor == Some((x, y));
            let moves = shift.filter(|shift| shift.holds(x, y));
            let top = y as f32 * metrics.height - moves.map(|s| s.pixels).unwrap_or(0.0);
            let band = moves.map(|s| s.band(metrics));
            let behind = match within(translucent, x, y) {
                true => opacity,
                false => 1.0,
            };
            let seam = !covered(floating, x, y)
                && dividers.iter().any(|area| {
                    x + 1 == area.x + area.width && y >= area.y && y < area.y + area.height
                });
            push_cell(
                &mut frame,
                cell,
                glyphs[x as usize],
                x,
                top,
                metrics,
                palette,
                fonts,
                band,
                on_cursor,
                behind,
                seam,
            );
        }
    }
    // And the lines sliding in, which the editor did not draw because they
    // are not in the window yet.
    if let Some(shift) = shift
        && let Some((row, rows)) = shift.incoming.as_ref()
    {
        let band = shift.band(metrics);
        for (line, cells) in rows.iter().enumerate() {
            let at = shift.area.y as i32 + row + line as i32;
            let top = at as f32 * metrics.height - shift.pixels;
            // The row is not in the frame yet, so whether it may have
            // ligatures is whether the window it is sliding into may.
            let glyphs = row_glyphs(cells, fonts, |n| {
                covered(ligatures, shift.area.x + n as u16, shift.area.y)
            });
            for (n, cell) in cells.iter().enumerate() {
                if cell.continuation {
                    continue;
                }
                let x = shift.area.x + n as u16;
                if x >= shift.area.x + shift.area.width {
                    break;
                }
                let seam = dividers
                    .iter()
                    .any(|area| x + 1 == area.x + area.width && shift.area.y >= area.y);
                push_cell(
                    &mut frame,
                    cell,
                    glyphs[n],
                    x,
                    top,
                    metrics,
                    palette,
                    fonts,
                    Some(band),
                    false,
                    1.0,
                    seam,
                );
            }
        }
    }
    frame
}

/// Which glyph each cell of a row draws, once the font has had its say.
///
/// One entry per cell. `None` is a cell with nothing to draw — a space, the
/// second half of a wide character, or a cell whose character was swallowed
/// by a ligature starting in an earlier one.
///
/// Runs are broken at a space, at a change of style, at anything not one
/// cell wide, and at the edge of where `ligate` allows them — the column it
/// is asked about. None of those can be inside a ligature, and short runs
/// are both cheaper to shape and likelier to be asked for again.
fn row_glyphs(
    cells: &[maxgus_tui::Cell],
    fonts: &mut Fonts,
    ligate: impl Fn(usize) -> bool,
) -> Vec<Option<u16>> {
    let style_of = |cell: &maxgus_tui::Cell| {
        Style::of(
            cell.face.attributes.bold.unwrap_or(false),
            cell.face.attributes.italic.unwrap_or(false),
        )
    };
    let joinable = |at: usize| {
        let cell = &cells[at];
        !cell.continuation
            && cell.ch != ' '
            && maxgus_tui::char_width(cell.ch) == 1
            && !crate::boxes::is_drawn(cell.ch)
            && ligate(at)
    };

    // One glyph per character to begin with, which is the answer whenever
    // the font has no opinion and the whole answer when ligatures are off.
    let mut out: Vec<Option<u16>> = cells
        .iter()
        .map(|cell| match cell.continuation || cell.ch == ' ' {
            true => None,
            false => Some(fonts.index_of(cell.ch, style_of(cell))),
        })
        .collect();

    let mut at = 0;
    while at < cells.len() {
        if !joinable(at) {
            at += 1;
            continue;
        }
        let style = style_of(&cells[at]);
        let mut text = String::new();
        // Where each character of the run starts, and which cell it is in.
        let mut columns: Vec<(usize, usize)> = Vec::new();
        let mut end = at;
        while end < cells.len() && joinable(end) && style_of(&cells[end]) == style {
            columns.push((text.len(), end));
            text.push(cells[end].ch);
            end += 1;
        }
        // A single character cannot be joined to anything, and asking about
        // it is the cost of shaping for an answer already known.
        if end > at + 1 {
            let shaped = fonts.shape(style, &text);
            let mut assigned: Vec<Option<u16>> = vec![None; columns.len()];
            let mut usable = true;
            for glyph in shaped.iter() {
                match columns
                    .iter()
                    .position(|(offset, _)| *offset == glyph.cluster)
                {
                    Some(n) => assigned[n] = Some(glyph.glyph),
                    // A cluster that is not the start of any character in
                    // the run means the shaper and this disagree about what
                    // was asked. Drawing the disagreement would drop
                    // characters, so the run is left as it was.
                    None => usable = false,
                }
            }
            if usable {
                for ((_, column), glyph) in columns.iter().zip(assigned) {
                    out[*column] = glyph;
                }
            }
        }
        at = end;
    }
    out
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
    // The glyph this cell draws, which is not always the one its character
    // would pick: a ligature puts one glyph in the first of the cells it
    // covers and nothing in the rest.
    glyph: Option<u16>,
    x: u16,
    top: f32,
    metrics: CellMetrics,
    palette: &Palette,
    fonts: &mut Fonts,
    band: Option<(f32, f32)>,
    on_cursor: bool,
    // How solid the background is. Below one where something blurred is
    // showing through from underneath.
    behind: f32,
    // Whether the window this cell is in has another to its right, in
    // which case a line goes down the cell's right edge.
    seam: bool,
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
    // Dim is the text part of the way to its background: still there, but
    // not asking to be read.
    if face.attributes.dim.unwrap_or(false) {
        for (channel, back) in foreground.iter_mut().zip(background).take(3) {
            *channel = *channel * 0.6 + back * 0.4;
        }
    }
    let left = x as f32 * metrics.width;
    // A wide character is two cells' worth of background, or the second
    // half of a CJK character in the region would be drawn plain.
    let width = metrics.width * maxgus_tui::char_width(cell.ch).max(1) as f32;

    // The background, always: a cell that shares the window's colour still
    // has to cover whatever the last frame left there.
    if let Some((top, height, _)) = clipped(top, metrics.height, band) {
        background[3] *= behind;
        frame.rects.push(Rect {
            position: [left, top],
            size: [width, height],
            color: background,
        });
    }

    // A rule through the middle of the cell, with a bit more of it below
    // than above: where the middle of lower-case letters is.
    let thickness = 1.0_f32.max(metrics.height / 16.0);
    let mut rule = |at: f32| {
        if let Some((rule, height, _)) = clipped(at, thickness, band) {
            frame.rects.push(Rect {
                position: [left, rule],
                size: [width, height],
                color: foreground,
            });
        }
    };
    if face.attributes.underline.unwrap_or(false) {
        rule(top + metrics.ascent + 1.0);
    }
    if face.attributes.strikethrough.unwrap_or(false) {
        rule(top + metrics.ascent * 0.65 - thickness / 2.0);
    }

    // Box-drawing and block characters are shapes, drawn as the shapes
    // they are so a row of them tiles and a frame of them joins up.
    if let Some(pieces) = crate::boxes::pieces(cell.ch, width, metrics.height) {
        for piece in pieces {
            if let Some((at, height, _)) = clipped(top + piece.y, piece.height, band) {
                let mut color = foreground;
                color[3] *= piece.alpha;
                frame.rects.push(Rect {
                    position: [left + piece.x, at],
                    size: [piece.width, height],
                    color,
                });
            }
        }
    } else if let Some(index) = glyph
        && let Some(glyph) = fonts.glyph_indexed(
            index,
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

    // The divider goes down the last pixel column of the cell, over the
    // background and whatever ink reached that far: it is the edge of the
    // window, and the window's text stops at it.
    if seam && let Some((top, height, _)) = clipped(top, metrics.height, band) {
        let thickness = 1.0_f32.max(metrics.width / 8.0).round();
        frame.rects.push(Rect {
            position: [left + width - thickness, top],
            size: [thickness, height],
            color: palette.divider,
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
            cursor: [1.0, 1.0, 1.0, 1.0],
            foreground: [1.0, 1.0, 1.0, 1.0],
            background: [0.0, 0.0, 0.0, 1.0],
            ansi: [[0.5, 0.5, 0.5, 1.0]; 16],
            divider: [0.3, 0.3, 0.3, 1.0],
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
        let frame = build(&surface, &mut fonts, &Look::new(&palette()));
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
        let frame = build(&surface, &mut fonts, &Look::new(&palette()));
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
        let still = build(&surface, &mut fonts, &Look::new(&palette()));
        let moved = build(
            &surface,
            &mut fonts,
            &Look {
                shift: Some(&shift),
                cursor: None,
                smear: None,
                ligatures: &[],
                ..Look::new(&palette())
            },
        );

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
        let still = build(&surface, &mut fonts, &Look::new(&palette()));
        let moved = build(
            &surface,
            &mut fonts,
            &Look {
                shift: Some(&shift),
                cursor: None,
                smear: None,
                ligatures: &[],
                ..Look::new(&palette())
            },
        );
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
            let frame = build(
                &surface,
                &mut fonts,
                &Look {
                    shift: Some(&shift),
                    cursor: None,
                    smear: None,
                    ligatures: &[],
                    ..Look::new(&palette())
                },
            );
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
            incoming: Some((area.height as i32, vec![arriving])),
        };
        let frame = build(
            &surface,
            &mut fonts,
            &Look {
                shift: Some(&shift),
                cursor: None,
                smear: None,
                ligatures: &[],
                ..Look::new(&palette())
            },
        );
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
        let whole = build(&surface, &mut fonts, &Look::new(&palette()));
        let shift = Shift {
            area: maxgus_tui::Rect::new(0, 0, 3, 1),
            pixels: metrics.height / 2.0,
            incoming: None,
        };
        let cut = build(
            &surface,
            &mut fonts,
            &Look {
                shift: Some(&shift),
                cursor: None,
                smear: None,
                ligatures: &[],
                ..Look::new(&palette())
            },
        );
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

    /// A row of cells all in one style, for asking what the font does with
    /// them.
    fn row(text: &str) -> Vec<maxgus_tui::Cell> {
        text.chars()
            .map(|ch| maxgus_tui::Cell::new(ch, Face::default()))
            .collect()
    }

    /// A font that joins characters, if the machine has one.
    ///
    /// Detected by asking rather than by name: a family that is not
    /// installed falls back to one that is, so the name proves nothing and
    /// only the answer does.
    fn ligature_font() -> Option<Fonts> {
        for family in [
            "FiraCode Nerd Font",
            "Fira Code",
            "JetBrainsMono Nerd Font",
            "JetBrains Mono",
        ] {
            let Ok(mut fonts) = Fonts::load(family, 16.0) else {
                continue;
            };
            if joins(&mut fonts, "!=") {
                return Some(fonts);
            }
        }
        None
    }

    /// Whether the font draws `text` with glyphs its characters would not
    /// have picked on their own — which is what a ligature is.
    ///
    /// Not "fewer glyphs than characters", which is how a proportional font
    /// does it. A monospace coding font cannot afford to lose a cell, so it
    /// substitutes *every* cell of the pair with a piece of the joined mark
    /// and keeps the count. `!=` in Fira Code is two glyphs, and neither is
    /// the `!` or the `=` it would draw alone.
    fn joins(fonts: &mut Fonts, text: &str) -> bool {
        let shaped = fonts.shape(Style::Regular, text);
        let alone: Vec<u16> = text
            .chars()
            .map(|ch| fonts.index_of(ch, Style::Regular))
            .collect();
        shaped.len() != alone.len() || shaped.iter().zip(&alone).any(|(s, a)| s.glyph != *a)
    }

    #[test]
    fn a_ligature_draws_glyphs_no_character_would_pick_alone() {
        let Some(mut fonts) = ligature_font() else {
            eprintln!("skipping: no font on this machine joins `!=`");
            return;
        };
        let cells = row("a != b");
        let joined = row_glyphs(&cells, &mut fonts, |_| true);
        let apart = row_glyphs(&cells, &mut fonts, |_| false);
        assert_ne!(
            (joined[2], joined[3]),
            (apart[2], apart[3]),
            "`!=` was drawn as a plain `!` and `=`, so nothing was joined"
        );
        // And only the run was touched: the letters either side are what
        // they always were.
        assert_eq!(joined[0], apart[0], "`a` changed");
        assert_eq!(joined[5], apart[5], "`b` changed");
        assert_eq!(joined[1], None, "a space drew something");
    }

    #[test]
    fn a_space_will_not_be_joined_across() {
        // `! =` is not `!=`. Shaping a whole line at once would leave the
        // font free to decide otherwise.
        let Some(mut fonts) = ligature_font() else {
            eprintln!("skipping: no font on this machine joins `!=`");
            return;
        };
        let cells = row("! =");
        let joined = row_glyphs(&cells, &mut fonts, |_| true);
        let apart = row_glyphs(&cells, &mut fonts, |_| false);
        assert_eq!(joined, apart, "a space was joined across: {joined:?}");
    }

    #[test]
    fn a_change_of_style_breaks_the_run() {
        // Half a mark in bold and half in regular is two fonts pretending
        // to be one glyph.
        let Some(mut fonts) = ligature_font() else {
            eprintln!("skipping: no font on this machine joins `!=`");
            return;
        };
        let mut bold = Face::default();
        bold.attributes.bold = Some(true);
        let cells = vec![
            maxgus_tui::Cell::new('!', Face::default()),
            maxgus_tui::Cell::new('=', bold),
        ];
        let joined = row_glyphs(&cells, &mut fonts, |_| true);
        let apart = row_glyphs(&cells, &mut fonts, |_| false);
        assert_eq!(joined, apart, "a ligature was formed across two styles");
    }

    #[test]
    fn a_ligature_forms_only_where_it_is_allowed() {
        // The same `!=` twice: in the code window and beside it in a help
        // page, where it is a `!` and an `=`.
        let Some(mut fonts) = ligature_font() else {
            eprintln!("skipping: no font on this machine joins `!=`");
            return;
        };
        let cells = row("!= !=");
        let glyphs = row_glyphs(&cells, &mut fonts, |x| x < 2);
        let apart = row_glyphs(&cells, &mut fonts, |_| false);
        assert_ne!(
            (glyphs[0], glyphs[1]),
            (apart[0], apart[1]),
            "code was not joined"
        );
        assert_eq!(
            (glyphs[3], glyphs[4]),
            (apart[3], apart[4]),
            "prose was joined"
        );
    }

    #[test]
    fn turning_ligatures_off_draws_what_the_characters_say() {
        // `set ligatures=#false` is how someone who does not want `!=` drawn
        // as one mark says so, and it has to actually undo it.
        let Some(mut fonts) = ligature_font() else {
            eprintln!("skipping: no font on this machine joins `!=`");
            return;
        };
        let cells = row("!=");
        let apart = row_glyphs(&cells, &mut fonts, |_| false);
        let expected: Vec<Option<u16>> = "!="
            .chars()
            .map(|ch| Some(fonts.index_of(ch, Style::Regular)))
            .collect();
        assert_eq!(apart, expected);
    }

    #[test]
    fn no_character_is_ever_lost_to_shaping() {
        // The invariant that holds whatever font is installed: a cell only
        // ever gives up its glyph to a ligature that starts before it, so
        // the number of glyphs never falls below the number of runs.
        let Some(mut fonts) = fonts() else {
            eprintln!("skipping: no monospace font is installed");
            return;
        };
        for text in [
            "fn main() { a != b; }",
            "-> => <= === |> ...",
            "x",
            "",
            "     ",
        ] {
            let cells = row(text);
            let glyphs = row_glyphs(&cells, &mut fonts, |_| true);
            assert_eq!(glyphs.len(), cells.len(), "`{text}` lost a cell");
            let ink = text.chars().filter(|c| *c != ' ').count();
            let drawn = glyphs.iter().filter(|g| g.is_some()).count();
            assert!(
                drawn <= ink,
                "`{text}` drew more glyphs than it has characters"
            );
            if ink > 0 {
                assert!(drawn > 0, "`{text}` drew nothing at all");
            }
        }
    }

    #[test]
    fn a_travelling_cursor_is_a_quad_of_its_own() {
        // While it is between two cells there is no cell to draw the other
        // way round, so the block has to be a shape rather than a cell.
        let Some(mut fonts) = fonts() else {
            eprintln!("skipping: no monospace font is installed");
            return;
        };
        let surface = stack("abc", 2);
        let smear = [[0.0, 0.0], [40.0, 0.0], [5.0, 20.0], [45.0, 20.0]];
        let frame = build(
            &surface,
            &mut fonts,
            &Look {
                shift: None,
                cursor: None,
                smear: Some(smear),
                ligatures: &[],
                ..Look::new(&palette())
            },
        );
        assert_eq!(frame.quads.len(), 1, "no block was drawn");
        let quad = frame.quads[0];
        assert_eq!(quad.top_left, [0.0, 0.0]);
        assert_eq!(quad.bottom_right, [45.0, 20.0]);
        assert_eq!(quad.color, palette().cursor);
        assert_ne!(
            quad.top_left[0] - quad.bottom_left[0],
            0.0,
            "a smear that is upright is not a smear"
        );
    }

    #[test]
    fn a_settled_cursor_draws_no_block_over_the_cell_it_is_on() {
        // The two together would be a cursor in two places, one of which is
        // not where point is.
        let Some(mut fonts) = fonts() else {
            eprintln!("skipping: no monospace font is installed");
            return;
        };
        let surface = stack("abc", 2);
        let frame = build(
            &surface,
            &mut fonts,
            &Look {
                shift: None,
                cursor: Some((1, 0)),
                smear: None,
                ligatures: &[],
                ..Look::new(&palette())
            },
        );
        assert!(
            frame.quads.is_empty(),
            "a block was drawn as well as a cell"
        );
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
        let frame = build(
            &surface,
            &mut fonts,
            &Look {
                shift: None,
                cursor: Some((1, 0)),
                smear: None,
                ligatures: &[],
                ..Look::new(&palette())
            },
        );
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
        let frame = build(&surface, &mut fonts, &Look::new(&palette()));
        assert_eq!(frame.rects.len(), 2, "a background and a rule");
        let rule = frame.rects[1];
        assert!(
            rule.position[1] > fonts.metrics().ascent,
            "the rule is above the baseline"
        );
        assert!(rule.size[1] >= 1.0, "a rule with no thickness");
    }

    #[test]
    fn a_strikethrough_is_a_rectangle_through_the_letters_and_dim_is_faded() {
        let Some(mut fonts) = fonts() else {
            eprintln!("skipping: no monospace font is installed");
            return;
        };
        let face = Face {
            attributes: Attributes {
                strikethrough: Some(true),
                dim: Some(true),
                ..Default::default()
            },
            ..Default::default()
        };
        let surface = surface_of("a", face);
        let palette = palette();
        let frame = build(&surface, &mut fonts, &Look::new(&palette));
        assert_eq!(frame.rects.len(), 2, "a background and a rule");
        let rule = frame.rects[1];
        let ascent = fonts.metrics().ascent;
        assert!(
            rule.position[1] > ascent * 0.4 && rule.position[1] < ascent * 0.8,
            "the rule is not through the middle of the letters: {}",
            rule.position[1]
        );
        let ink = frame.sprites[0].color;
        assert!(
            ink[0] < palette.foreground[0] && ink[0] > palette.background[0],
            "dim text is neither faded nor legible: {ink:?}"
        );
        assert_eq!(rule.color, ink, "the rule is not the text's colour");
    }

    #[test]
    fn a_wide_character_gets_a_background_two_cells_wide() {
        let Some(mut fonts) = fonts() else {
            eprintln!("skipping: no monospace font is installed");
            return;
        };
        let face = Face {
            background: Some(Color::Rgb(0, 0, 255)),
            ..Default::default()
        };
        let mut surface = Surface::new(Size::new(3, 1));
        surface.set(0, 0, Cell::new('日', face));
        let mut rest = Cell::new(' ', face);
        rest.continuation = true;
        surface.set(1, 0, rest);
        surface.set(2, 0, Cell::new('x', Face::default()));
        let frame = build(&surface, &mut fonts, &Look::new(&palette()));
        let metrics = fonts.metrics();
        let blue: Vec<&Rect> = frame
            .rects
            .iter()
            .filter(|r| r.color == linear_rgb(0, 0, 255))
            .collect();
        assert_eq!(blue.len(), 1, "one background for the character");
        assert_eq!(
            blue[0].size[0],
            metrics.width * 2.0,
            "the background covers one cell rather than both"
        );
    }

    #[test]
    fn a_block_is_drawn_as_a_rectangle_that_fills_its_cell() {
        let Some(mut fonts) = fonts() else {
            eprintln!("skipping: no monospace font is installed");
            return;
        };
        let surface = surface_of("█░", Face::default());
        let palette = palette();
        let frame = build(&surface, &mut fonts, &Look::new(&palette));
        let metrics = fonts.metrics();
        assert!(frame.sprites.is_empty(), "a block came from the font");
        // Two backgrounds, then the block and the shade.
        assert_eq!(frame.rects.len(), 4, "{:?}", frame.rects);
        let block = frame.rects[1];
        assert_eq!(block.position, [0.0, 0.0]);
        assert_eq!(block.size, [metrics.width, metrics.height]);
        assert_eq!(block.color, palette.foreground);
        let shade = frame.rects[3];
        assert_eq!(shade.position, [metrics.width, 0.0]);
        assert_eq!(shade.color[3], 0.25, "a light shade is a quarter solid");
    }

    #[test]
    fn a_divider_runs_down_the_edge_of_a_window_with_another_beside_it() {
        let Some(mut fonts) = fonts() else {
            eprintln!("skipping: no monospace font is installed");
            return;
        };
        let surface = surface_of("ab", Face::default());
        let palette = palette();
        let dividers = [maxgus_tui::Rect::new(0, 0, 1, 1)];
        let look = Look {
            dividers: &dividers,
            ..Look::new(&palette)
        };
        let frame = build(&surface, &mut fonts, &look);
        let metrics = fonts.metrics();
        let lines: Vec<&Rect> = frame
            .rects
            .iter()
            .filter(|r| r.color == palette.divider)
            .collect();
        assert_eq!(lines.len(), 1, "one divider for the one seam");
        let line = lines[0];
        assert_eq!(
            line.position[0] + line.size[0],
            metrics.width,
            "at the seam"
        );
        assert_eq!(line.size[1], metrics.height, "the height of the row");
        assert!(line.size[0] < metrics.width / 2.0, "thin");

        // A popup over the seam covers the divider.
        let floating = [maxgus_tui::Rect::new(0, 0, 2, 1)];
        let look = Look {
            floating: &floating,
            ..look
        };
        let frame = build(&surface, &mut fonts, &look);
        assert!(
            !frame.rects.iter().any(|r| r.color == palette.divider),
            "the divider is drawn over the popup"
        );
    }
}
