//! Buffer positions.
//!
//! `maxgus` follows Emacs in using a single integer "point" as the canonical
//! cursor location. Unlike Emacs, point is a *character* offset and is
//! zero-based; line/column pairs are derived on demand.

use serde::{Deserialize, Serialize};

/// A line/column pair. Both are zero-based; `column` counts characters, not
/// display cells (see [`crate::buffer::Buffer::display_column`] for the latter).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Position {
    pub line: usize,
    pub column: usize,
}

impl Position {
    pub const ZERO: Position = Position { line: 0, column: 0 };

    pub fn new(line: usize, column: usize) -> Self {
        Self { line, column }
    }
}

impl std::fmt::Display for Position {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Humans (and the mode line) count from one.
        write!(f, "{}:{}", self.line + 1, self.column + 1)
    }
}

/// A half-open character range `[start, end)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Range {
    pub start: usize,
    pub end: usize,
}

impl Range {
    /// Builds a range from two unordered offsets, as `region-beginning` /
    /// `region-ending` do for point and mark.
    pub fn ordered(a: usize, b: usize) -> Self {
        if a <= b {
            Self { start: a, end: b }
        } else {
            Self { start: b, end: a }
        }
    }

    pub fn new(start: usize, end: usize) -> Self {
        debug_assert!(start <= end);
        Self { start, end }
    }

    pub fn empty(at: usize) -> Self {
        Self { start: at, end: at }
    }

    pub fn len(&self) -> usize {
        self.end - self.start
    }

    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }

    pub fn contains(&self, offset: usize) -> bool {
        offset >= self.start && offset < self.end
    }

    /// True when the two ranges share at least one character position.
    pub fn overlaps(&self, other: &Range) -> bool {
        self.start < other.end && other.start < self.end
    }

    pub fn intersect(&self, other: &Range) -> Option<Range> {
        let start = self.start.max(other.start);
        let end = self.end.min(other.end);
        (start < end).then(|| Range::new(start, end))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn position_displays_one_based() {
        assert_eq!(Position::new(0, 0).to_string(), "1:1");
        assert_eq!(Position::new(12, 4).to_string(), "13:5");
    }

    #[test]
    fn ordered_range_normalises_arguments() {
        assert_eq!(Range::ordered(9, 3), Range::new(3, 9));
        assert_eq!(Range::ordered(3, 9), Range::new(3, 9));
        assert!(Range::ordered(4, 4).is_empty());
    }

    #[test]
    fn range_containment_is_half_open() {
        let r = Range::new(2, 5);
        assert!(r.contains(2));
        assert!(r.contains(4));
        assert!(!r.contains(5));
        assert_eq!(r.len(), 3);
    }

    #[test]
    fn range_intersection() {
        let a = Range::new(0, 10);
        let b = Range::new(4, 20);
        assert_eq!(a.intersect(&b), Some(Range::new(4, 10)));
        assert!(a.overlaps(&b));
        assert_eq!(a.intersect(&Range::new(10, 12)), None);
        assert!(!a.overlaps(&Range::new(10, 12)));
    }
}
