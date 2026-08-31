//! Where a long line breaks when the window wraps instead of truncating.
//!
//! With `truncate-lines` off, one line of the buffer can take several rows
//! of the screen, and everything that used to be able to say "row `n` shows
//! line `top_line + n`" has to ask instead. This is what it asks.
//!
//! The break falls where the edge falls, mid-word if that is where the edge
//! is, which is what `toggle-truncate-lines` does in Emacs. Wrapping at word
//! boundaries is a different mode there (`visual-line-mode`) and would be a
//! different setting here.
//!
//! Columns are counted from the start of the *line*, not of the row, so a
//! tab stops where it would have stopped unwrapped. Deciding tab stops from
//! the start of each screen row would make a tab jump about as a window is
//! resized, which is the sort of thing that makes wrapped text hard to read.

use maxgus_text::Buffer;

/// Where one screen row of a wrapped line begins: the character offset, and
/// its display column measured from the start of the line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RowStart {
    pub offset: usize,
    pub column: usize,
}

/// Every screen row `line` occupies, in order. Never empty: a line with
/// nothing on it still takes a row.
pub fn rows_of(buffer: &Buffer, line: usize, width: usize) -> Vec<RowStart> {
    let start = buffer.line_start(line);
    let mut rows = vec![RowStart {
        offset: start,
        column: 0,
    }];
    if width == 0 || line >= buffer.len_lines() {
        return rows;
    }
    let end = maxgus_text::Motion::line_end(buffer.rope(), start);
    // Where the row being filled began, so the test is against how much of
    // *this row* is used rather than how long the line is.
    let mut row_column = 0usize;
    let mut column = 0usize;
    let mut offset = start;
    while offset < end {
        let c = buffer.rope().char(offset);
        let advance = buffer.char_display_width(c, column);
        // `column > row_column` keeps a character wider than the whole
        // window on a row of its own instead of breaking before every one
        // of them for ever.
        if column + advance > row_column + width && column > row_column {
            rows.push(RowStart { offset, column });
            row_column = column;
        }
        column += advance;
        offset += 1;
    }
    rows
}

/// How many screen rows `line` takes. At least one.
pub fn row_count(buffer: &Buffer, line: usize, width: usize) -> usize {
    rows_of(buffer, line, width).len()
}

/// Which row of its own line `offset` falls on, and the column that row
/// starts at.
pub fn row_at(buffer: &Buffer, offset: usize, width: usize) -> (usize, usize) {
    let line = buffer.line_of(offset.min(buffer.len_chars()));
    let rows = rows_of(buffer, line, width);
    let index = rows
        .iter()
        .rposition(|row| row.offset <= offset)
        .unwrap_or(0);
    (index, rows[index].column)
}

/// A place on the screen: a line of the buffer and a row within it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Place {
    pub line: usize,
    pub row: usize,
}

impl Place {
    pub fn new(line: usize, row: usize) -> Place {
        Place { line, row }
    }
}

/// `n` screen rows further down, stopping at the last row of the buffer.
pub fn forward(buffer: &Buffer, from: Place, n: usize, width: usize) -> Place {
    let lines = buffer.len_lines();
    let mut at = from;
    for _ in 0..n {
        let rows = row_count(buffer, at.line, width);
        if at.row + 1 < rows {
            at.row += 1;
        } else if at.line + 1 < lines {
            at = Place::new(at.line + 1, 0);
        } else {
            break;
        }
    }
    at
}

/// `n` screen rows further up, stopping at the first row of the buffer.
pub fn backward(buffer: &Buffer, from: Place, n: usize, width: usize) -> Place {
    let mut at = from;
    for _ in 0..n {
        if at.row > 0 {
            at.row -= 1;
        } else if at.line > 0 {
            at.line -= 1;
            at.row = row_count(buffer, at.line, width) - 1;
        } else {
            break;
        }
    }
    at
}

/// How many rows apart two places are, `None` when they are further apart
/// than `most`.
///
/// Bounded on purpose. Every caller is asking a question about one window —
/// "is point on screen, and if not by how much" — and a walk that gives up
/// after a screenful answers it just as well as one that counts to the end
/// of a hundred-thousand-line buffer.
pub fn rows_between(
    buffer: &Buffer,
    from: Place,
    to: Place,
    width: usize,
    most: usize,
) -> Option<usize> {
    if from > to {
        return None;
    }
    let mut at = from;
    for n in 0..=most {
        if at == to {
            return Some(n);
        }
        let next = forward(buffer, at, 1, width);
        if next == at {
            return None;
        }
        at = next;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(text: &str) -> Buffer {
        let mut buffer = Buffer::new(maxgus_text::BufferId(1), "wrap");
        buffer.insert(0, text).expect("a buffer that takes text");
        buffer
    }

    #[test]
    fn a_line_that_fits_takes_one_row() {
        let buffer = text("short\nalso short\n");
        assert_eq!(row_count(&buffer, 0, 20), 1);
        assert_eq!(
            rows_of(&buffer, 0, 20),
            [RowStart {
                offset: 0,
                column: 0
            }]
        );
    }

    #[test]
    fn an_empty_line_still_takes_a_row() {
        // Otherwise a run of blank lines would collapse and every line after
        // it would be drawn in the wrong place.
        let buffer = text("\n\n");
        assert_eq!(row_count(&buffer, 0, 20), 1);
        assert_eq!(row_count(&buffer, 1, 20), 1);
    }

    #[test]
    fn a_long_line_breaks_where_the_edge_falls() {
        // Mid-word, which is what `toggle-truncate-lines` does in Emacs.
        // Wrapping at word boundaries is a different mode there.
        let buffer = text("abcdefghij\n");
        let rows = rows_of(&buffer, 0, 4);
        assert_eq!(
            rows.iter().map(|row| row.column).collect::<Vec<_>>(),
            [0, 4, 8],
            "ten characters at four to a row is three rows"
        );
        assert_eq!(
            rows.iter().map(|row| row.offset).collect::<Vec<_>>(),
            [0, 4, 8]
        );
    }

    #[test]
    fn a_line_exactly_as_wide_as_the_window_does_not_spill_onto_a_second_row() {
        // The off-by-one that puts a blank row under every full line.
        let buffer = text("abcd\n");
        assert_eq!(row_count(&buffer, 0, 4), 1);
        let buffer = text("abcde\n");
        assert_eq!(row_count(&buffer, 0, 4), 2);
    }

    #[test]
    fn a_tab_stops_where_it_would_have_stopped_unwrapped() {
        // Columns are counted from the start of the line, not of the row.
        // Tab stops decided per row would make a tab jump about as the
        // window is resized.
        let buffer = text("\tx\n");
        // Whatever the tab stop is, a window exactly that wide is filled by
        // the tab alone, and `x` starts the next row at the stop rather than
        // at nought.
        let tab = buffer.char_display_width('\t', 0);
        let rows = rows_of(&buffer, 0, tab);
        assert_eq!(rows.len(), 2, "the tab fills the row and `x` follows");
        assert_eq!(rows[1].column, tab);
    }

    #[test]
    fn a_character_wider_than_the_window_gets_a_row_to_itself() {
        // Rather than breaking before every one of them for ever.
        let buffer = text("\t\t\n");
        // Two columns of window and a tab stop wider than that.
        let rows = rows_of(&buffer, 0, 2);
        assert_eq!(rows.len(), 2, "one tab to a row, each overflowing it");
    }

    #[test]
    fn no_width_is_no_wrapping_rather_than_no_rows() {
        // A window squeezed to nothing still has to report a row per line,
        // or the drawing loop divides by it.
        let buffer = text("anything at all\n");
        assert_eq!(row_count(&buffer, 0, 0), 1);
    }

    #[test]
    fn which_row_an_offset_falls_on() {
        let buffer = text("abcdefghij\n");
        assert_eq!(row_at(&buffer, 0, 4), (0, 0));
        assert_eq!(row_at(&buffer, 3, 4), (0, 0));
        assert_eq!(row_at(&buffer, 4, 4), (1, 4));
        assert_eq!(row_at(&buffer, 9, 4), (2, 8));
    }

    #[test]
    fn walking_rows_crosses_lines_and_stops_at_the_ends() {
        let buffer = text("abcdefghij\nshort\n");
        // Line 0 is three rows at four columns, line 1 is one.
        assert_eq!(forward(&buffer, Place::new(0, 0), 1, 4), Place::new(0, 1));
        assert_eq!(forward(&buffer, Place::new(0, 2), 1, 4), Place::new(1, 0));
        assert_eq!(
            forward(&buffer, Place::new(0, 0), 100, 4),
            Place::new(2, 0),
            "it should stop at the last line rather than run past it"
        );
        assert_eq!(backward(&buffer, Place::new(1, 0), 1, 4), Place::new(0, 2));
        assert_eq!(
            backward(&buffer, Place::new(0, 0), 5, 4),
            Place::new(0, 0),
            "it should stop at the top rather than wrap"
        );
    }

    #[test]
    fn counting_rows_between_two_places_gives_up_rather_than_walking_a_buffer() {
        // Every caller is asking about one window, and a walk that gives up
        // after a screenful answers that just as well.
        let buffer = text("abcdefghij\nshort\nagain\n");
        assert_eq!(
            rows_between(&buffer, Place::new(0, 0), Place::new(1, 0), 4, 10),
            Some(3)
        );
        assert_eq!(
            rows_between(&buffer, Place::new(0, 0), Place::new(1, 0), 4, 2),
            None,
            "further than it was asked to look"
        );
        assert_eq!(
            rows_between(&buffer, Place::new(1, 0), Place::new(0, 0), 4, 10),
            None,
            "backwards is not a distance"
        );
    }
}
