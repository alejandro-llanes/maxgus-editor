//! The escape-sequence half: what a terminal program's output means.
//!
//! `vte` runs the state machine that recognises the sequences; everything here
//! is what to *do* about each one. Splitting it that way means the parsing is
//! somebody else's well-tested problem and this file is a table of terminal
//! behaviour, which is the part worth reading and testing.
//!
//! What is implemented is what real programs use: colours to twenty-four bits,
//! cursor movement and saving, scrolling regions, insert and delete of lines
//! and characters, the alternate screen, bracketed paste, and window titles.
//! Sequences outside that are dropped rather than guessed at — a wrong guess
//! corrupts the screen, where an ignored sequence usually does not.

use crate::grid::Grid;
use maxgus_faces::{Attributes, Color, Face};

/// Which keys the program has asked to receive differently.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Modes {
    /// `DECCKM`: the arrows send `SS3` rather than `CSI`, which is what a
    /// readline prompt expects while it is in charge.
    pub application_cursor: bool,
    /// `DECTCEM`: whether to draw the cursor at all.
    pub cursor_visible: bool,
    /// Bracketed paste: pasted text is wrapped so a shell can tell it from
    /// typing and not run half of it.
    pub bracketed_paste: bool,
    /// The alternate screen is in use, so there is no scrollback to show.
    pub alternate_screen: bool,
    /// Mouse reporting is on, which is the difference between the program
    /// wanting the mouse and us being free to select text with it.
    pub mouse_reporting: bool,
}

/// A terminal: a screen, a parser, and the state the two share.
pub struct Emulator {
    grid: Grid,
    /// Kept aside while the alternate screen is in use.
    saved_grid: Option<Grid>,
    parser: vte::Parser,
    state: State,
}

/// Everything `Perform` needs that is not the grid itself.
struct State {
    pen: Face,
    saved_cursor: Option<(crate::grid::Cursor, Face)>,
    modes: Modes,
    title: Option<String>,
    /// Bytes the program has asked us to send back, such as a cursor report.
    replies: Vec<u8>,
    bell: bool,
    /// Set when the alternate screen is entered or left, so the caller can
    /// swap the grids — `Perform` cannot, since it does not own them.
    switch_screen: Option<bool>,
}

impl Emulator {
    pub fn new(rows: usize, columns: usize, scrollback: usize) -> Emulator {
        Emulator {
            grid: Grid::new(rows, columns, scrollback),
            saved_grid: None,
            parser: vte::Parser::new(),
            state: State {
                pen: Face::default(),
                saved_cursor: None,
                modes: Modes {
                    cursor_visible: true,
                    ..Modes::default()
                },
                title: None,
                replies: Vec::new(),
                bell: false,
                switch_screen: None,
            },
        }
    }

    pub fn grid(&self) -> &Grid {
        &self.grid
    }

    pub fn modes(&self) -> Modes {
        self.state.modes
    }

    pub fn title(&self) -> Option<&str> {
        self.state.title.as_deref()
    }

    /// Bytes the program asked for, taken away as they are sent.
    pub fn take_replies(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.state.replies)
    }

    /// True once since last asked, if the program rang the bell.
    pub fn take_bell(&mut self) -> bool {
        std::mem::take(&mut self.state.bell)
    }

    /// Feeds output from the program.
    pub fn advance(&mut self, bytes: &[u8]) {
        let mut performer = Performer {
            grid: &mut self.grid,
            state: &mut self.state,
        };
        self.parser.advance(&mut performer, bytes);
        // The alternate screen is a whole second grid, so swapping it has to
        // happen out here where both are owned.
        if let Some(wanted) = self.state.switch_screen.take() {
            self.switch_screen(wanted);
        }
    }

    fn switch_screen(&mut self, alternate: bool) {
        if alternate == self.state.modes.alternate_screen {
            return;
        }
        let (rows, columns) = (self.grid.rows(), self.grid.columns());
        if alternate {
            // No scrollback on the alternate screen: `vim` scrolling its own
            // window is not producing history anybody wants to keep.
            let fresh = Grid::new(rows, columns, 0);
            self.saved_grid = Some(std::mem::replace(&mut self.grid, fresh));
        } else if let Some(mut saved) = self.saved_grid.take() {
            saved.resize(rows, columns);
            self.grid = saved;
        }
        self.state.modes.alternate_screen = alternate;
    }

    pub fn resize(&mut self, rows: usize, columns: usize) {
        self.grid.resize(rows, columns);
        if let Some(saved) = self.saved_grid.as_mut() {
            saved.resize(rows, columns);
        }
    }
}

struct Performer<'a> {
    grid: &'a mut Grid,
    state: &'a mut State,
}

impl Performer<'_> {
    /// One CSI parameter, defaulting when it is absent or zero.
    fn arg(params: &vte::Params, at: usize, default: u16) -> u16 {
        match params
            .iter()
            .nth(at)
            .and_then(|values| values.first())
            .copied()
        {
            Some(0) | None => default,
            Some(value) => value,
        }
    }

    fn arg_raw(params: &vte::Params, at: usize) -> u16 {
        params
            .iter()
            .nth(at)
            .and_then(|values| values.first())
            .copied()
            .unwrap_or(0)
    }

    /// `SGR`: the pen.
    fn select_graphic_rendition(&mut self, params: &vte::Params) {
        let mut values: Vec<u16> = Vec::new();
        // Both spellings of an extended colour are accepted: `38;5;n` as
        // separate parameters and `38:5:n` as one with sub-parameters.
        let mut groups: Vec<Vec<u16>> = Vec::new();
        for group in params.iter() {
            groups.push(group.to_vec());
        }
        if groups.is_empty() {
            self.state.pen = Face::default();
            return;
        }
        for group in &groups {
            if group.len() > 1 {
                if let Some(colour) = extended_colour(&group[1..]) {
                    match group[0] {
                        38 => self.state.pen.foreground = Some(colour),
                        48 => self.state.pen.background = Some(colour),
                        _ => {}
                    }
                }
                continue;
            }
            values.push(group[0]);
        }
        let mut index = 0;
        while index < values.len() {
            let value = values[index];
            match value {
                0 => self.state.pen = Face::default(),
                1 => self.state.pen.attributes.bold = Some(true),
                2 => self.state.pen.attributes.dim = Some(true),
                3 => self.state.pen.attributes.italic = Some(true),
                4 => self.state.pen.attributes.underline = Some(true),
                7 => self.state.pen.attributes.reverse = Some(true),
                9 => self.state.pen.attributes.strikethrough = Some(true),
                22 => {
                    self.state.pen.attributes.bold = Some(false);
                    self.state.pen.attributes.dim = Some(false);
                }
                23 => self.state.pen.attributes.italic = Some(false),
                24 => self.state.pen.attributes.underline = Some(false),
                27 => self.state.pen.attributes.reverse = Some(false),
                29 => self.state.pen.attributes.strikethrough = Some(false),
                30..=37 => self.state.pen.foreground = Some(Color::Indexed((value - 30) as u8)),
                39 => self.state.pen.foreground = None,
                40..=47 => self.state.pen.background = Some(Color::Indexed((value - 40) as u8)),
                49 => self.state.pen.background = None,
                90..=97 => self.state.pen.foreground = Some(Color::Indexed((value - 90 + 8) as u8)),
                100..=107 => {
                    self.state.pen.background = Some(Color::Indexed((value - 100 + 8) as u8));
                }
                38 | 48 => {
                    // The `;` spelling: the colour's parts follow as separate
                    // parameters, so they are consumed here.
                    if let Some(colour) = extended_colour(&values[index + 1..]) {
                        let used = match values.get(index + 1) {
                            Some(2) => 4,
                            Some(5) => 2,
                            _ => 1,
                        };
                        if value == 38 {
                            self.state.pen.foreground = Some(colour);
                        } else {
                            self.state.pen.background = Some(colour);
                        }
                        index += used;
                    }
                }
                _ => {}
            }
            index += 1;
        }
    }

    fn set_mode(&mut self, params: &vte::Params, private: bool, on: bool) {
        for group in params.iter() {
            let Some(&mode) = group.first() else { continue };
            if !private {
                continue;
            }
            match mode {
                1 => self.state.modes.application_cursor = on,
                25 => self.state.modes.cursor_visible = on,
                1000 | 1002 | 1003 | 1006 | 1015 => self.state.modes.mouse_reporting = on,
                2004 => self.state.modes.bracketed_paste = on,
                // The three spellings of "use the alternate screen". 1049 and
                // 1047 also clear it on the way in, which is what stops the
                // last screen of `less` being left behind under `vim`.
                47 | 1047 | 1049 => {
                    if on && mode != 47 {
                        self.state.saved_cursor = Some((self.grid.cursor, self.state.pen));
                    }
                    self.state.switch_screen = Some(on);
                    if !on
                        && mode == 1049
                        && let Some((cursor, pen)) = self.state.saved_cursor.take()
                    {
                        self.grid.move_to(cursor.row, cursor.column);
                        self.state.pen = pen;
                    }
                }
                1048 => {
                    if on {
                        self.state.saved_cursor = Some((self.grid.cursor, self.state.pen));
                    } else if let Some((cursor, pen)) = self.state.saved_cursor.take() {
                        self.grid.move_to(cursor.row, cursor.column);
                        self.state.pen = pen;
                    }
                }
                _ => {}
            }
        }
    }
}

/// `38;5;n`, `38;2;r;g;b` and their `:`-separated twins.
fn extended_colour(rest: &[u16]) -> Option<Color> {
    match rest.first()? {
        5 => Some(Color::Indexed(*rest.get(1)? as u8)),
        2 => {
            // Some programs send a colour-space id first, so a five-part form
            // is read from the end.
            let parts = if rest.len() >= 5 {
                &rest[2..5]
            } else {
                rest.get(1..4)?
            };
            Some(Color::Rgb(
                *parts.first()? as u8,
                *parts.get(1)? as u8,
                *parts.get(2)? as u8,
            ))
        }
        _ => None,
    }
}

impl vte::Perform for Performer<'_> {
    fn print(&mut self, c: char) {
        let pen = self.state.pen;
        self.grid.put(c, pen);
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            0x07 => self.state.bell = true,
            0x08 => self.grid.move_by(0, -1),
            0x09 => {
                // To the next multiple of eight, the fixed tab stop every
                // terminal has had since hardware ones.
                let next = (self.grid.cursor.column / 8 + 1) * 8;
                let column = next.min(self.grid.columns() - 1);
                let row = self.grid.cursor.row;
                self.grid.move_to(row, column);
            }
            0x0a..=0x0c => self.grid.line_feed(),
            0x0d => self.grid.carriage_return(),
            _ => {}
        }
    }

    fn csi_dispatch(
        &mut self,
        params: &vte::Params,
        intermediates: &[u8],
        ignore: bool,
        action: char,
    ) {
        if ignore {
            return;
        }
        let private = intermediates.first() == Some(&b'?');
        let one = |at: usize| Performer::arg(params, at, 1) as usize;
        let pen = self.state.pen;

        match action {
            'A' => self.grid.move_by(-(one(0) as isize), 0),
            'B' | 'e' => self.grid.move_by(one(0) as isize, 0),
            'C' | 'a' => self.grid.move_by(0, one(0) as isize),
            'D' => self.grid.move_by(0, -(one(0) as isize)),
            'E' => {
                self.grid.move_by(one(0) as isize, 0);
                self.grid.carriage_return();
            }
            'F' => {
                self.grid.move_by(-(one(0) as isize), 0);
                self.grid.carriage_return();
            }
            'G' | '`' => {
                let row = self.grid.cursor.row;
                self.grid.move_to(row, one(0) - 1);
            }
            'd' => {
                let column = self.grid.cursor.column;
                self.grid.move_to(one(0) - 1, column);
            }
            'H' | 'f' => self.grid.move_to(one(0) - 1, one(1) - 1),
            'J' => {
                let (row, last) = (self.grid.cursor.row, self.grid.rows() - 1);
                let columns = self.grid.columns() - 1;
                match Performer::arg_raw(params, 0) {
                    0 => {
                        let column = self.grid.cursor.column;
                        self.grid.erase_in_line(column, columns, pen);
                        if row < last {
                            self.grid.erase_rows(row + 1, last, pen);
                        }
                    }
                    1 => {
                        if row > 0 {
                            self.grid.erase_rows(0, row - 1, pen);
                        }
                        let column = self.grid.cursor.column;
                        self.grid.erase_in_line(0, column, pen);
                    }
                    2 | 3 => self.grid.erase_rows(0, last, pen),
                    _ => {}
                }
            }
            'K' => {
                let (column, last) = (self.grid.cursor.column, self.grid.columns() - 1);
                match Performer::arg_raw(params, 0) {
                    0 => self.grid.erase_in_line(column, last, pen),
                    1 => self.grid.erase_in_line(0, column, pen),
                    2 => self.grid.erase_in_line(0, last, pen),
                    _ => {}
                }
            }
            'L' => self.grid.insert_lines(one(0)),
            'M' => self.grid.delete_lines(one(0)),
            'P' => self.grid.delete_cells(one(0), pen),
            '@' => self.grid.insert_cells(one(0), pen),
            'X' => {
                let column = self.grid.cursor.column;
                let to = (column + one(0)).saturating_sub(1);
                self.grid.erase_in_line(column, to, pen);
            }
            'S' => self.grid.scroll_up(one(0)),
            'T' => self.grid.scroll_down(one(0)),
            'm' => self.select_graphic_rendition(params),
            'h' => self.set_mode(params, private, true),
            'l' => self.set_mode(params, private, false),
            'r' => {
                let top = one(0) - 1;
                let bottom = Performer::arg(params, 1, self.grid.rows() as u16) as usize - 1;
                if top < bottom && bottom < self.grid.rows() {
                    self.grid.region = (top, bottom);
                    self.grid.move_to(top, 0);
                }
            }
            's' => self.state.saved_cursor = Some((self.grid.cursor, self.state.pen)),
            'u' => {
                if let Some((cursor, pen)) = self.state.saved_cursor.take() {
                    self.grid.move_to(cursor.row, cursor.column);
                    self.state.pen = pen;
                }
            }
            'n' => {
                // A cursor position report. Answering matters: a shell that
                // asks and is not told will sit there waiting.
                if Performer::arg_raw(params, 0) == 6 {
                    let reply = format!(
                        "\x1b[{};{}R",
                        self.grid.cursor.row + 1,
                        self.grid.cursor.column + 1
                    );
                    self.state.replies.extend_from_slice(reply.as_bytes());
                }
            }
            'c' => self.state.replies.extend_from_slice(b"\x1b[?6c"),
            _ => {}
        }
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], _ignore: bool, byte: u8) {
        match (intermediates.first(), byte) {
            (None, b'D') => self.grid.line_feed(),
            (None, b'E') => {
                self.grid.line_feed();
                self.grid.carriage_return();
            }
            (None, b'M') => self.grid.reverse_line_feed(),
            (None, b'7') => self.state.saved_cursor = Some((self.grid.cursor, self.state.pen)),
            (None, b'8') => {
                if let Some((cursor, pen)) = self.state.saved_cursor.take() {
                    self.grid.move_to(cursor.row, cursor.column);
                    self.state.pen = pen;
                }
            }
            (None, b'c') => {
                self.grid.reset();
                self.state.pen = Face::default();
                self.state.modes = Modes {
                    cursor_visible: true,
                    ..Modes::default()
                };
            }
            _ => {}
        }
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        // `0` sets icon and title, `2` sets the title. Both are what a shell
        // uses to say what it is running, which is what the tab shows.
        let Some(kind) = params.first() else { return };
        if matches!(*kind, b"0" | b"2")
            && let Some(text) = params.get(1)
        {
            self.state.title = Some(String::from_utf8_lossy(text).into_owned());
        }
    }
}

/// The default attributes, for tests and for resetting.
pub fn plain() -> Attributes {
    Attributes::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An emulator fed some bytes.
    fn run(rows: usize, columns: usize, bytes: &str) -> Emulator {
        let mut emulator = Emulator::new(rows, columns, 100);
        emulator.advance(bytes.as_bytes());
        emulator
    }

    /// The screen as text, trailing blanks trimmed.
    fn screen(emulator: &Emulator) -> Vec<String> {
        emulator
            .grid()
            .lines()
            .iter()
            .map(|line| line.text())
            .collect()
    }

    fn history(emulator: &Emulator) -> Vec<String> {
        emulator
            .grid()
            .scrollback()
            .iter()
            .map(|line| line.text())
            .collect()
    }

    fn face_at(emulator: &Emulator, row: usize, column: usize) -> Face {
        emulator
            .grid()
            .line(row)
            .and_then(|l| l.cells.get(column))
            .map(|c| c.face)
            .unwrap_or_default()
    }

    #[test]
    fn text_lands_where_it_is_written() {
        let e = run(3, 20, "hello\r\nworld");
        assert_eq!(screen(&e)[0], "hello");
        assert_eq!(screen(&e)[1], "world");
        assert_eq!(e.grid().cursor, crate::grid::Cursor { row: 1, column: 5 });
    }

    #[test]
    fn a_line_that_exactly_fills_the_width_does_not_leave_a_blank_row() {
        // The wrap is deferred until another character actually arrives. A
        // terminal that wraps eagerly puts an empty line after every line of
        // exactly the screen's width, which is most of `ls` output.
        let e = run(4, 5, "abcde");
        assert_eq!(screen(&e)[0], "abcde");
        assert_eq!(screen(&e)[1], "", "wrapped too early");
        assert_eq!(e.grid().cursor.row, 0, "the cursor left the line early");

        let e = run(4, 5, "abcdef");
        assert_eq!(screen(&e)[0], "abcde");
        assert_eq!(screen(&e)[1], "f");
        assert!(
            e.grid().line(0).unwrap().wrapped,
            "the line is not marked as wrapped"
        );
    }

    #[test]
    fn the_cursor_moves_the_way_the_sequences_say() {
        let e = run(5, 10, "\x1b[3;4Hx");
        assert_eq!(screen(&e)[2], "   x", "CUP is one-based");

        let e = run(5, 10, "abc\x1b[2Dx");
        assert_eq!(screen(&e)[0], "axc", "CUB then overwrite");

        let e = run(5, 10, "a\x1b[2Bb");
        assert_eq!(screen(&e)[2], " b", "CUD keeps the column");

        // Movement is clamped rather than wrapping or panicking.
        let e = run(3, 5, "\x1b[99;99Hx");
        assert_eq!(screen(&e)[2], "    x");
    }

    #[test]
    fn erasing_clears_what_it_says_and_no_more() {
        let e = run(3, 10, "abcdef\x1b[1;4H\x1b[K");
        assert_eq!(screen(&e)[0], "abc", "EL 0 clears from the cursor on");

        let e = run(3, 10, "abcdef\x1b[1;4H\x1b[1K");
        assert_eq!(screen(&e)[0], "    ef", "EL 1 clears up to the cursor");

        let e = run(3, 10, "one\r\ntwo\r\nthree\x1b[2;1H\x1b[J");
        assert_eq!(screen(&e), ["one", "", ""], "ED 0 clears downwards");

        let e = run(3, 10, "one\r\ntwo\r\nthree\x1b[2J");
        assert_eq!(screen(&e), ["", "", ""]);
    }

    #[test]
    fn colours_arrive_in_all_four_spellings() {
        let e = run(1, 20, "\x1b[31mr");
        assert_eq!(face_at(&e, 0, 0).foreground, Some(Color::Indexed(1)));

        let e = run(1, 20, "\x1b[91mr");
        assert_eq!(
            face_at(&e, 0, 0).foreground,
            Some(Color::Indexed(9)),
            "bright red"
        );

        let e = run(1, 20, "\x1b[38;5;208mx");
        assert_eq!(
            face_at(&e, 0, 0).foreground,
            Some(Color::Indexed(208)),
            "256-colour"
        );

        let e = run(1, 20, "\x1b[38;2;10;20;30mx");
        assert_eq!(
            face_at(&e, 0, 0).foreground,
            Some(Color::Rgb(10, 20, 30)),
            "truecolor"
        );

        // The colon spelling, which is the one the standard actually defines
        // and which `htop` among others sends.
        let e = run(1, 20, "\x1b[38:2::10:20:30mx");
        assert_eq!(
            face_at(&e, 0, 0).foreground,
            Some(Color::Rgb(10, 20, 30)),
            "colon truecolor"
        );

        let e = run(1, 20, "\x1b[48;5;4mx");
        assert_eq!(face_at(&e, 0, 0).background, Some(Color::Indexed(4)));
    }

    #[test]
    fn attributes_are_set_and_cleared_independently() {
        let e = run(1, 20, "\x1b[1;4mx\x1b[24my");
        assert_eq!(face_at(&e, 0, 0).attributes.bold, Some(true));
        assert_eq!(face_at(&e, 0, 0).attributes.underline, Some(true));
        assert_eq!(
            face_at(&e, 0, 1).attributes.bold,
            Some(true),
            "bold should survive"
        );
        assert_eq!(face_at(&e, 0, 1).attributes.underline, Some(false));

        // A bare `m` is a reset, which is how most programs finish a line.
        // Reset leaves the attribute *unset* rather than explicitly off: a
        // cell with nothing of its own takes the terminal's own face, which
        // is what a theme is for.
        let e = run(1, 20, "\x1b[1;31mx\x1b[my");
        assert_eq!(face_at(&e, 0, 1).foreground, None);
        assert_eq!(face_at(&e, 0, 1).attributes.bold, None);
    }

    #[test]
    fn output_past_the_bottom_scrolls_into_the_history() {
        let e = run(2, 10, "one\r\ntwo\r\nthree");
        assert_eq!(screen(&e), ["two", "three"]);
        assert_eq!(
            history(&e),
            ["one"],
            "the first line should be in the scrollback"
        );
    }

    #[test]
    fn a_scrolling_region_scrolls_without_making_history() {
        // A program scrolling a window inside the screen is not producing
        // output anybody wants to scroll back to.
        let e = run(4, 10, "\x1b[2;3r\x1b[2;1Ha\r\nb\r\nc");
        assert_eq!(screen(&e)[1], "b");
        assert_eq!(screen(&e)[2], "c");
        assert!(
            history(&e).is_empty(),
            "a region scroll leaked into the scrollback"
        );
    }

    #[test]
    fn lines_and_characters_are_inserted_and_deleted() {
        let e = run(4, 10, "one\r\ntwo\r\n\x1b[1;1H\x1b[L");
        assert_eq!(screen(&e), ["", "one", "two", ""], "IL pushes down");

        let e = run(4, 10, "one\r\ntwo\r\n\x1b[1;1H\x1b[M");
        assert_eq!(screen(&e), ["two", "", "", ""], "DL pulls up");

        let e = run(2, 10, "abcdef\x1b[1;3H\x1b[2P");
        assert_eq!(screen(&e)[0], "abef", "DCH pulls the line left");

        let e = run(2, 10, "abcdef\x1b[1;3H\x1b[2@");
        assert_eq!(screen(&e)[0], "ab  cdef", "ICH pushes the line right");
    }

    #[test]
    fn the_alternate_screen_is_given_back_untouched() {
        // This is what lets `vim` take the terminal and leave the shell's
        // output exactly as it was.
        // Wide enough for the text below to sit on one line; at ten columns
        // it wraps and no single line contains the phrase being looked for.
        let mut e = Emulator::new(3, 40, 100);
        e.advance(b"shell output\r\n");
        e.advance(b"\x1b[?1049h");
        assert!(e.modes().alternate_screen);
        e.advance(b"\x1b[2Jfull screen program");
        assert!(screen(&e).iter().any(|l| l.contains("full screen")));
        assert!(
            !screen(&e).iter().any(|l| l.contains("shell")),
            "the shell's text is still there"
        );

        e.advance(b"\x1b[?1049l");
        assert!(!e.modes().alternate_screen);
        assert!(
            screen(&e).iter().any(|l| l.contains("shell")),
            "the shell's output did not come back: {:?}",
            screen(&e)
        );
    }

    #[test]
    fn the_alternate_screen_keeps_no_history() {
        let mut e = Emulator::new(2, 10, 100);
        e.advance(b"\x1b[?1049h");
        e.advance(b"a\r\nb\r\nc\r\nd");
        assert!(
            history(&e).is_empty(),
            "the alternate screen made scrollback"
        );
    }

    #[test]
    fn modes_the_program_asks_for_are_remembered() {
        let e = run(2, 10, "\x1b[?2004h");
        assert!(
            e.modes().bracketed_paste,
            "a paste must be bracketed once asked for"
        );

        let e = run(2, 10, "\x1b[?25l");
        assert!(!e.modes().cursor_visible);

        let e = run(2, 10, "\x1b[?1h");
        assert!(
            e.modes().application_cursor,
            "the arrows change shape at a prompt"
        );

        let e = run(2, 10, "\x1b[?1000h");
        assert!(e.modes().mouse_reporting);
    }

    #[test]
    fn the_title_a_program_sets_is_kept() {
        let e = run(2, 20, "\x1b]0;~/projects\x07");
        assert_eq!(e.title(), Some("~/projects"));
        let e = run(2, 20, "\x1b]2;vim README\x1b\\");
        assert_eq!(e.title(), Some("vim README"));
    }

    #[test]
    fn a_program_that_asks_where_the_cursor_is_gets_an_answer() {
        // Not answering is not harmless: a shell that asks and is not told
        // sits there waiting for the reply.
        let mut e = Emulator::new(5, 10, 0);
        e.advance(b"\x1b[3;4H\x1b[6n");
        assert_eq!(String::from_utf8(e.take_replies()).unwrap(), "\x1b[3;4R");
        assert!(
            e.take_replies().is_empty(),
            "the reply should be taken away once sent"
        );
    }

    #[test]
    fn a_tab_goes_to_the_next_stop_of_eight() {
        let e = run(1, 30, "a\tb\tc");
        assert_eq!(screen(&e)[0], "a       b       c");
    }

    #[test]
    fn saving_and_restoring_the_cursor_restores_the_pen_too() {
        let e = run(3, 20, "\x1b[31m\x1b7\x1b[2;5H\x1b[32mx\x1b8y");
        assert_eq!(
            e.grid().cursor.column,
            1,
            "the cursor came back to the start"
        );
        assert_eq!(
            face_at(&e, 0, 0).foreground,
            Some(Color::Indexed(1)),
            "the pen came back red"
        );
        assert_eq!(face_at(&e, 1, 4).foreground, Some(Color::Indexed(2)));
    }

    #[test]
    fn reverse_index_at_the_top_scrolls_the_screen_down() {
        let e = run(3, 10, "a\r\nb\r\nc\x1b[1;1H\x1bM");
        assert_eq!(screen(&e), ["", "a", "b"]);
    }

    #[test]
    fn a_wide_character_takes_two_columns() {
        let e = run(1, 10, "\u{4f60}x");
        assert_eq!(screen(&e)[0], "\u{4f60}x");
        assert!(e.grid().line(0).unwrap().cells[1].wide_continuation);
        assert_eq!(e.grid().cursor.column, 3);
    }

    #[test]
    fn growing_the_screen_takes_lines_back_out_of_the_history() {
        // Otherwise making the terminal taller shows blank rows under text
        // that has only just scrolled off.
        let mut e = Emulator::new(2, 10, 100);
        e.advance(b"one\r\ntwo\r\nthree");
        assert_eq!(history(&e), ["one"]);

        e.resize(4, 10);
        assert_eq!(screen(&e)[0], "one", "the scrolled line did not come back");
        assert!(history(&e).is_empty());
    }

    #[test]
    fn shrinking_the_screen_keeps_the_text_and_the_cursor_together() {
        let mut e = Emulator::new(4, 10, 100);
        e.advance(b"one\r\ntwo\r\nthree\r\nfour");
        e.resize(2, 10);
        assert_eq!(screen(&e), ["three", "four"]);
        assert_eq!(e.grid().cursor.row, 1, "the cursor left the line it was on");
    }

    #[test]
    fn a_narrower_screen_truncates_rather_than_losing_the_line() {
        let mut e = Emulator::new(2, 10, 10);
        e.advance(b"abcdefgh");
        e.resize(2, 4);
        assert_eq!(e.grid().columns(), 4);
        assert_eq!(screen(&e)[0], "abcd");
    }

    #[test]
    fn nonsense_sequences_are_ignored_rather_than_guessed_at() {
        // A wrong guess corrupts the screen; an ignored sequence usually does
        // not. Whatever happens, it must not panic.
        let e = run(3, 10, "a\x1b[?????99999999Zb\x1b[999999999;999999999Hc");
        assert!(screen(&e).iter().any(|line| line.contains('a')));
        let e = run(3, 10, "\x1b]999;\x07\x1bZ\x1b#8\x1b[38;5m\x1b[38;2;1m");
        assert_eq!(e.grid().rows(), 3);
    }
}
