//! Declarative specifications the config file produces.
//!
//! These are plain data: `maxgus-faces` turns a [`ThemeSpec`] into resolved
//! terminal attributes, `maxgus-core` turns a [`KeymapSpec`] into a live keymap,
//! and `maxgus-lsp` uses [`LspSpec`] to launch servers. Keeping them here means
//! the parser has no dependency on any of those crates.

use maxgus_keys::{KeySequence, Keymap};

/// The `grammars { … }` block: where to find tree-sitter grammars the
/// editor was not built with.
///
/// Empty by default, and empty means none are looked for. Loading a grammar
/// means loading a shared library, so it happens only where a configuration
/// file has said where to look.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GrammarConfig {
    /// Directories holding `libtree-sitter-<language>.so` and its kin.
    pub search: Vec<std::path::PathBuf>,
    /// Directories holding `<language>/highlights.scm`.
    pub queries: Vec<std::path::PathBuf>,
    /// Grammars named outright, for one that is not where the search
    /// directories would look.
    pub named: Vec<NamedGrammar>,
}

/// One `grammar "go" library="…" queries="…"` node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedGrammar {
    pub language: String,
    pub library: std::path::PathBuf,
    /// The query to colour it with. Without one, the `queries` directories
    /// are searched as they are for a discovered grammar.
    pub queries: Option<std::path::PathBuf>,
}

/// One `keymap "name" { … }` block.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeymapSpec {
    /// `global`, a major-mode name such as `rust-mode`, or a minor-mode name.
    pub name: String,
    pub bindings: Vec<(KeySequence, String)>,
    /// Sequences removed with `unbind`.
    pub unbound: Vec<KeySequence>,
}

impl KeymapSpec {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Default::default()
        }
    }

    /// Builds a keymap from this spec. Conflicting bindings — one sequence
    /// used as a prefix of another — are reported by the underlying keymap.
    pub fn to_keymap(&self) -> maxgus_keys::Result<Keymap> {
        let mut map = Keymap::new(self.name.clone());
        for (seq, command) in &self.bindings {
            map.define(seq, command.clone())?;
        }
        Ok(map)
    }

    /// Applies this spec on top of an existing map: bindings override, and
    /// `unbind` removes.
    pub fn apply_to(&self, map: &mut Keymap) -> maxgus_keys::Result<()> {
        for seq in &self.unbound {
            map.undefine(seq);
        }
        for (seq, command) in &self.bindings {
            map.define(seq, command.clone())?;
        }
        Ok(())
    }
}

/// One `face "name" …` line: a colour and attribute override.
///
/// Colours are kept as written so `maxgus-faces` can resolve `#rrggbb`, an
/// ANSI index, or a named colour against the terminal's capabilities.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FaceSpec {
    pub name: String,
    pub foreground: Option<String>,
    pub background: Option<String>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub reverse: Option<bool>,
    pub dim: Option<bool>,
    pub strikethrough: Option<bool>,
    /// `inherit` copies unset attributes from another face.
    pub inherit: Option<String>,
    /// One-based line in the config file, so a complaint about this face can
    /// say where it is. Zero for a face built in code.
    pub line: usize,
}

impl FaceSpec {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Default::default()
        }
    }

    /// True when the spec sets nothing at all.
    pub fn is_empty(&self) -> bool {
        self.foreground.is_none()
            && self.background.is_none()
            && self.bold.is_none()
            && self.italic.is_none()
            && self.underline.is_none()
            && self.reverse.is_none()
            && self.dim.is_none()
            && self.strikethrough.is_none()
            && self.inherit.is_none()
    }

    /// Overlays `other` onto this spec; `other`'s set fields win.
    pub fn overlay(&mut self, other: &FaceSpec) {
        macro_rules! take {
            ($($field:ident),*) => {$(
                if other.$field.is_some() {
                    self.$field = other.$field.clone();
                }
            )*};
        }
        take!(
            foreground,
            background,
            bold,
            italic,
            underline,
            reverse,
            dim,
            strikethrough,
            inherit
        );
    }
}

/// One `theme "name" { … }` block.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ThemeSpec {
    pub name: String,
    pub faces: Vec<FaceSpec>,
    /// The built-in theme this one starts from, so anything it leaves unset
    /// has a sensible value. `None` means the built-in of the same name, and
    /// failing that the default — which is what a block customising a
    /// built-in wants, and what a light theme must override.
    pub base: Option<String>,
}

impl ThemeSpec {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            faces: Vec::new(),
            base: None,
        }
    }

    pub fn face(&self, name: &str) -> Option<&FaceSpec> {
        self.faces.iter().find(|f| f.name == name)
    }

    /// Merges `other`'s faces into this theme, overlaying same-named faces.
    pub fn merge(&mut self, other: &ThemeSpec) {
        if other.base.is_some() {
            self.base = other.base.clone();
        }
        for face in &other.faces {
            match self.faces.iter_mut().find(|f| f.name == face.name) {
                Some(existing) => existing.overlay(face),
                None => self.faces.push(face.clone()),
            }
        }
    }
}

/// One `lsp "language" …` line.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LspSpec {
    /// Language identifier, matching the one derived from the file extension.
    pub language: String,
    /// Executable to launch.
    pub command: String,
    pub args: Vec<String>,
    /// Files or directories whose presence marks the project root. When empty
    /// the editor falls back to the nearest VCS directory.
    pub root_markers: Vec<String>,
}

impl LspSpec {
    pub fn new(language: impl Into<String>, command: impl Into<String>) -> Self {
        Self {
            language: language.into(),
            command: command.into(),
            args: Vec::new(),
            root_markers: Vec::new(),
        }
    }
}

/// The `tree { … }` block configuring the file tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeConfig {
    /// Show dotfiles.
    pub show_hidden: bool,
    /// Directory and file names never shown.
    pub ignore: Vec<String>,
    /// Width of the tree side window, in columns.
    pub width: usize,
    /// Keep the tree selection in sync with the current buffer, as
    /// `treemacs-follow-mode` does.
    pub follow: bool,
    /// Sort directories before files.
    pub directories_first: bool,
    /// Show the git status indicator column.
    pub git_status: bool,
}

impl Default for TreeConfig {
    fn default() -> Self {
        Self {
            show_hidden: false,
            ignore: ["target", "node_modules", ".git"]
                .map(String::from)
                .to_vec(),
            width: 32,
            follow: true,
            directories_first: true,
            git_status: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seq(s: &str) -> KeySequence {
        KeySequence::parse(s).unwrap()
    }

    #[test]
    fn a_keymap_spec_builds_a_keymap() {
        let mut spec = KeymapSpec::new("global");
        spec.bindings.push((seq("C-x C-f"), "find-file".into()));
        let map = spec.to_keymap().unwrap();
        assert_eq!(map.lookup(&seq("C-x C-f")).command(), Some("find-file"));
        assert_eq!(map.name(), "global");
    }

    #[test]
    fn applying_a_spec_overrides_and_unbinds() {
        let mut base = Keymap::new("global");
        base.define_str("C-x C-f", "find-file").unwrap();
        base.define_str("C-z", "suspend").unwrap();

        let mut spec = KeymapSpec::new("global");
        spec.bindings.push((seq("C-x C-f"), "my-find-file".into()));
        spec.unbound.push(seq("C-z"));
        spec.apply_to(&mut base).unwrap();

        assert_eq!(base.lookup(&seq("C-x C-f")).command(), Some("my-find-file"));
        assert!(base.lookup(&seq("C-z")).is_undefined());
    }

    #[test]
    fn conflicting_bindings_surface_as_an_error() {
        let mut spec = KeymapSpec::new("bad");
        spec.bindings.push((seq("C-a"), "one".into()));
        spec.bindings.push((seq("C-a C-b"), "two".into()));
        assert!(spec.to_keymap().is_err());
    }

    #[test]
    fn face_overlay_keeps_unset_fields() {
        let mut base = FaceSpec {
            name: "default".into(),
            foreground: Some("#ffffff".into()),
            background: Some("#000000".into()),
            bold: Some(true),
            ..Default::default()
        };
        let over = FaceSpec {
            name: "default".into(),
            foreground: Some("#cccccc".into()),
            italic: Some(true),
            ..Default::default()
        };
        base.overlay(&over);
        assert_eq!(base.foreground.as_deref(), Some("#cccccc"));
        assert_eq!(
            base.background.as_deref(),
            Some("#000000"),
            "not overridden"
        );
        assert_eq!(base.bold, Some(true));
        assert_eq!(base.italic, Some(true));
    }

    #[test]
    fn an_empty_face_spec_is_recognised() {
        assert!(FaceSpec::new("x").is_empty());
        let mut f = FaceSpec::new("x");
        f.bold = Some(false);
        assert!(
            !f.is_empty(),
            "setting bold=false is still setting something"
        );
    }

    #[test]
    fn merging_themes_overlays_matching_faces_and_appends_new_ones() {
        let mut base = ThemeSpec::new("dark");
        base.faces.push(FaceSpec {
            name: "default".into(),
            bold: Some(true),
            ..Default::default()
        });

        let mut over = ThemeSpec::new("dark");
        over.faces.push(FaceSpec {
            name: "default".into(),
            foreground: Some("#fff".into()),
            ..Default::default()
        });
        over.faces.push(FaceSpec::new("region"));

        base.merge(&over);
        assert_eq!(base.faces.len(), 2);
        let d = base.face("default").unwrap();
        assert_eq!(d.bold, Some(true));
        assert_eq!(d.foreground.as_deref(), Some("#fff"));
        assert!(base.face("region").is_some());
        assert!(base.face("missing").is_none());
    }

    #[test]
    fn tree_defaults_ignore_the_usual_build_directories() {
        let t = TreeConfig::default();
        assert!(!t.show_hidden);
        assert!(t.ignore.contains(&"target".to_string()));
        assert!(t.ignore.contains(&".git".to_string()));
        assert!(t.follow);
        assert!(t.directories_first);
    }

    #[test]
    fn an_lsp_spec_defaults_to_no_arguments() {
        let s = LspSpec::new("rust", "rust-analyzer");
        assert_eq!(s.language, "rust");
        assert!(s.args.is_empty());
        assert!(s.root_markers.is_empty());
    }
}
