//! Incremental and regexp search.
//!
//! Both `isearch-forward` and `query-replace-regexp` are built on this module.
//! Offsets in and out are character offsets, matching the rest of the text
//! layer; the regex engine works in bytes and results are converted back.

use crate::{Result, position::Range};
use regex::{Regex, RegexBuilder};
use ropey::Rope;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchDirection {
    Forward,
    Backward,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchKind {
    Literal,
    Regexp,
}

/// One search hit, with any capture groups the pattern defined.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    pub range: Range,
    /// Capture groups by index; group 0 is the whole match. `None` marks a
    /// group that did not participate.
    pub captures: Vec<Option<String>>,
}

impl Match {
    pub fn group(&self, n: usize) -> Option<&str> {
        self.captures.get(n).and_then(|g| g.as_deref())
    }
}

/// A compiled search, reusable across incremental keystrokes.
#[derive(Debug, Clone)]
pub struct SearchQuery {
    regex: Regex,
    pattern: String,
    kind: SearchKind,
    case_fold: bool,
}

impl SearchQuery {
    /// Compiles `pattern`. When `case_fold` is `None`, Emacs' `case-fold-search`
    /// heuristic applies: fold case unless the pattern contains an uppercase
    /// letter.
    pub fn new(pattern: &str, kind: SearchKind, case_fold: Option<bool>) -> Result<Self> {
        let case_fold = case_fold.unwrap_or_else(|| !pattern.chars().any(char::is_uppercase));
        let source = match kind {
            SearchKind::Literal => regex::escape(pattern),
            SearchKind::Regexp => pattern.to_string(),
        };
        let regex = RegexBuilder::new(&source)
            .case_insensitive(case_fold)
            .multi_line(true)
            .build()?;
        Ok(Self {
            regex,
            pattern: pattern.to_string(),
            kind,
            case_fold,
        })
    }

    pub fn pattern(&self) -> &str {
        &self.pattern
    }

    pub fn kind(&self) -> SearchKind {
        self.kind
    }

    pub fn case_fold(&self) -> bool {
        self.case_fold
    }

    pub fn is_empty(&self) -> bool {
        self.pattern.is_empty()
    }

    fn to_match(&self, rope: &Rope, caps: regex::Captures<'_>) -> Match {
        let whole = caps.get(0).expect("group 0 always participates");
        Match {
            range: Range::new(
                rope.byte_to_char(whole.start()),
                rope.byte_to_char(whole.end()),
            ),
            captures: caps
                .iter()
                .map(|g| g.map(|m| m.as_str().to_string()))
                .collect(),
        }
    }

    /// First match at or after `from`.
    ///
    /// The `_in` variants take the buffer's text directly. Rendering a rope to
    /// a string costs the whole buffer, and an incremental search does one
    /// search per keystroke, so a caller that already has the text — because
    /// it caches it between keystrokes — should hand it over rather than make
    /// this allocate it again.
    pub fn search_forward(&self, rope: &Rope, from: usize) -> Option<Match> {
        self.search_forward_in(&rope.to_string(), rope, from)
    }

    /// [`SearchQuery::search_forward`] against already-rendered `text`, which
    /// must be the contents of `rope`.
    pub fn search_forward_in(&self, text: &str, rope: &Rope, from: usize) -> Option<Match> {
        if self.is_empty() {
            return None;
        }
        let start = rope.char_to_byte(from.min(rope.len_chars()));
        self.regex
            .captures_at(text, start)
            .map(|c| self.to_match(rope, c))
    }

    /// Last match that ends at or before `from`.
    pub fn search_backward(&self, rope: &Rope, from: usize) -> Option<Match> {
        self.search_backward_in(&rope.to_string(), rope, from)
    }

    /// [`SearchQuery::search_backward`] against already-rendered `text`.
    pub fn search_backward_in(&self, text: &str, rope: &Rope, from: usize) -> Option<Match> {
        if self.is_empty() {
            return None;
        }
        let limit = rope.char_to_byte(from.min(rope.len_chars()));
        // The regex crate has no reverse scan, so take the last match that
        // fits entirely before the limit.
        self.regex
            .captures_iter(text)
            .take_while(|c| c.get(0).expect("group 0").end() <= limit)
            .last()
            .map(|c| self.to_match(rope, c))
    }

    /// Last match that *starts* at or before `from`.
    ///
    /// This is what an incremental backward search needs while its query is
    /// growing: the match extends rightward as characters are typed, so
    /// limiting by the match end — as [`SearchQuery::search_backward`] does —
    /// would reject the very match the user is looking at.
    pub fn search_backward_from(&self, rope: &Rope, from: usize) -> Option<Match> {
        self.search_backward_from_in(&rope.to_string(), rope, from)
    }

    /// [`SearchQuery::search_backward_from`] against already-rendered `text`.
    pub fn search_backward_from_in(&self, text: &str, rope: &Rope, from: usize) -> Option<Match> {
        if self.is_empty() {
            return None;
        }
        let limit = rope.char_to_byte(from.min(rope.len_chars()));
        self.regex
            .captures_iter(text)
            .take_while(|c| c.get(0).expect("group 0").start() <= limit)
            .last()
            .map(|c| self.to_match(rope, c))
    }

    /// Searches in `direction`, wrapping around the buffer once — the
    /// behaviour of `isearch` after it reports "Wrapped".
    pub fn search_wrapping(
        &self,
        rope: &Rope,
        from: usize,
        direction: SearchDirection,
    ) -> Option<Match> {
        match direction {
            SearchDirection::Forward => self
                .search_forward(rope, from)
                .or_else(|| self.search_forward(rope, 0)),
            SearchDirection::Backward => self
                .search_backward(rope, from)
                .or_else(|| self.search_backward(rope, rope.len_chars())),
        }
    }

    /// Every match in the buffer, used by `occur` and `highlight-regexp`.
    pub fn find_all(&self, rope: &Rope) -> Vec<Match> {
        if self.is_empty() {
            return Vec::new();
        }
        let text = rope.to_string();
        self.regex
            .captures_iter(&text)
            .map(|c| self.to_match(rope, c))
            .collect()
    }

    /// Every match inside `range`, used to highlight hits on screen.
    pub fn find_in_range(&self, rope: &Rope, range: Range) -> Vec<Match> {
        self.find_all(rope)
            .into_iter()
            .filter(|m| {
                m.range.overlaps(&range) || (m.range.is_empty() && range.contains(m.range.start))
            })
            .collect()
    }

    /// Expands a replacement template against `m`. `\1`..`\9` insert capture
    /// groups, `\&` inserts the whole match and `\\` a literal backslash — the
    /// syntax `query-replace-regexp` accepts. Literal searches take the
    /// template verbatim.
    pub fn expand_replacement(&self, template: &str, m: &Match) -> String {
        if self.kind == SearchKind::Literal {
            return template.to_string();
        }
        let mut out = String::with_capacity(template.len());
        let mut chars = template.chars().peekable();
        while let Some(c) = chars.next() {
            if c != '\\' {
                out.push(c);
                continue;
            }
            match chars.next() {
                Some('&') => out.push_str(m.group(0).unwrap_or_default()),
                Some('\\') => out.push('\\'),
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(d) if d.is_ascii_digit() => {
                    let idx = d.to_digit(10).expect("checked ascii digit") as usize;
                    out.push_str(m.group(idx).unwrap_or_default());
                }
                Some(other) => out.push(other),
                None => out.push('\\'),
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rope(s: &str) -> Rope {
        Rope::from_str(s)
    }

    fn literal(p: &str) -> SearchQuery {
        SearchQuery::new(p, SearchKind::Literal, None).unwrap()
    }

    #[test]
    fn literal_search_escapes_metacharacters() {
        let r = rope("a.c abc");
        let q = literal("a.c");
        assert_eq!(q.search_forward(&r, 0).unwrap().range, Range::new(0, 3));
        assert_eq!(q.find_all(&r).len(), 1, "`.` must not match `b`");
    }

    #[test]
    fn smart_case_folds_only_for_lowercase_patterns() {
        let r = rope("Foo foo");
        assert!(literal("foo").case_fold());
        assert_eq!(literal("foo").find_all(&r).len(), 2);
        assert!(!literal("Foo").case_fold());
        assert_eq!(literal("Foo").find_all(&r).len(), 1);
    }

    #[test]
    fn explicit_case_fold_overrides_the_heuristic() {
        let r = rope("Foo foo");
        let q = SearchQuery::new("Foo", SearchKind::Literal, Some(true)).unwrap();
        assert_eq!(q.find_all(&r).len(), 2);
    }

    #[test]
    fn forward_search_starts_at_the_given_offset() {
        let r = rope("ab ab ab");
        let q = literal("ab");
        assert_eq!(q.search_forward(&r, 0).unwrap().range, Range::new(0, 2));
        assert_eq!(q.search_forward(&r, 1).unwrap().range, Range::new(3, 5));
        assert!(q.search_forward(&r, 7).is_none());
    }

    #[test]
    fn backward_search_returns_the_nearest_earlier_match() {
        let r = rope("ab ab ab");
        let q = literal("ab");
        assert_eq!(q.search_backward(&r, 8).unwrap().range, Range::new(6, 8));
        assert_eq!(q.search_backward(&r, 5).unwrap().range, Range::new(3, 5));
        assert!(q.search_backward(&r, 1).is_none());
    }

    #[test]
    fn start_anchored_backward_search_accepts_a_match_that_extends_past_the_limit() {
        let r = rope("alpha beta alpha");
        let q = literal("alpha");
        // The match at 11 ends at 16, past the limit, so the end-anchored
        // search skips it while the start-anchored one finds it.
        assert_eq!(q.search_backward(&r, 11).unwrap().range, Range::new(0, 5));
        assert_eq!(
            q.search_backward_from(&r, 11).unwrap().range,
            Range::new(11, 16)
        );
        assert_eq!(
            q.search_backward_from(&r, 0).unwrap().range,
            Range::new(0, 5)
        );
        assert!(literal("zzz").search_backward_from(&r, 16).is_none());
    }

    #[test]
    fn the_rendered_text_variants_agree_with_the_rope_ones() {
        let r = rope("alpha beta alpha gamma");
        let text = r.to_string();
        let q = literal("alpha");
        for from in 0..=r.len_chars() {
            assert_eq!(
                q.search_forward(&r, from),
                q.search_forward_in(&text, &r, from)
            );
            assert_eq!(
                q.search_backward(&r, from),
                q.search_backward_in(&text, &r, from)
            );
            assert_eq!(
                q.search_backward_from(&r, from),
                q.search_backward_from_in(&text, &r, from)
            );
        }
    }

    #[test]
    fn wrapping_search_restarts_from_the_far_end() {
        let r = rope("ab cd");
        let q = literal("ab");
        let m = q.search_wrapping(&r, 3, SearchDirection::Forward).unwrap();
        assert_eq!(m.range, Range::new(0, 2));
        let q = literal("cd");
        let m = q.search_wrapping(&r, 0, SearchDirection::Backward).unwrap();
        assert_eq!(m.range, Range::new(3, 5));
    }

    #[test]
    fn regexp_search_exposes_capture_groups() {
        let r = rope("key = value");
        let q = SearchQuery::new(r"(\w+)\s*=\s*(\w+)", SearchKind::Regexp, None).unwrap();
        let m = q.search_forward(&r, 0).unwrap();
        assert_eq!(m.group(1), Some("key"));
        assert_eq!(m.group(2), Some("value"));
        assert_eq!(m.group(9), None);
    }

    #[test]
    fn replacement_templates_expand_groups_and_escapes() {
        let r = rope("alpha beta");
        let q = SearchQuery::new(r"(\w+) (\w+)", SearchKind::Regexp, None).unwrap();
        let m = q.search_forward(&r, 0).unwrap();
        assert_eq!(q.expand_replacement(r"\2 \1", &m), "beta alpha");
        assert_eq!(q.expand_replacement(r"[\&]", &m), "[alpha beta]");
        assert_eq!(q.expand_replacement(r"a\\b", &m), r"a\b");
        assert_eq!(q.expand_replacement(r"a\nb", &m), "a\nb");
    }

    #[test]
    fn literal_replacements_are_taken_verbatim() {
        let r = rope("alpha");
        let q = literal("alpha");
        let m = q.search_forward(&r, 0).unwrap();
        assert_eq!(q.expand_replacement(r"\1 raw", &m), r"\1 raw");
    }

    #[test]
    fn offsets_are_characters_not_bytes() {
        let r = rope("äöü needle");
        let q = literal("needle");
        assert_eq!(q.search_forward(&r, 0).unwrap().range, Range::new(4, 10));
    }

    #[test]
    fn invalid_regexp_is_reported_as_an_error() {
        assert!(SearchQuery::new("(unclosed", SearchKind::Regexp, None).is_err());
    }

    #[test]
    fn empty_pattern_never_matches() {
        let r = rope("text");
        let q = literal("");
        assert!(q.is_empty());
        assert!(q.search_forward(&r, 0).is_none());
        assert!(q.find_all(&r).is_empty());
    }

    #[test]
    fn find_in_range_limits_hits_to_the_visible_span() {
        let r = rope("ab ab ab");
        let q = literal("ab");
        assert_eq!(q.find_in_range(&r, Range::new(0, 5)).len(), 2);
        assert_eq!(q.find_in_range(&r, Range::new(6, 8)).len(), 1);
    }

    #[test]
    fn multiline_anchors_match_per_line() {
        let r = rope("one\ntwo\nthree");
        let q = SearchQuery::new("^t", SearchKind::Regexp, None).unwrap();
        assert_eq!(q.find_all(&r).len(), 2);
    }
}
