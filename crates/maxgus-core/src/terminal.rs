//! Terminal tabs: the state the editor keeps for each running shell.
//!
//! The emulator itself lives in `maxgus-term` and knows nothing about the
//! editor. What is here is the rest of a tab — which terminal it is, what it
//! is called, where it is scrolled to, and what is selected in it — plus the
//! list of them and which one is showing.

use crate::task::TerminalId;
use maxgus_term::{Emulator, Selection, selection::Mode as SelectionMode};

/// One tab: a running shell and the state of looking at it.
pub struct Terminal {
    pub id: TerminalId,
    pub emulator: Emulator,
    /// What the program called itself, when it said. Falls back to the shell.
    pub title: String,
    /// How far back through the scrollback the window is looking. Zero is the
    /// live screen, which is where new output always brings it back to.
    pub scroll: usize,
    pub selection: Option<Selection>,
    /// Where the cursor is while reading rather than typing. `Some` means
    /// copy mode: keys move this cursor instead of reaching the shell, which
    /// is the only way to select text without a mouse.
    pub copy_cursor: Option<maxgus_term::Position>,
    /// Set once the shell has ended, so the tab can say so instead of
    /// disappearing under whatever the user was reading.
    pub exited: Option<i32>,
}

impl Terminal {
    pub fn new(id: TerminalId, title: String, rows: usize, columns: usize) -> Terminal {
        Terminal {
            id,
            emulator: Emulator::new(rows, columns, SCROLLBACK),
            title,
            scroll: 0,
            selection: None,
            copy_cursor: None,
            exited: None,
        }
    }

    pub fn in_copy_mode(&self) -> bool {
        self.copy_cursor.is_some()
    }

    /// Enters copy mode with the cursor where the shell's cursor is.
    pub fn begin_copy_mode(&mut self) {
        let grid = self.emulator.grid();
        let line = grid.scrollback().len() + grid.cursor.row;
        self.copy_cursor = Some(maxgus_term::Position::new(line, grid.cursor.column));
        self.selection = None;
    }

    pub fn end_copy_mode(&mut self) {
        self.copy_cursor = None;
        self.selection = None;
    }

    /// Moves the copy cursor, keeping it inside the text and dragging the
    /// selection along when a mark has been set.
    pub fn move_copy_cursor(&mut self, lines: isize, columns: isize) {
        let Some(cursor) = self.copy_cursor else {
            return;
        };
        let grid = self.emulator.grid();
        let last_line = grid.total_lines().saturating_sub(1);
        let line = (cursor.line as isize + lines).clamp(0, last_line as isize) as usize;
        let width = grid.columns().saturating_sub(1);
        let column = (cursor.column as isize + columns).clamp(0, width as isize) as usize;
        self.copy_cursor = Some(maxgus_term::Position::new(line, column));
        if let Some(selection) = self.selection.as_mut() {
            selection.extend_to(maxgus_term::Position::new(line, column));
        }
        self.follow_copy_cursor();
    }

    /// Moves the copy cursor to an absolute position.
    pub fn move_copy_cursor_to(&mut self, line: usize, column: usize) {
        if self.copy_cursor.is_none() {
            return;
        }
        let grid = self.emulator.grid();
        let line = line.min(grid.total_lines().saturating_sub(1));
        let column = column.min(grid.columns().saturating_sub(1));
        self.copy_cursor = Some(maxgus_term::Position::new(line, column));
        if let Some(selection) = self.selection.as_mut() {
            selection.extend_to(maxgus_term::Position::new(line, column));
        }
        self.follow_copy_cursor();
    }

    /// Scrolls so the copy cursor is on screen, which is what makes moving
    /// off the top of the window walk back through the history.
    fn follow_copy_cursor(&mut self) {
        let Some(cursor) = self.copy_cursor else {
            return;
        };
        let rows = self.emulator.grid().rows();
        let history = self.emulator.grid().scrollback().len();
        let top = history.saturating_sub(self.scroll);
        if cursor.line < top {
            self.scroll = history - cursor.line;
        } else if cursor.line >= top + rows {
            self.scroll = history.saturating_sub(cursor.line + 1 - rows);
        }
    }

    /// Sets the mark where the copy cursor is, starting a selection.
    pub fn set_mark(&mut self, mode: SelectionMode) {
        if let Some(cursor) = self.copy_cursor {
            self.selection = Some(Selection::new(cursor, mode));
        }
    }

    /// What the tab is labelled: whatever the program last called itself.
    pub fn label(&self) -> &str {
        self.emulator.title().unwrap_or(&self.title)
    }

    /// Folds in output from the program.
    pub fn receive(&mut self, bytes: &[u8]) {
        self.emulator.advance(bytes);
        // New output brings the window back to the live screen, as every
        // terminal does: otherwise running a command while scrolled back
        // looks like it produced nothing.
        self.scroll = 0;
    }

    /// The first line on screen, counted from the start of the scrollback.
    pub fn top_line(&self) -> usize {
        let grid = self.emulator.grid();
        grid.scrollback().len().saturating_sub(self.scroll)
    }

    /// Scrolls back by `lines`, stopping at the oldest line there is.
    pub fn scroll_back(&mut self, lines: usize) {
        let most = self.emulator.grid().scrollback().len();
        self.scroll = (self.scroll + lines).min(most);
    }

    pub fn scroll_forward(&mut self, lines: usize) {
        self.scroll = self.scroll.saturating_sub(lines);
    }

    /// Where the cursor is on screen, or `None` when it is scrolled out of
    /// view or the program asked for it to be hidden.
    pub fn cursor(&self) -> Option<(usize, usize)> {
        if !self.emulator.modes().cursor_visible || self.scroll > 0 {
            return None;
        }
        let cursor = self.emulator.grid().cursor;
        Some((cursor.row, cursor.column))
    }

    /// Starts a selection at an absolute position.
    pub fn begin_selection(&mut self, line: usize, column: usize, mode: SelectionMode) {
        self.selection = Some(Selection::new(
            maxgus_term::Position::new(line, column),
            mode,
        ));
    }

    pub fn extend_selection(&mut self, line: usize, column: usize) {
        if let Some(selection) = self.selection.as_mut() {
            selection.extend_to(maxgus_term::Position::new(line, column));
        }
    }

    /// The selected text, if any.
    pub fn selected_text(&self) -> Option<String> {
        let selection = self.selection?;
        let text = selection.text(self.emulator.grid());
        (!text.is_empty()).then_some(text)
    }
}

/// How many lines of history a terminal keeps.
///
/// Ten thousand is enough to scroll back through a build; keeping everything
/// would let one runaway program grow the editor without limit.
pub const SCROLLBACK: usize = 10_000;

/// Every terminal tab, and which is showing.
#[derive(Default)]
pub struct Terminals {
    tabs: Vec<Terminal>,
    current: usize,
    next_id: u64,
}

impl Terminals {
    pub fn new() -> Terminals {
        Terminals::default()
    }

    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }

    pub fn len(&self) -> usize {
        self.tabs.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Terminal> {
        self.tabs.iter()
    }

    /// The index of the tab being shown.
    pub fn current_index(&self) -> usize {
        self.current
    }

    pub fn current(&self) -> Option<&Terminal> {
        self.tabs.get(self.current)
    }

    pub fn current_mut(&mut self) -> Option<&mut Terminal> {
        self.tabs.get_mut(self.current)
    }

    pub fn get_mut(&mut self, id: TerminalId) -> Option<&mut Terminal> {
        self.tabs.iter_mut().find(|tab| tab.id == id)
    }

    /// Opens a tab and shows it. The caller starts the shell.
    pub fn open(&mut self, title: String, rows: usize, columns: usize) -> TerminalId {
        self.next_id += 1;
        let id = TerminalId(self.next_id);
        self.tabs.push(Terminal::new(id, title, rows, columns));
        self.current = self.tabs.len() - 1;
        id
    }

    /// Closes a tab, showing the one to its left.
    pub fn close(&mut self, id: TerminalId) -> Option<TerminalId> {
        let at = self.tabs.iter().position(|tab| tab.id == id)?;
        self.tabs.remove(at);
        // The one to the left, which is where the eye already is. Going to
        // the right would show a tab that has just shifted under the cursor.
        self.current = self.current.min(self.tabs.len().saturating_sub(1));
        if at < self.current {
            self.current -= 1;
        }
        Some(id)
    }

    /// Moves `delta` tabs along, wrapping.
    pub fn select_relative(&mut self, delta: isize) {
        if self.tabs.is_empty() {
            return;
        }
        let count = self.tabs.len() as isize;
        self.current = (self.current as isize + delta).rem_euclid(count) as usize;
    }

    /// Shows the tab at `index`, if there is one.
    pub fn select(&mut self, index: usize) -> bool {
        if index < self.tabs.len() {
            self.current = index;
            return true;
        }
        false
    }

    /// Resizes every terminal, which is what a window resize has to do.
    pub fn resize(&mut self, rows: usize, columns: usize) {
        for tab in &mut self.tabs {
            tab.emulator.resize(rows, columns);
        }
    }
}

impl std::fmt::Debug for Terminals {
    /// By what it holds: an emulator contains a parser state machine that has
    /// no `Debug` of its own, and a screen's worth of cells is not something
    /// anybody wants in a panic message.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Terminals")
            .field(
                "tabs",
                &self.tabs.iter().map(|tab| tab.id).collect::<Vec<_>>(),
            )
            .field("current", &self.current)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn three() -> Terminals {
        let mut terminals = Terminals::new();
        for name in ["one", "two", "three"] {
            terminals.open(name.into(), 10, 40);
        }
        terminals
    }

    #[test]
    fn opening_a_tab_shows_it() {
        let terminals = three();
        assert_eq!(terminals.len(), 3);
        assert_eq!(terminals.current().map(|t| t.title.as_str()), Some("three"));
    }

    #[test]
    fn every_tab_gets_an_id_of_its_own() {
        // Ids rather than positions, so an answer arriving for a tab that has
        // since been closed cannot land on whoever took its place.
        let terminals = three();
        let ids: Vec<_> = terminals.iter().map(|tab| tab.id).collect();
        let unique: std::collections::BTreeSet<_> = ids.iter().collect();
        assert_eq!(unique.len(), 3, "ids were reused: {ids:?}");
    }

    #[test]
    fn moving_between_tabs_wraps_both_ways() {
        let mut terminals = three();
        terminals.select(0);
        terminals.select_relative(-1);
        assert_eq!(
            terminals.current_index(),
            2,
            "moving left from the first should wrap"
        );
        terminals.select_relative(1);
        assert_eq!(terminals.current_index(), 0);
    }

    #[test]
    fn closing_a_tab_shows_the_one_to_its_left() {
        let mut terminals = three();
        let second = terminals.iter().nth(1).unwrap().id;
        terminals.select(2);
        terminals.close(second);
        assert_eq!(terminals.len(), 2);
        // The third tab is still the one showing, at its new position.
        assert_eq!(terminals.current().map(|t| t.title.as_str()), Some("three"));
    }

    #[test]
    fn closing_the_last_tab_leaves_a_usable_index() {
        let mut terminals = three();
        let ids: Vec<_> = terminals.iter().map(|tab| tab.id).collect();
        for id in ids {
            terminals.close(id);
        }
        assert!(terminals.is_empty());
        assert!(terminals.current().is_none(), "the index outlived the tabs");
    }

    #[test]
    fn output_brings_the_window_back_to_the_live_screen() {
        // Running a command while scrolled back would otherwise look like it
        // produced nothing at all.
        let mut terminal = Terminal::new(TerminalId(1), "sh".into(), 3, 20);
        terminal.receive(b"one\r\ntwo\r\nthree\r\nfour\r\n");
        terminal.scroll_back(2);
        assert!(terminal.scroll > 0);

        terminal.receive(b"five\r\n");
        assert_eq!(terminal.scroll, 0, "new output did not bring the view back");
    }

    #[test]
    fn scrolling_stops_at_the_oldest_line_there_is() {
        let mut terminal = Terminal::new(TerminalId(1), "sh".into(), 2, 20);
        terminal.receive(b"a\r\nb\r\nc\r\n");
        terminal.scroll_back(1000);
        let history = terminal.emulator.grid().scrollback().len();
        assert_eq!(
            terminal.scroll, history,
            "scrolled past the start of history"
        );
        terminal.scroll_forward(1000);
        assert_eq!(terminal.scroll, 0);
    }

    #[test]
    fn the_cursor_is_hidden_when_it_is_not_where_the_user_is_looking() {
        let mut terminal = Terminal::new(TerminalId(1), "sh".into(), 2, 20);
        terminal.receive(b"a\r\nb\r\nc\r\n");
        assert!(terminal.cursor().is_some());

        terminal.scroll_back(1);
        assert!(
            terminal.cursor().is_none(),
            "the cursor was drawn on a scrolled-back screen"
        );

        terminal.scroll_forward(1);
        terminal.receive(b"\x1b[?25l");
        assert!(
            terminal.cursor().is_none(),
            "the program asked for it to be hidden"
        );
    }

    #[test]
    fn a_tab_is_named_by_whatever_the_program_calls_itself() {
        let mut terminal = Terminal::new(TerminalId(1), "bash".into(), 2, 20);
        assert_eq!(terminal.label(), "bash");
        terminal.receive(b"\x1b]0;vim README.md\x07");
        assert_eq!(terminal.label(), "vim README.md");
    }

    #[test]
    fn selected_text_is_taken_from_the_screen() {
        let mut terminal = Terminal::new(TerminalId(1), "sh".into(), 3, 20);
        terminal.receive(b"hello world\r\n");
        terminal.begin_selection(0, 0, SelectionMode::Character);
        terminal.extend_selection(0, 4);
        assert_eq!(terminal.selected_text().as_deref(), Some("hello"));
    }

    #[test]
    fn nothing_selected_copies_nothing() {
        let terminal = Terminal::new(TerminalId(1), "sh".into(), 3, 20);
        assert!(terminal.selected_text().is_none());
    }
}
