//! The incremental highlighter.

use crate::{
    Result, SyntaxError,
    languages::{self, SyntaxLanguage},
    span::{Capture, Highlight, InputEdit, flatten},
};
use tree_sitter::{Parser, Point, Query, QueryCursor, StreamingIterator, Tree};

/// Owns the parser, query and syntax tree for one buffer.
pub struct Highlighter {
    language: SyntaxLanguage,
    parser: Parser,
    query: Query,
    tree: Option<Tree>,
    /// Face name per capture index, resolved once at construction. `None` for
    /// captures the editor has no face for.
    capture_faces: Vec<Option<&'static str>>,
}

impl std::fmt::Debug for Highlighter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Highlighter")
            .field("language", &self.language.name)
            .field("has_tree", &self.tree.is_some())
            .finish()
    }
}

impl Highlighter {
    /// Builds a highlighter for `language`.
    pub fn new(language: &str) -> Result<Highlighter> {
        let lang = languages::language(language)
            .ok_or_else(|| SyntaxError::UnknownLanguage(language.to_string()))?;
        Highlighter::with_grammar(language, lang)
    }

    /// The same, for a grammar that has already been resolved — which is how
    /// one loaded from disk gets here.
    pub fn with_grammar(language: &str, lang: SyntaxLanguage) -> Result<Highlighter> {
        let mut parser = Parser::new();
        parser
            .set_language(&lang.language)
            .map_err(|source| SyntaxError::Language {
                language: language.to_string(),
                source,
            })?;
        let query =
            Query::new(&lang.language, &lang.highlights).map_err(|source| SyntaxError::Query {
                language: language.to_string(),
                source: Box::new(source),
            })?;
        let capture_faces = query
            .capture_names()
            .iter()
            .map(|name| maxgus_faces::names::face_for_capture(name))
            .collect();
        Ok(Highlighter {
            language: lang,
            parser,
            query,
            tree: None,
            capture_faces,
        })
    }

    pub fn language(&self) -> &str {
        &self.language.name
    }

    /// True once the buffer has been parsed at least once.
    pub fn has_tree(&self) -> bool {
        self.tree.is_some()
    }

    /// Parses `text`, reusing the previous tree when one is present. Call
    /// [`Highlighter::edit`] for each change before re-parsing so the reuse is
    /// actually incremental.
    pub fn parse(&mut self, text: &str) -> Result<()> {
        let tree = self
            .parser
            .parse(text, self.tree.as_ref())
            .ok_or_else(|| SyntaxError::ParseFailed(self.language.name.to_string()))?;
        self.tree = Some(tree);
        Ok(())
    }

    /// Discards the tree so the next parse starts from scratch.
    pub fn reset(&mut self) {
        self.tree = None;
        self.parser.reset();
    }

    /// Tells the tree about an edit. `old_text` is the buffer contents *before*
    /// the edit, needed to compute the row/column of the edited region.
    pub fn edit(&mut self, edit: InputEdit, old_text: &str, new_text: &str) {
        let Some(tree) = self.tree.as_mut() else {
            return;
        };
        tree.edit(&tree_sitter::InputEdit {
            start_byte: edit.start_byte,
            old_end_byte: edit.old_end_byte,
            new_end_byte: edit.new_end_byte,
            start_position: point_at(old_text, edit.start_byte),
            old_end_position: point_at(old_text, edit.old_end_byte),
            new_end_position: point_at(new_text, edit.new_end_byte),
        });
    }

    /// Highlights the whole buffer.
    pub fn highlights(&self, text: &str) -> Vec<Highlight> {
        self.highlights_in(text, 0..text.len())
    }

    /// Highlights only `byte_range`, which is what the renderer needs for the
    /// visible portion of a window.
    ///
    /// Spans are clipped to the range, so a construct that starts off-screen
    /// still colours its visible tail.
    pub fn highlights_in(&self, text: &str, byte_range: std::ops::Range<usize>) -> Vec<Highlight> {
        let Some(tree) = self.tree.as_ref() else {
            return Vec::new();
        };
        let start = byte_range.start.min(text.len());
        let end = byte_range.end.min(text.len());
        if start >= end {
            return Vec::new();
        }

        let mut cursor = QueryCursor::new();
        cursor.set_byte_range(start..end);
        let mut captures = Vec::new();
        let mut it = cursor.captures(&self.query, tree.root_node(), text.as_bytes());
        while let Some((m, capture_index)) = it.next() {
            let capture = m.captures[*capture_index];
            let Some(Some(face)) = self.capture_faces.get(capture.index as usize) else {
                continue;
            };
            let node = capture.node;
            captures.push(Capture {
                start: node.start_byte().max(start),
                end: node.end_byte().min(end),
                face,
                pattern: m.pattern_index,
            });
        }
        flatten(captures)
    }

    /// The node kind at `byte`, or `None` before the first parse. Used by
    /// `describe-syntax-at-point`.
    pub fn node_kind_at(&self, byte: usize) -> Option<&'static str> {
        let tree = self.tree.as_ref()?;
        let node = tree.root_node().descendant_for_byte_range(byte, byte)?;
        Some(node.kind())
    }

    /// Byte range of the smallest named node containing `byte`, which is what
    /// `expand-region` grows through.
    pub fn node_range_at(&self, byte: usize) -> Option<(usize, usize)> {
        let tree = self.tree.as_ref()?;
        let node = tree
            .root_node()
            .named_descendant_for_byte_range(byte, byte)?;
        Some((node.start_byte(), node.end_byte()))
    }

    /// The range of the smallest named node that *strictly* contains
    /// `range`.
    ///
    /// Asking for the node at a range that exactly matches a node returns that
    /// node again, so growing a region needs the parent. Repeated calls walk
    /// outwards one construct at a time, which is what `expand-region` does.
    pub fn enclosing_node_range(&self, range: std::ops::Range<usize>) -> Option<(usize, usize)> {
        let tree = self.tree.as_ref()?;
        let mut node = tree
            .root_node()
            .named_descendant_for_byte_range(range.start, range.end)?;
        while node.start_byte() == range.start && node.end_byte() == range.end {
            node = node.parent()?;
        }
        Some((node.start_byte(), node.end_byte()))
    }

    /// True when the tree contains a parse error, so the mode line can say so.
    pub fn has_error(&self) -> bool {
        self.tree
            .as_ref()
            .is_some_and(|t| t.root_node().has_error())
    }

    /// An s-expression rendering of the tree, for debugging a grammar.
    pub fn tree_sexp(&self) -> Option<String> {
        self.tree.as_ref().map(|t| t.root_node().to_sexp())
    }
}

/// The row/column of `byte` in `text`, as tree-sitter counts them: row is a
/// zero-based line index, column is a byte offset within that line.
fn point_at(text: &str, byte: usize) -> Point {
    let byte = byte.min(text.len());
    let before = &text[..byte];
    let row = before.bytes().filter(|b| *b == b'\n').count();
    let column = before.rfind('\n').map_or(byte, |nl| byte - nl - 1);
    Point::new(row, column)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rust(text: &str) -> Highlighter {
        let mut h = Highlighter::new("rust").unwrap();
        h.parse(text).unwrap();
        h
    }

    /// The faces covering `needle`'s first occurrence in `text`.
    fn face_of(h: &Highlighter, text: &str, needle: &str) -> Vec<&'static str> {
        let at = text
            .find(needle)
            .unwrap_or_else(|| panic!("`{needle}` not in the source"));
        let range = at..at + needle.len();
        h.highlights(text)
            .into_iter()
            .filter(|s| s.start < range.end && range.start < s.end)
            .map(|s| s.face)
            .collect()
    }

    #[test]
    fn an_unknown_language_is_an_error() {
        assert!(matches!(
            Highlighter::new("cobol"),
            Err(SyntaxError::UnknownLanguage(_))
        ));
    }

    #[test]
    fn a_highlighter_reports_its_language() {
        assert_eq!(Highlighter::new("rust").unwrap().language(), "rust");
    }

    #[test]
    fn nothing_is_highlighted_before_the_first_parse() {
        let h = Highlighter::new("rust").unwrap();
        assert!(!h.has_tree());
        assert!(h.highlights("fn main() {}").is_empty());
        assert!(h.node_kind_at(0).is_none());
        assert!(h.tree_sexp().is_none());
    }

    #[test]
    fn rust_keywords_strings_and_comments_get_their_faces() {
        let src = "// note\nfn main() {\n    let s = \"hi\";\n}\n";
        let h = rust(src);
        assert!(h.has_tree());
        assert!(face_of(&h, src, "// note").contains(&"font-lock-comment"));
        assert!(face_of(&h, src, "fn").contains(&"font-lock-keyword"));
        assert!(face_of(&h, src, "\"hi\"").contains(&"font-lock-string"));
    }

    #[test]
    fn highlights_are_ordered_and_non_overlapping() {
        let src = "fn f(x: u32) -> u32 { x + 1 }";
        let h = rust(src);
        let spans = h.highlights(src);
        assert!(!spans.is_empty());
        for pair in spans.windows(2) {
            assert!(
                pair[0].end <= pair[1].start,
                "{:?} then {:?}",
                pair[0],
                pair[1]
            );
        }
        assert!(spans.last().unwrap().end <= src.len());
    }

    #[test]
    fn restricting_the_byte_range_clips_the_spans() {
        let src = "fn a() {}\nfn b() {}\n";
        let h = rust(src);
        let second_line = src.find("fn b").unwrap();
        let spans = h.highlights_in(src, second_line..src.len());
        assert!(!spans.is_empty());
        assert!(
            spans.iter().all(|s| s.start >= second_line),
            "a span leaked before the requested range"
        );
    }

    #[test]
    fn a_construct_starting_before_the_range_still_colours_its_tail() {
        let src = "/* a long\n   block comment */\nfn f() {}";
        let h = rust(src);
        let from = src.find("block").unwrap();
        let spans = h.highlights_in(src, from..from + 5);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].face, "font-lock-comment");
        assert_eq!(spans[0].start, from, "clipped to the range start");
    }

    #[test]
    fn an_empty_or_inverted_range_yields_nothing() {
        let src = "fn f() {}";
        let h = rust(src);
        assert!(h.highlights_in(src, 3..3).is_empty());
        // Built from variables so the lint does not flag a literal empty range.
        let (lo, hi) = (8usize, 2usize);
        assert!(h.highlights_in(src, lo..hi).is_empty());
        assert!(h.highlights_in(src, 100..200).is_empty());
    }

    #[test]
    fn reparsing_after_an_edit_updates_the_highlights() {
        let before = "let x = 1;";
        let mut h = rust(before);
        assert!(face_of(&h, before, "let").contains(&"font-lock-keyword"));

        // Comment the line out.
        let after = "// let x = 1;";
        h.edit(InputEdit::insertion(0, 3), before, after);
        h.parse(after).unwrap();
        assert!(face_of(&h, after, "let").contains(&"font-lock-comment"));
    }

    #[test]
    fn an_edit_before_the_first_parse_is_harmless() {
        let mut h = Highlighter::new("rust").unwrap();
        h.edit(InputEdit::insertion(0, 3), "", "abc");
        assert!(!h.has_tree());
        h.parse("fn f() {}").unwrap();
        assert!(h.has_tree());
    }

    #[test]
    fn resetting_discards_the_tree() {
        let mut h = rust("fn f() {}");
        h.reset();
        assert!(!h.has_tree());
        assert!(h.highlights("fn f() {}").is_empty());
    }

    #[test]
    fn incremental_and_full_parses_agree() {
        let before = "fn a() { 1 }\nfn b() { 2 }\n";
        let after = "fn a() { 1 }\nfn c() { 2 }\n";
        let mut incremental = rust(before);
        // Replace `b` with `c`: one byte at a known offset.
        let at = before.find("fn b").unwrap() + 3;
        incremental.edit(InputEdit::new(at, at + 1, at + 1), before, after);
        incremental.parse(after).unwrap();

        let fresh = rust(after);
        assert_eq!(incremental.highlights(after), fresh.highlights(after));
    }

    #[test]
    fn parse_errors_are_reported_without_losing_highlighting() {
        let src = "fn f( {";
        let h = rust(src);
        assert!(h.has_error());
        // Broken code still colours the keyword it did recognise.
        assert!(face_of(&h, src, "fn").contains(&"font-lock-keyword"));
    }

    #[test]
    fn valid_code_reports_no_error() {
        assert!(!rust("fn f() {}").has_error());
    }

    #[test]
    fn node_lookups_describe_the_tree() {
        let src = "fn main() {}";
        let h = rust(src);
        assert_eq!(h.node_kind_at(0), Some("fn"));
        let (start, end) = h.node_range_at(src.find("main").unwrap()).unwrap();
        assert_eq!(&src[start..end], "main");
        assert!(h.tree_sexp().unwrap().contains("function_item"));
    }

    #[test]
    fn the_enclosing_node_grows_outward_one_construct_at_a_time() {
        let src = "fn main() {\n    let x = 1;\n}\n";
        let h = rust(src);
        let at = src.find("x = 1").unwrap();

        // Starting from a point, the innermost node.
        let (start, end) = h.enclosing_node_range(at..at).unwrap();
        assert_eq!(&src[start..end], "x");

        // Asking again from that exact range must not answer with it again.
        let (start, end) = h.enclosing_node_range(start..end).unwrap();
        assert!(&src[start..end] != "x", "it did not grow");
        assert!(
            src[start..end].contains("x = 1"),
            "got `{}`",
            &src[start..end]
        );

        // And it keeps growing until it runs out.
        let (start, end) = h.enclosing_node_range(start..end).unwrap();
        assert!(
            src[start..end].contains("let x = 1"),
            "got `{}`",
            &src[start..end]
        );
    }

    #[test]
    fn growing_from_the_whole_file_has_nowhere_left_to_go() {
        let src = "fn main() {}";
        let h = rust(src);
        let mut range = 0..0;
        // Walking outwards must terminate rather than loop.
        for _ in 0..20 {
            match h.enclosing_node_range(range.clone()) {
                Some((start, end)) => range = start..end,
                None => return,
            }
        }
        assert_eq!(range, 0..src.len(), "it should have reached the whole file");
    }

    /// Captures deliberately left without a face of their own.
    ///
    /// `embedded` marks a span written in *another* language — `${…}` inside a
    /// template string, `$(…)` in a shell script. Painting it would colour
    /// over whatever the inner language should be showing there.
    const UNMAPPED_ON_PURPOSE: &[&str] = &[
        "embedded", // `@spell` marks prose for a spell checker, not for a colour.
        "spell",
        // `@none` is markdown's way of saying "clear whatever was here",
        // which is what having no face already does.
        "none",
    ];

    #[test]
    fn every_capture_the_grammars_use_maps_to_a_real_face() {
        // A query that compiles is not a query that colours anything: the
        // capture names have to be ones `face_for_capture` knows, and what it
        // returns has to be a face that exists. C's `delimiter` was neither
        // until it was noticed here — it parsed, matched, and painted nothing.
        let mut missing: Vec<String> = Vec::new();
        for name in languages::supported_languages() {
            let l = languages::language(name).expect("listed");
            let query = tree_sitter::Query::new(&l.language, &l.highlights).expect("compiles");
            for capture in query.capture_names() {
                // A leading underscore is the tree-sitter convention for a
                // capture used by the query itself and never highlighted.
                if capture.starts_with('_') || UNMAPPED_ON_PURPOSE.contains(capture) {
                    continue;
                }
                match maxgus_faces::names::face_for_capture(capture) {
                    Some(face) => assert!(
                        maxgus_faces::names::is_known(face),
                        "`{name}` capture `{capture}` maps to `{face}`, which is not a face"
                    ),
                    None => missing.push(format!("{name}: {capture}")),
                }
            }
        }
        assert!(
            missing.is_empty(),
            "captures that would paint nothing: {missing:?}"
        );
    }

    #[test]
    fn the_deliberate_exceptions_are_still_exceptions() {
        // The list above is only honest while those captures really are
        // unmapped; one of them gaining a face should be noticed, not hidden.
        for capture in UNMAPPED_ON_PURPOSE {
            assert_eq!(
                maxgus_faces::names::face_for_capture(capture),
                None,
                "`{capture}` now maps to something, so it no longer belongs on the list"
            );
        }
    }

    #[test]
    fn c_delimiters_are_coloured_like_every_other_language() {
        // The C grammar spells them `@delimiter`; the rest say
        // `@punctuation.delimiter`. Both must reach `font-lock-punctuation`,
        // or C is the odd one out on screen for no reason a reader could see.
        let src = "int main(void) { return 0; }";
        let mut h = Highlighter::new("c").expect("a C grammar");
        h.parse(src).unwrap();
        let semicolon = src.find(';').expect("there is one");
        let span = h
            .highlights(src)
            .into_iter()
            .find(|s| s.start <= semicolon && semicolon < s.end)
            .expect("the semicolon is covered by a span");
        assert_eq!(span.face, "font-lock-punctuation");
    }

    #[test]
    fn every_supported_language_highlights_a_sample() {
        let samples: &[(&str, &str)] = &[
            ("rust", "fn main() { let x = 1; }"),
            ("python", "def f(x):\n    return x + 1\n"),
            ("javascript", "function f(x) { return x + 1; }"),
            ("json", r#"{"key": "value", "n": 1}"#),
            ("c", "int main(void) { return 0; }"),
            ("html", "<p class=\"x\">hi</p>"),
            ("yaml", "key: value\nlist:\n  - one\n"),
            ("toml", "[table]\nkey = \"value\"\nn = 1\n"),
            ("ini", "[section]\nkey = value\n"),
            ("xml", "<?xml version=\"1.0\"?><a href=\"x\">hi</a>"),
            ("markdown", "# Heading\n\nA paragraph, and `code`.\n"),
        ];
        for (lang, src) in samples {
            let mut h = Highlighter::new(lang).unwrap_or_else(|e| panic!("`{lang}`: {e}"));
            h.parse(src).unwrap();
            assert!(!h.has_error(), "`{lang}` failed to parse its own sample");
            assert!(
                !h.highlights(src).is_empty(),
                "`{lang}` produced no highlights"
            );
        }
        assert_eq!(samples.len(), languages::supported_languages().len());
    }

    #[test]
    fn highlighting_handles_multibyte_text() {
        let src = "// héllo wörld\nfn f() {}";
        let h = rust(src);
        let spans = h.highlights(src);
        // Every boundary must sit on a character boundary, or slicing panics.
        for s in &spans {
            assert!(
                src.is_char_boundary(s.start),
                "{} is not a boundary",
                s.start
            );
            assert!(src.is_char_boundary(s.end), "{} is not a boundary", s.end);
        }
        assert!(face_of(&h, src, "héllo").contains(&"font-lock-comment"));
    }

    #[test]
    fn an_empty_buffer_parses_to_nothing() {
        let mut h = Highlighter::new("rust").unwrap();
        h.parse("").unwrap();
        assert!(h.has_tree());
        assert!(h.highlights("").is_empty());
    }

    #[test]
    fn point_at_counts_rows_and_byte_columns() {
        assert_eq!(point_at("abc", 0), Point::new(0, 0));
        assert_eq!(point_at("abc", 2), Point::new(0, 2));
        assert_eq!(point_at("ab\ncd", 3), Point::new(1, 0));
        assert_eq!(point_at("ab\ncd", 5), Point::new(1, 2));
        assert_eq!(
            point_at("ab\ncd", 99),
            Point::new(1, 2),
            "clamps to the end"
        );
        // Columns are byte offsets, so a multibyte char counts as its length.
        assert_eq!(point_at("ä\nx", 3), Point::new(1, 0));
    }
}
