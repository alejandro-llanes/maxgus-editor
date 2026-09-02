//! Box-drawing and block characters, drawn as geometry rather than glyphs.
//!
//! A font's `█` is a glyph like any other: it has side bearings, it sits on
//! the baseline, and the cell it is drawn into is the editor's size rather
//! than the font's. So a row of them shows a seam at every cell and a strip
//! of background along the top, and a terminal program that draws its
//! frames out of `─` and `│` gets frames with gaps at the corners. These
//! characters describe shapes, not letters, so they are drawn as the shapes
//! they describe: rectangles that fill exactly the cell they are given.

/// A rectangle inside a cell, in pixels from the cell's top left, and how
/// solid it is — the shades `░▒▓` are the foreground at a quarter, a half
/// and three quarters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Piece {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub alpha: f32,
}

/// Whether `ch` is drawn here rather than from the font.
pub fn is_drawn(ch: char) -> bool {
    pieces(ch, 8.0, 16.0).is_some()
}

/// The rectangles that make up `ch` in a cell `width` by `height` pixels,
/// or `None` for a character the font should draw.
pub fn pieces(ch: char, width: f32, height: f32) -> Option<Vec<Piece>> {
    match ch {
        '\u{2500}'..='\u{257F}' => lines(ch, width, height),
        '\u{2580}'..='\u{259F}' => Some(blocks(ch, width, height)),
        _ => None,
    }
}

fn solid(x: f32, y: f32, width: f32, height: f32) -> Piece {
    Piece {
        x,
        y,
        width,
        height,
        alpha: 1.0,
    }
}

/// The block elements: halves, eighths, quadrants and shades.
fn blocks(ch: char, w: f32, h: f32) -> Vec<Piece> {
    let eighth = |n: u32| n as f32 / 8.0;
    // Which quadrants a character fills: upper left, upper right, lower
    // left, lower right.
    let quadrants = |ul: bool, ur: bool, ll: bool, lr: bool| {
        let (hw, hh) = (w / 2.0, h / 2.0);
        [(ul, 0.0, 0.0), (ur, hw, 0.0), (ll, 0.0, hh), (lr, hw, hh)]
            .into_iter()
            .filter(|(on, _, _)| *on)
            .map(|(_, x, y)| solid(x, y, hw, hh))
            .collect()
    };
    match ch {
        '▀' => vec![solid(0.0, 0.0, w, h / 2.0)],
        // Lower one eighth up to the full block.
        '▁'..='█' => {
            let n = ch as u32 - '▁' as u32 + 1;
            vec![solid(0.0, h - h * eighth(n), w, h * eighth(n))]
        }
        // Left seven eighths down to one eighth.
        '▉'..='▏' => {
            let n = 7 - (ch as u32 - '▉' as u32);
            vec![solid(0.0, 0.0, w * eighth(n), h)]
        }
        '▐' => vec![solid(w / 2.0, 0.0, w / 2.0, h)],
        '░' | '▒' | '▓' => {
            let alpha = match ch {
                '░' => 0.25,
                '▒' => 0.5,
                _ => 0.75,
            };
            vec![Piece {
                x: 0.0,
                y: 0.0,
                width: w,
                height: h,
                alpha,
            }]
        }
        '▔' => vec![solid(0.0, 0.0, w, h * eighth(1))],
        '▕' => vec![solid(w - w * eighth(1), 0.0, w * eighth(1), h)],
        '▖' => quadrants(false, false, true, false),
        '▗' => quadrants(false, false, false, true),
        '▘' => quadrants(true, false, false, false),
        '▙' => quadrants(true, false, true, true),
        '▚' => quadrants(true, false, false, true),
        '▛' => quadrants(true, true, true, false),
        '▜' => quadrants(true, true, false, true),
        '▝' => quadrants(false, true, false, false),
        '▞' => quadrants(false, true, true, false),
        '▟' => quadrants(false, true, true, true),
        _ => Vec::new(),
    }
}

/// How an arm of a box-drawing character is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Weight {
    None,
    Light,
    Heavy,
    Double,
}

use Weight::{Double as D, Heavy as H, Light as L, None as N};

/// The four arms of a character, going left, up, right and down from the
/// centre of the cell.
#[derive(Debug, Clone, Copy)]
struct Arms {
    left: Weight,
    up: Weight,
    right: Weight,
    down: Weight,
}

impl Arms {
    fn new(left: Weight, up: Weight, right: Weight, down: Weight) -> Arms {
        Arms {
            left,
            up,
            right,
            down,
        }
    }
}

/// The light and heavy line characters, with an arm each way, in the order
/// the block lays them out: each group of sixteen is the combinations of
/// light and heavy for one shape.
fn arms(ch: char) -> Option<Arms> {
    let a = Arms::new;
    Some(match ch {
        '─' => a(L, N, L, N),
        '━' => a(H, N, H, N),
        '│' => a(N, L, N, L),
        '┃' => a(N, H, N, H),
        '┌' => a(N, N, L, L),
        '┍' => a(N, N, H, L),
        '┎' => a(N, N, L, H),
        '┏' => a(N, N, H, H),
        '┐' => a(L, N, N, L),
        '┑' => a(H, N, N, L),
        '┒' => a(L, N, N, H),
        '┓' => a(H, N, N, H),
        '└' => a(N, L, L, N),
        '┕' => a(N, L, H, N),
        '┖' => a(N, H, L, N),
        '┗' => a(N, H, H, N),
        '┘' => a(L, L, N, N),
        '┙' => a(H, L, N, N),
        '┚' => a(L, H, N, N),
        '┛' => a(H, H, N, N),
        '├' => a(N, L, L, L),
        '┝' => a(N, L, H, L),
        '┞' => a(N, H, L, L),
        '┟' => a(N, L, L, H),
        '┠' => a(N, H, L, H),
        '┡' => a(N, H, H, L),
        '┢' => a(N, L, H, H),
        '┣' => a(N, H, H, H),
        '┤' => a(L, L, N, L),
        '┥' => a(H, L, N, L),
        '┦' => a(L, H, N, L),
        '┧' => a(L, L, N, H),
        '┨' => a(L, H, N, H),
        '┩' => a(H, H, N, L),
        '┪' => a(H, L, N, H),
        '┫' => a(H, H, N, H),
        '┬' => a(L, N, L, L),
        '┭' => a(H, N, L, L),
        '┮' => a(L, N, H, L),
        '┯' => a(H, N, H, L),
        '┰' => a(L, N, L, H),
        '┱' => a(H, N, L, H),
        '┲' => a(L, N, H, H),
        '┳' => a(H, N, H, H),
        '┴' => a(L, L, L, N),
        '┵' => a(H, L, L, N),
        '┶' => a(L, L, H, N),
        '┷' => a(H, L, H, N),
        '┸' => a(L, H, L, N),
        '┹' => a(H, H, L, N),
        '┺' => a(L, H, H, N),
        '┻' => a(H, H, H, N),
        '┼' => a(L, L, L, L),
        '┽' => a(H, L, L, L),
        '┾' => a(L, L, H, L),
        '┿' => a(H, L, H, L),
        '╀' => a(L, H, L, L),
        '╁' => a(L, L, L, H),
        '╂' => a(L, H, L, H),
        '╃' => a(H, H, L, L),
        '╄' => a(L, H, H, L),
        '╅' => a(H, L, L, H),
        '╆' => a(L, L, H, H),
        '╇' => a(H, H, H, L),
        '╈' => a(H, L, H, H),
        '╉' => a(H, H, L, H),
        '╊' => a(L, H, H, H),
        '╋' => a(H, H, H, H),
        '═' => a(D, N, D, N),
        '║' => a(N, D, N, D),
        '╒' => a(N, N, D, L),
        '╓' => a(N, N, L, D),
        '╔' => a(N, N, D, D),
        '╕' => a(D, N, N, L),
        '╖' => a(L, N, N, D),
        '╗' => a(D, N, N, D),
        '╘' => a(N, L, D, N),
        '╙' => a(N, D, L, N),
        '╚' => a(N, D, D, N),
        '╛' => a(D, L, N, N),
        '╜' => a(L, D, N, N),
        '╝' => a(D, D, N, N),
        '╞' => a(N, L, D, L),
        '╟' => a(N, D, L, D),
        '╠' => a(N, D, D, D),
        '╡' => a(D, L, N, L),
        '╢' => a(L, D, N, D),
        '╣' => a(D, D, N, D),
        '╤' => a(D, N, D, L),
        '╥' => a(L, N, L, D),
        '╦' => a(D, N, D, D),
        '╧' => a(D, L, D, N),
        '╨' => a(L, D, L, N),
        '╩' => a(D, D, D, N),
        '╪' => a(D, L, D, L),
        '╫' => a(L, D, L, D),
        '╬' => a(D, D, D, D),
        // The arcs are corners; the difference is not one a rectangle can
        // draw, and a square corner beside the font's round one is closer
        // than a glyph that does not meet the lines either side of it.
        '╭' => a(N, N, L, L),
        '╮' => a(L, N, N, L),
        '╯' => a(L, L, N, N),
        '╰' => a(N, L, L, N),
        '╴' => a(L, N, N, N),
        '╵' => a(N, L, N, N),
        '╶' => a(N, N, L, N),
        '╷' => a(N, N, N, L),
        '╸' => a(H, N, N, N),
        '╹' => a(N, H, N, N),
        '╺' => a(N, N, H, N),
        '╻' => a(N, N, N, H),
        '╼' => a(L, N, H, N),
        '╽' => a(N, L, N, H),
        '╾' => a(H, N, L, N),
        '╿' => a(N, H, N, L),
        _ => return None,
    })
}

/// The line-drawing characters.
fn lines(ch: char, w: f32, h: f32) -> Option<Vec<Piece>> {
    // Strokes: a light line is a sixteenth of the cell's height and never
    // thinner than a pixel; a heavy one is twice that.
    let light = (h / 16.0).round().max(1.0);
    let heavy = light * 2.0;
    match ch {
        // Dashed: the line cut into two, three or four pieces.
        '┄' | '┅' | '┆' | '┇' | '┈' | '┉' | '┊' | '┋' | '╌' | '╍' | '╎' | '╏' =>
        {
            let (dashes, vertical, thick) = match ch {
                '┄' => (3, false, light),
                '┅' => (3, false, heavy),
                '┆' => (3, true, light),
                '┇' => (3, true, heavy),
                '┈' => (4, false, light),
                '┉' => (4, false, heavy),
                '┊' => (4, true, light),
                '┋' => (4, true, heavy),
                '╌' => (2, false, light),
                '╍' => (2, false, heavy),
                '╎' => (2, true, light),
                _ => (2, true, heavy),
            };
            let along = if vertical { h } else { w };
            let step = along / dashes as f32;
            let gap = (step / 3.0).max(1.0);
            Some(
                (0..dashes)
                    .map(|n| {
                        let start = n as f32 * step + gap / 2.0;
                        let length = step - gap;
                        match vertical {
                            true => solid((w - thick) / 2.0, start, thick, length),
                            false => solid(start, (h - thick) / 2.0, length, thick),
                        }
                    })
                    .collect(),
            )
        }
        // The diagonals are left to the font: they are not rectangles.
        '╱' | '╲' | '╳' => None,
        _ => arms(ch).map(|arms| strokes(arms, w, h, light, heavy)),
    }
}

/// Draws four arms meeting at the centre of the cell.
///
/// A single arm is one strip from its edge to just past the centre, so two
/// meeting at a corner overlap into a square there. A double arm is two
/// strips either side of the centre line, and those do not cross one
/// another: where two double arms meet, each strip stops at the first strip
/// of the other it runs into, which is what makes `╬` a cross with a hole
/// in the middle and `╔` a corner within a corner. A single arm meeting a
/// double one runs through to the further of its two strips, so `╧` is a
/// stem down to the lower line rather than one stopping in mid-air.
fn strokes(arms: Arms, w: f32, h: f32, light: f32, heavy: f32) -> Vec<Piece> {
    let (cx, cy) = (w / 2.0, h / 2.0);
    // How far apart the two strips of a double line are, centre to centre.
    let gap = (light * 2.0).max(2.0);
    let thickness = |weight: Weight| match weight {
        Weight::Heavy => heavy,
        _ => light,
    };
    let offsets = |weight: Weight| -> Vec<f32> {
        match weight {
            Weight::None => Vec::new(),
            Weight::Double => vec![-gap, gap],
            _ => vec![0.0],
        }
    };
    let furthest = |a: Weight, b: Weight| -> f32 {
        offsets(a)
            .into_iter()
            .chain(offsets(b))
            .map(f32::abs)
            .fold(0.0, f32::max)
    };
    let mut out = Vec::new();

    // One arm at a time. `along` is the axis the arm runs on, and `across`
    // the two arms either side of it, in the order the arm's negative and
    // positive offsets lean towards; `opposite` is the arm it runs into.
    let mut arm = |weight: Weight,
                   opposite: Weight,
                   across: (Weight, Weight),
                   vertical: bool,
                   from_start: bool| {
        if weight == N {
            return;
        }
        let t = thickness(weight);
        let (centre, extent) = match vertical {
            true => (cy, h),
            false => (cx, w),
        };
        for offset in offsets(weight) {
            // Where along its axis the strip stops, measured from the centre
            // towards the far side.
            let reach = match weight {
                D => {
                    // The arm on the side this strip leans towards.
                    let side = if offset < 0.0 { across.0 } else { across.1 };
                    let other = if offset < 0.0 { across.1 } else { across.0 };
                    if side == D {
                        // Stops at the first strip of the double arm it
                        // runs into.
                        -gap + t / 2.0
                    } else if offsets(opposite).contains(&offset) {
                        // Continues into the opposite arm.
                        0.0
                    } else {
                        // Turns the corner into whatever is on the other
                        // side, or ends at the centre.
                        furthest(other, N) + t / 2.0
                    }
                }
                _ => match opposite {
                    N => furthest(across.0, across.1) + t / 2.0,
                    _ => t / 2.0,
                },
            };
            let (start, length) = match from_start {
                true => (0.0, centre + reach),
                false => (centre - reach, extent - (centre - reach)),
            };
            let side = centre_of(vertical, cx, cy) + offset - t / 2.0;
            out.push(match vertical {
                true => solid(side, start, t, length),
                false => solid(start, side, length, t),
            });
        }
    };
    // Going left: the negative offset is the upper strip, which leans to
    // the up arm.
    arm(arms.left, arms.right, (arms.up, arms.down), false, true);
    arm(arms.right, arms.left, (arms.up, arms.down), false, false);
    arm(arms.up, arms.down, (arms.left, arms.right), true, true);
    arm(arms.down, arms.up, (arms.left, arms.right), true, false);
    out
}

/// The centre coordinate a strip is offset from: across the arm's axis.
fn centre_of(vertical: bool, cx: f32, cy: f32) -> f32 {
    match vertical {
        true => cx,
        false => cy,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn covers(pieces: &[Piece], x: f32, y: f32) -> bool {
        pieces
            .iter()
            .any(|p| x >= p.x && x < p.x + p.width && y >= p.y && y < p.y + p.height)
    }

    #[test]
    fn a_full_block_fills_the_whole_cell() {
        let pieces = pieces('█', 10.0, 20.0).unwrap();
        assert_eq!(pieces, vec![solid(0.0, 0.0, 10.0, 20.0)]);
    }

    #[test]
    fn the_eighths_and_quadrants_are_where_the_chart_puts_them() {
        assert_eq!(
            pieces('▄', 8.0, 16.0).unwrap(),
            vec![solid(0.0, 8.0, 8.0, 8.0)]
        );
        assert_eq!(
            pieces('▎', 8.0, 16.0).unwrap(),
            vec![solid(0.0, 0.0, 2.0, 16.0)]
        );
        let p = pieces('▚', 8.0, 16.0).unwrap();
        assert!(covers(&p, 1.0, 1.0) && covers(&p, 7.0, 15.0));
        assert!(!covers(&p, 7.0, 1.0) && !covers(&p, 1.0, 15.0));
    }

    #[test]
    fn the_shades_are_the_foreground_at_a_fraction() {
        let alphas: Vec<f32> = ['░', '▒', '▓']
            .into_iter()
            .map(|ch| pieces(ch, 8.0, 16.0).unwrap()[0].alpha)
            .collect();
        assert_eq!(alphas, vec![0.25, 0.5, 0.75]);
    }

    #[test]
    fn a_horizontal_line_runs_edge_to_edge_through_the_middle() {
        let p = pieces('─', 10.0, 16.0).unwrap();
        assert!(covers(&p, 0.0, 7.9) && covers(&p, 9.9, 7.9));
        assert!(
            !covers(&p, 5.0, 7.0) && !covers(&p, 5.0, 8.6),
            "a pixel thick"
        );
        let p = pieces('━', 10.0, 16.0).unwrap();
        assert!(
            covers(&p, 0.0, 7.0) && covers(&p, 9.9, 8.9),
            "two pixels thick"
        );
        assert!(!covers(&p, 5.0, 6.9) && !covers(&p, 5.0, 9.0));
    }

    #[test]
    fn a_corner_meets_itself() {
        // `┌`: right and down, with the corner square filled.
        let p = pieces('┌', 10.0, 16.0).unwrap();
        assert!(covers(&p, 9.9, 8.0), "the right arm reaches the edge");
        assert!(covers(&p, 5.0, 15.9), "the down arm reaches the edge");
        assert!(covers(&p, 4.6, 7.6), "the corner is filled");
        assert!(!covers(&p, 2.0, 8.0), "there is no left arm");
        assert!(!covers(&p, 5.0, 3.0), "there is no up arm");
    }

    #[test]
    fn a_double_cross_has_a_hole_in_the_middle() {
        let p = pieces('╬', 16.0, 16.0).unwrap();
        assert!(!covers(&p, 8.0, 8.0), "the centre is open");
        assert!(covers(&p, 1.0, 6.0) && covers(&p, 1.0, 10.0), "left arm");
        assert!(covers(&p, 6.0, 1.0) && covers(&p, 10.0, 1.0), "up arm");
        assert!(covers(&p, 15.0, 6.0) && covers(&p, 15.0, 10.0), "right arm");
        assert!(covers(&p, 6.0, 15.0) && covers(&p, 10.0, 15.0), "down arm");
    }

    #[test]
    fn a_double_corner_is_a_corner_within_a_corner() {
        // `╔`: the outer strips turn at the outer corner, the inner at the
        // inner one, and nothing pokes out beyond either.
        let p = pieces('╔', 16.0, 16.0).unwrap();
        assert!(covers(&p, 6.0, 6.0), "the outer corner is closed");
        assert!(covers(&p, 10.0, 10.0), "the inner corner is closed");
        assert!(!covers(&p, 8.0, 8.0), "the gap between them is open");
        assert!(!covers(&p, 3.0, 6.0), "nothing pokes out to the left");
        assert!(!covers(&p, 6.0, 3.0), "nothing pokes out at the top");
    }

    #[test]
    fn a_single_stem_reaches_the_further_of_two_double_lines() {
        // `╧`: up single, horizontal double.
        let p = pieces('╧', 16.0, 16.0).unwrap();
        assert!(covers(&p, 8.0, 1.0), "the stem starts at the top");
        assert!(covers(&p, 8.0, 9.5), "the stem crosses to the lower line");
        assert!(!covers(&p, 8.0, 13.0), "and stops there");
        assert!(
            covers(&p, 1.0, 6.0) && covers(&p, 15.0, 10.0),
            "both lines run through"
        );
    }

    #[test]
    fn dashes_leave_gaps_and_diagonals_are_left_to_the_font() {
        let p = pieces('┄', 12.0, 16.0).unwrap();
        assert_eq!(p.len(), 3);
        assert!(!covers(&p, 4.0, 8.0), "there is a gap between dashes");
        assert!(pieces('╱', 8.0, 16.0).is_none());
        assert!(pieces('a', 8.0, 16.0).is_none());
    }

    #[test]
    fn every_character_in_the_blocks_is_accounted_for() {
        for code in 0x2500..=0x259F {
            let ch = char::from_u32(code).unwrap();
            let drawn = is_drawn(ch);
            assert!(
                drawn || matches!(ch, '╱' | '╲' | '╳'),
                "{ch} (U+{code:04X}) is not drawn"
            );
        }
    }
}
