//! The grammar registry.
//!
//! Each entry pairs a language identifier — the same one `maxgus-text` derives
//! from a file extension — with a compiled-in tree-sitter grammar and its
//! highlights query.

use tree_sitter::Language;

/// A grammar plus the query that drives highlighting for it.
#[derive(Clone)]
pub struct SyntaxLanguage {
    /// The identifier used everywhere else in the editor, e.g. `rust`.
    pub name: String,
    pub language: Language,
    /// The contents of the grammar's `highlights.scm`.
    ///
    /// Borrowed for a compiled-in grammar, where the query is a constant in
    /// the grammar's own crate, and owned for one loaded from disk.
    pub highlights: std::borrow::Cow<'static, str>,
}

impl std::fmt::Debug for SyntaxLanguage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `Language` has no useful Debug output of its own.
        f.debug_struct("SyntaxLanguage")
            .field("name", &self.name)
            .field("highlights_len", &self.highlights.len())
            .finish()
    }
}

/// Language identifiers with a compiled-in grammar.
///
/// Anything else can still be coloured by a grammar installed on the system
/// — see [`crate::dynamic`] and `docs/grammars.md`. These are the ones that
/// need nothing installed.
///
/// KDL and Rhai are not here and were meant to be. Neither has a grammar on
/// crates.io that this tree-sitter can link: `tree-sitter-kdl` is still
/// bound to tree-sitter 0.20, whose C runtime cannot be linked beside
/// 0.26's — the symbols collide — and Rhai has no published crate at all.
/// Both load from disk like any other, which is what [`crate::dynamic`] is
/// for.
pub const SUPPORTED_LANGUAGES: &[&str] = &[
    "c",
    "html",
    "ini",
    "javascript",
    "json",
    "markdown",
    "python",
    "rust",
    "toml",
    "xml",
    "yaml",
];

/// The grammar for `name`, if one is compiled in.
pub fn language(name: &str) -> Option<SyntaxLanguage> {
    compiled_in(name)
}

/// The grammar compiled into this binary for `name`, if there is one.
pub fn compiled_in(name: &str) -> Option<SyntaxLanguage> {
    // The grammar crates disagree on whether the constant is `HIGHLIGHTS_QUERY`
    // or `HIGHLIGHT_QUERY`, hence the spelling differences below.
    let (language, highlights): (Language, &'static str) = match name {
        "rust" => (
            tree_sitter_rust::LANGUAGE.into(),
            tree_sitter_rust::HIGHLIGHTS_QUERY,
        ),
        "python" => (
            tree_sitter_python::LANGUAGE.into(),
            tree_sitter_python::HIGHLIGHTS_QUERY,
        ),
        "javascript" => (
            tree_sitter_javascript::LANGUAGE.into(),
            tree_sitter_javascript::HIGHLIGHT_QUERY,
        ),
        "json" => (
            tree_sitter_json::LANGUAGE.into(),
            tree_sitter_json::HIGHLIGHTS_QUERY,
        ),
        "c" => (
            tree_sitter_c::LANGUAGE.into(),
            tree_sitter_c::HIGHLIGHT_QUERY,
        ),
        "html" => (
            tree_sitter_html::LANGUAGE.into(),
            tree_sitter_html::HIGHLIGHTS_QUERY,
        ),
        "yaml" => (
            tree_sitter_yaml::LANGUAGE.into(),
            tree_sitter_yaml::HIGHLIGHTS_QUERY,
        ),
        "toml" => (
            tree_sitter_toml_ng::LANGUAGE.into(),
            tree_sitter_toml_ng::HIGHLIGHTS_QUERY,
        ),
        "ini" => (
            tree_sitter_ini::LANGUAGE.into(),
            tree_sitter_ini::HIGHLIGHTS_QUERY,
        ),
        // XML's crate carries a DTD grammar as well; this is the one for
        // documents.
        "xml" => (
            tree_sitter_xml::LANGUAGE_XML.into(),
            tree_sitter_xml::XML_HIGHLIGHT_QUERY,
        ),
        // Markdown is two grammars — the block structure and the inline
        // spans within it — and this editor gives a buffer one. The block
        // grammar is the one worth having: headings, fences, lists, quotes
        // and links. Emphasis inside a paragraph goes uncoloured.
        "markdown" => (
            tree_sitter_md::LANGUAGE.into(),
            tree_sitter_md::HIGHLIGHT_QUERY_BLOCK,
        ),
        _ => return None,
    };
    Some(SyntaxLanguage {
        name: SUPPORTED_LANGUAGES
            .iter()
            .find(|n| **n == name)?
            .to_string(),
        language,
        highlights: std::borrow::Cow::Borrowed(highlights),
    })
}

/// True when a grammar is compiled in for `name`.
pub fn is_supported(name: &str) -> bool {
    SUPPORTED_LANGUAGES.contains(&name)
}

/// Every supported language identifier.
pub fn supported_languages() -> &'static [&'static str] {
    SUPPORTED_LANGUAGES
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_listed_language_has_a_grammar() {
        for name in SUPPORTED_LANGUAGES {
            let l = language(name).unwrap_or_else(|| panic!("`{name}` has no grammar"));
            assert_eq!(l.name, *name);
            assert!(
                !l.highlights.is_empty(),
                "`{name}` has an empty highlights query"
            );
        }
    }

    #[test]
    fn unsupported_languages_report_nothing() {
        assert!(language("cobol").is_none());
        assert!(!is_supported("cobol"));
        assert!(is_supported("rust"));
    }

    #[test]
    fn the_language_list_has_no_duplicates() {
        let mut names = SUPPORTED_LANGUAGES.to_vec();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(names.len(), before);
    }

    #[test]
    fn every_grammar_accepts_its_own_highlights_query() {
        for name in SUPPORTED_LANGUAGES {
            let l = language(name).expect("listed above");
            tree_sitter::Query::new(&l.language, &l.highlights)
                .unwrap_or_else(|e| panic!("`{name}` query does not compile: {e}"));
        }
    }

    #[test]
    fn every_grammar_loads_into_a_parser() {
        for name in SUPPORTED_LANGUAGES {
            let l = language(name).expect("listed above");
            let mut p = tree_sitter::Parser::new();
            p.set_language(&l.language)
                .unwrap_or_else(|e| panic!("`{name}` is incompatible: {e}"));
        }
    }
}
