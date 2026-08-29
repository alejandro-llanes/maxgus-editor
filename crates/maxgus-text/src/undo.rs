//! Grouped undo.
//!
//! Edits are collected into [`UndoGroup`]s that correspond to one user-visible
//! command. Consecutive self-inserting characters amalgamate into a single
//! group, so `C-/` after typing a word removes the whole word rather than one
//! letter at a time — matching Emacs' `amalgamating-undo-limit` behaviour.
//!
//! Redo is provided through an explicit redo stack (Emacs 28's `undo-redo`),
//! which is discarded as soon as a fresh edit lands.

use crate::edit::Edit;

/// The maximum number of self-inserted characters that amalgamate into one
/// undo group, mirroring Emacs' `amalgamating-undo-limit`.
pub const AMALGAMATING_UNDO_LIMIT: usize = 20;

/// One user-visible unit of change.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UndoGroup {
    pub edits: Vec<Edit>,
    pub point_before: usize,
    pub point_after: usize,
    /// True while the group may still absorb further self-insertions.
    pub amalgamating: bool,
}

impl UndoGroup {
    pub fn is_empty(&self) -> bool {
        self.edits.is_empty()
    }

    /// The group that reverses this one: inverted edits, in reverse order.
    pub fn invert(&self) -> UndoGroup {
        UndoGroup {
            edits: self.edits.iter().rev().map(Edit::invert).collect(),
            point_before: self.point_after,
            point_after: self.point_before,
            amalgamating: false,
        }
    }
}

/// Undo and redo history for a single buffer.
#[derive(Debug, Default)]
pub struct UndoStack {
    done: Vec<UndoGroup>,
    undone: Vec<UndoGroup>,
    /// The group currently accumulating edits, if a command is in flight.
    pending: Option<UndoGroup>,
    /// Saved-file watermark, used to recompute the modified flag when the user
    /// undoes back to the state on disk.
    saved_depth: Option<usize>,
}

impl UndoStack {
    pub fn new() -> Self {
        Self { saved_depth: Some(0), ..Default::default() }
    }

    /// Opens a new undo group. `amalgamating` groups may merge with the
    /// previous group when it is also amalgamating and still under the limit.
    pub fn begin(&mut self, point: usize, amalgamating: bool) {
        debug_assert!(self.pending.is_none(), "undo group already open");
        if amalgamating
            && let Some(prev) = self.done.last()
            && prev.amalgamating
            && prev.edits.len() < AMALGAMATING_UNDO_LIMIT
        {
            // Re-open the previous group so typing coalesces.
            let mut prev = self.done.pop().expect("checked by `last`");
            prev.point_after = point;
            self.pending = Some(prev);
            return;
        }
        self.pending =
            Some(UndoGroup { edits: Vec::new(), point_before: point, point_after: point, amalgamating });
    }

    /// True when a group is currently open.
    pub fn is_recording(&self) -> bool {
        self.pending.is_some()
    }

    /// Records an edit into the open group. Panics in debug builds if no group
    /// is open; in release it opens an implicit one so edits are never lost.
    pub fn push(&mut self, edit: Edit) {
        if self.pending.is_none() {
            self.begin(edit.point_before, false);
        }
        let group = self.pending.as_mut().expect("group opened above");
        group.edits.push(edit);
    }

    /// Closes the open group, discarding it when it produced no edits.
    pub fn commit(&mut self, point_after: usize) {
        let Some(mut group) = self.pending.take() else { return };
        if group.is_empty() {
            return;
        }
        group.point_after = point_after;
        self.done.push(group);
        // Any fresh edit invalidates the redo history.
        if !self.undone.is_empty() {
            self.undone.clear();
            // The saved state is no longer reachable by redoing forward.
            if self.saved_depth.is_some_and(|d| d > self.done.len()) {
                self.saved_depth = None;
            }
        }
    }

    /// Abandons the open group without recording it. The caller is responsible
    /// for having reverted the edits themselves.
    pub fn abort(&mut self) {
        self.pending = None;
    }

    /// Pops the most recent group so the caller can apply its inverse.
    pub fn undo(&mut self) -> Option<UndoGroup> {
        let group = self.done.pop()?;
        self.undone.push(group.clone());
        Some(group)
    }

    /// Pops the most recently undone group so the caller can re-apply it.
    pub fn redo(&mut self) -> Option<UndoGroup> {
        let group = self.undone.pop()?;
        self.done.push(group.clone());
        Some(group)
    }

    pub fn can_undo(&self) -> bool {
        !self.done.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.undone.is_empty()
    }

    /// Marks the current history depth as the on-disk state.
    pub fn mark_saved(&mut self) {
        self.saved_depth = Some(self.done.len());
        // Typing after a save has to start a *new* group. Amalgamation works
        // by popping the previous group off `done` and re-opening it, which
        // takes the length back below the watermark just recorded — so one
        // more character read as unmodified and `save-buffer` answered "(No
        // changes need to be saved)" with that character still only in the
        // buffer. Emacs draws an undo boundary here for the same reason.
        if let Some(last) = self.done.last_mut() {
            last.amalgamating = false;
        }
    }

    /// Declares the buffer different from its file without an edit to show for
    /// it, as changing the coding system does. The saved state is not at any
    /// depth any more, so no amount of undoing gets back to it.
    pub fn mark_modified(&mut self) {
        self.saved_depth = None;
    }

    /// True when the buffer differs from the last saved state.
    pub fn is_modified(&self) -> bool {
        match self.saved_depth {
            Some(depth) => depth != self.done.len(),
            None => true,
        }
    }

    /// Number of committed groups, exposed for tests and the mode line.
    pub fn depth(&self) -> usize {
        self.done.len()
    }

    /// Forgets all history, as `buffer-disable-undo` does.
    pub fn clear(&mut self) {
        self.done.clear();
        self.undone.clear();
        self.pending = None;
        self.saved_depth = Some(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::Edit;

    fn stack_with_two_groups() -> UndoStack {
        let mut s = UndoStack::new();
        s.begin(0, false);
        s.push(Edit::insert(0, "hello", 0));
        s.commit(5);
        s.begin(5, false);
        s.push(Edit::insert(5, " world", 5));
        s.commit(11);
        s
    }

    #[test]
    fn groups_round_trip_through_undo_and_redo() {
        let mut s = stack_with_two_groups();
        assert_eq!(s.depth(), 2);
        let g = s.undo().unwrap();
        assert_eq!(g.edits[0], Edit::insert(5, " world", 5));
        assert!(s.can_redo());
        let g = s.redo().unwrap();
        assert_eq!(g.edits[0], Edit::insert(5, " world", 5));
        assert_eq!(s.depth(), 2);
    }

    #[test]
    fn empty_groups_are_discarded() {
        let mut s = UndoStack::new();
        s.begin(0, false);
        s.commit(0);
        assert!(!s.can_undo());
        assert_eq!(s.depth(), 0);
    }

    #[test]
    fn self_insertions_amalgamate_up_to_the_limit() {
        let mut s = UndoStack::new();
        for i in 0..AMALGAMATING_UNDO_LIMIT {
            s.begin(i, true);
            s.push(Edit::insert(i, "x", i));
            s.commit(i + 1);
        }
        assert_eq!(s.depth(), 1, "all insertions merged into one group");
        // The next insertion exceeds the limit and starts a fresh group.
        s.begin(AMALGAMATING_UNDO_LIMIT, true);
        s.push(Edit::insert(AMALGAMATING_UNDO_LIMIT, "x", AMALGAMATING_UNDO_LIMIT));
        s.commit(AMALGAMATING_UNDO_LIMIT + 1);
        assert_eq!(s.depth(), 2);
    }

    #[test]
    fn non_amalgamating_groups_stay_separate() {
        let s = stack_with_two_groups();
        assert_eq!(s.depth(), 2);
    }

    #[test]
    fn inverted_group_reverses_edit_order() {
        let mut s = UndoStack::new();
        s.begin(0, false);
        s.push(Edit::insert(0, "a", 0));
        s.push(Edit::insert(1, "b", 1));
        s.commit(2);
        let inv = s.undo().unwrap().invert();
        assert_eq!(inv.edits[0], Edit::delete(1, "b", 1));
        assert_eq!(inv.edits[1], Edit::delete(0, "a", 0));
        assert_eq!(inv.point_after, 0);
    }

    #[test]
    fn new_edit_clears_the_redo_stack() {
        let mut s = stack_with_two_groups();
        s.undo();
        assert!(s.can_redo());
        s.begin(5, false);
        s.push(Edit::insert(5, "!", 5));
        s.commit(6);
        assert!(!s.can_redo());
    }

    #[test]
    fn a_buffer_can_be_declared_modified_without_an_edit() {
        // Changing the coding system alters what would be written without
        // altering the text, so there is no group to count.
        let mut s = UndoStack::new();
        assert!(!s.is_modified());
        s.mark_modified();
        assert!(s.is_modified());
    }

    #[test]
    fn undoing_cannot_undo_a_declared_modification() {
        // There is no depth the saved state sits at any more, so nothing gets
        // back to it but saving.
        let mut s = UndoStack::new();
        s.mark_modified();
        assert!(s.is_modified());
        s.mark_saved();
        assert!(!s.is_modified(), "saving is what clears it");
    }

    #[test]
    fn typing_after_a_save_still_counts_as_modified() {
        // Amalgamation re-opens the previous group by popping it off `done`,
        // which takes the length back below the saved watermark. Typing one
        // more character after saving then read as *unmodified*, and
        // `save-buffer` answered "(No changes need to be saved)" while the
        // character sat there unsaved.
        let mut s = UndoStack::new();
        s.begin(0, true);
        s.push(Edit::insert(0, "A", 0));
        s.commit(1);
        s.mark_saved();
        assert!(!s.is_modified(), "just saved");

        s.begin(1, true);
        s.push(Edit::insert(1, "B", 1));
        s.commit(2);
        assert!(s.is_modified(), "the second character has not been saved");
    }

    #[test]
    fn modified_flag_follows_the_saved_watermark() {
        let mut s = UndoStack::new();
        assert!(!s.is_modified());
        s.begin(0, false);
        s.push(Edit::insert(0, "a", 0));
        s.commit(1);
        assert!(s.is_modified());
        s.mark_saved();
        assert!(!s.is_modified());
        s.undo();
        assert!(s.is_modified(), "undoing past the save point re-modifies");
        s.redo();
        assert!(!s.is_modified(), "redoing back to it clears the flag again");
    }
}
