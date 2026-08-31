//! Fonts, and the atlas their glyphs are rasterised into.
//!
//! The editor draws a grid of cells, so what is needed of a font is narrow:
//! one advance width, one line height, and a coverage bitmap per character.
//! Glyphs are rasterised once, on first sight, and packed into a single
//! texture the renderer samples — one texture bind for the whole frame rather
//! than one per glyph.
//!
//! Four faces are kept, because the themes ask for bold and italic. A face
//! the system does not have falls back to the regular one rather than to
//! nothing: a missing bold should look unemphasised, not invisible.
//!
//! Glyphs are keyed by the font's own index rather than by character, which
//! is what makes ligatures possible: `!=` drawn as one glyph is a glyph no
//! character names. Which glyphs a run of text comes to is the shaper's
//! answer, not a lookup — see [`Fonts::shape`].

use std::collections::HashMap;

/// Which of the four faces a glyph is drawn in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Style {
    Regular,
    Bold,
    Italic,
    BoldItalic,
}

impl Style {
    pub fn of(bold: bool, italic: bool) -> Style {
        match (bold, italic) {
            (false, false) => Style::Regular,
            (true, false) => Style::Bold,
            (false, true) => Style::Italic,
            (true, true) => Style::BoldItalic,
        }
    }

    fn query(self) -> (fontdb::Weight, fontdb::Style) {
        let weight = match self {
            Style::Bold | Style::BoldItalic => fontdb::Weight::BOLD,
            _ => fontdb::Weight::NORMAL,
        };
        let slant = match self {
            Style::Italic | Style::BoldItalic => fontdb::Style::Italic,
            _ => fontdb::Style::Normal,
        };
        (weight, slant)
    }

    pub const ALL: [Style; 4] = [
        Style::Regular,
        Style::Bold,
        Style::Italic,
        Style::BoldItalic,
    ];
}

/// The size of one cell, in pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CellMetrics {
    pub width: f32,
    pub height: f32,
    /// Distance from the top of the cell to the baseline.
    pub ascent: f32,
}

impl CellMetrics {
    /// How many whole cells fit in a window of this size.
    ///
    /// At least one of each: a window too small for a cell still has to be
    /// given a grid, and an editor with no columns has nothing to draw into.
    pub fn grid(&self, width: f32, height: f32) -> (u16, u16) {
        let columns = (width / self.width).floor().max(1.0);
        let rows = (height / self.height).floor().max(1.0);
        (columns as u16, rows as u16)
    }
}

/// One rasterised glyph's place in the atlas.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Glyph {
    /// Where in the atlas texture, in pixels.
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    /// Where to put it relative to the cell's origin and baseline.
    pub left: f32,
    pub top: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct Key {
    style: Style,
    /// The font's own glyph index. Not a character: a ligature is one glyph
    /// standing for several characters and has no character of its own.
    glyph: u16,
}

/// One glyph the shaper produced, and which character of the run it came
/// from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shaped {
    /// The byte offset into the shaped text of the first character this
    /// glyph stands for. A ligature reports the offset of the first of the
    /// characters it swallowed, which is the cell it gets drawn in.
    pub cluster: usize,
    pub glyph: u16,
}

/// A texture of rasterised glyphs, packed in shelves.
///
/// Shelf packing rather than anything cleverer: glyphs from one font at one
/// size are all much the same height, which is the case shelves are good at,
/// and a perfect packing would save a texture that is already small.
pub struct Atlas {
    pixels: Vec<u8>,
    width: u32,
    height: u32,
    /// The shelf being filled: its top, its height, and how far along it is.
    shelf_y: u32,
    shelf_height: u32,
    pen_x: u32,
    glyphs: HashMap<Key, Option<Glyph>>,
    /// True when a glyph has been added since the texture was last uploaded.
    dirty: bool,
}

impl Atlas {
    pub fn new(width: u32, height: u32) -> Atlas {
        Atlas {
            pixels: vec![0; (width * height) as usize],
            width,
            height,
            shelf_y: 0,
            shelf_height: 0,
            pen_x: 0,
            glyphs: HashMap::new(),
            dirty: true,
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// One byte of coverage per pixel, which is what the shader samples.
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn mark_uploaded(&mut self) {
        self.dirty = false;
    }

    /// Puts a rasterised bitmap in the atlas, or returns `None` when it will
    /// not fit — at which point the caller is out of texture.
    fn insert(&mut self, width: u32, height: u32, coverage: &[u8]) -> Option<(u32, u32)> {
        if width > self.width {
            return None;
        }
        if self.pen_x + width > self.width {
            // Next shelf.
            self.shelf_y += self.shelf_height;
            self.shelf_height = 0;
            self.pen_x = 0;
        }
        if self.shelf_y + height > self.height {
            return None;
        }
        let (x, y) = (self.pen_x, self.shelf_y);
        for row in 0..height {
            let from = (row * width) as usize;
            let to = ((y + row) * self.width + x) as usize;
            self.pixels[to..to + width as usize]
                .copy_from_slice(&coverage[from..from + width as usize]);
        }
        self.pen_x += width;
        self.shelf_height = self.shelf_height.max(height);
        self.dirty = true;
        Some((x, y))
    }
}

/// The faces, their metrics, and the atlas they are rasterised into.
pub struct Fonts {
    faces: Vec<(Style, fontdue::Font)>,
    /// The bytes each face was built from, kept because the shaper needs the
    /// font's own tables and `fontdue` does not hand them back out.
    data: Vec<(Style, std::sync::Arc<Vec<u8>>)>,
    size: f32,
    metrics: CellMetrics,
    atlas: Atlas,
    /// What the shaper said about a run of text, so it is asked once rather
    /// than once a frame. A screen holds a few hundred distinct runs and
    /// redraws them sixty times a second.
    shaped: HashMap<(Style, String), std::sync::Arc<Vec<Shaped>>>,
}

impl Fonts {
    /// Loads `family` at `size` pixels, falling back through `family` then a
    /// list of monospace families the system is likely to have.
    pub fn load(family: &str, size: f32) -> anyhow::Result<Fonts> {
        let mut database = fontdb::Database::new();
        database.load_system_fonts();
        let mut faces = Vec::new();
        let mut data = Vec::new();
        for style in Style::ALL {
            if let Some((font, bytes)) = load_face(&database, family, style) {
                faces.push((style, font));
                data.push((style, std::sync::Arc::new(bytes)));
            }
        }
        if !faces.iter().any(|(style, _)| *style == Style::Regular) {
            anyhow::bail!("no font found for `{family}` and none of the fallbacks are installed");
        }
        let regular = &faces[0].1;
        let line = regular.horizontal_line_metrics(size).ok_or_else(|| {
            anyhow::anyhow!("`{family}` has no horizontal metrics and cannot be laid out")
        })?;
        // The advance of a character every monospace font has, rather than the
        // font's own maximum: the maximum counts glyphs no editor will draw
        // and leaves visible gaps between columns.
        let width = regular.metrics('M', size).advance_width.max(1.0);
        let metrics = CellMetrics {
            width: width.ceil(),
            height: (line.ascent - line.descent + line.line_gap).ceil().max(1.0),
            ascent: line.ascent.ceil(),
        };
        Ok(Fonts {
            faces,
            data,
            size,
            metrics,
            atlas: Atlas::new(1024, 1024),
            shaped: HashMap::new(),
        })
    }

    pub fn metrics(&self) -> CellMetrics {
        self.metrics
    }

    pub fn atlas(&self) -> &Atlas {
        &self.atlas
    }

    pub fn atlas_mut(&mut self) -> &mut Atlas {
        &mut self.atlas
    }

    /// The glyph for a character, rasterising it if this is the first sight
    /// of it. `None` for a character with nothing to draw, such as a space.
    pub fn glyph(&mut self, character: char, style: Style) -> Option<Glyph> {
        let index = self.face(style).lookup_glyph_index(character);
        self.glyph_indexed(index, style)
    }

    /// The font's own index for a character, without rasterising it.
    pub fn index_of(&self, character: char, style: Style) -> u16 {
        self.face(style).lookup_glyph_index(character)
    }

    /// What the shaper makes of `text` in this style: the glyphs to draw and
    /// which character of the text each one came from.
    ///
    /// This is the whole of ligatures. Asked for `!=`, a font made to join
    /// them answers with one glyph reporting itself as standing for the
    /// first character, and the second character gets no glyph of its own —
    /// so the pair is drawn as the single mark the font's designer drew,
    /// across the two cells the two characters occupy. A font not made to
    /// join them answers with two glyphs and nothing changes.
    ///
    /// Which is why this asks rather than deciding: the answer belongs to
    /// the font, and a font that has no opinion is not a font this has to
    /// know about.
    pub fn shape(&mut self, style: Style, text: &str) -> std::sync::Arc<Vec<Shaped>> {
        let key = (style, text.to_string());
        if let Some(known) = self.shaped.get(&key) {
            return known.clone();
        }
        // A screen's worth of distinct runs is small, but a session's worth
        // is not: every line ever scrolled past would be kept for a run that
        // may never come round again.
        if self.shaped.len() > 4096 {
            self.shaped.clear();
        }
        let shaped = std::sync::Arc::new(self.ask_the_shaper(style, text));
        self.shaped.insert(key, shaped.clone());
        shaped
    }

    fn ask_the_shaper(&self, style: Style, text: &str) -> Vec<Shaped> {
        let fallback = || {
            let font = self.face(style);
            text.char_indices()
                .map(|(cluster, ch)| Shaped {
                    cluster,
                    glyph: font.lookup_glyph_index(ch),
                })
                .collect::<Vec<_>>()
        };
        let Some(bytes) = self.bytes(style) else {
            return fallback();
        };
        let Some(face) = rustybuzz::Face::from_slice(bytes, 0) else {
            return fallback();
        };
        let mut buffer = rustybuzz::UnicodeBuffer::new();
        buffer.push_str(text);
        buffer.guess_segment_properties();
        let shaped = rustybuzz::shape(&face, &[], buffer);
        let glyphs: Vec<Shaped> = shaped
            .glyph_infos()
            .iter()
            .map(|info| Shaped {
                cluster: info.cluster as usize,
                glyph: info.glyph_id as u16,
            })
            .collect();
        // A shaper that came back with nothing to draw for text that has
        // something in it is a shaper that has failed, and drawing nothing
        // is worse than drawing it unjoined.
        match glyphs.is_empty() && !text.is_empty() {
            true => fallback(),
            false => glyphs,
        }
    }

    /// The bytes the face for `style` was built from, or the regular one's.
    fn bytes(&self, style: Style) -> Option<&[u8]> {
        self.data
            .iter()
            .find(|(had, _)| *had == style)
            .or_else(|| self.data.first())
            .map(|(_, bytes)| bytes.as_slice())
    }

    /// The glyph at a font's own index, rasterising it on first sight.
    ///
    /// `None` for a glyph with nothing to draw, such as a space — and for
    /// one the atlas has no room left for, which is the same thing as far as
    /// a frame is concerned.
    pub fn glyph_indexed(&mut self, index: u16, style: Style) -> Option<Glyph> {
        let key = Key {
            style,
            glyph: index,
        };
        if let Some(known) = self.glyphs_get(key) {
            return known;
        }
        let font = self.face(style);
        let (metrics, coverage) = font.rasterize_indexed(index, self.size);
        let glyph = if metrics.width == 0 || metrics.height == 0 {
            None
        } else {
            self.atlas
                .insert(metrics.width as u32, metrics.height as u32, &coverage)
                .map(|(x, y)| Glyph {
                    x,
                    y,
                    width: metrics.width as u32,
                    height: metrics.height as u32,
                    left: metrics.xmin as f32,
                    top: -(metrics.ymin as f32) - metrics.height as f32,
                })
        };
        self.atlas.glyphs.insert(key, glyph);
        glyph
    }

    fn glyphs_get(&self, key: Key) -> Option<Option<Glyph>> {
        self.atlas.glyphs.get(&key).copied()
    }

    /// The face for a style, or the regular one when it was not installed.
    fn face(&self, style: Style) -> &fontdue::Font {
        self.faces
            .iter()
            .find(|(had, _)| *had == style)
            .map(|(_, font)| font)
            .unwrap_or(&self.faces[0].1)
    }

    /// True when this build actually has the style, rather than falling back.
    pub fn has(&self, style: Style) -> bool {
        self.faces.iter().any(|(had, _)| *had == style)
    }
}

/// The families tried when the configured one is not installed.
///
/// Every one of them is monospace and common; a proportional fallback would
/// make a grid of cells look broken rather than merely different.
const FALLBACKS: &[&str] = &[
    "JetBrainsMono Nerd Font",
    "FiraCode Nerd Font",
    "DejaVu Sans Mono",
    "Liberation Mono",
    "Noto Sans Mono",
    "monospace",
];

/// The face for a style, and the bytes it was built from.
///
/// The bytes come back too because the shaper reads the font's own tables
/// and `fontdue` keeps what it parsed to itself. Loading them twice would be
/// two copies of every font in the process.
fn load_face(
    database: &fontdb::Database,
    family: &str,
    style: Style,
) -> Option<(fontdue::Font, Vec<u8>)> {
    let (weight, slant) = style.query();
    for name in std::iter::once(family).chain(FALLBACKS.iter().copied()) {
        let query = fontdb::Query {
            families: &[fontdb::Family::Name(name)],
            weight,
            stretch: fontdb::Stretch::Normal,
            style: slant,
        };
        let Some(id) = database.query(&query) else {
            continue;
        };
        let loaded = database.with_face_data(id, |data, index| {
            let font = fontdue::Font::from_bytes(
                data,
                fontdue::FontSettings {
                    collection_index: index,
                    ..Default::default()
                },
            )
            .ok()?;
            Some((font, data.to_vec()))
        });
        if let Some(Some(loaded)) = loaded {
            return Some(loaded);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_grid_is_the_cells_that_fit() {
        let metrics = CellMetrics {
            width: 10.0,
            height: 20.0,
            ascent: 15.0,
        };
        assert_eq!(metrics.grid(800.0, 600.0), (80, 30));
        // A partial cell is not a cell.
        assert_eq!(metrics.grid(805.0, 610.0), (80, 30));
    }

    #[test]
    fn a_window_too_small_for_a_cell_still_gets_a_grid() {
        let metrics = CellMetrics {
            width: 10.0,
            height: 20.0,
            ascent: 15.0,
        };
        assert_eq!(metrics.grid(4.0, 4.0), (1, 1));
        assert_eq!(metrics.grid(0.0, 0.0), (1, 1));
    }

    /// Coverage for a glyph of a single value, to tell them apart by eye.
    fn block(width: u32, height: u32, value: u8) -> Vec<u8> {
        vec![value; (width * height) as usize]
    }

    #[test]
    fn glyphs_are_packed_without_overlapping() {
        let mut atlas = Atlas::new(64, 64);
        let mut placed = Vec::new();
        for n in 0..20u32 {
            let (w, h) = (5 + n % 3, 8);
            let (x, y) = atlas.insert(w, h, &block(w, h, 255)).expect("it fits");
            placed.push((x, y, w, h));
        }
        for (i, a) in placed.iter().enumerate() {
            for b in &placed[i + 1..] {
                let apart =
                    a.0 + a.2 <= b.0 || b.0 + b.2 <= a.0 || a.1 + a.3 <= b.1 || b.1 + b.3 <= a.1;
                assert!(apart, "{a:?} and {b:?} overlap");
            }
        }
    }

    #[test]
    fn a_glyph_lands_where_the_atlas_says_it_did() {
        let mut atlas = Atlas::new(32, 32);
        let (x, y) = atlas.insert(4, 3, &block(4, 3, 200)).expect("it fits");
        for row in 0..3 {
            for column in 0..4 {
                let at = ((y + row) * atlas.width() + x + column) as usize;
                assert_eq!(atlas.pixels()[at], 200, "at {column},{row}");
            }
        }
    }

    #[test]
    fn a_full_atlas_says_so_rather_than_writing_out_of_bounds() {
        let mut atlas = Atlas::new(16, 16);
        let mut fitted = 0;
        for _ in 0..100 {
            if atlas.insert(8, 8, &block(8, 8, 1)).is_some() {
                fitted += 1;
            }
        }
        assert_eq!(fitted, 4, "a 16x16 atlas holds four 8x8 glyphs");
    }

    #[test]
    fn a_glyph_wider_than_the_atlas_never_fits() {
        let mut atlas = Atlas::new(16, 16);
        assert_eq!(atlas.insert(20, 4, &block(20, 4, 1)), None);
    }

    #[test]
    fn styles_cover_the_four_combinations() {
        assert_eq!(Style::of(false, false), Style::Regular);
        assert_eq!(Style::of(true, false), Style::Bold);
        assert_eq!(Style::of(false, true), Style::Italic);
        assert_eq!(Style::of(true, true), Style::BoldItalic);
    }
}

/// Tests that need the machine's own fonts.
///
/// A machine with no fonts at all cannot run these, so each says so and
/// passes rather than failing for a reason that is not about the code.
#[cfg(test)]
mod system_tests {
    use super::*;

    fn fonts() -> Option<Fonts> {
        Fonts::load("this-font-does-not-exist", 16.0).ok()
    }

    #[test]
    fn a_missing_family_falls_back_to_one_that_is_installed() {
        let Some(fonts) = fonts() else {
            eprintln!("skipping: no monospace font is installed");
            return;
        };
        let metrics = fonts.metrics();
        assert!(metrics.width > 0.0, "a cell with no width");
        assert!(
            metrics.height > metrics.width,
            "a cell wider than it is tall"
        );
        assert!(
            metrics.ascent > 0.0 && metrics.ascent < metrics.height,
            "the baseline is outside the cell: {metrics:?}"
        );
    }

    #[test]
    fn a_character_is_rasterised_once_and_remembered() {
        let Some(mut fonts) = fonts() else {
            eprintln!("skipping: no monospace font is installed");
            return;
        };
        let first = fonts.glyph('W', Style::Regular).expect("W has ink");
        let dirty_after_first = fonts.atlas().is_dirty();
        fonts.atlas_mut().mark_uploaded();
        let again = fonts.glyph('W', Style::Regular).expect("still there");
        assert_eq!(first, again, "it moved in the atlas");
        assert!(dirty_after_first, "the atlas did not need uploading");
        assert!(
            !fonts.atlas().is_dirty(),
            "a glyph already in the atlas asked for another upload"
        );
    }

    #[test]
    fn a_space_has_nothing_to_draw() {
        let Some(mut fonts) = fonts() else {
            eprintln!("skipping: no monospace font is installed");
            return;
        };
        assert_eq!(fonts.glyph(' ', Style::Regular), None);
    }

    #[test]
    fn every_style_answers_even_when_the_system_lacks_it() {
        let Some(mut fonts) = fonts() else {
            eprintln!("skipping: no monospace font is installed");
            return;
        };
        for style in Style::ALL {
            assert!(
                fonts.glyph('x', style).is_some(),
                "{style:?} produced no glyph, so it would draw nothing"
            );
        }
    }
}
