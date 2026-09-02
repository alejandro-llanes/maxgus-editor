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
//! nothing: a missing bold should look unemphasised, not invisible. It
//! falls back to the *family's* regular, not to some other family's bold:
//! a line half in one font and half in another is worse than one without
//! emphasis.
//!
//! No coding font has every character. What the family lacks — a Japanese
//! word, an arrow, a symbol from a Nerd Font — is drawn from whichever
//! installed font has it, found on first sight and remembered, so a file
//! with one line of Chinese in it is not a file with a line of boxes. An
//! emoji font keeps pictures rather than outlines, and those go into the
//! atlas as the pictures they are: it holds colour, and a glyph that is
//! only a shape is stored white and tinted by the face when drawn.
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
    /// True for a picture — an emoji from a bitmap font — which is drawn in
    /// its own colours rather than the face's.
    pub color: bool,
}

/// A glyph to draw: a font's own index, and which font it is an index into.
///
/// Not a character: a ligature is one glyph standing for several characters
/// and has no character of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GlyphRef {
    /// `0` is the family; `n` is the `n`th font standing in for what the
    /// family lacks, in the order they were found.
    pub font: u8,
    pub glyph: u16,
}

impl GlyphRef {
    /// An index into the family's own faces.
    pub fn own(glyph: u16) -> GlyphRef {
        GlyphRef { font: 0, glyph }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct Key {
    /// Which of the family's faces; a stand-in font has only the one.
    style: Style,
    glyph: GlyphRef,
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
    /// Bytes per pixel: red, green, blue, alpha.
    const RGBA: usize = 4;

    pub fn new(width: u32, height: u32) -> Atlas {
        Atlas {
            pixels: vec![0; (width * height) as usize * Atlas::RGBA],
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

    /// Four bytes per pixel, RGBA, which is what the shader samples. A
    /// glyph that is a shape is white with its coverage for alpha.
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn mark_uploaded(&mut self) {
        self.dirty = false;
    }

    /// Twice the size in each direction, with everything where it was.
    ///
    /// Glyphs are placed by absolute pixel, so nothing already in the atlas
    /// moves; the shelves keep filling from where they were, and the space
    /// to the right of the old ones is the price of not moving anything.
    fn grow(&mut self) {
        let (width, height) = (self.width * 2, self.height * 2);
        let mut pixels = vec![0; (width * height) as usize * Atlas::RGBA];
        let old_row = self.width as usize * Atlas::RGBA;
        for row in 0..self.height as usize {
            let from = row * old_row;
            let to = row * width as usize * Atlas::RGBA;
            pixels[to..to + old_row].copy_from_slice(&self.pixels[from..from + old_row]);
        }
        self.pixels = pixels;
        self.width = width;
        self.height = height;
        self.dirty = true;
    }

    /// Puts pixels in the atlas — four bytes each, RGBA — or returns `None`
    /// when they will not fit, at which point the caller is out of texture.
    fn insert(&mut self, width: u32, height: u32, rgba: &[u8]) -> Option<(u32, u32)> {
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
        let stride = width as usize * Atlas::RGBA;
        for row in 0..height {
            let from = row as usize * stride;
            let to = ((y + row) * self.width + x) as usize * Atlas::RGBA;
            self.pixels[to..to + stride].copy_from_slice(&rgba[from..from + stride]);
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
    /// The cell as the font's own metrics make it, before any line spacing.
    natural: CellMetrics,
    metrics: CellMetrics,
    atlas: Atlas,
    /// The atlas may double until it is this big, which is what the GPU
    /// said it would take. The downlevel guarantee until told otherwise.
    atlas_limit: u32,
    /// Every font the system has, kept for finding the ones the family
    /// cannot draw. Metadata only; a font is read when it is wanted.
    database: fontdb::Database,
    /// The fonts standing in for characters the family lacks, in the order
    /// they were needed. `GlyphRef::font` counts from one into this.
    stand_ins: Vec<StandIn>,
    /// Which stand-in has each character the family lacks, or that none
    /// does — so a character is searched for once, not once a frame.
    stand_in_for: HashMap<char, Option<u8>>,
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
        // The family is whichever of the asked-for one and the fallbacks
        // has a regular face; the other styles come from that family and
        // nowhere else, or a font with no bold would get another font's.
        let Some((chosen, regular, bytes)) = std::iter::once(family)
            .chain(FALLBACKS.iter().copied())
            .find_map(|name| load_face(&database, name, Style::Regular).map(|(f, b)| (name, f, b)))
        else {
            anyhow::bail!("no font found for `{family}` and none of the fallbacks are installed");
        };
        let mut faces = vec![(Style::Regular, regular)];
        let mut data = vec![(Style::Regular, std::sync::Arc::new(bytes))];
        for style in Style::ALL.into_iter().skip(1) {
            if let Some((font, bytes)) = load_face(&database, chosen, style) {
                faces.push((style, font));
                data.push((style, std::sync::Arc::new(bytes)));
            }
        }
        let regular = &faces[0].1;
        let line = regular.horizontal_line_metrics(size).ok_or_else(|| {
            anyhow::anyhow!("`{family}` has no horizontal metrics and cannot be laid out")
        })?;
        // The advance of a character every monospace font has, rather than the
        // font's own maximum: the maximum counts glyphs no editor will draw
        // and leaves visible gaps between columns. The widest of the styles,
        // since a bold face a hair wider than its regular would otherwise
        // run into the next cell.
        let width = faces
            .iter()
            .map(|(_, font)| font.metrics('M', size).advance_width)
            .fold(1.0_f32, f32::max);
        let metrics = CellMetrics {
            width: width.ceil(),
            height: (line.ascent - line.descent + line.line_gap).ceil().max(1.0),
            ascent: line.ascent.ceil(),
        };
        Ok(Fonts {
            faces,
            data,
            size,
            natural: metrics,
            metrics,
            atlas: Atlas::new(1024, 1024),
            atlas_limit: 2048,
            database,
            stand_ins: Vec::new(),
            stand_in_for: HashMap::new(),
            shaped: HashMap::new(),
        })
    }

    /// How big the atlas may get: the largest texture the GPU will take.
    pub fn set_atlas_limit(&mut self, limit: u32) {
        self.atlas_limit = limit.max(self.atlas.width).max(self.atlas.height);
    }

    pub fn metrics(&self) -> CellMetrics {
        self.metrics
    }

    /// Opens every line up by `extra` pixels over what the font asks for,
    /// half above the glyphs and half below, so the text stays centred in
    /// its taller cell. Zero is the font's own spacing. Meant for right
    /// after loading: a picture already fitted to the cell keeps the old
    /// cell's size.
    pub fn set_line_spacing(&mut self, extra: f32) {
        let extra = extra.max(0.0).round();
        self.metrics = CellMetrics {
            width: self.natural.width,
            height: self.natural.height + extra,
            ascent: self.natural.ascent + (extra / 2.0).floor(),
        };
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
        let index = self.index_of(character, style);
        self.glyph_indexed(index, style)
    }

    /// The glyph for a character, without rasterising it: the family's own
    /// when it has one, and otherwise a stand-in font's — or the family's
    /// `.notdef`, when no installed font has the character either.
    pub fn index_of(&mut self, character: char, style: Style) -> GlyphRef {
        let own = self.face(style).lookup_glyph_index(character);
        if own != 0 || character.is_whitespace() || character.is_control() {
            return GlyphRef::own(own);
        }
        match self.stand_in(character) {
            Some(n) => GlyphRef {
                font: n + 1,
                glyph: self.stand_ins[n as usize].glyph_index(character),
            },
            None => GlyphRef::own(0),
        }
    }

    /// Which stand-in font draws `character`, finding one if none of those
    /// already found does. `None` when no font on the system has it.
    fn stand_in(&mut self, character: char) -> Option<u8> {
        if let Some(known) = self.stand_in_for.get(&character) {
            return *known;
        }
        let found = self
            .stand_ins
            .iter()
            .position(|font| font.glyph_index(character) != 0)
            .map(|n| n as u8)
            .or_else(|| {
                // One byte's worth of fonts is more than any screen of text
                // has scripts; past it, characters are boxes.
                if self.stand_ins.len() >= u8::MAX as usize {
                    return None;
                }
                let font = find_stand_in(&self.database, character)?;
                self.stand_ins.push(font);
                Some((self.stand_ins.len() - 1) as u8)
            });
        self.stand_in_for.insert(character, found);
        found
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
    pub fn glyph_indexed(&mut self, index: GlyphRef, style: Style) -> Option<Glyph> {
        // A stand-in has one face, so the style is not part of what it is.
        let style = match index.font {
            0 => style,
            _ => Style::Regular,
        };
        let key = Key {
            style,
            glyph: index,
        };
        if let Some(known) = self.glyphs_get(key) {
            return known;
        }
        let (font, size, nudge) = match index.font {
            0 => (self.face(style), self.size, 0.0),
            n => match &self.stand_ins[n as usize - 1] {
                StandIn::Outline(font) => {
                    let (size, nudge) = self.fit(font, index.glyph);
                    (font, size, nudge)
                }
                StandIn::Bitmap { data, index: face } => {
                    let Some(picture) = picture(data, *face, index.glyph, self.metrics) else {
                        self.atlas.glyphs.insert(key, None);
                        return None;
                    };
                    let placed = self.place(picture.width, picture.height, &picture.rgba);
                    let glyph = placed.map(|(x, y)| Glyph {
                        x,
                        y,
                        width: picture.width,
                        height: picture.height,
                        left: picture.left,
                        top: picture.top,
                        color: true,
                    });
                    if glyph.is_some() {
                        self.atlas.glyphs.insert(key, glyph);
                    }
                    return glyph;
                }
            },
        };
        let (metrics, coverage) = font.rasterize_indexed(index.glyph, size);
        if metrics.width == 0 || metrics.height == 0 {
            self.atlas.glyphs.insert(key, None);
            return None;
        }
        let (width, height) = (metrics.width as u32, metrics.height as u32);
        let glyph = self
            .place(width, height, &white(&coverage))
            .map(|(x, y)| Glyph {
                x,
                y,
                width,
                height,
                left: metrics.xmin as f32 + nudge,
                top: -(metrics.ymin as f32) - metrics.height as f32,
                color: false,
            });
        if glyph.is_some() {
            self.atlas.glyphs.insert(key, glyph);
        }
        glyph
    }

    /// Puts pixels in the atlas, growing it as far as the GPU allows.
    ///
    /// A full atlas is not the end of the glyph: it doubles, until the GPU
    /// would refuse the texture. Past that a glyph that does not fit is not
    /// remembered as fitting nowhere, since room may yet be made.
    fn place(&mut self, width: u32, height: u32, rgba: &[u8]) -> Option<(u32, u32)> {
        let mut placed = self.atlas.insert(width, height, rgba);
        while placed.is_none() && self.atlas.width * 2 <= self.atlas_limit {
            self.atlas.grow();
            placed = self.atlas.insert(width, height, rgba);
        }
        placed
    }

    /// The size to cut a stand-in's glyph at, and how far right to put it.
    ///
    /// A stand-in was chosen for having the character, not for being the
    /// family's width, so its glyph is scaled down to the cell — or the two
    /// cells, when it is a wide one — and centred in it rather than let run
    /// into the neighbour.
    fn fit(&self, font: &fontdue::Font, glyph: u16) -> (f32, f32) {
        let advance = font.metrics_indexed(glyph, self.size).advance_width;
        let cells = match advance > self.metrics.width * 1.2 {
            true => 2.0,
            false => 1.0,
        };
        let room = self.metrics.width * cells;
        let scale = (room / advance.max(1.0)).min(1.0);
        let nudge = ((room - advance * scale) / 2.0).max(0.0).floor();
        (self.size * scale, nudge)
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

/// A shape as atlas pixels: white, with the coverage for alpha, so that
/// the face's colour is all that shows.
fn white(coverage: &[u8]) -> Vec<u8> {
    coverage
        .iter()
        .flat_map(|&alpha| [255, 255, 255, alpha])
        .collect()
}

/// A font standing in for characters the family lacks.
enum StandIn {
    /// Outlines, rasterised the way the family's own are.
    Outline(fontdue::Font),
    /// Pictures: a colour emoji font keeps a PNG per glyph per size rather
    /// than an outline, and the bytes are kept to be read when one is
    /// wanted. `index` is which face of a collection.
    Bitmap { data: Vec<u8>, index: u32 },
}

impl StandIn {
    /// The font's own index for `character`, or `0` when it has none.
    fn glyph_index(&self, character: char) -> u16 {
        match self {
            StandIn::Outline(font) => font.lookup_glyph_index(character),
            StandIn::Bitmap { data, index } => rustybuzz::ttf_parser::Face::parse(data, *index)
                .ok()
                .and_then(|face| face.glyph_index(character))
                .map(|glyph| glyph.0)
                .unwrap_or(0),
        }
    }
}

/// A picture of a glyph, fitted to the grid.
struct Picture {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
    left: f32,
    top: f32,
}

/// The picture a bitmap font keeps for a glyph, scaled to the cell.
///
/// Emoji are two cells wide wherever the editor counts widths, so the
/// picture is fitted into two cells by one and centred there. The size
/// asked for is the cell's height, and the font answers with the nearest
/// it has — the one size, usually, and larger than a cell.
fn picture(data: &[u8], index: u32, glyph: u16, cell: CellMetrics) -> Option<Picture> {
    let face = rustybuzz::ttf_parser::Face::parse(data, index).ok()?;
    let pixels_per_em = cell.height.round().clamp(1.0, u16::MAX as f32) as u16;
    let image = face.glyph_raster_image(rustybuzz::ttf_parser::GlyphId(glyph), pixels_per_em)?;
    if image.format != rustybuzz::ttf_parser::RasterImageFormat::PNG {
        return None;
    }
    fit_picture(image.data, cell)
}

/// Scales a PNG to fit two cells by one, and says where in them it goes.
fn fit_picture(png: &[u8], cell: CellMetrics) -> Option<Picture> {
    let decoded = image::load_from_memory_with_format(png, image::ImageFormat::Png).ok()?;
    let (room_width, room_height) = (cell.width * 2.0, cell.height);
    let scale = (room_width / decoded.width() as f32).min(room_height / decoded.height() as f32);
    let width = (decoded.width() as f32 * scale).round().max(1.0) as u32;
    let height = (decoded.height() as f32 * scale).round().max(1.0) as u32;
    let scaled = image::imageops::resize(
        &decoded.to_rgba8(),
        width,
        height,
        image::imageops::FilterType::Triangle,
    );
    Some(Picture {
        width,
        height,
        rgba: scaled.into_raw(),
        left: ((room_width - width as f32) / 2.0).floor(),
        // Relative to the baseline, as an outline's `top` is.
        top: ((room_height - height as f32) / 2.0).floor() - cell.ascent,
    })
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

/// The fonts asked first to stand in for a character the family lacks,
/// before every font on the system is asked in turn.
///
/// Monospace ones first, since they sit best in the grid; then the wide
/// ones that between them cover most of what a source file holds — icons,
/// the CJK scripts, the symbol blocks — and last the broad proportional
/// families that have a bit of everything.
const STAND_INS: &[&str] = &[
    "Symbols Nerd Font Mono",
    "Symbols Nerd Font",
    "Noto Sans Mono CJK JP",
    "Noto Sans Mono CJK SC",
    "Noto Sans Mono CJK TC",
    "Noto Sans Mono CJK KR",
    "Noto Sans Mono",
    "DejaVu Sans Mono",
    "Noto Sans CJK JP",
    "Noto Sans CJK SC",
    "Noto Sans Symbols",
    "Noto Sans Symbols 2",
    "Noto Sans Math",
    "Noto Color Emoji",
    "Apple Color Emoji",
    "Segoe UI Emoji",
    "Noto Emoji",
    "DejaVu Sans",
    "Noto Sans",
    "Symbola",
    "Unifont",
];

/// What to ask the database for, given a family name: the generic
/// `monospace` is fontconfig's word for "whatever is set up as the
/// monospace font", and is a family only to fontconfig.
fn family(name: &str) -> fontdb::Family<'_> {
    match name.eq_ignore_ascii_case("monospace") {
        true => fontdb::Family::Monospace,
        false => fontdb::Family::Name(name),
    }
}

/// Reads one face out of the database.
fn read_face(database: &fontdb::Database, id: fontdb::ID) -> Option<(fontdue::Font, Vec<u8>)> {
    database.with_face_data(id, |data, index| {
        let font = fontdue::Font::from_bytes(
            data,
            fontdue::FontSettings {
                collection_index: index,
                ..Default::default()
            },
        )
        .ok()?;
        Some((font, data.to_vec()))
    })?
}

/// The face for a style in one family, and the bytes it was built from.
///
/// The bytes come back too because the shaper reads the font's own tables
/// and `fontdue` keeps what it parsed to itself. Loading them twice would be
/// two copies of every font in the process.
///
/// One family only: which styles it has is the caller's question, and a
/// bold from another family is not an answer to it.
fn load_face(
    database: &fontdb::Database,
    name: &str,
    style: Style,
) -> Option<(fontdue::Font, Vec<u8>)> {
    let (weight, slant) = style.query();
    let query = fontdb::Query {
        families: &[family(name)],
        weight,
        stretch: fontdb::Stretch::Normal,
        style: slant,
    };
    let id = database.query(&query)?;
    // The database answers with its nearest match, which for a family with
    // no bold is its regular under another name. That is what the family's
    // regular is for already, and having it twice is having it drawn twice.
    let face = database.face(id)?;
    let same_weight = match weight == fontdb::Weight::BOLD {
        true => face.weight >= fontdb::Weight::SEMIBOLD,
        false => face.weight < fontdb::Weight::SEMIBOLD,
    };
    let same_slant = (face.style == fontdb::Style::Normal) == (slant == fontdb::Style::Normal);
    if !same_weight || !same_slant {
        return None;
    }
    read_face(database, id)
}

/// Whether a face can draw `character`: has an outline for it, or — for a
/// colour emoji font, which keeps pictures rather than outlines — a
/// picture. A face that merely maps the character to an empty glyph has
/// nothing to show for it.
fn draws(database: &fontdb::Database, id: fontdb::ID, character: char) -> bool {
    database
        .with_face_data(id, |data, index| {
            let Ok(face) = rustybuzz::ttf_parser::Face::parse(data, index) else {
                return false;
            };
            let Some(glyph) = face.glyph_index(character) else {
                return false;
            };
            face.glyph_bounding_box(glyph).is_some()
                || face.glyph_raster_image(glyph, u16::MAX).is_some()
        })
        .unwrap_or(false)
}

/// Reads a stand-in out of the database: as outlines when the face has
/// them, and as the pictures it keeps instead when it does not.
fn read_stand_in(database: &fontdb::Database, id: fontdb::ID, character: char) -> Option<StandIn> {
    database.with_face_data(id, |data, index| {
        let face = rustybuzz::ttf_parser::Face::parse(data, index).ok()?;
        let glyph = face.glyph_index(character)?;
        if face.glyph_bounding_box(glyph).is_none() {
            return Some(StandIn::Bitmap {
                data: data.to_vec(),
                index,
            });
        }
        let font = fontdue::Font::from_bytes(
            data,
            fontdue::FontSettings {
                collection_index: index,
                ..Default::default()
            },
        )
        .ok()?;
        Some(StandIn::Outline(font))
    })?
}

/// A font on the system that can draw `character`, if one can.
///
/// The preferred families are asked in order, and when none of them has it
/// every face in the database is, monospace ones first. Only regular faces
/// count: a stand-in bold would be one style of a font the family is not.
fn find_stand_in(database: &fontdb::Database, character: char) -> Option<StandIn> {
    let regular = |name: &str| {
        database.query(&fontdb::Query {
            families: &[family(name)],
            weight: fontdb::Weight::NORMAL,
            stretch: fontdb::Stretch::Normal,
            style: fontdb::Style::Normal,
        })
    };
    let preferred = STAND_INS.iter().filter_map(|name| regular(name));
    let is_plain = |face: &fontdb::FaceInfo| {
        face.style == fontdb::Style::Normal && face.weight == fontdb::Weight::NORMAL
    };
    let monospace = database
        .faces()
        .filter(|face| face.monospaced && is_plain(face))
        .map(|face| face.id);
    let rest = database
        .faces()
        .filter(|face| !face.monospaced && is_plain(face))
        .map(|face| face.id);
    preferred
        .chain(monospace)
        .chain(rest)
        .find(|id| draws(database, *id, character))
        .and_then(|id| read_stand_in(database, id, character))
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
    /// A shape of one coverage throughout, as the atlas takes it.
    fn block(width: u32, height: u32, value: u8) -> Vec<u8> {
        white(&vec![value; (width * height) as usize])
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
                let at = ((y + row) * atlas.width() + x + column) as usize * 4;
                assert_eq!(
                    &atlas.pixels()[at..at + 4],
                    [255, 255, 255, 200],
                    "at {column},{row}"
                );
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
    fn a_grown_atlas_keeps_every_glyph_where_it_was() {
        let mut atlas = Atlas::new(16, 16);
        let (x, y) = atlas.insert(4, 3, &block(4, 3, 200)).expect("it fits");
        atlas.mark_uploaded();
        atlas.grow();
        assert_eq!((atlas.width(), atlas.height()), (32, 32));
        assert!(atlas.is_dirty(), "a bigger texture has to be uploaded");
        for row in 0..3 {
            for column in 0..4 {
                let at = ((y + row) * atlas.width() + x + column) as usize * 4;
                assert_eq!(
                    &atlas.pixels()[at..at + 4],
                    [255, 255, 255, 200],
                    "at {column},{row}"
                );
            }
        }
        // And there is room now for what there was not before.
        assert!(atlas.insert(16, 16, &block(16, 16, 1)).is_some());
    }

    #[test]
    fn a_glyph_wider_than_the_atlas_never_fits() {
        let mut atlas = Atlas::new(16, 16);
        assert_eq!(atlas.insert(20, 4, &block(20, 4, 1)), None);
    }

    #[test]
    fn a_picture_is_fitted_to_two_cells_and_centred_in_them() {
        // A 128x128 emoji, as Noto's are, in cells of 10x20 with the
        // baseline at 16.
        let mut png = std::io::Cursor::new(Vec::new());
        image::RgbaImage::from_pixel(128, 128, image::Rgba([255, 0, 0, 255]))
            .write_to(&mut png, image::ImageFormat::Png)
            .expect("a png is written");
        let cell = CellMetrics {
            width: 10.0,
            height: 20.0,
            ascent: 16.0,
        };
        let picture = fit_picture(&png.into_inner(), cell).expect("a png is a picture");
        assert_eq!(
            (picture.width, picture.height),
            (20, 20),
            "the two cells are the limit"
        );
        assert_eq!(picture.left, 0.0);
        assert_eq!(
            picture.top, -16.0,
            "the top of the cell, relative to the baseline"
        );
        assert_eq!(picture.rgba.len(), 20 * 20 * 4);
        assert_eq!(
            &picture.rgba[..4],
            [255, 0, 0, 255],
            "the colours are the picture's own"
        );
    }

    #[test]
    fn a_picture_wider_than_tall_is_centred_vertically() {
        let mut png = std::io::Cursor::new(Vec::new());
        image::RgbaImage::from_pixel(40, 10, image::Rgba([0, 0, 255, 255]))
            .write_to(&mut png, image::ImageFormat::Png)
            .expect("a png is written");
        let cell = CellMetrics {
            width: 10.0,
            height: 20.0,
            ascent: 16.0,
        };
        let picture = fit_picture(&png.into_inner(), cell).expect("a png is a picture");
        assert_eq!((picture.width, picture.height), (20, 5));
        assert_eq!(picture.left, 0.0);
        assert_eq!(picture.top, 7.0 - 16.0);
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
    fn line_spacing_makes_the_cell_taller_and_keeps_the_text_centred() {
        let Some(mut fonts) = fonts() else {
            eprintln!("skipping: no monospace font is installed");
            return;
        };
        let natural = fonts.metrics();
        fonts.set_line_spacing(6.0);
        let spaced = fonts.metrics();
        assert_eq!(
            spaced.width, natural.width,
            "spacing is between lines, not columns"
        );
        assert_eq!(spaced.height, natural.height + 6.0);
        assert_eq!(spaced.ascent, natural.ascent + 3.0, "half above the glyphs");
        // The same cells fit across; fewer fit down.
        let (columns, rows) = natural.grid(800.0, 600.0);
        let (still, fewer) = spaced.grid(800.0, 600.0);
        assert_eq!(still, columns);
        assert!(fewer < rows);
        // Back to the font's own, and an odd number leaves the extra pixel below.
        fonts.set_line_spacing(0.0);
        assert_eq!(fonts.metrics(), natural);
        fonts.set_line_spacing(5.0);
        assert_eq!(fonts.metrics().ascent, natural.ascent + 2.0);
        assert_eq!(fonts.metrics().height, natural.height + 5.0);
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
    fn a_character_the_family_lacks_is_drawn_from_a_font_that_has_it() {
        let Some(mut fonts) = fonts() else {
            eprintln!("skipping: no monospace font is installed");
            return;
        };
        // Whatever monospace font was found is not a CJK font.
        let index = fonts.index_of('日', Style::Regular);
        if index == GlyphRef::own(0) {
            eprintln!("skipping: no installed font draws 日");
            return;
        }
        assert_ne!(index.font, 0, "the family claims a glyph it does not have");
        assert_ne!(index.glyph, 0, "the stand-in's .notdef is no better");
        assert_eq!(
            fonts.index_of('日', Style::Bold),
            index,
            "a stand-in has one face, and bold is not a different glyph"
        );
        let glyph = fonts
            .glyph_indexed(index, Style::Regular)
            .expect("it has ink");
        let cell = fonts.metrics().width;
        assert!(
            glyph.left >= 0.0 && glyph.left + glyph.width as f32 <= cell * 2.0 + 1.0,
            "a wide character overflows its two cells: {glyph:?} in {cell}"
        );
    }

    #[test]
    fn an_emoji_is_drawn_as_the_picture_its_font_keeps() {
        let Some(mut fonts) = fonts() else {
            eprintln!("skipping: no monospace font is installed");
            return;
        };
        let index = fonts.index_of('🚀', Style::Regular);
        if index == GlyphRef::own(0) {
            eprintln!("skipping: no installed font draws 🚀");
            return;
        }
        let glyph = fonts
            .glyph_indexed(index, Style::Regular)
            .expect("it has ink");
        let metrics = fonts.metrics();
        assert!(
            glyph.left >= 0.0 && glyph.left + glyph.width as f32 <= metrics.width * 2.0 + 1.0,
            "an emoji overflows its two cells: {glyph:?} in {metrics:?}"
        );
        assert!(
            glyph.height as f32 <= metrics.height,
            "an emoji taller than its line: {glyph:?} in {metrics:?}"
        );
        eprintln!(
            "🚀 is {}",
            if glyph.color {
                "a picture"
            } else {
                "an outline"
            }
        );
        if glyph.color {
            // The picture, not a white shape of it.
            let atlas = fonts.atlas();
            let coloured = (0..glyph.height).any(|row| {
                let at = ((glyph.y + row) * atlas.width() + glyph.x) as usize * 4;
                atlas.pixels()[at..at + glyph.width as usize * 4]
                    .chunks(4)
                    .any(|px| px[3] > 0 && (px[0] != px[1] || px[1] != px[2]))
            });
            assert!(coloured, "a colour emoji came out grey");
        }
    }

    #[test]
    fn a_full_atlas_grows_rather_than_dropping_glyphs() {
        let Some(mut fonts) = fonts() else {
            eprintln!("skipping: no monospace font is installed");
            return;
        };
        fonts.atlas = Atlas::new(4, 4);
        fonts.set_atlas_limit(64);
        let glyph = fonts.glyph('W', Style::Regular).expect("W has ink");
        assert!(fonts.atlas().width() > 4, "the atlas did not grow");
        assert!(glyph.width <= fonts.atlas().width());
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
