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

/// A node of the history: one group, and where it came from.
#[derive(Debug, Clone)]
struct Node {
    group: UndoGroup,
    /// `None` for the root, which is the buffer as it was to begin with.
    parent: Option<usize>,
    /// In the order they were made, so the most recent branch is the one a
    /// plain redo takes.
    children: Vec<usize>,
}

/// Undo history for a single buffer, as a tree.
///
/// Linear undo throws away the future the moment you type after undoing: the
/// text you undid past is gone, and no amount of redoing brings it back. A
/// tree keeps it. Undoing walks towards the root, redoing walks back down,
/// and typing after an undo starts a *branch* beside the one you left rather
/// than deleting it — so both versions of the paragraph are still there, and
/// `M-x undo-tree-visualize` is how you get to the other one.
///
/// The nodes are an arena: a tree of owned children would need either
/// reference counting or an unsafe cell to be walked upwards, and an index is
/// simpler than both.
#[derive(Debug, Default)]
pub struct UndoStack {
    nodes: Vec<Node>,
    /// Where in the history the buffer currently is. The root is 0 and is the
    /// state the buffer was opened in, which is why it holds no group.
    current: usize,
    /// The group currently accumulating edits, if a command is in flight.
    pending: Option<UndoGroup>,
    /// The node the file on disk corresponds to, when it is still reachable.
    saved: Option<usize>,
}

impl UndoStack {
    pub fn new() -> Self {
        Self {
            nodes: vec![Node {
                group: UndoGroup::default(),
                parent: None,
                children: Vec::new(),
            }],
            current: 0,
            pending: None,
            saved: Some(0),
        }
    }

    /// Opens a group, amalgamating into the last one when both ask for it.
    pub fn begin(&mut self, point: usize, amalgamating: bool) {
        if self.pending.is_some() {
            return;
        }
        // A run of self-insertions joins the group it is continuing, so `C-/`
        // takes back the word rather than the letter.
        // Taking the node back off the end is the only safe way to reopen
        // it: removing from the middle would renumber every node after it and
        // every parent and child index that names one. A run of typing always
        // ends at the last node, so the common case is covered, and anything
        // else starts a group of its own rather than risking the arena.
        let reopenable = amalgamating
            && self.current != 0
            && self.current + 1 == self.nodes.len()
            && self.nodes[self.current].group.amalgamating
            && self.nodes[self.current].group.edits.len() < AMALGAMATING_UNDO_LIMIT
            && self.nodes[self.current].children.is_empty();
        if reopenable {
            let node = self.nodes.pop().expect("checked to be the last");
            let parent = node.parent.unwrap_or(0);
            self.nodes[parent].children.retain(|c| *c != self.current);
            if self.saved == Some(self.current) {
                // What was saved is being unmade, so the buffer differs from
                // disk until the group is committed and compared again.
                self.saved = None;
            }
            self.current = parent;
            self.pending = Some(node.group);
            return;
        }
        self.pending = Some(UndoGroup {
            edits: Vec::new(),
            point_before: point,
            point_after: point,
            amalgamating,
        });
    }

    pub fn is_recording(&self) -> bool {
        self.pending.is_some()
    }

    /// Records one edit into the open group, opening one if none is.
    pub fn push(&mut self, edit: Edit) {
        if self.pending.is_none() {
            self.begin(edit.point_before, false);
        }
        let group = self.pending.as_mut().expect("group opened above");
        group.edits.push(edit);
    }

    /// Closes the open group, discarding it when it produced no edits.
    ///
    /// A group committed anywhere but at a leaf becomes a new branch: what
    /// was undone past stays where it was, reachable from the visualiser.
    pub fn commit(&mut self, point_after: usize) {
        let Some(mut group) = self.pending.take() else {
            return;
        };
        if group.is_empty() {
            return;
        }
        group.point_after = point_after;
        let id = self.nodes.len();
        self.nodes.push(Node {
            group,
            parent: Some(self.current),
            children: Vec::new(),
        });
        self.nodes[self.current].children.push(id);
        self.current = id;
    }

    /// Abandons the open group without recording it. The caller is responsible
    /// for having reverted the edits themselves.
    pub fn abort(&mut self) {
        self.pending = None;
    }

    /// Steps towards the root, returning the group whose inverse undoes it.
    pub fn undo(&mut self) -> Option<UndoGroup> {
        let node = self.nodes.get(self.current)?;
        let parent = node.parent?;
        let group = node.group.clone();
        self.current = parent;
        Some(group)
    }

    /// Steps back down the branch the history was last on.
    pub fn redo(&mut self) -> Option<UndoGroup> {
        let child = *self.nodes.get(self.current)?.children.last()?;
        self.current = child;
        Some(self.nodes[child].group.clone())
    }

    pub fn can_undo(&self) -> bool {
        self.nodes
            .get(self.current)
            .is_some_and(|node| node.parent.is_some())
    }

    pub fn can_redo(&self) -> bool {
        self.nodes
            .get(self.current)
            .is_some_and(|node| !node.children.is_empty())
    }

    // ---- the tree ------------------------------------------------------

    /// How many branches lead forward from where the history is.
    ///
    /// More than one means an undo was typed over, and the other version of
    /// the text is still there.
    pub fn branches(&self) -> usize {
        self.nodes
            .get(self.current)
            .map(|node| node.children.len())
            .unwrap_or(0)
    }

    /// Which branch a plain redo would take: the last one made.
    pub fn branch(&self) -> usize {
        self.branches().saturating_sub(1)
    }

    /// Chooses the branch a redo takes, by moving it to the end.
    ///
    /// The order is the history's own record of which was made when, so this
    /// is a rotation rather than a sort: picking a branch and then undoing
    /// past it and redoing again comes back to the one that was picked.
    pub fn set_branch(&mut self, index: usize) -> bool {
        let current = self.current;
        let Some(node) = self.nodes.get_mut(current) else {
            return false;
        };
        if index >= node.children.len() {
            return false;
        }
        let chosen = node.children.remove(index);
        node.children.push(chosen);
        true
    }

    /// Where the history is now.
    pub fn position(&self) -> usize {
        self.current
    }

    /// Moves to a node by its index, which is what the visualiser does.
    ///
    /// Returns the groups to apply, in order: the inverses of everything
    /// between here and the common ancestor, then the groups down to there.
    pub fn path_to(&self, target: usize) -> Option<Vec<UndoGroup>> {
        if target >= self.nodes.len() {
            return None;
        }
        let up = self.ancestry(self.current);
        let down = self.ancestry(target);
        // The deepest node they share.
        let meeting = up
            .iter()
            .find(|node| down.contains(node))
            .copied()
            .unwrap_or(0);
        let mut groups = Vec::new();
        for node in up.iter().take_while(|node| **node != meeting) {
            groups.push(self.nodes[*node].group.invert());
        }
        let descent: Vec<usize> = down
            .iter()
            .take_while(|node| **node != meeting)
            .copied()
            .collect();
        for node in descent.into_iter().rev() {
            groups.push(self.nodes[node].group.clone());
        }
        Some(groups)
    }

    /// Records that the history is now at `target`, after `path_to` was
    /// applied. Kept apart from `path_to` so the caller can fail to apply the
    /// groups without the history claiming to have moved.
    pub fn arrive_at(&mut self, target: usize) {
        if target < self.nodes.len() {
            self.current = target;
        }
    }

    /// A node and every ancestor, nearest first.
    fn ancestry(&self, mut node: usize) -> Vec<usize> {
        let mut out = vec![node];
        while let Some(parent) = self.nodes.get(node).and_then(|n| n.parent) {
            out.push(parent);
            node = parent;
        }
        out
    }

    /// The whole tree, for drawing: each node with its parent and depth.
    pub fn shape(&self) -> Vec<TreeNode> {
        let on_path = self.ancestry(self.current);
        self.nodes
            .iter()
            .enumerate()
            .map(|(id, node)| TreeNode {
                id,
                parent: node.parent,
                children: node.children.clone(),
                depth: self.ancestry(id).len() - 1,
                current: id == self.current,
                saved: self.saved == Some(id),
                on_current_path: on_path.contains(&id),
                edits: node.group.edits.len(),
            })
            .collect()
    }

    /// How many changes the history holds, which is what `C-h v` reports.
    pub fn depth(&self) -> usize {
        self.nodes.len().saturating_sub(1)
    }

    // ---- what the file on disk holds ------------------------------------

    /// Marks the current position as the on-disk state.
    ///
    /// A save is also an undo boundary, as it is in Emacs: without that, the
    /// next character typed would amalgamate into the group that was saved
    /// and take the marker with it, and the buffer could never be undone back
    /// to what is on disk.
    pub fn mark_saved(&mut self) {
        self.saved = Some(self.current);
        if let Some(node) = self.nodes.get_mut(self.current) {
            node.group.amalgamating = false;
        }
    }

    /// Says the buffer differs from disk however the history looks — what a
    /// change the history does not know about means.
    pub fn mark_modified(&mut self) {
        self.saved = None;
    }

    /// True when the buffer is not what the file holds.
    ///
    /// A tree makes this exact where a depth count could only guess: undoing
    /// back to the state that was saved, by whatever route, reaches the same
    /// node, and the buffer is not modified again.
    pub fn is_modified(&self) -> bool {
        self.pending.as_ref().is_some_and(|g| !g.is_empty()) || self.saved != Some(self.current)
    }

    pub fn clear(&mut self) {
        *self = UndoStack::new();
    }
}

/// One node of the history, as the visualiser draws it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeNode {
    pub id: usize,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub depth: usize,
    /// True for the node the buffer is at.
    pub current: bool,
    /// True for the node the file on disk holds.
    pub saved: bool,
    /// True for the nodes between the current one and the root.
    pub on_current_path: bool,
    pub edits: usize,
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
        s.push(Edit::insert(
            AMALGAMATING_UNDO_LIMIT,
            "x",
            AMALGAMATING_UNDO_LIMIT,
        ));
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

#[cfg(test)]
mod tree_tests {
    use super::*;
    use crate::edit::Edit;

    fn insertion(at: usize, text: &str) -> Edit {
        Edit::insert(at, text.to_string(), at)
    }

    /// The text an insertion carries, for telling branches apart.
    fn inserted(edit: &Edit) -> &str {
        match &edit.kind {
            crate::edit::EditKind::Insert { text } => text,
            crate::edit::EditKind::Delete { text } => text,
            crate::edit::EditKind::Replace { inserted, .. } => inserted,
        }
    }

    /// Records one whole group of one insertion.
    fn change(stack: &mut UndoStack, at: usize, text: &str) {
        stack.begin(at, false);
        stack.push(insertion(at, text));
        stack.commit(at + text.len());
    }

    #[test]
    fn undoing_and_typing_keeps_the_branch_that_was_left() {
        // Linear undo throws the first `b` away the moment `c` is typed. The
        // whole point of a tree is that it does not.
        let mut stack = UndoStack::new();
        change(&mut stack, 0, "a");
        change(&mut stack, 1, "b");
        stack.undo();
        change(&mut stack, 1, "c");

        assert_eq!(stack.depth(), 3, "the abandoned branch was thrown away");
        // Back to where the two branches part, and both are there.
        stack.undo();
        assert_eq!(stack.branches(), 2, "only one way forward");
    }

    #[test]
    fn a_plain_redo_takes_the_branch_that_was_made_last() {
        let mut stack = UndoStack::new();
        change(&mut stack, 0, "a");
        stack.undo();
        change(&mut stack, 0, "b");
        stack.undo();

        let group = stack.redo().expect("a branch to redo");
        assert_eq!(inserted(&group.edits[0]), "b", "it took the older branch");
    }

    #[test]
    fn a_branch_can_be_chosen_and_stays_chosen() {
        let mut stack = UndoStack::new();
        change(&mut stack, 0, "a");
        stack.undo();
        change(&mut stack, 0, "b");
        stack.undo();

        assert!(stack.set_branch(0), "there is a first branch");
        assert_eq!(inserted(&stack.redo().expect("a branch").edits[0]), "a");
        // And undoing past it and redoing comes back to the same one.
        stack.undo();
        assert_eq!(inserted(&stack.redo().expect("a branch").edits[0]), "a");
    }

    #[test]
    fn a_branch_that_is_not_there_is_refused() {
        let mut stack = UndoStack::new();
        change(&mut stack, 0, "a");
        assert!(!stack.set_branch(5));
    }

    #[test]
    fn moving_to_another_node_says_what_to_apply() {
        let mut stack = UndoStack::new();
        change(&mut stack, 0, "a"); // node 1
        stack.undo();
        change(&mut stack, 0, "b"); // node 2, a sibling of 1
        // From node 2 to node 1: undo `b`, then do `a`.
        let path = stack.path_to(1).expect("node 1 exists");
        assert_eq!(path.len(), 2);
        assert_eq!(
            inserted(&path[0].edits[0]),
            "b",
            "the first step is not the undo"
        );
        assert_eq!(inserted(&path[1].edits[0]), "a");
    }

    #[test]
    fn moving_to_where_it_already_is_says_to_apply_nothing() {
        let mut stack = UndoStack::new();
        change(&mut stack, 0, "a");
        assert_eq!(stack.path_to(stack.position()), Some(Vec::new()));
    }

    #[test]
    fn moving_to_a_node_that_does_not_exist_is_refused() {
        let stack = UndoStack::new();
        assert_eq!(stack.path_to(99), None);
    }

    #[test]
    fn the_shape_says_which_node_is_where() {
        let mut stack = UndoStack::new();
        change(&mut stack, 0, "a");
        stack.undo();
        change(&mut stack, 0, "b");

        let shape = stack.shape();
        assert_eq!(shape.len(), 3, "root and two branches");
        assert_eq!(shape[0].parent, None, "the root has no parent");
        assert_eq!(shape[1].parent, Some(0));
        assert_eq!(shape[2].parent, Some(0));
        assert!(shape[2].current, "the current node is the one just made");
        assert!(shape[2].on_current_path && shape[0].on_current_path);
        assert!(
            !shape[1].on_current_path,
            "the other branch is not the path"
        );
        assert_eq!(shape[1].depth, 1);
    }

    #[test]
    fn amalgamation_only_reopens_the_node_it_can_take_back() {
        // Reopening works by popping the last node. A node that is not last
        // must start a fresh group instead, or every index after it moves.
        let mut stack = UndoStack::new();
        stack.begin(0, true);
        stack.push(insertion(0, "a"));
        stack.commit(1); // node 1, amalgamating
        stack.undo();
        change(&mut stack, 0, "z"); // node 2, so node 1 is no longer last
        stack.undo();
        assert!(stack.set_branch(0), "back onto the first branch");
        stack.redo(); // at node 1 again, which is not the last node

        stack.begin(1, true);
        stack.push(insertion(1, "b"));
        stack.commit(2);

        // The tree still describes itself: every parent and child exists and
        // agrees, which is what popping the wrong node would have broken.
        let shape = stack.shape();
        assert_eq!(shape.len(), 4, "a node was taken out from under the others");
        for node in &shape {
            if let Some(parent) = node.parent {
                assert!(
                    shape[parent].children.contains(&node.id),
                    "node {} says its parent is {parent}, which disowns it",
                    node.id
                );
            }
            for child in &node.children {
                assert_eq!(
                    shape[*child].parent,
                    Some(node.id),
                    "node {} claims child {child}, which disagrees",
                    node.id
                );
            }
        }
    }

    #[test]
    fn a_run_of_typing_still_amalgamates_into_one_group() {
        let mut stack = UndoStack::new();
        for (n, letter) in ["a", "b", "c"].iter().enumerate() {
            stack.begin(n, true);
            stack.push(insertion(n, letter));
            stack.commit(n + 1);
        }
        assert_eq!(stack.depth(), 1, "typing three letters made three groups");
        let group = stack.undo().expect("one group");
        assert_eq!(group.edits.len(), 3);
    }

    #[test]
    fn undoing_back_to_what_was_saved_is_not_modified_again() {
        let mut stack = UndoStack::new();
        change(&mut stack, 0, "a");
        stack.mark_saved();
        change(&mut stack, 1, "b");
        assert!(stack.is_modified());
        stack.undo();
        assert!(!stack.is_modified(), "it came back to what is on disk");
    }

    #[test]
    fn reaching_the_saved_state_along_another_branch_still_counts() {
        // A tree can say this exactly where a depth count could only guess:
        // the saved node is a node, and being at it is being at it.
        let mut stack = UndoStack::new();
        change(&mut stack, 0, "a");
        stack.mark_saved();
        change(&mut stack, 1, "b");
        stack.undo();
        change(&mut stack, 1, "c"); // a second branch from the saved node
        assert!(stack.is_modified());
        stack.undo();
        assert!(!stack.is_modified(), "the saved node is the saved node");
    }
}

#[cfg(test)]
mod boundary_tests {
    use super::*;
    use crate::edit::Edit;

    #[test]
    fn saving_ends_the_group_so_the_next_keystroke_starts_another() {
        // Without this the run of typing that was saved is reopened by the
        // next character, and the node that was on disk stops existing.
        let mut stack = UndoStack::new();
        stack.begin(0, true);
        stack.push(Edit::insert(0, "a".to_string(), 0));
        stack.commit(1);
        stack.mark_saved();

        stack.begin(1, true);
        stack.push(Edit::insert(1, "b".to_string(), 1));
        stack.commit(2);

        assert_eq!(stack.depth(), 2, "the save was not a boundary");
        assert!(stack.is_modified());
        stack.undo();
        assert!(
            !stack.is_modified(),
            "it cannot get back to what is on disk"
        );
    }
}
