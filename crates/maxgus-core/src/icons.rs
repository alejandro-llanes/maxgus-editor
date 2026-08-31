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
    // Codicons, which are the icons an LSP kind was drawn for: the editor
    // that invented the protocol drew one per `SymbolKind`, so a reader who
    // has seen a symbol outline anywhere else has already learnt these.
    //
    // They are also *one set*. What was here before was a handful from Font
    // Awesome, a handful from Material and a handful from an extension pack,
    // which read as a ransom note even where the font had them — and seven
    // of them it did not.
    match kind {
        2..=4 => '\u{ea8b}',  // module, namespace, package
        5 | 23 => '\u{eb5b}', // class, struct
        11 => '\u{eb61}',     // interface
        6 | 9 => '\u{ea8c}',  // method, constructor
        7 => '\u{eb65}',      // property
        8 => '\u{eb5f}',      // field
        10 => '\u{ea95}',     // enum
        22 => '\u{eb5e}',     // enum member
        24 => '\u{ea86}',     // event
        12 => '\u{ea8c}',     // function
        13 => '\u{ea88}',     // variable
        14 => '\u{eb5d}',     // constant
        15 => '\u{eb8d}',     // string
        16 => '\u{ea90}',     // number
        17 => '\u{ea8f}',     // boolean
        18 => '\u{ea8a}',     // array
        19 => '\u{ea8b}',     // object
        20 => '\u{eb11}',     // key
        21 => '\u{eb63}',     // null
        25 => '\u{eb64}',     // operator
        26 => '\u{ea92}',     // type parameter
        _ => '\u{eb60}',      // anything the protocol adds later
    }
}

pub const FILE: char = '\u{f4a5}';
/// A directory, closed and open.
pub const DIRECTORY: char = '\u{f4d4}';
pub const DIRECTORY_OPEN: char = '\u{f0770}';
/// A symbolic link.
pub const SYMLINK: char = '\u{f481}';

/// The mark on a row that can be opened, closed and open.
///
/// A chevron rather than `>` and `v`, which are letters pretending to be
/// arrows. Codicons, to match the symbol kinds.
pub const CHEVRON_RIGHT: char = '\u{eab6}';
pub const CHEVRON_DOWN: char = '\u{eab4}';

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
    /// Every glyph this module can draw.
    fn all() -> Vec<(String, char)> {
        let mut out = vec![
            ("FILE".into(), super::FILE),
            ("DIRECTORY".into(), super::DIRECTORY),
            ("DIRECTORY_OPEN".into(), super::DIRECTORY_OPEN),
            ("SYMLINK".into(), super::SYMLINK),
            ("CHEVRON_RIGHT".into(), super::CHEVRON_RIGHT),
            ("CHEVRON_DOWN".into(), super::CHEVRON_DOWN),
            ("MODIFIED".into(), super::MODIFIED),
            ("SAVED".into(), super::SAVED),
            ("READ_ONLY".into(), super::READ_ONLY),
            ("BRANCH".into(), super::BRANCH),
            ("ERROR".into(), super::ERROR),
            ("WARNING".into(), super::WARNING),
            ("POSITION".into(), super::POSITION),
        ];
        for kind in 0u8..=30 {
            out.push((format!("symbol kind {kind}"), super::for_symbol(kind)));
        }
        for name in [
            "main.rs",
            "lib.py",
            "app.js",
            "x.json",
            "y.c",
            "z.html",
            "a.yaml",
            "Cargo.toml",
            ".gitignore",
            "b.md",
            "c.png",
            "d.zip",
            "e.txt",
            "Makefile",
            "f.sh",
            "g.go",
            "h.rb",
            "i.ts",
            "j.css",
            "k.xml",
        ] {
            out.push((name.into(), super::for_file(std::path::Path::new(name))));
        }
        out
    }

    #[test]
    fn no_glyph_comes_from_the_range_nerd_fonts_renumbered() {
        // The bug this is here for, and it was eight glyphs deep: Nerd
        // Fonts v3 moved the Material Design icons out of `0xf534..0xfd46`
        // and up to `0xf0001..`, leaving the old codepoints unassigned. A
        // glyph left behind there does not fail, or warn, or look wrong to
        // whoever wrote it — it draws a hollow box on every machine with a
        // font from the last few years. `DIRECTORY_OPEN` was one, so every
        // open directory in the tree had one, and nothing said so.
        //
        // The replacements are Codicons and the v3 Material range, both of
        // which are outside this window.
        for (name, glyph) in all() {
            let code = glyph as u32;
            assert!(
                !(0xf534..=0xfd46).contains(&code),
                "`{name}` is {glyph:?} (U+{code:04X}), which Nerd Fonts v3 \
                 no longer assigns — it will draw as a hollow box"
            );
        }
    }

    #[test]
    fn every_symbol_kind_has_a_glyph_of_its_own_family() {
        // Codicons, which is one set: the previous mix of Font Awesome,
        // Material and an extension pack read as a ransom note even where
        // the font had all of it.
        for kind in 1u8..=26 {
            let code = super::for_symbol(kind) as u32;
            assert!(
                (0xea60..=0xebeb).contains(&code),
                "symbol kind {kind} is U+{code:04X}, which is not a codicon"
            );
        }
    }

    #[test]
    fn a_kind_the_protocol_has_not_invented_yet_still_draws_something() {
        // `SymbolKind` is a number on the wire and a server may send one
        // this was never told about.
        assert_eq!(super::for_symbol(200), super::for_symbol(0));
    }

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
