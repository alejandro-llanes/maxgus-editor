//! The doc box as a card: what the language server said about the symbol
//! under point, set in prose beside the line it is on.
//!
//! The grid version — see the core's `draw_doc` — sizes and places a box
//! of cells. This does the same sums in pixels, for a card the grid could
//! not hold: rounded, translucent over the blur, its sentences in a reading
//! face and wrapped where the pixels run out. It follows the same rules
//! about where to go, so the box is where a hand used to the terminal
//! expects it: under the symbol's line when there is room, over it when
//! there is not, and against the window's right edge either way.

use crate::font::Fonts;
use crate::quads::{Palette, Panel, Rect, Sprite};
use maxgus_core::Editor;

/// A card and what is written on it, ready for the frame.
#[derive(Debug, Clone, PartialEq)]
pub struct Card {
    pub panel: Panel,
    /// The chips behind code and the rules across the text.
    pub over: Vec<Rect>,
    pub sprites: Vec<Sprite>,
    /// The cells the card covers, grown to whole cells: what has to be
    /// drawn into the backdrop for the blur behind the card to be of
    /// anything.
    pub cells: maxgus_tui::Rect,
}

/// How to draw it: the display's scale, and how solid the card is over
/// what is blurred behind it — or over nothing, when nothing is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Look {
    pub scale: f32,
    pub opacity: f32,
    pub blurring: bool,
}

/// Measures characters with the fonts themselves. Measuring rasterises,
/// which is no waste: every character measured is about to be drawn.
#[cfg(feature = "full")]
struct WithFonts<'a>(&'a mut Fonts);

#[cfg(feature = "full")]
impl crate::prose::Ruler for WithFonts<'_> {
    fn advance(&mut self, character: char, style: crate::font::Style, code: bool) -> f32 {
        match code {
            true => self.0.metrics().width,
            false => self.0.prose_glyph(character, style).1,
        }
    }
}

/// A build without a language server has nothing to put on a card.
#[cfg(not(feature = "full"))]
pub fn build(_: &Editor, _: &mut Fonts, _: &Palette, _: Look) -> Option<Card> {
    None
}

/// The card for the doc now showing, if there is one and it has a window
/// big enough to sit in.
#[cfg(feature = "full")]
pub fn build(editor: &Editor, fonts: &mut Fonts, palette: &Palette, look: Look) -> Option<Card> {
    let doc = editor.doc.as_ref()?;
    let window = editor.windows.get(doc.window)?;
    let area = maxgus_core::text_area(editor, doc.window)?;
    if area.width < 20 || area.height < 6 {
        return None;
    }
    let cell = fonts.metrics();
    let prose = fonts.prose_metrics();
    let (cw, ch) = (cell.width, cell.height);
    let lines = maxgus_core::markup::render(&doc.text, usize::MAX);

    // The card's proportions come from the text's: a line's worth of
    // padding, corners the size of a letter, a border a pixel thick on
    // the display it is on.
    let pad_x = (cw * 1.25).round();
    let pad_y = (prose.height * 0.5).round();
    let radius = (prose.height * 0.45).round();
    let border = look.scale.max(1.0).round();
    let margin = cw.round();
    let gap = (ch * 0.25).round();

    // Three fifths of the window at most, and never wider than the text
    // wants; at least twenty cells, so a one-word answer is still a card.
    let least = 20.0 * cw;
    let most = (area.width as f32 * 3.0 / 5.0 * cw)
        .max(least)
        .min(area.width as f32 * cw - margin);
    let mut ruler = WithFonts(fonts);
    let natural = crate::prose::natural_width(&lines, prose, cell, &mut ruler);
    let width = (natural + pad_x * 2.0).clamp(least, most.max(least));
    // Half the window: enough for a heading, a signature, the parameters
    // and a sentence, which is what a reply is.
    let tallest = (area.height as f32 / 2.0).max(3.0) * ch;
    let layout = crate::prose::lay_out(
        &lines,
        width - pad_x * 2.0,
        tallest - pad_y * 2.0,
        prose,
        cell,
        &mut ruler,
    );
    // Wrapping may have left the card wider than its text turned out.
    let width = (layout.width + pad_x * 2.0).clamp(least, width);
    let height = layout.height + pad_y * 2.0;

    // Under the symbol's row when there is room, over it when there is
    // not, and pinned to the top when there is no room either way.
    let row = (area.y as f32 + doc.line.saturating_sub(window.top_line) as f32) * ch;
    let (top, bottom) = (area.y as f32 * ch, (area.y + area.height) as f32 * ch);
    let below = row + ch + gap;
    let y = match below + height <= bottom {
        true => below,
        false => (row - gap - height).max(top),
    };
    let x = ((area.x + area.width) as f32 * cw - margin - width).max(area.x as f32 * cw);

    let theme = &editor.theme;
    let alpha = match look.blurring {
        true => look.opacity,
        false => 1.0,
    };
    let fill = {
        let [r, g, b, _] = palette.resolve(theme.resolve("doc").background, palette.background);
        [r, g, b, alpha]
    };
    let edge = {
        let face = theme.resolve("doc-border");
        let [r, g, b, _] = palette.resolve(face.foreground, palette.foreground);
        [r, g, b, 0.85]
    };
    let panel = Panel {
        position: [x, y],
        size: [width, height],
        shape: [radius, border],
        fill,
        border: edge,
    };

    let (left, above) = (x + pad_x, y + pad_y);
    let mut over = Vec::new();
    let chip = {
        let face = theme.resolve("doc-code");
        let [r, g, b, _] = palette.resolve(face.background, palette.background);
        [r, g, b, alpha.max(0.85)]
    };
    for band in &layout.chips {
        over.push(Rect {
            position: [left + band.x, above + band.y],
            size: [band.width.min(width - pad_x * 2.0 + 4.0), band.height],
            color: chip,
        });
    }
    for rule in &layout.rules {
        over.push(Rect {
            position: [left, above + rule],
            size: [width - pad_x * 2.0, border],
            color: [edge[0], edge[1], edge[2], 0.6],
        });
    }

    // Text on the card takes the doc face's colour where it has none of
    // its own, as it takes the panel's background in the grid.
    let panel_text = palette.resolve(theme.resolve("doc").foreground, palette.foreground);
    let mut sprites = Vec::new();
    for letter in &layout.letters {
        let glyph = match letter.code {
            true => fonts.glyph(letter.character, letter.style),
            false => fonts.prose_glyph(letter.character, letter.style).0,
        };
        let Some(glyph) = glyph else {
            continue;
        };
        let color = match letter.face {
            "default" => panel_text,
            face => palette.resolve(theme.resolve(face).foreground, panel_text),
        };
        let color = match glyph.color {
            true => [1.0, 1.0, 1.0, 1.0],
            false => color,
        };
        sprites.push(Sprite {
            position: [
                left + letter.x + glyph.left,
                above + letter.baseline + glyph.top,
            ],
            size: [glyph.width as f32, glyph.height as f32],
            source: [glyph.x as f32, glyph.y as f32],
            source_size: [glyph.width as f32, glyph.height as f32],
            color,
        });
    }

    let cells = maxgus_tui::Rect::new(
        (x / cw).floor() as u16,
        (y / ch).floor() as u16,
        ((x + width) / cw).ceil() as u16 - (x / cw).floor() as u16,
        ((y + height) / ch).ceil() as u16 - (y / ch).floor() as u16,
    );
    Some(Card {
        panel,
        over,
        sprites,
        cells,
    })
}

#[cfg(all(test, feature = "full"))]
mod tests {
    use super::*;

    /// An editor of eighty by twenty-four, with a reply about `add` on
    /// the fourth line, and the fonts to set it in, if any are installed.
    fn scene(text: &str, line: usize) -> Option<(Editor, Fonts, Palette)> {
        let fonts = match Fonts::load("this-font-does-not-exist", 16.0) {
            Ok(fonts) => fonts,
            Err(_) => {
                eprintln!("skipping: no monospace font is installed");
                return None;
            }
        };
        let theme = maxgus_faces::defaults::builtin("maxgus-dark").expect("the built-in theme");
        let palette = Palette::of(&theme);
        let settings = maxgus_config::Settings::default();
        let mut editor = Editor::new(settings, theme, maxgus_tui::Rect::new(0, 0, 80, 24));
        let body: String = (1..=60).map(|n| format!("line {n}\n")).collect();
        let id = editor.buffers.visit_file("/project/main.rs", &body);
        editor.switch_to_buffer(id).unwrap();
        editor.doc = Some(maxgus_core::Doc {
            text: text.into(),
            line,
            window: editor.windows.current_id(),
        });
        Some((editor, fonts, palette))
    }

    const REPLY: &str = "### `add`\n\n---\n```rust\nfn add(a: i32, b: i32) -> i32\n```\n\nAdds two numbers together.";

    fn look() -> Look {
        Look {
            scale: 1.0,
            opacity: 0.9,
            blurring: true,
        }
    }

    #[test]
    fn the_card_sits_under_the_symbols_line_against_the_right_edge() {
        let Some((editor, mut fonts, palette)) = scene(REPLY, 3) else {
            return;
        };
        let cell = fonts.metrics();
        let card = build(&editor, &mut fonts, &palette, look()).expect("a card");
        let area = maxgus_core::text_area(&editor, editor.windows.current_id()).unwrap();
        let [x, y] = card.panel.position;
        let [width, height] = card.panel.size;
        // Under the line the symbol is on, so it stays in view.
        assert!(y > (area.y + 3) as f32 * cell.height, "y {y}");
        assert!(y < (area.y + 6) as f32 * cell.height, "y {y}");
        // Against the right edge, a cell short of it.
        let right = (area.x + area.width) as f32 * cell.width;
        assert!(
            x + width <= right - cell.width * 0.5,
            "{x} + {width} > {right}"
        );
        assert!(x + width >= right - cell.width * 2.0);
        // Three fifths of the window at most, twenty cells at least.
        assert!(width >= 20.0 * cell.width);
        assert!(width <= area.width as f32 * 0.6 * cell.width + cell.width);
        // Tinted, in the theme's colours, with an edge that shows.
        assert_eq!(card.panel.fill[3], 0.9);
        assert!(card.panel.shape[0] > 0.0 && card.panel.shape[1] >= 1.0);
        // Every letter of it, the heading, the signature and the sentence;
        // the spaces between have nothing to draw.
        let letters = REPLY.chars().filter(|c| c.is_alphanumeric()).count();
        assert!(
            card.sprites.len() >= letters,
            "{} of {letters}",
            card.sprites.len()
        );
        for sprite in &card.sprites {
            let [sx, sy] = sprite.position;
            assert!(
                sx >= x && sx + sprite.size[0] <= x + width,
                "a letter past the edge"
            );
            assert!(
                sy >= y && sy + sprite.size[1] <= y + height,
                "a letter above or below"
            );
        }
        // A chip under the code and a rule under the heading, both inside.
        assert!(card.over.len() >= 2);
        // The cells the card covers, which is what the blur is asked for.
        let (cw, ch) = (cell.width, cell.height);
        assert!(
            card.cells.x as f32 * cw <= x
                && (card.cells.x + card.cells.width) as f32 * cw >= x + width
        );
        assert!(
            card.cells.y as f32 * ch <= y
                && (card.cells.y + card.cells.height) as f32 * ch >= y + height
        );
    }

    #[test]
    fn a_card_over_a_symbol_near_the_bottom_goes_above_it() {
        let Some((mut editor, mut fonts, palette)) = scene(REPLY, 21) else {
            return;
        };
        let cell = fonts.metrics();
        let area = maxgus_core::text_area(&editor, editor.windows.current_id()).unwrap();
        let row = (area.y + 21) as f32 * cell.height;
        let card = build(&editor, &mut fonts, &palette, look()).expect("a card");
        let [_, y] = card.panel.position;
        assert!(
            y + card.panel.size[1] <= row,
            "{y} + {} over the line at {row}",
            card.panel.size[1]
        );
        assert!(y >= area.y as f32 * cell.height);
        // Solid when nothing is blurred behind it.
        let plain = build(
            &editor,
            &mut fonts,
            &palette,
            Look {
                blurring: false,
                ..look()
            },
        )
        .unwrap();
        assert_eq!(plain.panel.fill[3], 1.0);
        // And no card at all for a window too small to hold one.
        editor.set_frame(maxgus_tui::Rect::new(0, 0, 18, 5));
        assert!(build(&editor, &mut fonts, &palette, look()).is_none());
    }

    #[test]
    fn a_long_reply_is_cut_to_half_the_window_and_says_so() {
        let text: String = (1..=40)
            .map(|n| format!("Paragraph {n} of the answer.\n\n"))
            .collect();
        let Some((editor, mut fonts, palette)) = scene(&text, 0) else {
            return;
        };
        let cell = fonts.metrics();
        let area = maxgus_core::text_area(&editor, editor.windows.current_id()).unwrap();
        let card = build(&editor, &mut fonts, &palette, look()).expect("a card");
        assert!(card.panel.size[1] <= area.height as f32 / 2.0 * cell.height + cell.height);
        assert!(card.panel.size[1] > 3.0 * cell.height);
        // The last letters set are the notice, in the shadow's colour.
        let last = card.sprites.last().unwrap();
        let shadow = palette.resolve(
            editor.theme.resolve("shadow").foreground,
            palette.foreground,
        );
        assert_eq!(last.color, shadow);
    }
}
