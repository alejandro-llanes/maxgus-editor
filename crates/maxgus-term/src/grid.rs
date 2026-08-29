//! The screen a terminal program is drawing on.
//!
//! A grid of cells, a cursor, a scrolling region, and a scrollback of lines
//! that have moved off the top. The alternate screen is a second grid with no
//! scrollback of its own, which is what lets `vim` take the whole terminal and
//! give it back untouched.
//!
//! Nothing here parses anything: the grid is told what to do in terms of rows
//! and columns, which is what makes it testable without an escape sequence in
//! sight.

use maxgus_faces::{Color, Face};

/// One character cell.
#[derive(Debug, Clone, PartialEq)]
pub struct Cell {
    pub ch: char,
    pub face: Face,
    /// True for the right-hand half of a double-width character. Nothing is
    /// drawn for it; the character to its left covers both columns.
    pub wide_continuation: bool,
}

impl Default for Cell {
    fn default() -> Cell {
        Cell {
            ch: ' ',
            face: Face::default(),
            wide_continuation: false,
        }
    }
}

impl Cell {
    pub fn new(ch: char, face: Face) -> Cell {
        Cell {
            ch,
            face,
            wide_continuation: false,
        }
    }

    pub fn is_blank(&self) -> bool {
        self.ch == ' ' && self.face.background.is_none()
    }
}

/// One row, plus whether it ran off the end into the next.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Line {
    pub cells: Vec<Cell>,
    /// True when the line filled up and the text carried on below, which is
    /// what tells a copy where to leave the newline out.
    pub wrapped: bool,
}

impl Line {
    fn blank(columns: usize) -> Line {
        Line {
            cells: vec![Cell::default(); columns],
            wrapped: false,
        }
    }

    /// The text of the line with trailing blanks removed.
    pub fn text(&self) -> String {
        let mut text: String = self
            .cells
            .iter()
            .filter(|cell| !cell.wide_continuation)
            .map(|cell| cell.ch)
            .collect();
        let trimmed = text.trim_end_matches(' ').len();
        text.truncate(trimmed);
        text
    }
}

/// Where the cursor is, in rows and columns from the top left.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Cursor {
    pub row: usize,
    pub column: usize,
}

/// The screen, its scrollback and its cursor.
#[derive(Debug, Clone)]
pub struct Grid {
    rows: usize,
    columns: usize,
    lines: Vec<Line>,
    /// Lines that have scrolled off the top, oldest first.
    scrollback: std::collections::VecDeque<Line>,
    scrollback_limit: usize,
    pub cursor: Cursor,
    /// The rows the scrolling region covers, inclusive.
    pub region: (usize, usize),
    /// The next character written goes here rather than at `cursor.column`,
    /// because the last one filled the final column. Deferring the wrap is
    /// what stops a line that exactly fills the width from leaving a blank
    /// row behind it.
    pending_wrap: bool,
}

impl Grid {
    pub fn new(rows: usize, columns: usize, scrollback_limit: usize) -> Grid {
        let rows = rows.max(1);
        let columns = columns.max(1);
        Grid {
            rows,
            columns,
            lines: vec![Line::blank(columns); rows],
            scrollback: std::collections::VecDeque::new(),
            scrollback_limit,
            cursor: Cursor::default(),
            region: (0, rows - 1),
            pending_wrap: false,
        }
    }

    pub fn rows(&self) -> usize {
        self.rows
    }

    pub fn columns(&self) -> usize {
        self.columns
    }

    pub fn line(&self, row: usize) -> Option<&Line> {
        self.lines.get(row)
    }

    pub fn lines(&self) -> &[Line] {
        &self.lines
    }

    pub fn scrollback(&self) -> &std::collections::VecDeque<Line> {
        &self.scrollback
    }

    /// Every line, scrollback first, as one sequence — which is what a copy
    /// and a search read.
    pub fn all_lines(&self) -> impl Iterator<Item = &Line> {
        self.scrollback.iter().chain(self.lines.iter())
    }

    pub fn total_lines(&self) -> usize {
        self.scrollback.len() + self.rows
    }

    /// Puts a character at the cursor and moves it on.
    pub fn put(&mut self, ch: char, face: Face) {
        let width = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if width == 0 {
            // A combining mark belongs to the character before it.
            return;
        }
        if self.pending_wrap {
            self.wrap_line();
        }
        // A double-width character that will not fit wraps rather than being
        // split down the middle.
        if width == 2 && self.cursor.column + 1 >= self.columns {
            self.wrap_line();
        }
        let (row, column) = (self.cursor.row, self.cursor.column);
        if let Some(line) = self.lines.get_mut(row) {
            if let Some(cell) = line.cells.get_mut(column) {
                *cell = Cell::new(ch, face);
            }
            if width == 2
                && let Some(cell) = line.cells.get_mut(column + 1)
            {
                *cell = Cell {
                    ch: ' ',
                    face,
                    wide_continuation: true,
                };
            }
        }
        let next = column + width;
        if next >= self.columns {
            // Held back until the next character actually arrives.
            self.cursor.column = self.columns - 1;
            self.pending_wrap = true;
        } else {
            self.cursor.column = next;
        }
    }

    fn wrap_line(&mut self) {
        if let Some(line) = self.lines.get_mut(self.cursor.row) {
            line.wrapped = true;
        }
        self.cursor.column = 0;
        self.pending_wrap = false;
        self.line_feed();
    }

    /// Moves down a row, scrolling the region when already at its foot.
    pub fn line_feed(&mut self) {
        self.pending_wrap = false;
        if self.cursor.row == self.region.1 {
            self.scroll_up(1);
        } else if self.cursor.row + 1 < self.rows {
            self.cursor.row += 1;
        }
    }

    /// Moves up a row, scrolling the region when already at its head.
    pub fn reverse_line_feed(&mut self) {
        self.pending_wrap = false;
        if self.cursor.row == self.region.0 {
            self.scroll_down(1);
        } else if self.cursor.row > 0 {
            self.cursor.row -= 1;
        }
    }

    pub fn carriage_return(&mut self) {
        self.cursor.column = 0;
        self.pending_wrap = false;
    }

    /// Scrolls the region up by `amount`, which is what moves lines into the
    /// scrollback — but only when the region is the whole screen. A program
    /// scrolling a window inside the screen is not producing history.
    pub fn scroll_up(&mut self, amount: usize) {
        let (top, bottom) = self.region;
        let amount = amount.min(bottom - top + 1);
        let whole_screen = top == 0 && bottom == self.rows - 1;
        for _ in 0..amount {
            let line = self.lines.remove(top);
            if whole_screen && self.scrollback_limit > 0 {
                self.scrollback.push_back(line);
                while self.scrollback.len() > self.scrollback_limit {
                    self.scrollback.pop_front();
                }
            }
            self.lines.insert(bottom, Line::blank(self.columns));
        }
        self.pending_wrap = false;
    }

    pub fn scroll_down(&mut self, amount: usize) {
        let (top, bottom) = self.region;
        let amount = amount.min(bottom - top + 1);
        for _ in 0..amount {
            self.lines.remove(bottom);
            self.lines.insert(top, Line::blank(self.columns));
        }
        self.pending_wrap = false;
    }

    pub fn move_to(&mut self, row: usize, column: usize) {
        self.cursor.row = row.min(self.rows - 1);
        self.cursor.column = column.min(self.columns - 1);
        self.pending_wrap = false;
    }

    pub fn move_by(&mut self, rows: isize, columns: isize) {
        let row = (self.cursor.row as isize + rows).clamp(0, self.rows as isize - 1);
        let column = (self.cursor.column as isize + columns).clamp(0, self.columns as isize - 1);
        self.cursor = Cursor {
            row: row as usize,
            column: column as usize,
        };
        self.pending_wrap = false;
    }

    /// Clears cells in the current row from `from` to `to`, inclusive.
    pub fn erase_in_line(&mut self, from: usize, to: usize, face: Face) {
        let columns = self.columns;
        if let Some(line) = self.lines.get_mut(self.cursor.row) {
            for column in from..=to.min(columns - 1) {
                if let Some(cell) = line.cells.get_mut(column) {
                    *cell = Cell {
                        ch: ' ',
                        face,
                        wide_continuation: false,
                    };
                }
            }
            line.wrapped = false;
        }
        self.pending_wrap = false;
    }

    /// Clears whole rows, inclusive.
    pub fn erase_rows(&mut self, from: usize, to: usize, face: Face) {
        for row in from..=to.min(self.rows - 1) {
            if let Some(line) = self.lines.get_mut(row) {
                line.cells = vec![
                    Cell {
                        ch: ' ',
                        face,
                        wide_continuation: false
                    };
                    self.columns
                ];
                line.wrapped = false;
            }
        }
        self.pending_wrap = false;
    }

    /// Inserts blank lines at the cursor, pushing the rest of the region down.
    pub fn insert_lines(&mut self, amount: usize) {
        let (top, bottom) = self.region;
        if self.cursor.row < top || self.cursor.row > bottom {
            return;
        }
        for _ in 0..amount.min(bottom - self.cursor.row + 1) {
            self.lines.remove(bottom);
            self.lines
                .insert(self.cursor.row, Line::blank(self.columns));
        }
    }

    /// Deletes lines at the cursor, pulling the rest of the region up.
    pub fn delete_lines(&mut self, amount: usize) {
        let (top, bottom) = self.region;
        if self.cursor.row < top || self.cursor.row > bottom {
            return;
        }
        for _ in 0..amount.min(bottom - self.cursor.row + 1) {
            self.lines.remove(self.cursor.row);
            self.lines.insert(bottom, Line::blank(self.columns));
        }
    }

    /// Inserts blank cells at the cursor, pushing the rest of the line right.
    pub fn insert_cells(&mut self, amount: usize, face: Face) {
        let (columns, column) = (self.columns, self.cursor.column);
        if let Some(line) = self.lines.get_mut(self.cursor.row) {
            for _ in 0..amount.min(columns - column) {
                line.cells.pop();
                line.cells.insert(
                    column,
                    Cell {
                        ch: ' ',
                        face,
                        wide_continuation: false,
                    },
                );
            }
        }
    }

    /// Deletes cells at the cursor, pulling the rest of the line left.
    pub fn delete_cells(&mut self, amount: usize, face: Face) {
        let (columns, column) = (self.columns, self.cursor.column);
        if let Some(line) = self.lines.get_mut(self.cursor.row) {
            for _ in 0..amount.min(columns - column) {
                line.cells.remove(column);
                line.cells.push(Cell {
                    ch: ' ',
                    face,
                    wide_continuation: false,
                });
            }
        }
    }

    /// Resizes the screen, keeping what fits.
    ///
    /// Lines are truncated or padded rather than reflowed. Reflowing is what a
    /// full emulator does and it is a great deal of work for a case — resizing
    /// a terminal that has wrapped output in it — that costs the user only the
    /// shape of history they have already read.
    pub fn resize(&mut self, rows: usize, columns: usize) {
        let (rows, columns) = (rows.max(1), columns.max(1));
        for line in self.lines.iter_mut().chain(self.scrollback.iter_mut()) {
            line.cells.resize(columns, Cell::default());
        }
        // Growing takes lines back out of the scrollback, so making a window
        // taller shows what was just pushed off it rather than blank rows.
        while self.lines.len() < rows {
            match self.scrollback.pop_back() {
                Some(line) => {
                    self.lines.insert(0, line);
                    self.cursor.row += 1;
                }
                None => self.lines.push(Line::blank(columns)),
            }
        }
        while self.lines.len() > rows {
            // Drop from the top, which is where the oldest is, and keep the
            // cursor with the text it is sitting in.
            let line = self.lines.remove(0);
            if self.scrollback_limit > 0 {
                self.scrollback.push_back(line);
                while self.scrollback.len() > self.scrollback_limit {
                    self.scrollback.pop_front();
                }
            }
            self.cursor.row = self.cursor.row.saturating_sub(1);
        }
        self.rows = rows;
        self.columns = columns;
        self.region = (0, rows - 1);
        self.cursor.row = self.cursor.row.min(rows - 1);
        self.cursor.column = self.cursor.column.min(columns - 1);
        self.pending_wrap = false;
    }

    /// Empties the screen and the scrollback both.
    pub fn reset(&mut self) {
        self.lines = vec![Line::blank(self.columns); self.rows];
        self.scrollback.clear();
        self.cursor = Cursor::default();
        self.region = (0, self.rows - 1);
        self.pending_wrap = false;
    }
}

/// A face carrying a colour the terminal named rather than the theme.
pub fn ansi_face(foreground: Option<Color>, background: Option<Color>) -> Face {
    Face {
        foreground,
        background,
        ..Face::default()
    }
}
