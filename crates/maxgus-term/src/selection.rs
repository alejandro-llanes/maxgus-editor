//! Selecting text on the terminal and taking a copy of it.
//!
//! Positions are *absolute*: line zero is the oldest line still in the
//! scrollback, not the top of the screen. Output arriving while a selection is
//! up would otherwise slide the selection down the screen with it, and marking
//! a line only to have it become a different line is the kind of thing that
//! gets the wrong command pasted somewhere it matters.

use crate::grid::Grid;

/// A line and column, counted from the start of the scrollback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Position {
    pub line: usize,
    pub column: usize,
}

impl Position {
    pub fn new(line: usize, column: usize) -> Position {
        Position { line, column }
    }
}

/// What a drag or a keypress is selecting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// From one character to another, following the text round line ends.
    #[default]
    Character,
    /// Whole lines.
    Line,
    /// A rectangle, for pulling one column out of tabular output.
    Block,
}

/// A selection in progress or finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    /// Where it started, which does not move as it is extended.
    pub anchor: Position,
    pub cursor: Position,
    pub mode: Mode,
}

impl Selection {
    pub fn new(at: Position, mode: Mode) -> Selection {
        Selection {
            anchor: at,
            cursor: at,
            mode,
        }
    }

    pub fn extend_to(&mut self, at: Position) {
        self.cursor = at;
    }

    /// The two ends in reading order, whichever way round they were made.
    pub fn ends(&self) -> (Position, Position) {
        if self.anchor <= self.cursor {
            (self.anchor, self.cursor)
        } else {
            (self.cursor, self.anchor)
        }
    }

    /// True when `line`/`column` is inside the selection, which is what the
    /// drawing asks once per cell.
    pub fn contains(&self, line: usize, column: usize) -> bool {
        let (start, end) = self.ends();
        match self.mode {
            Mode::Line => (start.line..=end.line).contains(&line),
            Mode::Block => {
                let (left, right) = (start.column.min(end.column), start.column.max(end.column));
                (start.line..=end.line).contains(&line) && (left..=right).contains(&column)
            }
            Mode::Character => {
                if line < start.line || line > end.line {
                    return false;
                }
                let from = if line == start.line { start.column } else { 0 };
                let to = if line == end.line {
                    end.column
                } else {
                    usize::MAX
                };
                (from..=to).contains(&column)
            }
        }
    }

    /// The selected text, ready for the kill ring.
    ///
    /// A line the terminal wrapped is joined to the next without a newline:
    /// it was one line when it was written, and pasting it back with a break
    /// in the middle would run half a command.
    pub fn text(&self, grid: &Grid) -> String {
        let (start, end) = self.ends();
        let lines: Vec<&crate::grid::Line> = grid.all_lines().collect();
        let mut out = String::new();

        for line_number in start.line..=end.line.min(lines.len().saturating_sub(1)) {
            let Some(line) = lines.get(line_number) else {
                break;
            };
            let text: Vec<char> = line
                .cells
                .iter()
                .map(|cell| {
                    if cell.wide_continuation {
                        '\0'
                    } else {
                        cell.ch
                    }
                })
                .collect();

            let (from, to) = match self.mode {
                Mode::Line => (0, text.len()),
                Mode::Block => {
                    let left = start.column.min(end.column);
                    let right = start.column.max(end.column);
                    (left.min(text.len()), (right + 1).min(text.len()))
                }
                Mode::Character => {
                    let from = if line_number == start.line {
                        start.column
                    } else {
                        0
                    };
                    let to = if line_number == end.line {
                        end.column + 1
                    } else {
                        text.len()
                    };
                    (from.min(text.len()), to.min(text.len()))
                }
            };
            let mut piece: String = text[from..to.max(from)]
                .iter()
                .filter(|c| **c != '\0')
                .collect();
            // Trailing blanks on a terminal line are padding, not text.
            if self.mode != Mode::Block {
                let trimmed = piece.trim_end_matches(' ').len();
                piece.truncate(trimmed);
            }
            out.push_str(&piece);

            let last = line_number == end.line;
            // A wrapped line continues below; a block selection is a column
            // and every row of it is its own line.
            if !last && (!line.wrapped || self.mode == Mode::Block) {
                out.push('\n');
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use maxgus_faces::Face;

    /// A grid with `text` typed into it, one line per element, and a width
    /// that forces the given wrapping.
    fn grid_of(columns: usize, lines: &[&str]) -> Grid {
        let mut grid = Grid::new(lines.len().max(1), columns, 10);
        for (row, text) in lines.iter().enumerate() {
            grid.move_to(row, 0);
            for ch in text.chars() {
                grid.put(ch, Face::default());
            }
        }
        grid
    }

    #[test]
    fn a_selection_reads_the_same_either_way_round() {
        let grid = grid_of(20, &["hello world", "second line"]);
        let forwards = Selection {
            anchor: Position::new(0, 0),
            cursor: Position::new(0, 4),
            mode: Mode::Character,
        };
        let backwards = Selection {
            anchor: Position::new(0, 4),
            cursor: Position::new(0, 0),
            mode: Mode::Character,
        };
        assert_eq!(forwards.text(&grid), "hello");
        assert_eq!(
            backwards.text(&grid),
            "hello",
            "dragging left should select the same text"
        );
    }

    #[test]
    fn a_selection_across_lines_takes_the_ends_of_each() {
        let grid = grid_of(20, &["hello world", "second line"]);
        let selection = Selection {
            anchor: Position::new(0, 6),
            cursor: Position::new(1, 5),
            mode: Mode::Character,
        };
        assert_eq!(selection.text(&grid), "world\nsecond");
    }

    #[test]
    fn a_wrapped_line_is_copied_back_as_one_line() {
        // It was one line when it was written. Pasting it with a break in the
        // middle would run half a command.
        let mut grid = Grid::new(2, 5, 10);
        for ch in "abcdefgh".chars() {
            grid.put(ch, Face::default());
        }
        assert!(grid.line(0).unwrap().wrapped);

        let selection = Selection {
            anchor: Position::new(0, 0),
            cursor: Position::new(1, 2),
            mode: Mode::Character,
        };
        assert_eq!(selection.text(&grid), "abcdefgh");
    }

    #[test]
    fn a_line_selection_takes_whole_lines_whatever_the_columns_say() {
        let grid = grid_of(20, &["hello world", "second line"]);
        let selection = Selection {
            anchor: Position::new(0, 7),
            cursor: Position::new(1, 2),
            mode: Mode::Line,
        };
        assert_eq!(selection.text(&grid), "hello world\nsecond line");
    }

    #[test]
    fn a_block_selection_pulls_one_column_out_of_a_table() {
        // Which is the whole reason for having one: copying the second field
        // of every row without the rest.
        let grid = grid_of(20, &["aaa bbb ccc", "ddd eee fff", "ggg hhh iii"]);
        let selection = Selection {
            anchor: Position::new(0, 4),
            cursor: Position::new(2, 6),
            mode: Mode::Block,
        };
        assert_eq!(selection.text(&grid), "bbb\neee\nhhh");
    }

    #[test]
    fn trailing_padding_is_not_part_of_the_text() {
        let grid = grid_of(20, &["short"]);
        let selection = Selection {
            anchor: Position::new(0, 0),
            cursor: Position::new(0, 19),
            mode: Mode::Character,
        };
        assert_eq!(
            selection.text(&grid),
            "short",
            "the blanks to the right came too"
        );
    }

    #[test]
    fn what_is_inside_the_selection_is_what_gets_marked() {
        let selection = Selection {
            anchor: Position::new(1, 3),
            cursor: Position::new(2, 5),
            mode: Mode::Character,
        };
        assert!(!selection.contains(1, 2), "before the start");
        assert!(selection.contains(1, 3));
        assert!(selection.contains(1, 99), "to the end of the first line");
        assert!(selection.contains(2, 0), "from the start of the last");
        assert!(!selection.contains(2, 6), "past the end");
        assert!(!selection.contains(3, 0), "a line below it entirely");

        let block = Selection {
            mode: Mode::Block,
            ..selection
        };
        assert!(!block.contains(1, 99), "a block stops at its right edge");
        assert!(block.contains(1, 4));
    }

    #[test]
    fn a_selection_holds_still_while_output_scrolls_under_it() {
        // Absolute positions are the whole point: a line marked in the
        // scrollback must stay the line that was marked.
        let mut grid = Grid::new(2, 20, 10);
        for ch in "first".chars() {
            grid.put(ch, Face::default());
        }
        grid.line_feed();
        grid.carriage_return();
        for ch in "second".chars() {
            grid.put(ch, Face::default());
        }
        let selection = Selection {
            anchor: Position::new(0, 0),
            cursor: Position::new(0, 4),
            mode: Mode::Character,
        };
        assert_eq!(selection.text(&grid), "first");

        // Two more lines push `first` into the scrollback, where it is still
        // absolute line zero.
        grid.line_feed();
        grid.carriage_return();
        for ch in "third".chars() {
            grid.put(ch, Face::default());
        }
        assert_eq!(
            selection.text(&grid),
            "first",
            "the selection followed the screen"
        );
    }
}
