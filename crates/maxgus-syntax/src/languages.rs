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
    pub name: &'static str,
    pub language: Language,
    /// The contents of the grammar's `highlights.scm`.
    pub highlights: &'static str,
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
pub const SUPPORTED_LANGUAGES: &[&str] = &[
    "rust",
    "python",
    "javascript",
    "json",
    "c",
    "bash",
    "html",
    "css",
];

/// The grammar for `name`, if one is compiled in.
pub fn language(name: &str) -> Option<SyntaxLanguage> {
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
        "bash" => (
            tree_sitter_bash::LANGUAGE.into(),
            tree_sitter_bash::HIGHLIGHT_QUERY,
        ),
        "html" => (
            tree_sitter_html::LANGUAGE.into(),
            tree_sitter_html::HIGHLIGHTS_QUERY,
        ),
        "css" => (
            tree_sitter_css::LANGUAGE.into(),
            tree_sitter_css::HIGHLIGHTS_QUERY,
        ),
        _ => return None,
    };
    // `name` is one of the literals above, so it outlives the call.
    let name = SUPPORTED_LANGUAGES.iter().find(|n| **n == name).copied()?;
    Some(SyntaxLanguage {
        name,
        language,
        highlights,
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
            tree_sitter::Query::new(&l.language, l.highlights)
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
