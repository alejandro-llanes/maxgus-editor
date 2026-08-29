//! Highlight spans and the flattening rule that resolves nested captures.

/// A run of bytes drawn in one face.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Highlight {
    /// Byte offset into the buffer text.
    pub start: usize,
    pub end: usize,
    /// The face name this span is drawn in.
    pub face: &'static str,
}

impl Highlight {
    pub fn new(start: usize, end: usize, face: &'static str) -> Self {
        Self { start, end, face }
    }

    pub fn len(&self) -> usize {
        self.end.saturating_sub(self.start)
    }

    pub fn is_empty(&self) -> bool {
        self.start >= self.end
    }

    pub fn contains(&self, byte: usize) -> bool {
        byte >= self.start && byte < self.end
    }
}

/// A capture before nesting has been resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capture {
    pub start: usize,
    pub end: usize,
    pub face: &'static str,
    /// Index of the query pattern that produced this capture. Lower wins when
    /// two patterns capture exactly the same range, matching tree-sitter's own
    /// "first pattern wins" rule.
    pub pattern: usize,
}

/// Resolves overlapping captures into a flat, ordered, non-overlapping list.
///
/// Tree-sitter captures nest: an `identifier` inside a `call_expression` yields
/// two captures covering overlapping ranges. The inner, more specific capture
/// should win over the part it covers, with the outer one showing through on
/// either side. That is what this does.
pub fn flatten(mut captures: Vec<Capture>) -> Vec<Highlight> {
    if captures.is_empty() {
        return Vec::new();
    }
    // Outer spans first at a given start, so the stack nests correctly.
    captures.sort_by(|a, b| {
        a.start
            .cmp(&b.start)
            .then(b.end.cmp(&a.end))
            .then(a.pattern.cmp(&b.pattern))
    });
    // Identical ranges: keep only the first, earliest-pattern capture.
    captures.dedup_by(|a, b| a.start == b.start && a.end == b.end);

    let mut out: Vec<Highlight> = Vec::with_capacity(captures.len());
    // Open spans, outermost at the bottom.
    let mut stack: Vec<(usize, &'static str)> = Vec::new();
    let mut pos = 0usize;

    let emit = |out: &mut Vec<Highlight>, from: usize, to: usize, face: &'static str| {
        if from >= to {
            return;
        }
        // Merge with the previous span when it is the same face and adjacent.
        match out.last_mut() {
            Some(last) if last.end == from && last.face == face => last.end = to,
            _ => out.push(Highlight::new(from, to, face)),
        }
    };

    for cap in captures {
        // Close every span that ends before this capture starts.
        while let Some(&(end, face)) = stack.last() {
            if end > cap.start {
                break;
            }
            emit(&mut out, pos, end, face);
            pos = pos.max(end);
            stack.pop();
        }
        // The enclosing span shows through up to where this capture begins.
        if let Some(&(_, face)) = stack.last() {
            emit(&mut out, pos, cap.start, face);
        }
        pos = pos.max(cap.start);
        // A capture that the enclosing span has already run past is dead.
        if cap.end > pos {
            stack.push((cap.end, cap.face));
        }
    }
    while let Some((end, face)) = stack.pop() {
        emit(&mut out, pos, end, face);
        pos = pos.max(end);
    }
    out
}

/// A buffer edit, in the terms tree-sitter needs to patch a tree.
///
/// This mirrors `tree_sitter::InputEdit` but is expressed in bytes only; the
/// row/column pair is derived by the highlighter, which has the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputEdit {
    pub start_byte: usize,
    pub old_end_byte: usize,
    pub new_end_byte: usize,
}

impl InputEdit {
    pub fn new(start_byte: usize, old_end_byte: usize, new_end_byte: usize) -> Self {
        Self {
            start_byte,
            old_end_byte,
            new_end_byte,
        }
    }

    /// An insertion of `len` bytes at `at`.
    pub fn insertion(at: usize, len: usize) -> Self {
        Self::new(at, at, at + len)
    }

    /// A deletion of `len` bytes at `at`.
    pub fn deletion(at: usize, len: usize) -> Self {
        Self::new(at, at + len, at)
    }

    /// The single region in which `old` and `new` differ.
    ///
    /// A parser can reuse everything outside this region, so finding it is
    /// what turns a re-parse from a full one into an incremental one. Several
    /// separate edits collapse into one region spanning them all, which is
    /// still far less than the whole file. Returns `None` when the texts are
    /// identical.
    pub fn between(old: &str, new: &str) -> Option<InputEdit> {
        if old == new {
            return None;
        }
        let (old_bytes, new_bytes) = (old.as_bytes(), new.as_bytes());

        // How much of the start is shared, backed up to a character boundary
        // so the edit never splits one.
        let mut prefix = old_bytes
            .iter()
            .zip(new_bytes)
            .take_while(|(a, b)| a == b)
            .count();
        while prefix > 0 && (!old.is_char_boundary(prefix) || !new.is_char_boundary(prefix)) {
            prefix -= 1;
        }

        // How much of the end is shared, without overlapping the prefix.
        let most = (old_bytes.len() - prefix).min(new_bytes.len() - prefix);
        let mut suffix = (0..most)
            .take_while(|i| {
                old_bytes[old_bytes.len() - 1 - i] == new_bytes[new_bytes.len() - 1 - i]
            })
            .count();
        while suffix > 0
            && (!old.is_char_boundary(old_bytes.len() - suffix)
                || !new.is_char_boundary(new_bytes.len() - suffix))
        {
            suffix -= 1;
        }

        Some(InputEdit::new(
            prefix,
            old_bytes.len() - suffix,
            new_bytes.len() - suffix,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cap(start: usize, end: usize, face: &'static str) -> Capture {
        Capture {
            start,
            end,
            face,
            pattern: 0,
        }
    }

    #[test]
    fn no_captures_produce_no_spans() {
        assert!(flatten(Vec::new()).is_empty());
    }

    #[test]
    fn disjoint_captures_pass_through_in_order() {
        let out = flatten(vec![cap(4, 6, "b"), cap(0, 2, "a")]);
        assert_eq!(
            out,
            vec![Highlight::new(0, 2, "a"), Highlight::new(4, 6, "b")]
        );
    }

    #[test]
    fn a_nested_capture_overrides_the_part_it_covers() {
        // `foo(bar)` captured as a whole, with `bar` captured inside it.
        let out = flatten(vec![cap(0, 8, "outer"), cap(4, 7, "inner")]);
        assert_eq!(
            out,
            vec![
                Highlight::new(0, 4, "outer"),
                Highlight::new(4, 7, "inner"),
                Highlight::new(7, 8, "outer"),
            ]
        );
    }

    #[test]
    fn nesting_works_three_deep() {
        let out = flatten(vec![cap(0, 10, "a"), cap(2, 8, "b"), cap(4, 6, "c")]);
        assert_eq!(
            out,
            vec![
                Highlight::new(0, 2, "a"),
                Highlight::new(2, 4, "b"),
                Highlight::new(4, 6, "c"),
                Highlight::new(6, 8, "b"),
                Highlight::new(8, 10, "a"),
            ]
        );
    }

    #[test]
    fn an_inner_capture_flush_with_the_outer_start_leaves_no_gap() {
        let out = flatten(vec![cap(0, 8, "outer"), cap(0, 3, "inner")]);
        assert_eq!(
            out,
            vec![Highlight::new(0, 3, "inner"), Highlight::new(3, 8, "outer")]
        );
    }

    #[test]
    fn an_inner_capture_flush_with_the_outer_end_leaves_no_gap() {
        let out = flatten(vec![cap(0, 8, "outer"), cap(5, 8, "inner")]);
        assert_eq!(
            out,
            vec![Highlight::new(0, 5, "outer"), Highlight::new(5, 8, "inner")]
        );
    }

    #[test]
    fn two_siblings_inside_one_parent_both_show() {
        let out = flatten(vec![cap(0, 12, "p"), cap(2, 4, "a"), cap(8, 10, "b")]);
        assert_eq!(
            out,
            vec![
                Highlight::new(0, 2, "p"),
                Highlight::new(2, 4, "a"),
                Highlight::new(4, 8, "p"),
                Highlight::new(8, 10, "b"),
                Highlight::new(10, 12, "p"),
            ]
        );
    }

    #[test]
    fn identical_ranges_resolve_to_the_earlier_pattern() {
        let out = flatten(vec![
            Capture {
                start: 0,
                end: 5,
                face: "later",
                pattern: 7,
            },
            Capture {
                start: 0,
                end: 5,
                face: "earlier",
                pattern: 1,
            },
        ]);
        assert_eq!(out, vec![Highlight::new(0, 5, "earlier")]);
    }

    #[test]
    fn adjacent_spans_of_the_same_face_merge() {
        let out = flatten(vec![cap(0, 3, "same"), cap(3, 6, "same")]);
        assert_eq!(out, vec![Highlight::new(0, 6, "same")]);
    }

    #[test]
    fn empty_captures_are_dropped() {
        let out = flatten(vec![cap(0, 4, "a"), cap(2, 2, "empty")]);
        assert_eq!(out, vec![Highlight::new(0, 4, "a")]);
    }

    #[test]
    fn the_result_is_ordered_and_never_overlaps() {
        let out = flatten(vec![
            cap(0, 20, "a"),
            cap(3, 9, "b"),
            cap(5, 7, "c"),
            cap(12, 18, "d"),
            cap(12, 14, "e"),
        ]);
        for pair in out.windows(2) {
            assert!(
                pair[0].end <= pair[1].start,
                "{:?} overlaps {:?}",
                pair[0],
                pair[1]
            );
        }
        assert_eq!(out.first().unwrap().start, 0);
        assert_eq!(out.last().unwrap().end, 20);
    }

    #[test]
    fn highlight_geometry_helpers() {
        let h = Highlight::new(2, 5, "f");
        assert_eq!(h.len(), 3);
        assert!(!h.is_empty());
        assert!(h.contains(2));
        assert!(!h.contains(5));
        assert!(Highlight::new(3, 3, "f").is_empty());
    }

    #[test]
    fn the_changed_region_is_found_between_two_texts() {
        // An insertion in the middle.
        assert_eq!(
            InputEdit::between("abcdef", "abcXdef"),
            Some(InputEdit::new(3, 3, 4))
        );
        // A deletion.
        assert_eq!(
            InputEdit::between("abcdef", "abdef"),
            Some(InputEdit::new(2, 3, 2))
        );
        // A replacement.
        assert_eq!(
            InputEdit::between("abcdef", "abXYdef"),
            Some(InputEdit::new(2, 3, 4))
        );
        // Appending, and truncating.
        assert_eq!(
            InputEdit::between("abc", "abcdef"),
            Some(InputEdit::new(3, 3, 6))
        );
        assert_eq!(
            InputEdit::between("abcdef", "abc"),
            Some(InputEdit::new(3, 6, 3))
        );
    }

    #[test]
    fn identical_texts_have_no_changed_region() {
        assert_eq!(InputEdit::between("same", "same"), None);
        assert_eq!(InputEdit::between("", ""), None);
    }

    #[test]
    fn a_region_covers_several_separate_edits() {
        // Two changes far apart collapse into one region spanning both, which
        // is still much less than the whole text.
        let edit = InputEdit::between("aaaXbbbbbbbbbbYccc", "aaaZbbbbbbbbbbWccc").unwrap();
        assert_eq!(edit.start_byte, 3);
        assert_eq!(edit.old_end_byte, 15);
        assert_eq!(edit.new_end_byte, 15);
    }

    #[test]
    fn a_region_never_splits_a_character() {
        // The two strings share the first byte of `é` but differ after it.
        let edit = InputEdit::between("aéb", "aèb").unwrap();
        assert!(
            "aéb".is_char_boundary(edit.start_byte),
            "start splits a character"
        );
        assert!(
            "aéb".is_char_boundary(edit.old_end_byte),
            "old end splits a character"
        );
        assert!(
            "aèb".is_char_boundary(edit.new_end_byte),
            "new end splits a character"
        );
    }

    #[test]
    fn an_edit_at_the_very_start_or_end_is_found() {
        assert_eq!(
            InputEdit::between("bcd", "abcd"),
            Some(InputEdit::new(0, 0, 1))
        );
        assert_eq!(InputEdit::between("abc", ""), Some(InputEdit::new(0, 3, 0)));
        assert_eq!(InputEdit::between("", "abc"), Some(InputEdit::new(0, 0, 3)));
    }

    #[test]
    fn input_edits_describe_insertions_and_deletions() {
        let i = InputEdit::insertion(10, 3);
        assert_eq!(i, InputEdit::new(10, 10, 13));
        let d = InputEdit::deletion(10, 3);
        assert_eq!(d, InputEdit::new(10, 13, 10));
    }
}
