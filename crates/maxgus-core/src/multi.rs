//! Several cursors at once.
//!
//! One point is the real one — the window's — and the others are offsets kept
//! beside it. A command runs at the real point as it always has, and is then
//! run again at each of the others with point moved there and put back
//! afterwards. That is how `multiple-cursors` works in Emacs, and for the
//! same reason: it means every command that already exists works at every
//! cursor without knowing there is more than one.
//!
//! Not every command can be run that way. One that opens a prompt, switches
//! buffer or splits a window would be run several times over and mean
//! something absurd; those clear the cursors instead, which is what
//! `multiple-cursors` does when it meets a command it has not been told
//! about.

use maxgus_text::Range;

/// The extra cursors, sorted and distinct.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Cursors {
    offsets: Vec<usize>,
}

impl Cursors {
    pub fn new() -> Cursors {
        Cursors::default()
    }

    pub fn is_empty(&self) -> bool {
        self.offsets.is_empty()
    }

    pub fn len(&self) -> usize {
        self.offsets.len()
    }

    pub fn offsets(&self) -> &[usize] {
        &self.offsets
    }

    pub fn clear(&mut self) {
        self.offsets.clear();
    }

    /// Adds a cursor, keeping the list sorted and free of duplicates.
    ///
    /// `point` is passed so a cursor is never put where the real one already
    /// is: two cursors in one place would type every character twice.
    pub fn add(&mut self, offset: usize, point: usize) -> bool {
        if offset == point || self.offsets.contains(&offset) {
            return false;
        }
        let at = self.offsets.partition_point(|o| *o < offset);
        self.offsets.insert(at, offset);
        true
    }

    pub fn remove(&mut self, offset: usize) -> bool {
        let before = self.offsets.len();
        self.offsets.retain(|o| *o != offset);
        before != self.offsets.len()
    }

    /// Moves every cursor to account for an edit at `at` that changed the
    /// text length by `delta` characters.
    ///
    /// A cursor inside what was deleted collapses to where the deletion
    /// began; one after it shifts. Without this, editing at one cursor leaves
    /// every later cursor pointing at the wrong character.
    pub fn shift(&mut self, at: usize, removed: usize, inserted: usize) {
        for offset in &mut self.offsets {
            *offset = shift_offset(*offset, at, removed, inserted);
        }
        self.offsets.sort_unstable();
        self.offsets.dedup();
    }

    /// Moves every cursor by a net change in length at `at`.
    pub fn shift_by(&mut self, at: usize, delta: isize) {
        for offset in &mut self.offsets {
            *offset = shift_by_delta(*offset, at, delta);
        }
        self.offsets.sort_unstable();
        self.offsets.dedup();
    }

    /// Clamps every cursor into a buffer of `length` characters.
    pub fn clamp(&mut self, length: usize) {
        for offset in &mut self.offsets {
            *offset = (*offset).min(length);
        }
        self.offsets.sort_unstable();
        self.offsets.dedup();
    }

    /// The cursors, furthest through the buffer first.
    ///
    /// Running from the end means an edit at one cursor cannot move the ones
    /// still to be run: they are all before it.
    pub fn descending(&self) -> Vec<usize> {
        let mut out = self.offsets.clone();
        out.sort_unstable_by(|a, b| b.cmp(a));
        out
    }
}

/// Moves one offset to account for an edit at `at`.
///
/// The rule an offset has to follow when the text under it changes: before
/// the edit it does not move, inside what was deleted it collapses to where
/// the deletion began, and after it shifts by what the edit did.
pub fn shift_offset(offset: usize, at: usize, removed: usize, inserted: usize) -> usize {
    if offset <= at {
        return offset;
    }
    if offset <= at + removed {
        return at;
    }
    offset - removed + inserted
}

/// The same, from a net change in length: what a command that edited at `at`
/// did, when only the lengths before and after it are known.
pub fn shift_by_delta(offset: usize, at: usize, delta: isize) -> usize {
    match delta >= 0 {
        true => shift_offset(offset, at, 0, delta as usize),
        false => shift_offset(offset, at, delta.unsigned_abs(), 0),
    }
}

/// Where the next occurrence of `text` is, searching forward from `from` and
/// wrapping once.
///
/// Wrapping because `C->` past the last one should come back to the first
/// rather than stopping: a rename is usually a loop around the whole buffer.
pub fn next_occurrence(haystack: &str, text: &str, from: usize) -> Option<Range> {
    if text.is_empty() {
        return None;
    }
    let chars: Vec<char> = haystack.chars().collect();
    let needle: Vec<char> = text.chars().collect();
    if needle.len() > chars.len() {
        return None;
    }
    let last = chars.len() - needle.len();
    let found = (from..=last)
        .chain(0..from.min(last + 1))
        .find(|start| chars[*start..*start + needle.len()] == needle[..])?;
    Some(Range::new(found, found + needle.len()))
}

/// The same, searching backwards.
pub fn previous_occurrence(haystack: &str, text: &str, from: usize) -> Option<Range> {
    if text.is_empty() {
        return None;
    }
    let chars: Vec<char> = haystack.chars().collect();
    let needle: Vec<char> = text.chars().collect();
    if needle.len() > chars.len() {
        return None;
    }
    let last = chars.len() - needle.len();
    let found = (0..from.min(last + 1))
        .rev()
        .chain((from.min(last + 1)..=last).rev())
        .find(|start| chars[*start..*start + needle.len()] == needle[..])?;
    Some(Range::new(found, found + needle.len()))
}

/// Every occurrence of `text`, in order.
pub fn all_occurrences(haystack: &str, text: &str) -> Vec<Range> {
    if text.is_empty() {
        return Vec::new();
    }
    let chars: Vec<char> = haystack.chars().collect();
    let needle: Vec<char> = text.chars().collect();
    if needle.len() > chars.len() {
        return Vec::new();
    }
    (0..=chars.len() - needle.len())
        .filter(|start| chars[*start..*start + needle.len()] == needle[..])
        .map(|start| Range::new(start, start + needle.len()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursors_stay_sorted_and_distinct() {
        let mut cursors = Cursors::new();
        assert!(cursors.add(9, 0));
        assert!(cursors.add(3, 0));
        assert!(!cursors.add(3, 0), "a duplicate was taken");
        assert!(!cursors.add(0, 0), "a cursor was put on the real point");
        assert_eq!(cursors.offsets(), &[3, 9]);
    }

    #[test]
    fn an_edit_moves_the_cursors_after_it() {
        let mut cursors = Cursors::new();
        cursors.add(10, 0);
        cursors.add(20, 0);
        // Two characters typed at 5.
        cursors.shift(5, 0, 2);
        assert_eq!(cursors.offsets(), &[12, 22]);
    }

    #[test]
    fn an_edit_leaves_the_cursors_before_it_alone() {
        let mut cursors = Cursors::new();
        cursors.add(2, 0);
        cursors.shift(5, 0, 3);
        assert_eq!(cursors.offsets(), &[2]);
    }

    #[test]
    fn a_cursor_inside_a_deletion_collapses_to_where_it_began() {
        let mut cursors = Cursors::new();
        cursors.add(7, 0);
        cursors.add(20, 0);
        // Five characters deleted at 5, which covers the cursor at 7.
        cursors.shift(5, 5, 0);
        assert_eq!(cursors.offsets(), &[5, 15]);
    }

    #[test]
    fn cursors_are_clamped_into_the_buffer() {
        let mut cursors = Cursors::new();
        cursors.add(50, 0);
        cursors.add(60, 0);
        cursors.clamp(10);
        assert_eq!(cursors.offsets(), &[10], "they should collapse into one");
    }

    #[test]
    fn running_order_is_from_the_end_backwards() {
        // An edit at one cursor must not move the ones still to be run.
        let mut cursors = Cursors::new();
        cursors.add(3, 0);
        cursors.add(30, 0);
        cursors.add(11, 0);
        assert_eq!(cursors.descending(), vec![30, 11, 3]);
    }

    #[test]
    fn the_next_occurrence_is_found_forwards() {
        let text = "one two one two";
        let found = next_occurrence(text, "two", 0).expect("there is one");
        assert_eq!((found.start, found.end), (4, 7));
        let again = next_occurrence(text, "two", 5).expect("and another");
        assert_eq!((again.start, again.end), (12, 15));
    }

    #[test]
    fn searching_past_the_last_one_comes_back_to_the_first() {
        let text = "one two one";
        let found = next_occurrence(text, "one", 9).expect("it wraps");
        assert_eq!(found.start, 0);
    }

    #[test]
    fn the_previous_occurrence_is_found_backwards() {
        let text = "one two one two";
        let found = previous_occurrence(text, "two", 12).expect("there is one");
        assert_eq!(found.start, 4);
    }

    #[test]
    fn searching_back_past_the_first_wraps_to_the_last() {
        let text = "one two one";
        let found = previous_occurrence(text, "one", 0).expect("it wraps");
        assert_eq!(found.start, 8);
    }

    #[test]
    fn all_occurrences_are_found_in_order() {
        let found = all_occurrences("a b a b a", "a");
        assert_eq!(
            found.iter().map(|r| r.start).collect::<Vec<_>>(),
            vec![0, 4, 8]
        );
    }

    #[test]
    fn nothing_matches_nothing() {
        assert_eq!(next_occurrence("abc", "", 0), None);
        assert_eq!(all_occurrences("abc", ""), Vec::new());
        assert_eq!(next_occurrence("ab", "abcdef", 0), None);
    }

    #[test]
    fn occurrences_are_counted_in_characters_rather_than_bytes() {
        // A buffer counts characters; a byte offset into `é` is not a place.
        let text = "café café";
        let found = next_occurrence(text, "café", 1).expect("the second one");
        assert_eq!(found.start, 5, "it counted bytes");
    }
}
