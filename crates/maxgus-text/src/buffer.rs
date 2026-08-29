//! Buffers.
//!
//! A [`Buffer`] owns the text, point, mark and undo history for one editing
//! target. It is deliberately free of any UI concept: windows, scrolling and
//! rendering live in `maxgus-tui`, and command dispatch in `maxgus-core`.

use crate::{
    Result, TextError,
    edit::{Edit, EditKind},
    motion::Motion,
    position::{Position, Range},
    undo::UndoStack,
};
use ropey::Rope;
use std::path::{Path, PathBuf};
use unicode_width::UnicodeWidthChar;

/// A process-unique buffer handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BufferId(pub u64);

impl std::fmt::Display for BufferId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// The line terminator a file uses, preserved across save.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineEnding {
    #[default]
    Lf,
    Crlf,
}

impl LineEnding {
    pub fn as_str(self) -> &'static str {
        match self {
            LineEnding::Lf => "\n",
            LineEnding::Crlf => "\r\n",
        }
    }

    /// Guesses the dominant terminator, as `find-file` does when it sets the
    /// coding system. Ties go to LF.
    pub fn detect(text: &str) -> LineEnding {
        let crlf = text.matches("\r\n").count();
        let lf = text.matches('\n').count() - crlf;
        if crlf > lf {
            LineEnding::Crlf
        } else {
            LineEnding::Lf
        }
    }

    /// Emacs' mode-line indicator for the coding system.
    pub fn mode_line_mnemonic(self) -> char {
        match self {
            LineEnding::Lf => ':',
            LineEnding::Crlf => '\\',
        }
    }
}

/// One editing target: text plus the per-buffer state Emacs keeps.
#[derive(Debug)]
pub struct Buffer {
    pub id: BufferId,
    name: String,
    path: Option<PathBuf>,
    rope: Rope,
    point: usize,
    mark: Option<usize>,
    mark_active: bool,
    /// `mark-ring`, newest first, capped at `mark_ring_max`.
    mark_ring: Vec<usize>,
    mark_ring_max: usize,
    undo: UndoStack,
    /// Sticky column for `next-line` / `previous-line`, as `temporary-goal-column`.
    goal_column: Option<usize>,
    read_only: bool,
    disk_time: Option<std::time::SystemTime>,
    /// Language identifier used for syntax highlighting and LSP routing.
    language: Option<String>,
    line_ending: LineEnding,
    tab_width: usize,
    indent_with_tabs: bool,
    /// Accessible portion when narrowed (`C-x n n`); `None` means widened.
    narrowing: Option<Range>,
    /// Monotonic counter bumped on every applied edit, for cache invalidation.
    revision: u64,
    /// Nesting depth of open undo groups. Depth zero means a bare edit call
    /// opens and commits a group of its own.
    open_groups: usize,
    /// Edits applied since the list was last taken, as
    /// (offset, characters removed, characters inserted).
    ///
    /// Point and the mark move with an edit automatically, but anything else
    /// holding a position into this buffer — another window showing it — has
    /// to be told. This is what it is told.
    adjustments: Vec<(usize, usize, usize)>,
}

impl Buffer {
    /// A scratch buffer with no backing file.
    pub fn new(id: BufferId, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            path: None,
            rope: Rope::new(),
            point: 0,
            mark: None,
            mark_active: false,
            mark_ring: Vec::new(),
            mark_ring_max: 16,
            undo: UndoStack::new(),
            goal_column: None,
            read_only: false,
            disk_time: None,
            language: None,
            line_ending: LineEnding::default(),
            tab_width: 4,
            indent_with_tabs: false,
            narrowing: None,
            revision: 0,
            open_groups: 0,
            adjustments: Vec::new(),
        }
    }

    /// A buffer holding `text`, with the line ending detected from it.
    pub fn from_str(id: BufferId, name: impl Into<String>, text: &str) -> Self {
        let mut b = Self::new(id, name);
        b.line_ending = LineEnding::detect(text);
        // Normalise to LF internally; the original terminator is restored on save.
        b.rope = Rope::from_str(&text.replace("\r\n", "\n"));
        b
    }

    /// A buffer visiting `path`, with `text` as its contents.
    pub fn from_file(id: BufferId, path: impl Into<PathBuf>, text: &str) -> Self {
        let path = path.into();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
        let mut b = Self::from_str(id, name, text);
        b.language = language_for_path(&path);
        b.path = Some(path);
        b
    }

    // ---- identity ------------------------------------------------------

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn set_name(&mut self, name: impl Into<String>) {
        self.name = name.into();
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn set_path(&mut self, path: impl Into<PathBuf>) {
        let path = path.into();
        self.language = language_for_path(&path);
        self.path = Some(path);
    }

    pub fn language(&self) -> Option<&str> {
        self.language.as_deref()
    }

    pub fn set_language(&mut self, language: Option<String>) {
        self.language = language;
    }

    pub fn line_ending(&self) -> LineEnding {
        self.line_ending
    }

    pub fn set_line_ending(&mut self, ending: LineEnding) {
        self.line_ending = ending;
    }

    pub fn tab_width(&self) -> usize {
        self.tab_width
    }

    pub fn set_tab_width(&mut self, width: usize) {
        self.tab_width = width.max(1);
    }

    pub fn indent_with_tabs(&self) -> bool {
        self.indent_with_tabs
    }

    pub fn set_indent_with_tabs(&mut self, yes: bool) {
        self.indent_with_tabs = yes;
    }

    /// When the file was last modified, as of the read or write that put
    /// this text here.
    ///
    /// Compared against the file before saving, so an edit made outside the
    /// editor — a pull, a formatter, another editor — is noticed instead of
    /// being written over. `None` for a buffer with no file behind it, or a
    /// filesystem that would not say.
    pub fn disk_time(&self) -> Option<std::time::SystemTime> {
        self.disk_time
    }

    pub fn set_disk_time(&mut self, at: Option<std::time::SystemTime>) {
        self.disk_time = at;
    }

    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    pub fn set_read_only(&mut self, yes: bool) {
        self.read_only = yes;
    }

    pub fn is_modified(&self) -> bool {
        self.undo.is_modified()
    }

    /// Marks the buffer as differing from its file, for changes that alter how
    /// it will be written rather than what it contains.
    pub fn mark_modified(&mut self) {
        self.undo.mark_modified();
    }

    pub fn mark_saved(&mut self) {
        self.undo.mark_saved();
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    // ---- text access ---------------------------------------------------

    pub fn rope(&self) -> &Rope {
        &self.rope
    }

    pub fn len_chars(&self) -> usize {
        self.rope.len_chars()
    }

    pub fn len_lines(&self) -> usize {
        self.rope.len_lines()
    }

    pub fn is_empty(&self) -> bool {
        self.rope.len_chars() == 0
    }

    pub fn text(&self) -> String {
        self.rope.to_string()
    }

    /// The buffer contents ready to write to disk, with the original line
    /// terminator restored.
    pub fn to_disk_string(&self) -> String {
        match self.line_ending {
            LineEnding::Lf => self.rope.to_string(),
            LineEnding::Crlf => self.rope.to_string().replace('\n', "\r\n"),
        }
    }

    /// Line `line` without its terminator.
    pub fn line_text(&self, line: usize) -> String {
        if line >= self.rope.len_lines() {
            return String::new();
        }
        let slice = self.rope.line(line);
        let s = slice.to_string();
        s.trim_end_matches('\n').trim_end_matches('\r').to_string()
    }

    pub fn slice(&self, range: Range) -> String {
        let end = range.end.min(self.rope.len_chars());
        let start = range.start.min(end);
        self.rope.slice(start..end).to_string()
    }

    /// The character after point, `None` at end of buffer.
    pub fn char_after(&self, at: usize) -> Option<char> {
        (at < self.rope.len_chars()).then(|| self.rope.char(at))
    }

    /// The character before point, `None` at start of buffer.
    pub fn char_before(&self, at: usize) -> Option<char> {
        (at > 0 && at <= self.rope.len_chars()).then(|| self.rope.char(at - 1))
    }

    // ---- positions -----------------------------------------------------

    pub fn point(&self) -> usize {
        self.point
    }

    /// Moves point, clamping into the accessible portion and clearing the
    /// sticky goal column.
    pub fn set_point(&mut self, at: usize) {
        self.point = self.clamp(at);
        self.goal_column = None;
    }

    /// Moves point without disturbing the goal column, for `next-line`.
    pub fn set_point_keeping_goal(&mut self, at: usize) {
        self.point = self.clamp(at);
    }

    pub fn goal_column(&self) -> Option<usize> {
        self.goal_column
    }

    pub fn set_goal_column(&mut self, column: Option<usize>) {
        self.goal_column = column;
    }

    /// Clamps `at` into the accessible portion of the buffer.
    pub fn clamp(&self, at: usize) -> usize {
        at.clamp(self.point_min(), self.point_max())
    }

    /// `point-min`: 0, or the narrowing start.
    pub fn point_min(&self) -> usize {
        self.narrowing.map_or(0, |r| r.start)
    }

    /// `point-max`: the buffer length, or the narrowing end.
    pub fn point_max(&self) -> usize {
        self.narrowing
            .map_or(self.rope.len_chars(), |r| r.end.min(self.rope.len_chars()))
    }

    pub fn is_narrowed(&self) -> bool {
        self.narrowing.is_some()
    }

    /// `narrow-to-region`.
    pub fn narrow(&mut self, range: Range) {
        let end = range.end.min(self.rope.len_chars());
        let start = range.start.min(end);
        self.narrowing = Some(Range::new(start, end));
        self.point = self.clamp(self.point);
    }

    /// `widen`.
    pub fn widen(&mut self) {
        self.narrowing = None;
    }

    pub fn position_of(&self, offset: usize) -> Position {
        let offset = offset.min(self.rope.len_chars());
        let line = self.rope.char_to_line(offset);
        Position::new(line, offset - self.rope.line_to_char(line))
    }

    pub fn offset_of(&self, position: Position) -> usize {
        if position.line >= self.rope.len_lines() {
            return self.rope.len_chars();
        }
        let start = self.rope.line_to_char(position.line);
        let end = Motion::line_end(&self.rope, start);
        (start + position.column).min(end)
    }

    /// Column measured in terminal cells, expanding tabs to the next tab stop
    /// and accounting for wide characters.
    pub fn display_column(&self, offset: usize) -> usize {
        let start = Motion::line_start(&self.rope, offset);
        let mut col = 0usize;
        for c in self
            .rope
            .slice(start..offset.min(self.rope.len_chars()))
            .chars()
        {
            col += self.char_display_width(c, col);
        }
        col
    }

    /// The offset on `line` whose display column is nearest to `column`.
    pub fn offset_at_display_column(&self, line: usize, column: usize) -> usize {
        if line >= self.rope.len_lines() {
            return self.rope.len_chars();
        }
        let start = self.rope.line_to_char(line);
        let end = Motion::line_end(&self.rope, start);
        let mut col = 0usize;
        let mut offset = start;
        while offset < end {
            let c = self.rope.char(offset);
            let w = self.char_display_width(c, col);
            if col + w > column {
                break;
            }
            col += w;
            offset += 1;
        }
        offset
    }

    /// Width of `c` in cells when it starts at display column `col`.
    pub fn char_display_width(&self, c: char, col: usize) -> usize {
        match c {
            '\t' => self.tab_width - (col % self.tab_width),
            '\n' | '\r' => 0,
            // Control characters render as `^X`, two cells wide.
            c if (c as u32) < 0x20 => 2,
            c => c.width().unwrap_or(0),
        }
    }

    pub fn line_of(&self, offset: usize) -> usize {
        self.rope.char_to_line(offset.min(self.rope.len_chars()))
    }

    pub fn line_start(&self, line: usize) -> usize {
        if line >= self.rope.len_lines() {
            self.rope.len_chars()
        } else {
            self.rope.line_to_char(line)
        }
    }

    // ---- mark ----------------------------------------------------------

    pub fn mark(&self) -> Option<usize> {
        self.mark
    }

    /// `set-mark-command`: sets the mark, activates the region and pushes the
    /// previous mark onto the mark ring.
    pub fn set_mark(&mut self, at: usize) {
        if let Some(old) = self.mark {
            self.push_mark_ring(old);
        }
        self.mark = Some(self.clamp(at));
        self.mark_active = true;
    }

    pub fn set_mark_inactive(&mut self, at: usize) {
        self.mark = Some(self.clamp(at));
        self.mark_active = false;
    }

    /// `push-mark`: records `at` on the mark ring as well as setting the mark.
    ///
    /// This is what a command uses before moving point somewhere far away —
    /// jumping to a definition, to the end of the buffer, to a register. Only
    /// the ring makes the position reachable again, through `C-u C-SPC` or
    /// `M-,`; setting the mark alone would leave nothing to come back to.
    pub fn push_mark(&mut self, at: usize) {
        let at = self.clamp(at);
        self.push_mark_ring(at);
        self.mark = Some(at);
        self.mark_active = false;
    }

    pub fn deactivate_mark(&mut self) {
        self.mark_active = false;
    }

    pub fn activate_mark(&mut self) {
        if self.mark.is_some() {
            self.mark_active = true;
        }
    }

    pub fn is_mark_active(&self) -> bool {
        self.mark_active && self.mark.is_some()
    }

    fn push_mark_ring(&mut self, at: usize) {
        self.mark_ring.insert(0, at);
        self.mark_ring.truncate(self.mark_ring_max);
    }

    /// `C-u C-SPC`: swaps point with the newest mark-ring entry.
    pub fn pop_mark_ring(&mut self) -> Option<usize> {
        if self.mark_ring.is_empty() {
            return None;
        }
        let target = self.mark_ring.remove(0);
        self.mark_ring.push(self.point);
        self.mark_ring.truncate(self.mark_ring_max);
        Some(self.clamp(target))
    }

    /// `exchange-point-and-mark`.
    pub fn exchange_point_and_mark(&mut self) -> Result<()> {
        let mark = self.mark.ok_or(TextError::NoMark)?;
        self.mark = Some(self.point);
        self.point = self.clamp(mark);
        self.mark_active = true;
        Ok(())
    }

    /// The active region, or `None` when the mark is inactive or unset.
    pub fn region(&self) -> Option<Range> {
        self.is_mark_active()
            .then(|| Range::ordered(self.point, self.mark.expect("checked")))
    }

    /// The region regardless of whether it is active, as commands invoked with
    /// an explicit region argument use.
    pub fn region_even_if_inactive(&self) -> Option<Range> {
        self.mark.map(|m| Range::ordered(self.point, m))
    }

    // ---- editing -------------------------------------------------------

    /// Opens an undo group. `amalgamating` lets consecutive self-insertions
    /// coalesce into one undo step. Groups nest: only the outermost one is
    /// recorded, so a multi-edit command undoes in a single step.
    pub fn begin_undo_group(&mut self, amalgamating: bool) {
        if self.open_groups == 0 {
            self.undo.begin(self.point, amalgamating);
        }
        self.open_groups += 1;
    }

    /// Closes the innermost undo group, committing when the outermost closes.
    pub fn commit_undo_group(&mut self) {
        self.open_groups = self.open_groups.saturating_sub(1);
        if self.open_groups == 0 {
            self.undo.commit(self.point);
        }
    }

    /// Runs `f` inside a single undo group.
    pub fn transact<T>(&mut self, amalgamating: bool, f: impl FnOnce(&mut Self) -> T) -> T {
        self.begin_undo_group(amalgamating);
        let out = f(self);
        self.commit_undo_group();
        out
    }

    fn ensure_writable(&self) -> Result<()> {
        if self.read_only {
            return Err(TextError::Io(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!("buffer `{}` is read-only", self.name),
            )));
        }
        Ok(())
    }

    /// Applies `edit` to the rope without touching undo. Returns the offset
    /// point should move to.
    fn apply_raw(&mut self, edit: &Edit) -> usize {
        match &edit.kind {
            EditKind::Insert { text } => self.rope.insert(edit.at, text),
            EditKind::Delete { text } => {
                let end = edit.at + text.chars().count();
                self.rope.remove(edit.at..end);
            }
            EditKind::Replace { removed, inserted } => {
                let end = edit.at + removed.chars().count();
                self.rope.remove(edit.at..end);
                self.rope.insert(edit.at, inserted);
            }
        }
        // Narrowing bounds shift with edits before or inside them. Text typed
        // at the very end of the accessible region belongs inside it — hence
        // `<=` rather than `<` — or point would step outside the region it is
        // supposed to be confined to.
        if let Some(n) = self.narrowing.as_mut() {
            let delta = edit.inserted_chars() as isize - edit.removed_chars() as isize;
            if edit.at < n.start {
                n.start = n.start.saturating_add_signed(delta);
            }
            if edit.at <= n.end {
                n.end = n.end.saturating_add_signed(delta).max(n.start);
            }
        }
        // The mark moves with every applied edit, including the ones undo and
        // redo apply. Doing it here rather than in each editing method is what
        // makes that true: undo used to leave the mark where the text had been.
        let (at, removed, inserted) = (edit.at, edit.removed_chars(), edit.inserted_chars());
        if let Some(mark) = self.mark {
            self.mark = Some(Self::adjust_position(mark, at, removed, inserted));
        }
        for entry in &mut self.mark_ring {
            *entry = Self::adjust_position(*entry, at, removed, inserted);
        }
        self.revision += 1;
        self.adjustments.push((at, removed, inserted));
        edit.point_after()
    }

    /// Takes the edits applied since this was last called.
    pub fn take_adjustments(&mut self) -> Vec<(usize, usize, usize)> {
        std::mem::take(&mut self.adjustments)
    }

    /// Moves `position` across an edit, the way point and the mark move.
    ///
    /// A position after the edit shifts by the length difference; one inside
    /// the removed span collapses to where it began.
    pub fn adjust_position(position: usize, at: usize, removed: usize, inserted: usize) -> usize {
        let end = at + removed;
        if position >= end {
            position - removed + inserted
        } else if position > at {
            at
        } else {
            position
        }
    }

    /// Records `edit` in the undo history and applies it.
    fn perform(&mut self, edit: Edit) -> usize {
        self.undo.push(edit.clone());
        self.apply_raw(&edit)
    }

    /// `insert`: inserts `text` at `at` and returns the offset just past it.
    pub fn insert(&mut self, at: usize, text: &str) -> Result<usize> {
        self.ensure_writable()?;
        if text.is_empty() {
            return Ok(at);
        }
        let at = at.min(self.rope.len_chars());
        self.begin_undo_group(false);
        let end = self.perform(Edit::insert(at, text, self.point));
        // Text inserted at or before point pushes point along, as in Emacs.
        if at <= self.point {
            self.point += text.chars().count();
        }
        self.point = self.clamp(self.point);
        self.commit_undo_group();
        Ok(end)
    }

    /// `self-insert-command`: inserts at point inside an amalgamating undo
    /// group so a typed run undoes as one step.
    pub fn insert_at_point(&mut self, text: &str) -> Result<()> {
        self.ensure_writable()?;
        if text.is_empty() {
            return Ok(());
        }
        // A newline ends the amalgamation run, matching Emacs.
        let amalgamating = !text.contains('\n');
        self.begin_undo_group(amalgamating);
        let at = self.point;
        self.perform(Edit::insert(at, text, at));
        self.point = at + text.chars().count();
        self.point = self.clamp(self.point);
        self.goal_column = None;
        self.commit_undo_group();
        Ok(())
    }

    /// `delete-region`: removes `range` and returns the deleted text.
    pub fn delete(&mut self, range: Range) -> Result<String> {
        self.ensure_writable()?;
        let end = range.end.min(self.rope.len_chars());
        let start = range.start.min(end);
        if start == end {
            return Ok(String::new());
        }
        let text = self.rope.slice(start..end).to_string();
        self.begin_undo_group(false);
        self.perform(Edit::delete(start, text.clone(), self.point));
        let removed = end - start;
        self.point = if self.point >= end {
            self.point - removed
        } else if self.point > start {
            start
        } else {
            self.point
        };
        self.point = self.clamp(self.point);
        self.goal_column = None;
        self.commit_undo_group();
        Ok(text)
    }

    /// Replaces `range` with `text`, returning what was removed.
    pub fn replace(&mut self, range: Range, text: &str) -> Result<String> {
        self.ensure_writable()?;
        let end = range.end.min(self.rope.len_chars());
        let start = range.start.min(end);
        let removed = self.rope.slice(start..end).to_string();
        if removed.is_empty() && text.is_empty() {
            return Ok(String::new());
        }
        self.begin_undo_group(false);
        self.perform(Edit::replace(start, removed.clone(), text, self.point));
        let old_len = end - start;
        let new_len = text.chars().count();
        self.point = if self.point >= end {
            self.point - old_len + new_len
        } else if self.point > start {
            start + new_len
        } else {
            self.point
        };
        self.point = self.clamp(self.point);
        self.goal_column = None;
        self.commit_undo_group();
        Ok(removed)
    }

    /// Replaces the whole buffer, as `revert-buffer` does.
    pub fn replace_all(&mut self, text: &str) -> Result<()> {
        let all = Range::new(0, self.rope.len_chars());
        self.replace(all, &text.replace("\r\n", "\n")).map(|_| ())
    }

    // ---- undo ----------------------------------------------------------

    pub fn can_undo(&self) -> bool {
        self.undo.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.undo.can_redo()
    }

    /// `undo`: reverses the most recent group and restores point.
    pub fn undo(&mut self) -> Result<bool> {
        self.ensure_writable()?;
        self.open_groups = 0;
        self.undo.commit(self.point);
        let Some(group) = self.undo.undo() else {
            return Ok(false);
        };
        let inverse = group.invert();
        for edit in &inverse.edits {
            self.apply_raw(edit);
        }
        self.point = self.clamp(inverse.point_after);
        self.goal_column = None;
        Ok(true)
    }

    /// The history's shape, for the visualiser to draw.
    pub fn undo_shape(&self) -> Vec<crate::undo::TreeNode> {
        self.undo.shape()
    }

    /// Where the history is.
    pub fn undo_position(&self) -> usize {
        self.undo.position()
    }

    /// How many ways forward there are from here.
    pub fn undo_branches(&self) -> usize {
        self.undo.branches()
    }

    /// Chooses which branch a redo takes.
    pub fn set_undo_branch(&mut self, index: usize) -> bool {
        self.undo.set_branch(index)
    }

    /// Moves the buffer to another node of its history.
    ///
    /// The route is worked out first and applied as a whole, so a move that
    /// cannot be made leaves the buffer where it was rather than part-way.
    pub fn undo_go_to(&mut self, node: usize) -> Result<bool> {
        self.ensure_writable()?;
        self.open_groups = 0;
        self.undo.commit(self.point);
        let Some(groups) = self.undo.path_to(node) else {
            return Ok(false);
        };
        if groups.is_empty() {
            return Ok(false);
        }
        let mut point = self.point;
        for group in &groups {
            for edit in &group.edits {
                self.apply_raw(edit);
            }
            point = group.point_after;
        }
        self.undo.arrive_at(node);
        self.point = self.clamp(point);
        self.goal_column = None;
        Ok(true)
    }

    /// `undo-redo`: re-applies the most recently undone group.
    pub fn redo(&mut self) -> Result<bool> {
        self.ensure_writable()?;
        self.open_groups = 0;
        self.undo.commit(self.point);
        let Some(group) = self.undo.redo() else {
            return Ok(false);
        };
        for edit in &group.edits {
            self.apply_raw(edit);
        }
        self.point = self.clamp(group.point_after);
        self.goal_column = None;
        Ok(true)
    }

    /// Drops the undo history, as `buffer-disable-undo` does.
    pub fn clear_undo(&mut self) {
        self.open_groups = 0;
        self.undo.clear();
    }
}

/// Maps a file extension to the language identifier used for highlighting and
/// LSP server selection.
pub fn language_for_path(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    let by_name = match name {
        "Cargo.lock" => Some("toml"),
        "Makefile" | "makefile" | "GNUmakefile" => Some("make"),
        "Dockerfile" => Some("dockerfile"),
        _ => None,
    };
    if let Some(lang) = by_name {
        return Some(lang.to_string());
    }
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    let lang = match ext.as_str() {
        "rs" => "rust",
        "py" | "pyi" => "python",
        "js" | "mjs" | "cjs" | "jsx" => "javascript",
        "ts" | "tsx" => "typescript",
        "json" => "json",
        "c" | "h" => "c",
        "cc" | "cpp" | "cxx" | "hpp" | "hh" => "cpp",
        "sh" | "bash" | "zsh" => "bash",
        "html" | "htm" => "html",
        "css" => "css",
        "toml" => "toml",
        "md" | "markdown" => "markdown",
        "kdl" => "kdl",
        "go" => "go",
        "yml" | "yaml" => "yaml",
        _ => return None,
    };
    Some(lang.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buf(text: &str) -> Buffer {
        Buffer::from_str(BufferId(1), "test", text)
    }

    #[test]
    fn from_str_detects_and_normalises_line_endings() {
        let b = buf("a\r\nb\r\n");
        assert_eq!(b.line_ending(), LineEnding::Crlf);
        assert_eq!(b.text(), "a\nb\n", "stored as LF internally");
        assert_eq!(b.to_disk_string(), "a\r\nb\r\n", "restored on save");
        assert_eq!(buf("a\nb\n").line_ending(), LineEnding::Lf);
        assert_eq!(LineEnding::detect(""), LineEnding::Lf, "ties favour LF");
    }

    #[test]
    fn from_file_derives_name_and_language() {
        let b = Buffer::from_file(BufferId(2), "/tmp/src/main.rs", "fn main() {}");
        assert_eq!(b.name(), "main.rs");
        assert_eq!(b.language(), Some("rust"));
        assert_eq!(b.path().unwrap(), Path::new("/tmp/src/main.rs"));
    }

    #[test]
    fn language_detection_covers_names_and_extensions() {
        assert_eq!(
            language_for_path(Path::new("a/Cargo.lock")).as_deref(),
            Some("toml")
        );
        assert_eq!(
            language_for_path(Path::new("Makefile")).as_deref(),
            Some("make")
        );
        assert_eq!(
            language_for_path(Path::new("x.PY")).as_deref(),
            Some("python")
        );
        assert_eq!(language_for_path(Path::new("x.unknown")), None);
        assert_eq!(language_for_path(Path::new("noext")), None);
    }

    #[test]
    fn insert_moves_point_and_marks_the_buffer_modified() {
        let mut b = buf("");
        assert!(!b.is_modified());
        b.insert_at_point("hello").unwrap();
        assert_eq!(b.text(), "hello");
        assert_eq!(b.point(), 5);
        assert!(b.is_modified());
        b.mark_saved();
        assert!(!b.is_modified());
    }

    #[test]
    fn insert_before_point_pushes_point_forward() {
        let mut b = buf("world");
        b.set_point(5);
        b.insert(0, "hello ").unwrap();
        assert_eq!(b.text(), "hello world");
        assert_eq!(b.point(), 11);
    }

    #[test]
    fn insert_after_point_leaves_point_alone() {
        let mut b = buf("hello");
        b.set_point(0);
        b.insert(5, "!").unwrap();
        assert_eq!(b.point(), 0);
    }

    #[test]
    fn delete_returns_the_removed_text_and_repositions_point() {
        let mut b = buf("hello world");
        b.set_point(11);
        let removed = b.delete(Range::new(5, 11)).unwrap();
        assert_eq!(removed, " world");
        assert_eq!(b.text(), "hello");
        assert_eq!(b.point(), 5);
    }

    #[test]
    fn deleting_around_point_clamps_it_to_the_range_start() {
        let mut b = buf("abcdef");
        b.set_point(3);
        b.delete(Range::new(1, 5)).unwrap();
        assert_eq!(b.point(), 1);
        assert_eq!(b.text(), "af");
    }

    #[test]
    fn replace_adjusts_point_by_the_length_delta() {
        let mut b = buf("one two three");
        b.set_point(13);
        b.replace(Range::new(4, 7), "TWO!").unwrap();
        assert_eq!(b.text(), "one TWO! three");
        assert_eq!(b.point(), 14);
    }

    #[test]
    fn read_only_buffers_reject_every_mutation() {
        let mut b = buf("text");
        b.set_read_only(true);
        assert!(b.insert_at_point("x").is_err());
        assert!(b.delete(Range::new(0, 1)).is_err());
        assert!(b.replace(Range::new(0, 1), "y").is_err());
        assert_eq!(b.text(), "text");
    }

    #[test]
    fn undo_restores_text_and_point() {
        let mut b = buf("start");
        b.set_point(5);
        b.insert_at_point(" more").unwrap();
        assert_eq!(b.text(), "start more");
        assert!(b.undo().unwrap());
        assert_eq!(b.text(), "start");
        assert_eq!(b.point(), 5);
        assert!(b.redo().unwrap());
        assert_eq!(b.text(), "start more");
    }

    #[test]
    fn undo_on_a_fresh_buffer_reports_nothing_to_do() {
        let mut b = buf("x");
        assert!(!b.can_undo());
        assert!(!b.undo().unwrap());
        assert!(!b.redo().unwrap());
    }

    #[test]
    fn typed_characters_undo_as_one_group() {
        let mut b = buf("");
        for c in "word".chars() {
            b.insert_at_point(&c.to_string()).unwrap();
        }
        assert_eq!(b.text(), "word");
        b.undo().unwrap();
        assert_eq!(b.text(), "", "the whole run undoes together");
    }

    #[test]
    fn a_newline_breaks_the_amalgamation_run() {
        let mut b = buf("");
        b.insert_at_point("ab").unwrap();
        b.insert_at_point("\n").unwrap();
        b.insert_at_point("cd").unwrap();
        b.undo().unwrap();
        assert_eq!(b.text(), "ab\n");
        b.undo().unwrap();
        assert_eq!(b.text(), "ab");
    }

    #[test]
    fn mark_and_region_follow_emacs_semantics() {
        let mut b = buf("hello world");
        assert!(b.region().is_none());
        b.set_mark(0);
        b.set_point(5);
        assert_eq!(b.region(), Some(Range::new(0, 5)));
        b.deactivate_mark();
        assert!(b.region().is_none());
        assert_eq!(b.region_even_if_inactive(), Some(Range::new(0, 5)));
    }

    #[test]
    fn exchange_point_and_mark_swaps_and_reactivates() {
        let mut b = buf("hello world");
        b.set_mark(2);
        b.set_point(8);
        b.deactivate_mark();
        b.exchange_point_and_mark().unwrap();
        assert_eq!(b.point(), 2);
        assert_eq!(b.mark(), Some(8));
        assert!(b.is_mark_active());
    }

    #[test]
    fn exchange_without_a_mark_is_an_error() {
        let mut b = buf("x");
        assert!(matches!(
            b.exchange_point_and_mark(),
            Err(TextError::NoMark)
        ));
    }

    #[test]
    fn pushing_the_mark_leaves_something_to_come_back_to() {
        let mut b = buf("0123456789");
        b.set_point(4);
        // A command about to jump somewhere records where it was.
        b.push_mark(4);
        b.set_point(9);

        assert_eq!(b.mark(), Some(4));
        assert_eq!(b.pop_mark_ring(), Some(4), "the ring has the origin");
    }

    #[test]
    fn setting_the_mark_without_pushing_leaves_the_ring_alone() {
        let mut b = buf("0123456789");
        // `yank` marks where the insertion began, which is not a place worth
        // putting on the ring.
        b.set_mark_inactive(3);
        assert_eq!(b.mark(), Some(3));
        assert_eq!(b.pop_mark_ring(), None, "the ring is untouched");
    }

    #[test]
    fn repeated_pushes_stack_up_on_the_ring() {
        let mut b = buf("0123456789");
        b.push_mark(2);
        b.push_mark(5);
        b.set_point(9);
        assert_eq!(b.pop_mark_ring(), Some(5), "the most recent first");
        assert_eq!(b.pop_mark_ring(), Some(2));
    }

    #[test]
    fn the_mark_ring_records_previous_marks() {
        let mut b = buf("0123456789");
        b.set_mark(1);
        b.set_mark(5);
        b.set_point(9);
        assert_eq!(b.pop_mark_ring(), Some(1));
        assert!(
            b.pop_mark_ring().is_some(),
            "point was pushed on in exchange"
        );
    }

    #[test]
    fn the_mark_shifts_with_edits_before_it() {
        let mut b = buf("hello world");
        b.set_mark(6);
        b.insert(0, ">> ").unwrap();
        assert_eq!(b.mark(), Some(9));
        b.delete(Range::new(0, 3)).unwrap();
        assert_eq!(b.mark(), Some(6));
    }

    #[test]
    fn undo_moves_the_mark_with_the_text() {
        // Undo applies edits like any other change, so anything holding a
        // position has to follow them. It used to leave the mark behind, past
        // the end of a buffer that had shrunk back.
        let mut b = buf("");
        b.insert_at_point("hello world").unwrap();
        b.set_mark_inactive(11);
        assert_eq!(b.mark(), Some(11));

        b.undo().unwrap();
        assert_eq!(b.text(), "");
        assert_eq!(b.mark(), Some(0), "the mark followed the text away");
        assert!(b.mark().unwrap() <= b.len_chars());
    }

    #[test]
    fn redo_moves_the_mark_back_again() {
        let mut b = buf("");
        b.insert_at_point("alpha beta").unwrap();
        b.set_mark_inactive(6);
        b.undo().unwrap();
        b.redo().unwrap();
        assert_eq!(b.text(), "alpha beta");
        assert!(b.mark().unwrap() <= b.len_chars());
    }

    #[test]
    fn undo_moves_the_mark_ring_too() {
        // `C-u C-SPC` after an undo must not jump past the end either.
        let mut b = buf("0123456789");
        b.push_mark(9);
        b.set_point(0);
        b.delete(Range::new(0, 8)).unwrap();
        b.undo().unwrap();
        let popped = b.pop_mark_ring().expect("the ring still has the entry");
        assert!(popped <= b.len_chars(), "the ring entry is past the end");
    }

    #[test]
    fn a_mark_at_the_start_of_a_replacement_stays_put() {
        let mut b = buf("hello world");
        b.set_mark(0);
        b.set_point(5);
        b.replace(Range::new(0, 5), "HELLO").unwrap();
        assert_eq!(
            b.mark(),
            Some(0),
            "the mark anchors the region being replaced"
        );
        assert_eq!(b.region(), Some(Range::new(0, 5)));
    }

    #[test]
    fn a_mark_after_a_replacement_shifts_by_the_length_delta() {
        let mut b = buf("hello world");
        b.set_mark(11);
        b.replace(Range::new(0, 5), "hi").unwrap();
        assert_eq!(b.mark(), Some(8));
    }

    #[test]
    fn a_mark_inside_a_replacement_collapses_to_its_start() {
        let mut b = buf("hello world");
        b.set_mark(3);
        b.replace(Range::new(0, 5), "X").unwrap();
        assert_eq!(b.mark(), Some(0));
    }

    #[test]
    fn a_mark_inside_a_deleted_range_collapses_to_its_start() {
        let mut b = buf("abcdef");
        b.set_mark(4);
        b.delete(Range::new(2, 6)).unwrap();
        assert_eq!(b.mark(), Some(2));
    }

    #[test]
    fn positions_and_offsets_round_trip() {
        let b = buf("alpha\nbeta\ngamma");
        assert_eq!(b.position_of(0), Position::new(0, 0));
        assert_eq!(b.position_of(8), Position::new(1, 2));
        assert_eq!(b.offset_of(Position::new(1, 2)), 8);
        assert_eq!(
            b.offset_of(Position::new(1, 99)),
            10,
            "column clamps to line end"
        );
        assert_eq!(b.offset_of(Position::new(99, 0)), b.len_chars());
    }

    #[test]
    fn display_column_expands_tabs_to_tab_stops() {
        let mut b = buf("\tx\ty");
        b.set_tab_width(4);
        assert_eq!(b.display_column(0), 0);
        assert_eq!(b.display_column(1), 4, "tab fills to the next stop");
        assert_eq!(b.display_column(2), 5);
        assert_eq!(b.display_column(3), 8);
    }

    #[test]
    fn display_column_accounts_for_wide_characters() {
        let b = buf("漢字a");
        assert_eq!(b.display_column(1), 2);
        assert_eq!(b.display_column(2), 4);
        assert_eq!(b.display_column(3), 5);
    }

    #[test]
    fn offset_at_display_column_inverts_display_column() {
        let mut b = buf("\tabc");
        b.set_tab_width(4);
        assert_eq!(b.offset_at_display_column(0, 0), 0);
        assert_eq!(b.offset_at_display_column(0, 4), 1);
        assert_eq!(b.offset_at_display_column(0, 2), 0, "mid-tab snaps back");
        assert_eq!(b.offset_at_display_column(0, 99), 4, "clamps to line end");
    }

    #[test]
    fn line_text_strips_terminators() {
        let b = buf("one\ntwo\n");
        assert_eq!(b.line_text(0), "one");
        assert_eq!(b.line_text(1), "two");
        assert_eq!(b.line_text(9), "", "out of range lines are empty");
    }

    #[test]
    fn narrowing_restricts_point_and_widening_restores_it() {
        let mut b = buf("0123456789");
        b.narrow(Range::new(3, 7));
        assert!(b.is_narrowed());
        assert_eq!(b.point_min(), 3);
        assert_eq!(b.point_max(), 7);
        b.set_point(0);
        assert_eq!(b.point(), 3, "point clamps into the accessible portion");
        b.set_point(100);
        assert_eq!(b.point(), 7);
        b.widen();
        assert_eq!(b.point_min(), 0);
        assert_eq!(b.point_max(), 10);
    }

    #[test]
    fn typing_at_the_end_of_a_narrowed_region_stays_inside_it() {
        // Point stepping outside the accessible portion is what made a later
        // command build a backwards range and panic.
        let mut b = buf("0123456789");
        b.narrow(Range::new(0, 5));
        b.set_point(5);
        assert_eq!(b.point(), 5, "at the end of the region");

        b.insert_at_point("x").unwrap();
        assert_eq!(b.point_max(), 6, "the region grew to hold what was typed");
        assert_eq!(b.point(), 6);
        assert!(
            b.point() <= b.point_max(),
            "point must stay inside the region"
        );
        assert_eq!(b.slice(Range::new(b.point_min(), b.point_max())), "01234x");
    }

    #[test]
    fn point_stays_inside_the_region_after_any_edit() {
        for edit in ["insert at the end", "delete inside", "replace inside"] {
            let mut b = buf("0123456789");
            b.narrow(Range::new(2, 7));
            b.set_point(7);
            match edit {
                "insert at the end" => {
                    b.insert_at_point("ab").unwrap();
                }
                "delete inside" => {
                    b.delete(Range::new(3, 5)).unwrap();
                }
                _ => {
                    b.replace(Range::new(3, 6), "z").unwrap();
                }
            }
            assert!(
                b.point() >= b.point_min() && b.point() <= b.point_max(),
                "after `{edit}` point {} is outside {}..{}",
                b.point(),
                b.point_min(),
                b.point_max()
            );
        }
    }

    #[test]
    fn a_narrowed_region_never_ends_before_it_starts() {
        let mut b = buf("0123456789");
        b.narrow(Range::new(4, 6));
        // Remove everything, including the region itself.
        b.set_read_only(false);
        b.widen();
        b.narrow(Range::new(4, 6));
        b.delete(Range::new(0, 10)).unwrap();
        assert!(b.point_min() <= b.point_max(), "the region inverted");
    }

    #[test]
    fn narrowing_bounds_shift_with_edits_before_them() {
        let mut b = buf("0123456789");
        b.narrow(Range::new(3, 7));
        b.insert(0, "ab").unwrap();
        assert_eq!(b.point_min(), 5);
        assert_eq!(b.point_max(), 9);
    }

    #[test]
    fn edits_are_recorded_so_other_holders_of_a_position_can_follow() {
        let mut b = buf("hello world");
        assert!(b.take_adjustments().is_empty(), "nothing has happened yet");

        b.set_point(0);
        b.insert_at_point(">> ").unwrap();
        b.delete(Range::new(8, 14)).unwrap();
        let adjustments = b.take_adjustments();
        assert_eq!(adjustments, vec![(0, 0, 3), (8, 6, 0)]);
        assert!(b.take_adjustments().is_empty(), "taking them clears them");
    }

    #[test]
    fn a_position_moves_across_an_edit_the_way_point_does() {
        // Text inserted before it pushes it along.
        assert_eq!(Buffer::adjust_position(10, 0, 0, 3), 13);
        // Text removed before it pulls it back.
        assert_eq!(Buffer::adjust_position(10, 0, 4, 0), 6);
        // Text replaced before it shifts by the difference.
        assert_eq!(Buffer::adjust_position(10, 0, 4, 1), 7);
        // An edit after it leaves it alone.
        assert_eq!(Buffer::adjust_position(2, 5, 3, 0), 2);
        // A position inside what was removed collapses to where it began.
        assert_eq!(Buffer::adjust_position(7, 5, 4, 0), 5);
        // A position exactly at the edit is left where it is.
        assert_eq!(Buffer::adjust_position(5, 5, 0, 3), 8);
    }

    #[test]
    fn an_adjusted_position_agrees_with_what_happened_to_the_mark() {
        // The mark is adjusted inside the buffer; a position adjusted outside
        // it must end up in the same place, or the two would drift apart.
        for (at, removed, inserted, text) in [
            (0usize, 0usize, 3usize, ">> "),
            (2, 3, 0, ""),
            (4, 2, 5, "AAAAA"),
        ] {
            let mut b = buf("hello world");
            b.set_mark_inactive(9);
            b.replace(Range::new(at, at + removed), text).unwrap();
            assert_eq!(
                b.mark(),
                Some(Buffer::adjust_position(9, at, removed, inserted)),
                "edit at {at} removing {removed} inserting {inserted}"
            );
        }
    }

    #[test]
    fn revision_increments_on_every_applied_edit() {
        let mut b = buf("");
        assert_eq!(b.revision(), 0);
        b.insert_at_point("a").unwrap();
        b.insert_at_point("b").unwrap();
        assert_eq!(b.revision(), 2);
        b.undo().unwrap();
        assert!(b.revision() > 2, "undo is itself an applied edit");
    }

    #[test]
    fn char_before_and_after_handle_the_buffer_edges() {
        let b = buf("ab");
        assert_eq!(b.char_after(0), Some('a'));
        assert_eq!(b.char_after(2), None);
        assert_eq!(b.char_before(0), None);
        assert_eq!(b.char_before(2), Some('b'));
    }

    #[test]
    fn replace_all_reverts_the_whole_buffer() {
        let mut b = buf("old text");
        b.set_point(8);
        b.replace_all("new\r\ncontent").unwrap();
        assert_eq!(b.text(), "new\ncontent");
        assert!(b.point() <= b.len_chars());
    }

    #[test]
    fn empty_edits_are_no_ops() {
        let mut b = buf("text");
        b.insert(0, "").unwrap();
        assert_eq!(b.delete(Range::empty(2)).unwrap(), "");
        assert!(!b.can_undo(), "no undo group is recorded");
    }
}
