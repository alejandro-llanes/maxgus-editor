//! Diagnostics, stored per document.

use crate::position::{LspPosition, LspRange};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Diagnostic severity, in the protocol's numbering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Severity {
    Error = 1,
    Warning = 2,
    Information = 3,
    Hint = 4,
}

impl Severity {
    /// Parses the integer a server sends. Absent severity means error, per the
    /// specification's guidance that the client should decide — and an
    /// unlabelled diagnostic is most usefully treated as the worst case.
    pub fn from_code(code: Option<i64>) -> Severity {
        match code {
            Some(2) => Severity::Warning,
            Some(3) => Severity::Information,
            Some(4) => Severity::Hint,
            _ => Severity::Error,
        }
    }

    /// The face this severity is drawn in.
    pub fn face(self) -> &'static str {
        match self {
            Severity::Error => "diagnostic-error",
            Severity::Warning => "diagnostic-warning",
            Severity::Information => "diagnostic-info",
            Severity::Hint => "diagnostic-hint",
        }
    }

    /// A single-letter label for the mode line.
    pub fn letter(self) -> char {
        match self {
            Severity::Error => 'E',
            Severity::Warning => 'W',
            Severity::Information => 'I',
            Severity::Hint => 'H',
        }
    }
}

/// One diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub range: LspRange,
    pub severity: Severity,
    pub message: String,
    /// The rule or error code, when the server supplies one.
    pub code: Option<String>,
    /// The producing tool, e.g. `rustc` or `clippy`.
    pub source: Option<String>,
}

impl Diagnostic {
    pub fn new(range: LspRange, severity: Severity, message: impl Into<String>) -> Diagnostic {
        Diagnostic { range, severity, message: message.into(), code: None, source: None }
    }

    /// Parses one diagnostic from a server payload. Returns `None` when the
    /// object is missing the fields that make it meaningful.
    pub fn from_json(value: &serde_json::Value) -> Option<Diagnostic> {
        let object = value.as_object()?;
        let range = object.get("range")?;
        let position = |key: &str| -> Option<LspPosition> {
            let p = range.get(key)?;
            Some(LspPosition::new(
                p.get("line")?.as_u64()? as u32,
                p.get("character")?.as_u64()? as u32,
            ))
        };
        let range = LspRange::new(position("start")?, position("end")?);
        let message = object.get("message")?.as_str()?.to_string();
        let severity = Severity::from_code(object.get("severity").and_then(|v| v.as_i64()));
        // `code` may be a number or a string.
        let code = object.get("code").and_then(|v| match v {
            serde_json::Value::String(s) => Some(s.clone()),
            serde_json::Value::Number(n) => Some(n.to_string()),
            _ => None,
        });
        let source = object.get("source").and_then(|v| v.as_str()).map(str::to_string);
        Some(Diagnostic { range, severity, message, code, source })
    }

    /// The one-line form shown in the echo area.
    pub fn summary(&self) -> String {
        let prefix = match (&self.source, &self.code) {
            (Some(s), Some(c)) => format!("{s}[{c}]: "),
            (Some(s), None) => format!("{s}: "),
            (None, Some(c)) => format!("[{c}]: "),
            (None, None) => String::new(),
        };
        // Diagnostics are often multi-line; the echo area shows the first.
        let first = self.message.lines().next().unwrap_or_default();
        format!("{prefix}{first}")
    }

    pub fn face(&self) -> &'static str {
        self.severity.face()
    }
}

/// Diagnostics for every open document, keyed by URI.
#[derive(Debug, Clone, Default)]
pub struct DiagnosticSet {
    by_uri: HashMap<String, Vec<Diagnostic>>,
}

impl DiagnosticSet {
    pub fn new() -> DiagnosticSet {
        DiagnosticSet::default()
    }

    /// Replaces every diagnostic for `uri`, which is what
    /// `textDocument/publishDiagnostics` means. An empty list clears them.
    pub fn replace(&mut self, uri: impl Into<String>, mut diagnostics: Vec<Diagnostic>) {
        let uri = uri.into();
        if diagnostics.is_empty() {
            self.by_uri.remove(&uri);
            return;
        }
        // Sort by position, then by severity, so navigation is predictable.
        diagnostics.sort_by(|a, b| {
            a.range.start.cmp(&b.range.start).then(a.severity.cmp(&b.severity))
        });
        self.by_uri.insert(uri, diagnostics);
    }

    pub fn for_uri(&self, uri: &str) -> &[Diagnostic] {
        self.by_uri.get(uri).map_or(&[], Vec::as_slice)
    }

    pub fn clear(&mut self, uri: &str) {
        self.by_uri.remove(uri);
    }

    pub fn clear_all(&mut self) {
        self.by_uri.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.by_uri.is_empty()
    }

    /// Total across every document.
    pub fn total(&self) -> usize {
        self.by_uri.values().map(Vec::len).sum()
    }

    /// How many diagnostics of each severity `uri` has, for the mode line.
    pub fn counts(&self, uri: &str) -> [usize; 4] {
        let mut counts = [0usize; 4];
        for d in self.for_uri(uri) {
            counts[d.severity as usize - 1] += 1;
        }
        counts
    }

    /// Diagnostics whose range covers `position`, most severe first.
    pub fn at(&self, uri: &str, position: LspPosition) -> Vec<&Diagnostic> {
        let mut found: Vec<&Diagnostic> = self
            .for_uri(uri)
            .iter()
            .filter(|d| {
                // An empty range still matches the position it sits on.
                let after_start = position >= d.range.start;
                let before_end = position < d.range.end || d.range.is_empty() && position == d.range.start;
                after_start && before_end
            })
            .collect();
        found.sort_by_key(|d| d.severity);
        found
    }

    /// The first diagnostic strictly after `position`, for `next-error`.
    pub fn next_after(&self, uri: &str, position: LspPosition) -> Option<&Diagnostic> {
        self.for_uri(uri).iter().find(|d| d.range.start > position)
    }

    /// The last diagnostic strictly before `position`, for `previous-error`.
    pub fn previous_before(&self, uri: &str, position: LspPosition) -> Option<&Diagnostic> {
        self.for_uri(uri).iter().rev().find(|d| d.range.start < position)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn at(line: u32, from: u32, to: u32, severity: Severity) -> Diagnostic {
        Diagnostic::new(
            LspRange::new(LspPosition::new(line, from), LspPosition::new(line, to)),
            severity,
            format!("problem on line {line}"),
        )
    }

    #[test]
    fn severity_codes_parse_with_error_as_the_default() {
        assert_eq!(Severity::from_code(Some(1)), Severity::Error);
        assert_eq!(Severity::from_code(Some(2)), Severity::Warning);
        assert_eq!(Severity::from_code(Some(3)), Severity::Information);
        assert_eq!(Severity::from_code(Some(4)), Severity::Hint);
        assert_eq!(Severity::from_code(None), Severity::Error, "unlabelled is worst case");
        assert_eq!(Severity::from_code(Some(99)), Severity::Error);
    }

    #[test]
    fn severities_order_from_most_to_least_important() {
        assert!(Severity::Error < Severity::Warning);
        assert!(Severity::Warning < Severity::Hint);
    }

    #[test]
    fn each_severity_has_a_distinct_face_and_letter() {
        let all = [Severity::Error, Severity::Warning, Severity::Information, Severity::Hint];
        let mut letters: Vec<char> = all.iter().map(|s| s.letter()).collect();
        letters.sort_unstable();
        letters.dedup();
        assert_eq!(letters.len(), 4);
        for s in all {
            assert!(s.face().starts_with("diagnostic-"));
        }
    }

    #[test]
    fn a_diagnostic_parses_from_a_server_payload() {
        let d = Diagnostic::from_json(&json!({
            "range": {"start": {"line": 3, "character": 4}, "end": {"line": 3, "character": 9}},
            "severity": 2,
            "message": "unused variable",
            "code": "unused_variables",
            "source": "rustc"
        }))
        .unwrap();
        assert_eq!(d.range.start, LspPosition::new(3, 4));
        assert_eq!(d.range.end, LspPosition::new(3, 9));
        assert_eq!(d.severity, Severity::Warning);
        assert_eq!(d.code.as_deref(), Some("unused_variables"));
        assert_eq!(d.source.as_deref(), Some("rustc"));
        assert_eq!(d.summary(), "rustc[unused_variables]: unused variable");
    }

    #[test]
    fn a_numeric_code_is_accepted() {
        let d = Diagnostic::from_json(&json!({
            "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}},
            "message": "m",
            "code": 2304
        }))
        .unwrap();
        assert_eq!(d.code.as_deref(), Some("2304"));
    }

    #[test]
    fn a_diagnostic_missing_required_fields_is_rejected() {
        assert!(Diagnostic::from_json(&json!({"message": "no range"})).is_none());
        assert!(
            Diagnostic::from_json(&json!({
                "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}}
            }))
            .is_none(),
            "no message"
        );
        assert!(Diagnostic::from_json(&json!("not an object")).is_none());
    }

    #[test]
    fn the_summary_shows_only_the_first_line() {
        let mut d = at(0, 0, 1, Severity::Error);
        d.message = "first line\nsecond line".into();
        assert_eq!(d.summary(), "first line");
    }

    #[test]
    fn the_summary_adapts_to_which_metadata_is_present() {
        let mut d = at(0, 0, 1, Severity::Error);
        d.message = "msg".into();
        assert_eq!(d.summary(), "msg");
        d.source = Some("rustc".into());
        assert_eq!(d.summary(), "rustc: msg");
        d.code = Some("E0308".into());
        assert_eq!(d.summary(), "rustc[E0308]: msg");
        d.source = None;
        assert_eq!(d.summary(), "[E0308]: msg");
    }

    #[test]
    fn publishing_replaces_rather_than_appends() {
        let mut set = DiagnosticSet::new();
        set.replace("file:///a.rs", vec![at(0, 0, 1, Severity::Error)]);
        assert_eq!(set.for_uri("file:///a.rs").len(), 1);
        set.replace("file:///a.rs", vec![at(1, 0, 1, Severity::Warning), at(2, 0, 1, Severity::Hint)]);
        assert_eq!(set.for_uri("file:///a.rs").len(), 2);
        assert_eq!(set.total(), 2);
    }

    #[test]
    fn publishing_an_empty_list_clears_the_document() {
        let mut set = DiagnosticSet::new();
        set.replace("file:///a.rs", vec![at(0, 0, 1, Severity::Error)]);
        set.replace("file:///a.rs", Vec::new());
        assert!(set.for_uri("file:///a.rs").is_empty());
        assert!(set.is_empty());
    }

    #[test]
    fn diagnostics_are_kept_sorted_by_position() {
        let mut set = DiagnosticSet::new();
        set.replace(
            "u",
            vec![at(5, 0, 1, Severity::Error), at(1, 0, 1, Severity::Error), at(3, 0, 1, Severity::Error)],
        );
        let lines: Vec<u32> = set.for_uri("u").iter().map(|d| d.range.start.line).collect();
        assert_eq!(lines, vec![1, 3, 5]);
    }

    #[test]
    fn documents_are_kept_separate() {
        let mut set = DiagnosticSet::new();
        set.replace("a", vec![at(0, 0, 1, Severity::Error)]);
        set.replace("b", vec![at(0, 0, 1, Severity::Warning), at(1, 0, 1, Severity::Hint)]);
        assert_eq!(set.for_uri("a").len(), 1);
        assert_eq!(set.for_uri("b").len(), 2);
        assert_eq!(set.total(), 3);
        set.clear("a");
        assert!(set.for_uri("a").is_empty());
        assert_eq!(set.total(), 2);
        set.clear_all();
        assert!(set.is_empty());
    }

    #[test]
    fn an_unknown_document_has_no_diagnostics() {
        let set = DiagnosticSet::new();
        assert!(set.for_uri("never-seen").is_empty());
        assert_eq!(set.counts("never-seen"), [0, 0, 0, 0]);
    }

    #[test]
    fn counts_are_broken_down_by_severity() {
        let mut set = DiagnosticSet::new();
        set.replace(
            "u",
            vec![
                at(0, 0, 1, Severity::Error),
                at(1, 0, 1, Severity::Error),
                at(2, 0, 1, Severity::Warning),
                at(3, 0, 1, Severity::Hint),
            ],
        );
        assert_eq!(set.counts("u"), [2, 1, 0, 1]);
    }

    #[test]
    fn lookup_at_a_position_finds_covering_diagnostics() {
        let mut set = DiagnosticSet::new();
        set.replace("u", vec![at(2, 4, 9, Severity::Warning), at(2, 0, 20, Severity::Error)]);
        let found = set.at("u", LspPosition::new(2, 5));
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].severity, Severity::Error, "most severe first");
        assert!(set.at("u", LspPosition::new(2, 25)).is_empty());
        assert!(set.at("u", LspPosition::new(0, 0)).is_empty());
    }

    #[test]
    fn a_range_end_is_exclusive() {
        let mut set = DiagnosticSet::new();
        set.replace("u", vec![at(0, 2, 5, Severity::Error)]);
        assert_eq!(set.at("u", LspPosition::new(0, 4)).len(), 1);
        assert!(set.at("u", LspPosition::new(0, 5)).is_empty());
    }

    #[test]
    fn an_empty_range_still_matches_its_own_position() {
        let mut set = DiagnosticSet::new();
        set.replace("u", vec![at(0, 3, 3, Severity::Error)]);
        assert_eq!(set.at("u", LspPosition::new(0, 3)).len(), 1);
    }

    #[test]
    fn navigation_finds_the_next_and_previous_diagnostic() {
        let mut set = DiagnosticSet::new();
        set.replace(
            "u",
            vec![at(1, 0, 1, Severity::Error), at(5, 0, 1, Severity::Error), at(9, 0, 1, Severity::Error)],
        );
        assert_eq!(set.next_after("u", LspPosition::new(0, 0)).unwrap().range.start.line, 1);
        assert_eq!(set.next_after("u", LspPosition::new(1, 0)).unwrap().range.start.line, 5);
        assert!(set.next_after("u", LspPosition::new(9, 0)).is_none());
        assert_eq!(set.previous_before("u", LspPosition::new(9, 0)).unwrap().range.start.line, 5);
        assert!(set.previous_before("u", LspPosition::new(1, 0)).is_none());
    }
}
