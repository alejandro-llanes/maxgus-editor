//! Tree-sitter syntax highlighting.
//!
//! A [`Highlighter`] owns a parser, a compiled highlights query and the last
//! syntax tree for one buffer. Buffer edits are forwarded to the tree so
//! re-parsing is incremental, and highlighting a screenful only queries the
//! byte range on display.

pub mod highlight;
pub mod languages;
pub mod span;

pub use highlight::Highlighter;
pub use languages::{SyntaxLanguage, is_supported, language, supported_languages};
pub use span::{Highlight, InputEdit, flatten};

#[derive(Debug, thiserror::Error)]
pub enum SyntaxError {
    #[error("no tree-sitter grammar for language `{0}`")]
    UnknownLanguage(String),
    #[error("grammar for `{language}` is incompatible with this tree-sitter: {source}")]
    Language {
        language: String,
        #[source]
        source: tree_sitter::LanguageError,
    },
    #[error("highlights query for `{language}` failed to compile: {source}")]
    Query {
        language: String,
        #[source]
        source: Box<tree_sitter::QueryError>,
    },
    #[error("parsing `{0}` produced no tree")]
    ParseFailed(String),
}

pub type Result<T> = std::result::Result<T, SyntaxError>;
