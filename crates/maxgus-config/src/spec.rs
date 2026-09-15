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

    /// Builds a keymap from this spec, and says what could not be bound.
    pub fn to_keymap(&self) -> (Keymap, Vec<String>) {
        let mut map = Keymap::new(self.name.clone());
        let problems = self.apply_to(&mut map);
        (map, problems)
    }

    /// Applies this spec on top of an existing map: `unbind` removes, then
    /// bindings override. Returns a sentence for each thing that went wrong
    /// or was lost on the way.
    ///
    /// No problem stops the rest: the first binding that could not be made
    /// used to end the block, and every binding after it went unmade with
    /// only the first one mentioned. And a sequence bound over a prefix took
    /// the prefix's bindings with it in silence — `C-c f`, bound as the
    /// documentation's own example did, removed every `C-c f` key there was.
    pub fn apply_to(&self, map: &mut Keymap) -> Vec<String> {
        let mut problems = Vec::new();
        for seq in &self.unbound {
            // A prefix named here is meant whole, which is also the way to
            // bind a prefix's key to a command without being told about it.
            if map.remove(seq).is_empty() && map.name() == "global" {
                problems.push(format!(
                    "`unbind \"{}\"`: nothing is bound to it",
                    seq.notation()
                ));
            }
        }
        for (seq, command) in &self.bindings {
            let under = map.bindings_under(seq);
            if let Err(error) = map.define(seq, command.clone()) {
                problems.push(error.to_string());
                continue;
            }
            if !under.is_empty() {
                let named: Vec<String> = under
                    .iter()
                    .take(2)
                    .map(|(keys, _)| format!("`{}`", keys.notation()))
                    .collect();
                problems.push(format!(
                    "binding `{keys}` to `{command}` took away the {count} under it ({named}{more}); \
                     `unbind \"{keys}\"` first if that is meant",
                    keys = seq.notation(),
                    count = match under.len() {
                        1 => "binding".to_string(),
                        n => format!("{n} bindings"),
                    },
                    named = named.join(", "),
                    more = if under.len() > 2 { ", …" } else { "" },
                ));
            }
        }
        problems
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
    pub undercurl: Option<bool>,
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
            && self.undercurl.is_none()
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
            undercurl,
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
        let (map, problems) = spec.to_keymap();
        assert!(problems.is_empty(), "{problems:?}");
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
        assert!(spec.apply_to(&mut base).is_empty());

        assert_eq!(base.lookup(&seq("C-x C-f")).command(), Some("my-find-file"));
        assert!(base.lookup(&seq("C-z")).is_undefined());
    }

    #[test]
    fn conflicting_bindings_surface_as_a_problem_and_the_rest_are_made() {
        let mut spec = KeymapSpec::new("bad");
        spec.bindings.push((seq("C-a"), "one".into()));
        spec.bindings.push((seq("C-a C-b"), "two".into()));
        spec.bindings.push((seq("C-e"), "three".into()));
        let (map, problems) = spec.to_keymap();
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert_eq!(
            map.lookup(&seq("C-e")).command(),
            Some("three"),
            "the binding after the conflict was not made"
        );
    }

    #[test]
    fn binding_over_a_prefix_says_what_it_took_away() {
        let mut base = Keymap::new("global");
        base.define_str("C-c f p", "edit-configuration").unwrap();
        base.define_str("C-c f f", "find-file").unwrap();
        base.define_str("C-c f y", "yank-buffer-path").unwrap();

        let mut spec = KeymapSpec::new("global");
        spec.bindings
            .push((seq("C-c f"), "lsp-format-buffer".into()));
        let problems = spec.apply_to(&mut base);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(
            problems[0].contains("3 bindings") && problems[0].contains("unbind"),
            "{}",
            problems[0]
        );
        assert_eq!(
            base.lookup(&seq("C-c f")).command(),
            Some("lsp-format-buffer"),
            "the binding asked for was not made"
        );
    }

    #[test]
    fn unbinding_a_prefix_first_takes_it_whole_and_quietly() {
        let mut base = Keymap::new("global");
        base.define_str("C-c f p", "edit-configuration").unwrap();
        let mut spec = KeymapSpec::new("global");
        spec.unbound.push(seq("C-c f"));
        spec.bindings
            .push((seq("C-c f"), "lsp-format-buffer".into()));
        assert!(spec.apply_to(&mut base).is_empty());
        assert_eq!(
            base.lookup(&seq("C-c f")).command(),
            Some("lsp-format-buffer")
        );
    }

    #[test]
    fn unbinding_what_is_not_bound_is_mentioned() {
        let mut base = Keymap::new("global");
        let mut spec = KeymapSpec::new("global");
        spec.unbound.push(seq("C-q"));
        let problems = spec.apply_to(&mut base);
        assert!(problems[0].contains("nothing is bound"), "{problems:?}");
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
