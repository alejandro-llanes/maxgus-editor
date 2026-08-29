//! Nerd Font glyphs for the file tree and the mode line.
//!
//! Every glyph here is from the Nerd Font private use area, so a terminal
//! without one of those fonts draws a box instead. That is what
//! `nerd-font-icons` is for: turned off, the tree and mode line fall back to
//! plain text and lose nothing but decoration.
//!
//! The mapping is by *language* where the editor knows one, and by file name
//! or extension otherwise, so `Cargo.toml` and `.gitignore` get their own
//! glyphs rather than a generic file.

use std::path::Path;

/// The glyph for a file, chosen by name first and extension second.
pub fn for_file(path: &Path) -> char {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    if let Some(glyph) = for_file_name(name) {
        return glyph;
    }
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    for_extension(&extension)
}

/// Files recognised by their whole name rather than an extension.
fn for_file_name(name: &str) -> Option<char> {
    Some(match name {
        "Cargo.toml" | "Cargo.lock" => '\u{e7a8}', // rust
        "Makefile" | "makefile" | "GNUmakefile" => '\u{e779}',
        "Dockerfile" | "docker-compose.yml" => '\u{e7b0}',
        ".gitignore" | ".gitattributes" | ".gitmodules" => '\u{e702}',
        "LICENSE" | "LICENCE" | "COPYING" => '\u{f0219}',
        "README" | "README.md" => '\u{f02d}',
        _ => return None,
    })
}

/// The glyph for a language identifier, as the editor names it.
pub fn for_language(language: &str) -> char {
    match language {
        "rust" => '\u{e7a8}',
        "python" => '\u{e73c}',
        "javascript" => '\u{e781}',
        "typescript" => '\u{e628}',
        "json" => '\u{e60b}',
        "c" => '\u{e61e}',
        "cpp" => '\u{e61d}',
        "go" => '\u{e627}',
        "bash" => '\u{f489}',
        "html" => '\u{e736}',
        "css" => '\u{e749}',
        "toml" => '\u{e6b2}',
        "markdown" => '\u{f48a}',
        "yaml" => '\u{e6a8}',
        "kdl" => '\u{e615}',
        "make" => '\u{e779}',
        "dockerfile" => '\u{e7b0}',
        _ => FILE,
    }
}

/// The glyph for a bare extension, for files with no language of their own.
fn for_extension(extension: &str) -> char {
    match extension {
        "rs" => '\u{e7a8}',
        "py" | "pyi" => '\u{e73c}',
        "js" | "mjs" | "cjs" | "jsx" => '\u{e781}',
        "ts" | "tsx" => '\u{e628}',
        "json" => '\u{e60b}',
        "c" | "h" => '\u{e61e}',
        "cc" | "cpp" | "cxx" | "hpp" | "hh" => '\u{e61d}',
        "go" => '\u{e627}',
        "sh" | "bash" | "zsh" | "fish" => '\u{f489}',
        "html" | "htm" => '\u{e736}',
        "css" | "scss" | "sass" => '\u{e749}',
        "toml" => '\u{e6b2}',
        "md" | "markdown" => '\u{f48a}',
        "yml" | "yaml" => '\u{e6a8}',
        "kdl" => '\u{e615}',
        "lock" => '\u{f023}',
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "ico" => '\u{f03e}',
        "pdf" => '\u{f1c1}',
        "zip" | "gz" | "xz" | "zst" | "tar" | "bz2" => '\u{f410}',
        "txt" | "log" => '\u{f0f6}',
        _ => FILE,
    }
}

/// A file nothing more specific is known about.
/// The glyph for an LSP `SymbolKind`, 1 to 26.
///
/// The outline is read by shape before it is read by name, so a function and
/// a field should not look alike even when their names do.
pub fn for_symbol(kind: u8) -> char {
    match kind {
        2..=4 => '\u{f487}',        // module, namespace, package
        5 | 11 | 23 => '\u{f0e8}',  // class, interface, struct
        6 | 9 => '\u{f6a6}',        // method, constructor
        7 | 8 => '\u{f30b}',        // property, field
        10 | 22 | 24 => '\u{f0e7}', // enum, enum member, event
        12 => '\u{f0295}',          // function
        13 => '\u{f0b07}',          // variable
        14 => '\u{f8ff}',           // constant
        15 => '\u{f77e}',           // string
        16 => '\u{f89f}',           // number
        17 => '\u{f6a9}',           // boolean
        18 => '\u{f0169}',          // array
        19 => '\u{f0233}',          // object
        20 => '\u{f80a}',           // key
        21 => '\u{f6be}',           // null
        25 => '\u{f04d6}',          // operator
        26 => '\u{f0866}',          // type parameter
        _ => '\u{f4a5}',
    }
}

pub const FILE: char = '\u{f4a5}';
/// A directory, closed and open.
pub const DIRECTORY: char = '\u{f4d4}';
pub const DIRECTORY_OPEN: char = '\u{f770}';
/// A symbolic link.
pub const SYMLINK: char = '\u{f481}';

// ---- mode line ---------------------------------------------------------

/// The buffer has unsaved changes.
pub const MODIFIED: char = '\u{f444}';
/// The buffer matches its file.
pub const SAVED: char = '\u{f00c}';
/// The buffer cannot be written.
pub const READ_ONLY: char = '\u{f023}';
/// A version-control branch.
pub const BRANCH: char = '\u{e725}';
/// Error and warning counts.
pub const ERROR: char = '\u{f057}';
pub const WARNING: char = '\u{f071}';
/// Where point is.
pub const POSITION: char = '\u{f0c9}';

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_language_the_editor_knows_has_its_own_glyph() {
        assert_ne!(for_language("rust"), FILE);
        assert_ne!(for_language("python"), FILE);
        assert_ne!(for_language("rust"), for_language("python"));
    }

    #[test]
    fn a_language_nothing_is_known_about_falls_back_to_a_plain_file() {
        assert_eq!(for_language("cobol"), FILE);
    }

    #[test]
    fn a_file_is_recognised_by_name_before_extension() {
        // `Cargo.toml` is Rust's, not TOML's, which is the whole reason names
        // are checked first.
        assert_eq!(for_file(Path::new("/p/Cargo.toml")), for_language("rust"));
        assert_eq!(for_file(Path::new("/p/other.toml")), for_language("toml"));
    }

    #[test]
    fn extensions_are_matched_whatever_their_case() {
        assert_eq!(
            for_file(Path::new("/p/README.MD")),
            for_file(Path::new("/p/notes.md"))
        );
    }

    #[test]
    fn a_file_with_no_extension_at_all_is_still_a_file() {
        assert_eq!(for_file(Path::new("/p/mystery")), FILE);
        assert_eq!(for_file(Path::new("/p/")), FILE);
    }

    #[test]
    fn every_glyph_is_one_column_of_private_use_area() {
        // A glyph outside the Nerd Font range would be a typo that draws as
        // something unrelated rather than as a missing-glyph box.
        let all = [
            FILE,
            DIRECTORY,
            DIRECTORY_OPEN,
            SYMLINK,
            MODIFIED,
            SAVED,
            READ_ONLY,
            BRANCH,
            ERROR,
            WARNING,
            POSITION,
        ];
        for glyph in all {
            let c = glyph as u32;
            assert!(
                (0xe000..=0xf8ff).contains(&c) || (0xf0000..=0xfffff).contains(&c),
                "`{glyph}` (U+{c:X}) is not in a Nerd Font range"
            );
        }
    }
}
