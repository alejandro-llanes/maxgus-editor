//! The off-screen cell grid.

use crate::geometry::{Rect, Size};
use maxgus_faces::Face;
use unicode_width::UnicodeWidthChar;

/// One terminal cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub ch: char,
    pub face: Face,
    /// True for the second half of a double-width character. Nothing is
    /// written for these; the wide character to their left covers them.
    pub continuation: bool,
}

impl Default for Cell {
    fn default() -> Cell {
        Cell {
            ch: ' ',
            face: Face::default(),
            continuation: false,
        }
    }
}

impl Cell {
    pub fn new(ch: char, face: Face) -> Cell {
        Cell {
            ch,
            face,
            continuation: false,
        }
    }

    /// A blank cell in `face`, which is what clearing paints.
    pub fn blank(face: Face) -> Cell {
        Cell {
            ch: ' ',
            face,
            continuation: false,
        }
    }

    /// The width of this cell's character in terminal columns.
    pub fn width(&self) -> usize {
        if self.continuation {
            0
        } else {
            char_width(self.ch)
        }
    }
}

/// The number of columns `c` occupies. Control characters are rendered as a
/// caret escape and take two.
pub fn char_width(c: char) -> usize {
    match c {
        '\t' | '\n' | '\r' => 1,
        c if (c as u32) < 0x20 => 2,
        c => c.width().unwrap_or(0),
    }
}

/// A grid of cells, drawn into and then diffed against the previous frame.
#[derive(Debug, Clone, PartialEq)]
pub struct Surface {
    size: Size,
    cells: Vec<Cell>,
}

impl Surface {
    /// A surface of `size`, filled with blanks in the default face.
    pub fn new(size: Size) -> Surface {
        Surface {
            size,
            cells: vec![Cell::default(); size.area()],
        }
    }

    /// A surface filled with blanks in `face`.
    pub fn filled(size: Size, face: Face) -> Surface {
        Surface {
            size,
            cells: vec![Cell::blank(face); size.area()],
        }
    }

    pub fn size(&self) -> Size {
        self.size
    }

    pub fn width(&self) -> u16 {
        self.size.width
    }

    pub fn height(&self) -> u16 {
        self.size.height
    }

    pub fn area(&self) -> Rect {
        Rect::from_size(self.size)
    }

    fn index(&self, x: u16, y: u16) -> Option<usize> {
        (x < self.size.width && y < self.size.height)
            .then(|| y as usize * self.size.width as usize + x as usize)
    }

    pub fn get(&self, x: u16, y: u16) -> Option<&Cell> {
        self.index(x, y).map(|i| &self.cells[i])
    }

    /// Writes one cell. Out-of-bounds writes are dropped, so drawing code does
    /// not have to clip every coordinate itself.
    pub fn set(&mut self, x: u16, y: u16, cell: Cell) {
        if let Some(i) = self.index(x, y) {
            self.cells[i] = cell;
        }
    }

    /// Writes a character in `face`, returning how many columns it consumed.
    ///
    /// A double-width character also claims the cell to its right; when there
    /// is no room for the second half, a space is written instead so the line
    /// does not shift.
    pub fn set_char(&mut self, x: u16, y: u16, ch: char, face: Face) -> u16 {
        let width = char_width(ch);
        if width == 2 {
            if x + 1 >= self.size.width {
                self.set(x, y, Cell::blank(face));
                return 1;
            }
            self.set(x, y, Cell::new(ch, face));
            self.set(
                x + 1,
                y,
                Cell {
                    ch: ' ',
                    face,
                    continuation: true,
                },
            );
            return 2;
        }
        if width == 0 {
            // Combining marks and the like attach to the previous cell rather
            // than occupying one of their own.
            return 0;
        }
        self.set(x, y, Cell::new(ch, face));
        1
    }

    /// Writes `text` starting at `x`, clipped to `max_width` columns and to
    /// the surface edge. Returns the column just past what was written.
    pub fn set_string(&mut self, x: u16, y: u16, text: &str, face: Face, max_width: u16) -> u16 {
        let limit = x.saturating_add(max_width).min(self.size.width);
        let mut column = x;
        for ch in text.chars() {
            if column >= limit {
                break;
            }
            let width = char_width(ch) as u16;
            // A wide character that would straddle the limit is dropped.
            if width == 2 && column + 1 >= limit {
                break;
            }
            column += self.set_char(column, y, ch, face);
        }
        column
    }

    /// Fills `rect` with blanks in `face`.
    pub fn clear_rect(&mut self, rect: Rect, face: Face) {
        let Some(rect) = rect.intersect(&self.area()) else {
            return;
        };
        for (x, y) in rect.cells() {
            self.set(x, y, Cell::blank(face));
        }
    }

    /// Fills the whole surface with blanks in `face`.
    pub fn clear(&mut self, face: Face) {
        self.cells.fill(Cell::blank(face));
    }

    /// Takes a copy of another surface's cells.
    ///
    /// For a front end that has to keep what a frame looked like before
    /// something was drawn over it — which is what a blur behind a popup is
    /// a blur of. Resizes to match rather than refusing, so the caller does
    /// not have to keep the two in step itself.
    pub fn copy_from(&mut self, other: &Surface) {
        if self.size != other.size {
            self.resize(other.size);
        }
        self.cells.copy_from_slice(&other.cells);
    }

    /// Resizes, discarding the contents. Called on a terminal resize, where
    /// everything is redrawn anyway.
    pub fn resize(&mut self, size: Size) {
        if size == self.size {
            return;
        }
        self.size = size;
        self.cells = vec![Cell::default(); size.area()];
    }

    /// The text of one row, with continuation cells skipped. Used by tests and
    /// by `describe-screen-line`.
    pub fn row_text(&self, y: u16) -> String {
        let mut out = String::new();
        for x in 0..self.size.width {
            let Some(cell) = self.get(x, y) else { break };
            if !cell.continuation {
                out.push(cell.ch);
            }
        }
        out
    }

    /// Every row's text, for whole-screen assertions.
    pub fn to_lines(&self) -> Vec<String> {
        (0..self.size.height).map(|y| self.row_text(y)).collect()
    }

    /// Every cell, row-major. Used for whole-surface assertions.
    pub fn cells(&self) -> &[Cell] {
        &self.cells
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use maxgus_faces::Color;

    fn face() -> Face {
        Face::fg(Color::Indexed(1))
    }

    fn surface() -> Surface {
        Surface::new(Size::new(10, 3))
    }

    #[test]
    fn a_new_surface_is_blank() {
        let s = surface();
        assert_eq!(s.size(), Size::new(10, 3));
        assert_eq!(s.get(0, 0), Some(&Cell::default()));
        assert_eq!(s.row_text(0), "          ");
    }

    #[test]
    fn cells_can_be_written_and_read_back() {
        let mut s = surface();
        s.set(2, 1, Cell::new('x', face()));
        assert_eq!(s.get(2, 1).unwrap().ch, 'x');
        assert_eq!(s.get(2, 1).unwrap().face, face());
    }

    #[test]
    fn out_of_bounds_access_is_dropped_rather_than_panicking() {
        let mut s = surface();
        s.set(99, 99, Cell::new('x', face()));
        assert_eq!(s.get(99, 99), None);
        assert_eq!(s.get(10, 0), None, "column 10 is past the right edge");
        assert_eq!(s.get(0, 3), None, "row 3 is past the bottom");
    }

    #[test]
    fn strings_are_written_left_to_right() {
        let mut s = surface();
        let end = s.set_string(0, 0, "hello", face(), 10);
        assert_eq!(end, 5);
        assert_eq!(s.row_text(0), "hello     ");
    }

    #[test]
    fn strings_are_clipped_to_the_requested_width() {
        let mut s = surface();
        s.set_string(0, 0, "hello world", face(), 5);
        assert_eq!(s.row_text(0), "hello     ");
    }

    #[test]
    fn strings_are_clipped_to_the_surface_edge() {
        let mut s = surface();
        s.set_string(7, 0, "abcdef", face(), 100);
        assert_eq!(s.row_text(0), "       abc");
    }

    #[test]
    fn wide_characters_claim_two_cells() {
        let mut s = surface();
        let end = s.set_string(0, 0, "漢字", face(), 10);
        assert_eq!(end, 4);
        assert_eq!(s.get(0, 0).unwrap().ch, '漢');
        assert!(s.get(1, 0).unwrap().continuation);
        assert_eq!(s.get(2, 0).unwrap().ch, '字');
        assert!(s.get(3, 0).unwrap().continuation);
        // The rendered row skips continuations, so it reads naturally.
        assert_eq!(s.row_text(0), "漢字      ");
    }

    #[test]
    fn a_wide_character_that_would_straddle_the_edge_is_dropped() {
        let mut s = Surface::new(Size::new(3, 1));
        // `ab` fills columns 0 and 1, leaving only column 2 for a two-cell
        // character, so `漢` cannot be drawn.
        s.set_string(0, 0, "ab漢", face(), 3);
        assert_eq!(s.row_text(0), "ab ");

        // With room for both halves it is drawn.
        let mut s = Surface::new(Size::new(3, 1));
        s.set_string(0, 0, "a漢", face(), 3);
        assert_eq!(s.get(1, 0).unwrap().ch, '漢');
        assert!(s.get(2, 0).unwrap().continuation);
    }

    #[test]
    fn a_wide_character_written_at_the_last_column_becomes_a_space() {
        let mut s = Surface::new(Size::new(2, 1));
        let used = s.set_char(1, 0, '漢', face());
        assert_eq!(used, 1);
        assert_eq!(s.get(1, 0).unwrap().ch, ' ');
    }

    #[test]
    fn zero_width_characters_do_not_consume_a_cell() {
        let mut s = surface();
        // U+0301 is a combining acute accent.
        let used = s.set_char(0, 0, '\u{0301}', face());
        assert_eq!(used, 0);
        assert_eq!(char_width('\u{0301}'), 0);
    }

    #[test]
    fn control_characters_are_two_columns_wide() {
        assert_eq!(char_width('\u{0001}'), 2, "rendered as `^A`");
        assert_eq!(char_width('\t'), 1, "tabs are expanded before drawing");
    }

    #[test]
    fn clearing_a_rect_leaves_the_rest_alone() {
        let mut s = surface();
        s.set_string(0, 0, "abcdefghij", face(), 10);
        s.set_string(0, 1, "klmnopqrst", face(), 10);
        s.clear_rect(Rect::new(2, 0, 3, 1), Face::default());
        assert_eq!(s.row_text(0), "ab   fghij");
        assert_eq!(s.row_text(1), "klmnopqrst");
    }

    #[test]
    fn clearing_a_rect_outside_the_surface_is_harmless() {
        let mut s = surface();
        s.set_string(0, 0, "abcdefghij", face(), 10);
        s.clear_rect(Rect::new(50, 50, 5, 5), Face::default());
        assert_eq!(s.row_text(0), "abcdefghij");
    }

    #[test]
    fn clearing_paints_the_given_face_everywhere() {
        let mut s = surface();
        s.clear(face());
        assert!(s.cells().iter().all(|c| c.face == face() && c.ch == ' '));
    }

    #[test]
    fn resizing_discards_the_contents() {
        let mut s = surface();
        s.set_string(0, 0, "text", face(), 10);
        s.resize(Size::new(20, 5));
        assert_eq!(s.size(), Size::new(20, 5));
        assert_eq!(s.row_text(0), " ".repeat(20));
    }

    #[test]
    fn resizing_to_the_same_size_keeps_the_contents() {
        let mut s = surface();
        s.set_string(0, 0, "text", face(), 10);
        s.resize(Size::new(10, 3));
        assert_eq!(s.row_text(0), "text      ");
    }

    #[test]
    fn a_surface_renders_to_lines() {
        let mut s = Surface::new(Size::new(3, 2));
        s.set_string(0, 0, "abc", face(), 3);
        s.set_string(0, 1, "de", face(), 3);
        assert_eq!(s.to_lines(), vec!["abc", "de "]);
    }

    #[test]
    fn a_zero_sized_surface_is_usable_but_holds_nothing() {
        let mut s = Surface::new(Size::new(0, 0));
        s.set_string(0, 0, "ignored", face(), 10);
        assert!(s.to_lines().is_empty());
        assert_eq!(s.get(0, 0), None);
    }

    #[test]
    fn cell_width_accounts_for_continuations() {
        assert_eq!(Cell::new('a', face()).width(), 1);
        assert_eq!(Cell::new('漢', face()).width(), 2);
        assert_eq!(
            Cell {
                ch: ' ',
                face: face(),
                continuation: true
            }
            .width(),
            0
        );
    }
}
