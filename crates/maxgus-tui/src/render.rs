//! Frame diffing and output.
//!
//! Redisplay compares the new surface against the one on screen and emits only
//! what changed, batched into runs of adjacent cells that share a face. That
//! keeps a keystroke to a handful of bytes rather than a full repaint.

use crate::{Result, geometry::Size, surface::Surface};
use maxgus_faces::{ColorDepth, Face};
use std::io::Write;

/// A run of cells to write at one position in one face.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub x: u16,
    pub y: u16,
    pub face: Face,
    pub text: String,
}

impl Change {
    pub fn new(x: u16, y: u16, face: Face, text: impl Into<String>) -> Change {
        Change {
            x,
            y,
            face,
            text: text.into(),
        }
    }
}

/// Computes the changes that turn `previous` into `next`.
///
/// A size mismatch means the terminal was resized, in which case every cell of
/// `next` is emitted: there is no meaningful correspondence to diff against.
pub fn diff(previous: &Surface, next: &Surface) -> Vec<Change> {
    let full_repaint = previous.size() != next.size();
    let mut changes = Vec::new();

    for y in 0..next.height() {
        // The run currently being accumulated on this row.
        let mut run: Option<Change> = None;
        let mut x = 0u16;
        while x < next.width() {
            let Some(cell) = next.get(x, y) else { break };
            if cell.continuation {
                // Covered by the wide character to its left.
                x += 1;
                continue;
            }
            let changed = full_repaint || previous.get(x, y) != Some(cell);
            let width = cell.width().max(1) as u16;

            match (changed, run.as_mut()) {
                // Extend the current run when the face still matches.
                (true, Some(current)) if current.face == cell.face => current.text.push(cell.ch),
                (true, _) => {
                    if let Some(finished) = run.take() {
                        changes.push(finished);
                    }
                    run = Some(Change::new(x, y, cell.face, cell.ch.to_string()));
                }
                (false, _) => {
                    if let Some(finished) = run.take() {
                        changes.push(finished);
                    }
                }
            }
            x += width;
        }
        if let Some(finished) = run.take() {
            changes.push(finished);
        }
    }
    changes
}

/// Writes `changes` to `out`, degrading colours to what `depth` supports.
pub fn render_to<W: Write>(out: &mut W, changes: &[Change], depth: ColorDepth) -> Result<()> {
    use crossterm::{QueueableCommand, cursor::MoveTo, style::PrintStyledContent};

    for change in changes {
        out.queue(MoveTo(change.x, change.y))?;
        let style = change.face.to_style(depth);
        out.queue(PrintStyledContent(style.apply(change.text.as_str())))?;
    }
    out.flush()?;
    Ok(())
}

/// Applies `changes` to `surface`, which is how the renderer keeps its record
/// of what is on screen without re-drawing.
pub fn apply(surface: &mut Surface, changes: &[Change]) {
    for change in changes {
        let mut x = change.x;
        for ch in change.text.chars() {
            x += surface.set_char(x, change.y, ch, change.face);
        }
    }
}

/// Tracks what is currently on screen so successive frames can be diffed.
#[derive(Debug)]
pub struct Renderer {
    on_screen: Surface,
    depth: ColorDepth,
}

impl Renderer {
    pub fn new(size: Size, depth: ColorDepth) -> Renderer {
        Renderer {
            on_screen: Surface::new(size),
            depth,
        }
    }

    pub fn depth(&self) -> ColorDepth {
        self.depth
    }

    pub fn set_depth(&mut self, depth: ColorDepth) {
        self.depth = depth;
    }

    /// The surface currently displayed.
    pub fn on_screen(&self) -> &Surface {
        &self.on_screen
    }

    /// Diffs `next` against what is on screen, writes the difference, and
    /// records the new state. Returns the changes emitted.
    pub fn render<W: Write>(&mut self, out: &mut W, next: &Surface) -> Result<Vec<Change>> {
        let changes = diff(&self.on_screen, next);
        render_to(out, &changes, self.depth)?;
        self.on_screen = next.clone();
        Ok(changes)
    }

    /// Forgets what is on screen, so the next render repaints everything. Used
    /// after a resize or when another program has written to the terminal.
    pub fn invalidate(&mut self) {
        // A surface of a different size forces a full repaint on the next diff.
        self.on_screen = Surface::new(Size::new(0, 0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Size;
    use maxgus_faces::Color;

    fn red() -> Face {
        Face::fg(Color::Indexed(1))
    }

    fn blue() -> Face {
        Face::fg(Color::Indexed(4))
    }

    fn surface(lines: &[&str]) -> Surface {
        let width = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0) as u16;
        let mut s = Surface::new(Size::new(width, lines.len() as u16));
        for (y, line) in lines.iter().enumerate() {
            s.set_string(0, y as u16, line, Face::default(), width);
        }
        s
    }

    #[test]
    fn an_identical_frame_produces_no_output() {
        let a = surface(&["hello", "world"]);
        let b = a.clone();
        assert!(diff(&a, &b).is_empty());
    }

    #[test]
    fn only_changed_cells_are_emitted() {
        let a = surface(&["hello"]);
        let b = surface(&["hellX"]);
        let changes = diff(&a, &b);
        assert_eq!(changes, vec![Change::new(4, 0, Face::default(), "X")]);
    }

    #[test]
    fn adjacent_changes_batch_into_one_run() {
        let a = surface(&["abcdef"]);
        let b = surface(&["abXYZf"]);
        let changes = diff(&a, &b);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0], Change::new(2, 0, Face::default(), "XYZ"));
    }

    #[test]
    fn a_run_breaks_where_the_face_changes() {
        let a = surface(&["abcd"]);
        let mut b = a.clone();
        b.set_string(0, 0, "AB", red(), 4);
        b.set_string(2, 0, "CD", blue(), 4);
        let changes = diff(&a, &b);
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0], Change::new(0, 0, red(), "AB"));
        assert_eq!(changes[1], Change::new(2, 0, blue(), "CD"));
    }

    #[test]
    fn a_run_breaks_where_unchanged_cells_intervene() {
        let a = surface(&["abcdef"]);
        let b = surface(&["XbcdYf"]);
        let changes = diff(&a, &b);
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0], Change::new(0, 0, Face::default(), "X"));
        assert_eq!(changes[1], Change::new(4, 0, Face::default(), "Y"));
    }

    #[test]
    fn runs_do_not_span_rows() {
        let a = surface(&["ab", "cd"]);
        let b = surface(&["AB", "CD"]);
        let changes = diff(&a, &b);
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0].y, 0);
        assert_eq!(changes[1].y, 1);
    }

    #[test]
    fn a_face_only_change_is_still_a_change() {
        let a = surface(&["abc"]);
        let mut b = a.clone();
        b.set_string(0, 0, "abc", red(), 3);
        let changes = diff(&a, &b);
        assert_eq!(changes, vec![Change::new(0, 0, red(), "abc")]);
    }

    #[test]
    fn a_resize_forces_a_full_repaint() {
        let a = surface(&["ab"]);
        let b = surface(&["ab", "cd"]);
        let changes = diff(&a, &b);
        // Every row is emitted, even the one whose contents match.
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0], Change::new(0, 0, Face::default(), "ab"));
        assert_eq!(changes[1], Change::new(0, 1, Face::default(), "cd"));
    }

    #[test]
    fn continuation_cells_are_not_emitted_separately() {
        let a = Surface::new(Size::new(4, 1));
        let mut b = a.clone();
        b.set_string(0, 0, "漢字", Face::default(), 4);
        let changes = diff(&a, &b);
        assert_eq!(changes.len(), 1);
        assert_eq!(
            changes[0].text, "漢字",
            "the wide chars carry their own width"
        );
        assert_eq!(changes[0].x, 0);
    }

    #[test]
    fn applying_changes_reproduces_the_target_surface() {
        let a = surface(&["abcdef", "ghijkl"]);
        let mut b = a.clone();
        b.set_string(1, 0, "XY", red(), 4);
        b.set_string(3, 1, "Z", blue(), 1);

        let changes = diff(&a, &b);
        let mut reconstructed = a.clone();
        apply(&mut reconstructed, &changes);
        assert_eq!(reconstructed, b);
    }

    #[test]
    fn applying_a_wide_character_change_reproduces_the_continuation() {
        let a = Surface::new(Size::new(6, 1));
        let mut b = a.clone();
        b.set_string(0, 0, "a漢b", Face::default(), 6);
        let mut reconstructed = a.clone();
        apply(&mut reconstructed, &diff(&a, &b));
        assert_eq!(reconstructed, b);
    }

    #[test]
    fn the_renderer_writes_and_then_reports_no_further_changes() {
        let mut out = Vec::new();
        let mut renderer = Renderer::new(Size::new(5, 1), ColorDepth::Ansi16);
        let frame = surface(&["hello"]);

        let changes = renderer.render(&mut out, &frame).unwrap();
        assert_eq!(changes.len(), 1);
        assert!(!out.is_empty(), "something was written to the terminal");

        out.clear();
        let changes = renderer.render(&mut out, &frame).unwrap();
        assert!(changes.is_empty(), "an unchanged frame writes nothing");
        assert!(out.is_empty());
    }

    #[test]
    fn the_renderer_tracks_what_is_on_screen() {
        let mut out = Vec::new();
        let mut renderer = Renderer::new(Size::new(5, 1), ColorDepth::Ansi16);
        let frame = surface(&["hello"]);
        renderer.render(&mut out, &frame).unwrap();
        assert_eq!(renderer.on_screen(), &frame);
    }

    #[test]
    fn invalidating_forces_the_next_frame_to_repaint_in_full() {
        let mut out = Vec::new();
        let mut renderer = Renderer::new(Size::new(5, 1), ColorDepth::Ansi16);
        let frame = surface(&["hello"]);
        renderer.render(&mut out, &frame).unwrap();
        renderer.invalidate();
        let changes = renderer.render(&mut out, &frame).unwrap();
        assert_eq!(changes.len(), 1, "the whole row is repainted");
    }

    #[test]
    fn output_carries_cursor_moves_and_styling() {
        let mut out = Vec::new();
        let a = surface(&["ab"]);
        let mut b = a.clone();
        b.set_string(1, 0, "Z", red(), 1);
        render_to(&mut out, &diff(&a, &b), ColorDepth::Ansi16).unwrap();
        let text = String::from_utf8_lossy(&out);
        assert!(text.contains('\u{1b}'), "escape sequences were emitted");
        assert!(text.contains('Z'));
    }

    #[test]
    fn rendering_nothing_writes_nothing() {
        let mut out = Vec::new();
        render_to(&mut out, &[], ColorDepth::TrueColor).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn the_colour_depth_is_configurable() {
        let mut renderer = Renderer::new(Size::new(1, 1), ColorDepth::Ansi16);
        assert_eq!(renderer.depth(), ColorDepth::Ansi16);
        renderer.set_depth(ColorDepth::TrueColor);
        assert_eq!(renderer.depth(), ColorDepth::TrueColor);
    }

    #[test]
    fn a_diff_of_many_rows_emits_one_run_per_changed_row() {
        let a = Surface::new(Size::new(80, 24));
        let mut b = a.clone();
        for y in 0..24 {
            b.set_string(0, y, "x", red(), 80);
        }
        let changes = diff(&a, &b);
        assert_eq!(changes.len(), 24);
        assert!(changes.iter().all(|c| c.text == "x"));
    }
}
