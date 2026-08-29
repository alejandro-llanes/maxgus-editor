//! Primitive edits.
//!
//! Every buffer mutation funnels through an [`Edit`], which records enough
//! information to be applied and inverted. Undo is therefore just a matter of
//! replaying inverted edits in reverse order.

use crate::position::Range;

/// What an edit does at its anchor offset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditKind {
    Insert {
        text: String,
    },
    Delete {
        text: String,
    },
    /// A delete immediately followed by an insert at the same offset.
    Replace {
        removed: String,
        inserted: String,
    },
}

/// A single reversible change to a buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    /// Character offset at which the change begins.
    pub at: usize,
    pub kind: EditKind,
    /// Where point sat before the edit, so undo can restore it.
    pub point_before: usize,
}

impl Edit {
    pub fn insert(at: usize, text: impl Into<String>, point_before: usize) -> Self {
        Self {
            at,
            kind: EditKind::Insert { text: text.into() },
            point_before,
        }
    }

    pub fn delete(at: usize, text: impl Into<String>, point_before: usize) -> Self {
        Self {
            at,
            kind: EditKind::Delete { text: text.into() },
            point_before,
        }
    }

    pub fn replace(
        at: usize,
        removed: impl Into<String>,
        inserted: impl Into<String>,
        point_before: usize,
    ) -> Self {
        Self {
            at,
            kind: EditKind::Replace {
                removed: removed.into(),
                inserted: inserted.into(),
            },
            point_before,
        }
    }

    /// The edit that undoes this one. `point_before` is carried across so the
    /// inverse restores the original cursor location.
    pub fn invert(&self) -> Edit {
        let kind = match &self.kind {
            EditKind::Insert { text } => EditKind::Delete { text: text.clone() },
            EditKind::Delete { text } => EditKind::Insert { text: text.clone() },
            EditKind::Replace { removed, inserted } => EditKind::Replace {
                removed: inserted.clone(),
                inserted: removed.clone(),
            },
        };
        Edit {
            at: self.at,
            kind,
            point_before: self.point_before,
        }
    }

    /// Number of characters this edit removes.
    pub fn removed_chars(&self) -> usize {
        match &self.kind {
            EditKind::Insert { .. } => 0,
            EditKind::Delete { text } => text.chars().count(),
            EditKind::Replace { removed, .. } => removed.chars().count(),
        }
    }

    /// Number of characters this edit adds.
    pub fn inserted_chars(&self) -> usize {
        match &self.kind {
            EditKind::Insert { text } => text.chars().count(),
            EditKind::Delete { .. } => 0,
            EditKind::Replace { inserted, .. } => inserted.chars().count(),
        }
    }

    /// The span the edit occupied *before* it was applied.
    pub fn old_range(&self) -> Range {
        Range::new(self.at, self.at + self.removed_chars())
    }

    /// The span the edit occupies *after* it was applied.
    pub fn new_range(&self) -> Range {
        Range::new(self.at, self.at + self.inserted_chars())
    }

    /// Where point should end up once the edit has been applied.
    pub fn point_after(&self) -> usize {
        self.at + self.inserted_chars()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_inverts_to_delete() {
        let e = Edit::insert(3, "abc", 3);
        let inv = e.invert();
        assert_eq!(inv.kind, EditKind::Delete { text: "abc".into() });
        assert_eq!(inv.at, 3);
        assert_eq!(e, inv.invert());
    }

    #[test]
    fn replace_inverts_by_swapping_sides() {
        let e = Edit::replace(1, "old", "brand new", 4);
        let inv = e.invert();
        assert_eq!(
            inv.kind,
            EditKind::Replace {
                removed: "brand new".into(),
                inserted: "old".into()
            }
        );
        assert_eq!(e, inv.invert());
    }

    #[test]
    fn ranges_account_for_multibyte_chars() {
        let e = Edit::replace(0, "ää", "ø", 0);
        assert_eq!(e.removed_chars(), 2);
        assert_eq!(e.inserted_chars(), 1);
        assert_eq!(e.old_range(), Range::new(0, 2));
        assert_eq!(e.new_range(), Range::new(0, 1));
        assert_eq!(e.point_after(), 1);
    }
}
