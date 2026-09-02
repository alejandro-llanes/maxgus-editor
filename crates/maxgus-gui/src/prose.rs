//! Prose set to the pixel: what the language server said, in a reading
//! face rather than on the grid.
//!
//! The terminal draws the doc box into cells, and a window drew the same
//! cells for a while — a monospaced paragraph, word-wrapped at a column,
//! in a box of line-drawing characters. A window can do better than what a
//! terminal is stuck with: a proportional face for the sentences, code in
//! the code font where the markdown says it is code, lines broken where
//! the pixels run out rather than where the columns do, and a card with
//! corners instead of a box with `┌` at one of them.
//!
//! This is the layout, and only the layout: which character goes where, in
//! which face, over which band of colour. It is worked out against whatever
//! measures characters, so a test can hand it a ruler that says every
//! letter is six pixels wide and check where the words fell.

use crate::font::{CellMetrics, LineMetrics, Style};
use maxgus_core::markup::{Line, Span};

/// Measures characters: how far the pen moves past each.
pub trait Ruler {
    /// The advance of `character` in `style` — in the code face when
    /// `code`, in the prose face otherwise.
    fn advance(&mut self, character: char, style: Style, code: bool) -> f32;
}

/// One character, placed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Letter {
    pub character: char,
    /// The pen's position: the left of the advance, and the baseline,
    /// from the top left of the text, in pixels.
    pub x: f32,
    pub baseline: f32,
    pub style: Style,
    /// The face to colour it with, by name.
    pub face: &'static str,
    /// Set in the code font rather than the prose one.
    pub code: bool,
}

/// A band of the code face's background, behind a run of code.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Chip {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Where everything went.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Layout {
    pub letters: Vec<Letter>,
    pub chips: Vec<Chip>,
    /// The `y` of each rule, which is drawn a pixel high across the text.
    pub rules: Vec<f32>,
    /// The text's own width and height: what was used, not what was
    /// offered.
    pub width: f32,
    pub height: f32,
}

/// A character on a row, before the row has a `y`.
#[derive(Debug, Clone, Copy)]
struct Placed {
    x: f32,
    advance: f32,
    character: char,
    style: Style,
    face: &'static str,
    code: bool,
}

/// One row of the laid-out text.
#[derive(Debug, Default)]
struct Row {
    height: f32,
    ascent: f32,
    letters: Vec<Placed>,
    rule: bool,
}

impl Row {
    /// The pen's position after the last letter that is not a space: a
    /// row broken after a space does not count the space as width.
    fn end(&self) -> f32 {
        self.letters
            .iter()
            .rev()
            .find(|letter| !letter.character.is_whitespace())
            .map(|letter| letter.x + letter.advance)
            .unwrap_or(0.0)
    }

    /// The runs of code on the row: where each starts and how wide it is.
    fn chips(&self) -> Vec<(f32, f32)> {
        let mut chips: Vec<(f32, f32)> = Vec::new();
        let mut open: Option<(f32, f32)> = None;
        for letter in &self.letters {
            match (letter.code, open) {
                (true, Some((start, _))) => open = Some((start, letter.x + letter.advance)),
                (true, None) => open = Some((letter.x, letter.x + letter.advance)),
                (false, Some((start, end))) => {
                    chips.push((start, end - start));
                    open = None;
                }
                (false, None) => {}
            }
        }
        if let Some((start, end)) = open {
            chips.push((start, end - start));
        }
        chips
    }
}

/// How much a chip reaches past the code it is behind, so the letters do
/// not touch its edge.
const CHIP_REACH: f32 = 2.0;

/// The width `lines` would take unwrapped: what to size the box by, before
/// wrapping to what it turns out to be.
pub fn natural_width(
    lines: &[Line],
    prose: LineMetrics,
    cell: CellMetrics,
    ruler: &mut impl Ruler,
) -> f32 {
    lay_out(lines, f32::INFINITY, f32::INFINITY, prose, cell, ruler).width
}

/// Lays `lines` out no wider than `width`, and no taller than `height`.
///
/// What does not fit in the height is left out, and the last row that does
/// fit says how much was: a document that ends mid-sentence is the reader's
/// problem, and one that says "… 12 more lines" is not.
pub fn lay_out(
    lines: &[Line],
    width: f32,
    height: f32,
    prose: LineMetrics,
    cell: CellMetrics,
    ruler: &mut impl Ruler,
) -> Layout {
    let mut rows: Vec<Row> = Vec::new();
    for line in lines {
        match line {
            Line::Rule => rows.push(Row {
                height: (prose.height * 0.6).round().max(3.0),
                rule: true,
                ..Row::default()
            }),
            // A blank line is a gap between paragraphs, and half a line of
            // one is gap enough.
            Line::Text(spans) if spans.is_empty() => rows.push(Row {
                height: (prose.height * 0.5).round().max(1.0),
                ..Row::default()
            }),
            Line::Text(spans) => rows.extend(wrap(spans, width, prose, cell, ruler)),
        }
    }
    let mut layout = Layout::default();
    let mut y = 0.0;
    for row in fit(rows, height, prose, cell, ruler) {
        let baseline = y + row.ascent;
        for (x, chip_width) in row.chips() {
            layout.chips.push(Chip {
                x: (x - CHIP_REACH).max(0.0),
                y,
                width: chip_width + CHIP_REACH * 2.0,
                height: row.height,
            });
        }
        layout.width = layout.width.max(row.end());
        layout
            .letters
            .extend(row.letters.into_iter().map(|letter| Letter {
                character: letter.character,
                x: letter.x,
                baseline,
                style: letter.style,
                face: letter.face,
                code: letter.code,
            }));
        if row.rule {
            layout.rules.push(y + (row.height / 2.0).floor());
        }
        y += row.height;
    }
    layout.height = y;
    layout
}

/// Keeps the rows that fit in `height`, and when not all of them do,
/// spends the last on saying how many did not.
fn fit(
    mut rows: Vec<Row>,
    height: f32,
    prose: LineMetrics,
    cell: CellMetrics,
    ruler: &mut impl Ruler,
) -> Vec<Row> {
    let total = rows.len();
    let mut used = 0.0;
    let mut kept = 0;
    for row in &rows {
        if used + row.height > height {
            break;
        }
        used += row.height;
        kept += 1;
    }
    if kept == total {
        return rows;
    }
    rows.truncate(kept);
    // The notice takes a row of its own, and takes it from the end.
    while used + prose.height > height
        && let Some(row) = rows.pop()
    {
        used -= row.height;
    }
    let notice = Span {
        text: format!("… {} more lines", total - rows.len()),
        face: "shadow",
        bold: false,
        italic: false,
    };
    rows.extend(wrap(&[notice], f32::INFINITY, prose, cell, ruler));
    rows
}

/// Breaks one line of spans into rows no wider than `width`.
///
/// At spaces where it can, and within a word where it must: a word wider
/// than the whole box is cut, since the alternative is a word that is not
/// there. A line that is all code — a signature from a fenced block — is
/// not broken at its spaces either, but cut, as the terminal cuts it: a
/// signature broken at a space reads as two signatures.
fn wrap(
    spans: &[Span],
    width: f32,
    prose: LineMetrics,
    cell: CellMetrics,
    ruler: &mut impl Ruler,
) -> Vec<Row> {
    let is_code = |span: &Span| span.face == "doc-code";
    let all_code = spans.len() == 1 && is_code(&spans[0]);
    // A bullet's continuation rows line up with its text, not the bullet.
    let indent = match spans.first() {
        Some(first) if first.face == "font-lock-punctuation" && spans.len() > 1 => first
            .text
            .chars()
            .map(|c| ruler.advance(c, Style::of(first.bold, first.italic), false))
            .sum::<f32>(),
        _ => 0.0,
    };
    // A row of code stands on the code font's line, which may be the
    // taller; a sentence with a word of code in it stands on the prose's.
    let (row_height, row_ascent) = match all_code {
        true => (cell.height.max(prose.height), cell.ascent.max(prose.ascent)),
        false => (prose.height, prose.ascent),
    };
    let fresh = || Row {
        height: row_height,
        ascent: row_ascent,
        ..Row::default()
    };
    // Every character with what it needs, in order, so the breaking is
    // one pass over one list rather than a walk through the spans.
    let atoms: Vec<Placed> = spans
        .iter()
        .flat_map(|span| {
            let style = Style::of(span.bold, span.italic);
            let code = is_code(span);
            span.text
                .chars()
                .map(move |character| (character, style, span.face, code))
        })
        .map(|(character, style, face, code)| Placed {
            x: 0.0,
            advance: ruler.advance(character, style, code),
            character,
            style,
            face,
            code,
        })
        .collect();

    let mut rows: Vec<Row> = Vec::new();
    let mut row = fresh();
    let mut x = 0.0;
    let mut i = 0;
    while i < atoms.len() {
        let atom = atoms[i];
        let space = atom.character.is_whitespace();
        // A row does not begin with the space it broke on.
        if space && row.letters.is_empty() && !rows.is_empty() {
            i += 1;
            continue;
        }
        if x + atom.advance > width && !row.letters.is_empty() && !space {
            // Back to the last space, carrying what came after it to the
            // next row; or, with no space to go back to, cut here.
            let after_space = match all_code {
                true => None,
                false => row
                    .letters
                    .iter()
                    .rposition(|letter| letter.character.is_whitespace())
                    .map(|at| at + 1),
            };
            let carried = match after_space {
                Some(at) => row.letters.split_off(at),
                None => Vec::new(),
            };
            rows.push(std::mem::replace(&mut row, fresh()));
            x = indent;
            for mut letter in carried {
                letter.x = x;
                x += letter.advance;
                row.letters.push(letter);
            }
            continue;
        }
        row.letters.push(Placed { x, ..atom });
        x += atom.advance;
        i += 1;
    }
    if !row.letters.is_empty() || rows.is_empty() {
        rows.push(row);
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Six pixels a letter of prose, eight of code, whatever the letter.
    struct Even;

    impl Ruler for Even {
        fn advance(&mut self, _: char, _: Style, code: bool) -> f32 {
            match code {
                true => 8.0,
                false => 6.0,
            }
        }
    }

    const PROSE: LineMetrics = LineMetrics {
        height: 20.0,
        ascent: 15.0,
    };
    const CELL: CellMetrics = CellMetrics {
        width: 8.0,
        height: 24.0,
        ascent: 18.0,
    };

    fn text(spans: &[(&str, &'static str)]) -> Line {
        Line::Text(
            spans
                .iter()
                .map(|(text, face)| Span {
                    text: text.to_string(),
                    face,
                    bold: false,
                    italic: false,
                })
                .collect(),
        )
    }

    fn rows(layout: &Layout) -> Vec<String> {
        let mut baselines: Vec<f32> = layout.letters.iter().map(|l| l.baseline).collect();
        baselines.dedup();
        baselines
            .iter()
            .map(|b| {
                layout
                    .letters
                    .iter()
                    .filter(|l| l.baseline == *b)
                    .map(|l| l.character)
                    .collect()
            })
            .collect()
    }

    #[test]
    fn words_are_carried_to_the_next_row_where_the_pixels_run_out() {
        let line = text(&[("the quick brown fox", "default")]);
        // Room for sixteen letters: "the quick brown " is sixteen, and
        // "fox" does not fit after it.
        let layout = lay_out(&[line], 16.0 * 6.0, f32::INFINITY, PROSE, CELL, &mut Even);
        assert_eq!(rows(&layout), vec!["the quick brown ", "fox"]);
        assert_eq!(layout.height, 40.0);
        // The trailing space is not width; the longest row is fifteen.
        assert_eq!(layout.width, 15.0 * 6.0);
        // And the second row starts where the first did.
        let fox = layout.letters.iter().find(|l| l.character == 'f').unwrap();
        assert_eq!((fox.x, fox.baseline), (0.0, 35.0));
    }

    #[test]
    fn a_word_wider_than_the_box_is_cut_rather_than_lost() {
        let line = text(&[("abcdefghij", "default")]);
        let layout = lay_out(&[line], 4.0 * 6.0, f32::INFINITY, PROSE, CELL, &mut Even);
        assert_eq!(rows(&layout), vec!["abcd", "efgh", "ij"]);
    }

    #[test]
    fn code_is_set_in_the_code_font_over_a_chip_and_a_block_is_cut_not_wrapped() {
        let lines = [
            text(&[
                ("see ", "default"),
                ("add", "doc-code"),
                (" here", "default"),
            ]),
            text(&[("fn add(a: i32, b: i32) -> i32", "doc-code")]),
        ];
        let layout = lay_out(&lines, 20.0 * 8.0, f32::INFINITY, PROSE, CELL, &mut Even);
        let add: Vec<&Letter> = layout.letters.iter().filter(|l| l.code).take(3).collect();
        assert!(add.iter().all(|l| l.face == "doc-code"));
        // The chip sits behind the three letters of code, a reach either
        // side, on the prose row.
        assert_eq!(
            layout.chips[0],
            Chip {
                x: 4.0 * 6.0 - CHIP_REACH,
                y: 0.0,
                width: 3.0 * 8.0 + CHIP_REACH * 2.0,
                height: 20.0
            }
        );
        // The signature is twenty-nine characters in a row of twenty:
        // cut, at twenty, rather than broken at a space.
        let signature: Vec<String> = rows(&layout)[1..].to_vec();
        assert_eq!(signature, vec!["fn add(a: i32, b: i3", "2) -> i32"]);
        // On rows the code font's height, with a chip each the whole width.
        assert_eq!(layout.chips[1].height, 24.0);
        assert_eq!(layout.chips[1].width, 20.0 * 8.0 + CHIP_REACH * 2.0);
        assert_eq!(layout.height, 20.0 + 24.0 * 2.0);
    }

    #[test]
    fn a_bullets_continuation_lines_up_with_its_text() {
        let line = text(&[
            ("• ", "font-lock-punctuation"),
            ("one two three", "default"),
        ]);
        let layout = lay_out(&[line], 9.0 * 6.0, f32::INFINITY, PROSE, CELL, &mut Even);
        assert_eq!(rows(&layout), vec!["• one two ", "three"]);
        let three = layout
            .letters
            .iter()
            .find(|l| l.character == 't' && l.baseline > 15.0);
        assert_eq!(three.unwrap().x, 2.0 * 6.0);
    }

    #[test]
    fn rules_and_blank_lines_take_less_than_a_row_and_a_rule_knows_its_place() {
        let lines = [
            text(&[("a", "default")]),
            Line::Rule,
            Line::Text(Vec::new()),
            text(&[("b", "default")]),
        ];
        let layout = lay_out(&lines, f32::INFINITY, f32::INFINITY, PROSE, CELL, &mut Even);
        // 20 for the line, 12 for the rule, 10 for the gap, 20 for the line.
        assert_eq!(layout.height, 62.0);
        assert_eq!(layout.rules, vec![26.0]);
        let b = layout.letters.iter().find(|l| l.character == 'b').unwrap();
        assert_eq!(b.baseline, 42.0 + 15.0);
    }

    #[test]
    fn what_does_not_fit_gives_way_to_a_line_saying_how_much_there_was() {
        let lines: Vec<Line> = (1..=10)
            .map(|n| text(&[(&format!("line {n}"), "default")]))
            .collect();
        // Room for three rows and a bit.
        let layout = lay_out(&lines, f32::INFINITY, 65.0, PROSE, CELL, &mut Even);
        assert_eq!(
            rows(&layout),
            vec!["line 1", "line 2", "… 8 more lines"],
            "the third row is spent on the notice"
        );
        assert_eq!(layout.height, 60.0);
        let dots = layout.letters.iter().find(|l| l.character == '…').unwrap();
        assert_eq!(dots.face, "shadow");
    }

    #[test]
    fn the_natural_width_is_the_widest_row_unwrapped() {
        let lines = [
            text(&[("short", "default")]),
            text(&[("a rather longer line", "default")]),
        ];
        assert_eq!(natural_width(&lines, PROSE, CELL, &mut Even), 20.0 * 6.0);
    }
}
