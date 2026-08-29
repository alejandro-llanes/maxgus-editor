//! The buffer list.
//!
//! Every open buffer lives here, together with the most-recently-used ordering
//! that `C-x b` offers and `C-x <right>` walks. Naming follows Emacs: a buffer
//! visiting a file is named after the file, and a collision is resolved by
//! appending `<2>`, `<3>` and so on.

use maxgus_text::{Buffer, BufferId};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The name Emacs gives the buffer that exists when nothing else does.
pub const SCRATCH_NAME: &str = "*scratch*";

/// The greeting `*scratch*` starts with.
pub const SCRATCH_MESSAGE: &str = "\
;; This buffer is for text that is not saved.
;; To create a file, visit it with C-x C-f.

";

/// Every buffer, plus the order they were last selected in.
#[derive(Debug)]
pub struct BufferList {
    buffers: BTreeMap<BufferId, Buffer>,
    /// Most recently used first.
    order: Vec<BufferId>,
    next_id: u64,
}

impl Default for BufferList {
    fn default() -> BufferList {
        BufferList::new()
    }
}

impl BufferList {
    /// A list holding just `*scratch*`.
    pub fn new() -> BufferList {
        let mut list = BufferList { buffers: BTreeMap::new(), order: Vec::new(), next_id: 1 };
        let id = list.allocate_id();
        let mut scratch = Buffer::from_str(id, SCRATCH_NAME, SCRATCH_MESSAGE);
        scratch.set_point(scratch.len_chars());
        // The greeting is not an edit the user made.
        scratch.clear_undo();
        list.insert(scratch);
        list
    }

    fn allocate_id(&mut self) -> BufferId {
        let id = BufferId(self.next_id);
        self.next_id += 1;
        id
    }

    fn insert(&mut self, buffer: Buffer) -> BufferId {
        let id = buffer.id;
        self.buffers.insert(id, buffer);
        self.order.insert(0, id);
        id
    }

    pub fn len(&self) -> usize {
        self.buffers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buffers.is_empty()
    }

    pub fn get(&self, id: BufferId) -> Option<&Buffer> {
        self.buffers.get(&id)
    }

    pub fn get_mut(&mut self, id: BufferId) -> Option<&mut Buffer> {
        self.buffers.get_mut(&id)
    }

    /// Buffers in most-recently-used order.
    pub fn iter(&self) -> impl Iterator<Item = &Buffer> {
        self.order.iter().filter_map(move |id| self.buffers.get(id))
    }

    /// Buffer ids in most-recently-used order.
    pub fn ids(&self) -> &[BufferId] {
        &self.order
    }

    /// Buffer names in most-recently-used order, for completion.
    pub fn names(&self) -> Vec<String> {
        self.iter().map(|b| b.name().to_string()).collect()
    }

    /// Names of buffers a user would want offered by `C-x b`: internal ones,
    /// whose names begin with a space, are left out as Emacs leaves them out.
    pub fn visible_names(&self) -> Vec<String> {
        self.iter().filter(|b| !b.name().starts_with(' ')).map(|b| b.name().to_string()).collect()
    }

    pub fn find_by_name(&self, name: &str) -> Option<BufferId> {
        self.iter().find(|b| b.name() == name).map(|b| b.id)
    }

    pub fn find_by_path(&self, path: &Path) -> Option<BufferId> {
        self.iter().find(|b| b.path() == Some(path)).map(|b| b.id)
    }

    /// Marks `id` as most recently used.
    pub fn touch(&mut self, id: BufferId) {
        if !self.buffers.contains_key(&id) {
            return;
        }
        self.order.retain(|existing| *existing != id);
        self.order.insert(0, id);
    }

    /// The buffer `C-x b` would offer by default: the most recently used one
    /// that is not `current`.
    pub fn other(&self, current: BufferId) -> Option<BufferId> {
        self.order.iter().find(|id| **id != current).copied()
    }

    /// `next-buffer`: the one after `current` in the list, wrapping.
    pub fn next(&self, current: BufferId) -> Option<BufferId> {
        self.step(current, 1)
    }

    /// `previous-buffer`.
    pub fn previous(&self, current: BufferId) -> Option<BufferId> {
        self.step(current, -1)
    }

    fn step(&self, current: BufferId, delta: i32) -> Option<BufferId> {
        if self.order.is_empty() {
            return None;
        }
        let position = self.order.iter().position(|id| *id == current)? as i32;
        let index = (position + delta).rem_euclid(self.order.len() as i32) as usize;
        Some(self.order[index])
    }

    /// A name not already taken, appending `<2>`, `<3>` … as Emacs does.
    pub fn unique_name(&self, base: &str) -> String {
        if self.find_by_name(base).is_none() {
            return base.to_string();
        }
        // Start at two: the first duplicate is `<2>`.
        (2..).map(|n| format!("{base}<{n}>")).find(|name| self.find_by_name(name).is_none()).expect(
            "the sequence is unbounded, so some name is always free",
        )
    }

    /// Creates an empty buffer, uniquifying `name` if it is taken.
    pub fn create(&mut self, name: &str) -> BufferId {
        let id = self.allocate_id();
        let name = self.unique_name(name);
        self.insert(Buffer::new(id, name))
    }

    /// Creates a buffer holding `text`.
    pub fn create_with_text(&mut self, name: &str, text: &str) -> BufferId {
        let id = self.allocate_id();
        let name = self.unique_name(name);
        self.insert(Buffer::from_str(id, name, text))
    }

    /// Visits `path`. A buffer already visiting it is reused and returned
    /// unchanged, which is what makes `C-x C-f` on an open file switch to it
    /// rather than re-reading from disk.
    pub fn visit_file(&mut self, path: impl Into<PathBuf>, contents: &str) -> BufferId {
        let path = path.into();
        if let Some(existing) = self.find_by_path(&path) {
            self.touch(existing);
            return existing;
        }
        let id = self.allocate_id();
        let mut buffer = Buffer::from_file(id, path, contents);
        // `from_file` names the buffer after the file; two files with the same
        // base name need distinguishing.
        let unique = self.unique_name(buffer.name());
        buffer.set_name(unique);
        self.insert(buffer)
    }

    /// Replaces a buffer's contents from disk, as `revert-buffer` does.
    pub fn revert(&mut self, id: BufferId, contents: &str) -> crate::Result<()> {
        let buffer = self.get_mut(id).ok_or(crate::CoreError::NoSuchBuffer)?;
        let was_read_only = buffer.is_read_only();
        buffer.set_read_only(false);
        buffer.replace_all(contents)?;
        buffer.clear_undo();
        buffer.mark_saved();
        buffer.set_read_only(was_read_only);
        Ok(())
    }

    /// `kill-buffer`. Killing the last buffer is refused; killing the last
    /// *file* buffer leaves `*scratch*` behind, which is what Emacs does.
    ///
    /// Returns the buffer that should be displayed in its place.
    pub fn kill(&mut self, id: BufferId) -> crate::Result<BufferId> {
        if !self.buffers.contains_key(&id) {
            return Err(crate::CoreError::NoSuchBuffer);
        }
        if self.buffers.len() == 1 {
            return Err(crate::CoreError::LastBuffer);
        }
        self.buffers.remove(&id);
        self.order.retain(|existing| *existing != id);
        Ok(*self.order.first().expect("at least one buffer remains"))
    }

    /// Buffers with unsaved changes and a file to save them to.
    pub fn modified(&self) -> Vec<BufferId> {
        self.iter().filter(|b| b.is_modified() && b.path().is_some()).map(|b| b.id).collect()
    }

    /// True when any buffer would lose work if the editor exited now.
    pub fn has_unsaved_changes(&self) -> bool {
        !self.modified().is_empty()
    }

    /// Renames a buffer, uniquifying if the new name is taken.
    pub fn rename(&mut self, id: BufferId, name: &str) -> crate::Result<String> {
        if !self.buffers.contains_key(&id) {
            return Err(crate::CoreError::NoSuchBuffer);
        }
        // A buffer keeping its own name must not collide with itself.
        let unique = if self.get(id).is_some_and(|b| b.name() == name) {
            name.to_string()
        } else {
            self.unique_name(name)
        };
        self.get_mut(id).expect("checked above").set_name(unique.clone());
        Ok(unique)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list() -> BufferList {
        BufferList::new()
    }

    fn scratch(list: &BufferList) -> BufferId {
        list.find_by_name(SCRATCH_NAME).expect("scratch always exists")
    }

    #[test]
    fn a_new_list_holds_only_scratch() {
        let l = list();
        assert_eq!(l.len(), 1);
        let id = scratch(&l);
        let buffer = l.get(id).unwrap();
        assert_eq!(buffer.name(), SCRATCH_NAME);
        assert!(buffer.path().is_none());
        assert!(!buffer.is_modified(), "the greeting is not an unsaved edit");
        assert_eq!(buffer.point(), buffer.len_chars(), "point starts after the greeting");
    }

    #[test]
    fn creating_a_buffer_adds_it_at_the_front() {
        let mut l = list();
        let id = l.create("notes");
        assert_eq!(l.len(), 2);
        assert_eq!(l.ids().first(), Some(&id), "newest is most recently used");
        assert_eq!(l.get(id).unwrap().name(), "notes");
    }

    #[test]
    fn buffer_names_are_uniquified_with_angle_brackets() {
        let mut l = list();
        l.create("main.rs");
        let second = l.create("main.rs");
        let third = l.create("main.rs");
        assert_eq!(l.get(second).unwrap().name(), "main.rs<2>");
        assert_eq!(l.get(third).unwrap().name(), "main.rs<3>");
    }

    #[test]
    fn visiting_a_file_names_the_buffer_after_it() {
        let mut l = list();
        let id = l.visit_file("/project/src/main.rs", "fn main() {}");
        let buffer = l.get(id).unwrap();
        assert_eq!(buffer.name(), "main.rs");
        assert_eq!(buffer.path().unwrap(), Path::new("/project/src/main.rs"));
        assert_eq!(buffer.text(), "fn main() {}");
        assert_eq!(buffer.language(), Some("rust"));
    }

    #[test]
    fn two_files_with_the_same_base_name_are_distinguished() {
        let mut l = list();
        l.visit_file("/a/main.rs", "");
        let second = l.visit_file("/b/main.rs", "");
        assert_eq!(l.get(second).unwrap().name(), "main.rs<2>");
    }

    #[test]
    fn visiting_an_already_open_file_reuses_its_buffer() {
        let mut l = list();
        let first = l.visit_file("/a/main.rs", "original");
        l.get_mut(first).unwrap().insert_at_point(" edited").unwrap();

        let again = l.visit_file("/a/main.rs", "contents from disk");
        assert_eq!(again, first, "the same buffer is returned");
        assert_eq!(l.len(), 2, "no second buffer was created");
        assert!(
            l.get(first).unwrap().text().contains("edited"),
            "the open buffer's edits survive"
        );
    }

    #[test]
    fn visiting_an_open_file_makes_it_most_recently_used() {
        let mut l = list();
        let file = l.visit_file("/a/main.rs", "");
        l.create("other");
        assert_ne!(l.ids().first(), Some(&file));
        l.visit_file("/a/main.rs", "");
        assert_eq!(l.ids().first(), Some(&file));
    }

    #[test]
    fn touching_reorders_the_list() {
        let mut l = list();
        let a = l.create("a");
        let b = l.create("b");
        assert_eq!(l.ids()[0], b);
        l.touch(a);
        assert_eq!(l.ids()[0], a);
        assert_eq!(l.len(), 3, "touching does not add or remove");
    }

    #[test]
    fn touching_an_unknown_buffer_does_nothing() {
        let mut l = list();
        let before = l.ids().to_vec();
        l.touch(BufferId(999));
        assert_eq!(l.ids(), before.as_slice());
    }

    #[test]
    fn the_other_buffer_is_the_most_recent_one_that_is_not_current() {
        let mut l = list();
        let a = l.create("a");
        let b = l.create("b");
        assert_eq!(l.other(b), Some(a));
        assert_eq!(l.other(a), Some(b));
    }

    #[test]
    fn there_is_no_other_buffer_when_only_one_exists() {
        let l = list();
        assert_eq!(l.other(scratch(&l)), None);
    }

    #[test]
    fn next_and_previous_walk_the_list_and_wrap() {
        let mut l = list();
        let a = l.create("a");
        let b = l.create("b");
        let s = scratch(&l);
        // Order is b, a, *scratch*.
        assert_eq!(l.next(b), Some(a));
        assert_eq!(l.next(a), Some(s));
        assert_eq!(l.next(s), Some(b), "wraps around");
        assert_eq!(l.previous(b), Some(s), "and backwards");
    }

    #[test]
    fn stepping_from_an_unknown_buffer_yields_nothing() {
        let l = list();
        assert_eq!(l.next(BufferId(999)), None);
    }

    #[test]
    fn killing_returns_the_buffer_to_display_instead() {
        let mut l = list();
        let a = l.create("a");
        let b = l.create("b");
        let replacement = l.kill(b).unwrap();
        assert_eq!(replacement, a, "the next most recently used");
        assert_eq!(l.len(), 2);
        assert!(l.get(b).is_none());
    }

    #[test]
    fn the_last_buffer_cannot_be_killed() {
        let mut l = list();
        assert!(matches!(l.kill(scratch(&l)), Err(crate::CoreError::LastBuffer)));
        assert_eq!(l.len(), 1);
    }

    #[test]
    fn killing_an_unknown_buffer_is_an_error() {
        let mut l = list();
        l.create("a");
        assert!(matches!(l.kill(BufferId(999)), Err(crate::CoreError::NoSuchBuffer)));
    }

    #[test]
    fn a_killed_name_becomes_available_again() {
        let mut l = list();
        let a = l.create("notes");
        l.kill(a).unwrap();
        let b = l.create("notes");
        assert_eq!(l.get(b).unwrap().name(), "notes", "no `<2>` suffix");
    }

    #[test]
    fn reverting_replaces_the_contents_and_clears_the_modified_flag() {
        let mut l = list();
        let id = l.visit_file("/a/main.rs", "original");
        l.get_mut(id).unwrap().insert_at_point("edited ").unwrap();
        assert!(l.get(id).unwrap().is_modified());

        l.revert(id, "from disk").unwrap();
        let buffer = l.get(id).unwrap();
        assert_eq!(buffer.text(), "from disk");
        assert!(!buffer.is_modified());
        assert!(!buffer.can_undo(), "the revert is not undoable");
    }

    #[test]
    fn reverting_works_on_a_read_only_buffer_and_leaves_it_read_only() {
        let mut l = list();
        let id = l.visit_file("/a/main.rs", "original");
        l.get_mut(id).unwrap().set_read_only(true);
        l.revert(id, "from disk").unwrap();
        let buffer = l.get(id).unwrap();
        assert_eq!(buffer.text(), "from disk");
        assert!(buffer.is_read_only());
    }

    #[test]
    fn reverting_an_unknown_buffer_is_an_error() {
        let mut l = list();
        assert!(matches!(l.revert(BufferId(999), ""), Err(crate::CoreError::NoSuchBuffer)));
    }

    #[test]
    fn unsaved_changes_are_reported_only_for_file_buffers() {
        let mut l = list();
        let file = l.visit_file("/a/main.rs", "");
        let plain = l.create("notes");

        assert!(!l.has_unsaved_changes());
        l.get_mut(plain).unwrap().insert_at_point("typed").unwrap();
        assert!(
            !l.has_unsaved_changes(),
            "a buffer with no file has nowhere to be saved to"
        );

        l.get_mut(file).unwrap().insert_at_point("typed").unwrap();
        assert_eq!(l.modified(), vec![file]);
        assert!(l.has_unsaved_changes());

        l.get_mut(file).unwrap().mark_saved();
        assert!(!l.has_unsaved_changes());
    }

    #[test]
    fn renaming_uniquifies_against_existing_names() {
        let mut l = list();
        l.create("taken");
        let id = l.create("other");
        assert_eq!(l.rename(id, "taken").unwrap(), "taken<2>");
        assert_eq!(l.get(id).unwrap().name(), "taken<2>");
    }

    #[test]
    fn renaming_a_buffer_to_its_own_name_is_a_no_op() {
        let mut l = list();
        let id = l.create("notes");
        assert_eq!(l.rename(id, "notes").unwrap(), "notes", "no `<2>` against itself");
    }

    #[test]
    fn renaming_an_unknown_buffer_is_an_error() {
        let mut l = list();
        assert!(matches!(l.rename(BufferId(999), "x"), Err(crate::CoreError::NoSuchBuffer)));
    }

    #[test]
    fn internal_buffers_are_hidden_from_completion() {
        let mut l = list();
        l.create("visible");
        l.create(" internal");
        assert!(l.names().iter().any(|n| n == " internal"));
        assert!(!l.visible_names().iter().any(|n| n == " internal"));
        assert!(l.visible_names().iter().any(|n| n == "visible"));
        assert!(l.visible_names().iter().any(|n| n == SCRATCH_NAME));
    }

    #[test]
    fn buffers_can_be_found_by_name_and_by_path() {
        let mut l = list();
        let id = l.visit_file("/a/main.rs", "");
        assert_eq!(l.find_by_name("main.rs"), Some(id));
        assert_eq!(l.find_by_path(Path::new("/a/main.rs")), Some(id));
        assert_eq!(l.find_by_name("nonexistent"), None);
        assert_eq!(l.find_by_path(Path::new("/nowhere")), None);
    }

    #[test]
    fn buffer_ids_are_never_reused() {
        let mut l = list();
        let a = l.create("a");
        l.kill(a).unwrap();
        let b = l.create("b");
        assert_ne!(a, b, "a fresh id is allocated even after a kill");
    }
}
