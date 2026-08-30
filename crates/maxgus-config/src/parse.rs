//! The KDL walker.
//!
//! Everything here is total: a malformed or unrecognised node records a
//! [`Warning`] and parsing continues. Only a KDL syntax error — which means
//! the file cannot be read as a document at all — fails the load.

use crate::{
    Result,
    error::{ConfigError, Warning, line_of},
    settings::{Settings, closest_setting},
    spec::{FaceSpec, KeymapSpec, LspSpec, ThemeSpec, TreeConfig},
};
use kdl::{KdlDocument, KdlNode, KdlValue};
use maxgus_keys::KeySequence;

/// Everything a config file can express.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Config {
    pub settings: Settings,
    pub keymaps: Vec<KeymapSpec>,
    pub themes: Vec<ThemeSpec>,
    pub lsp: Vec<LspSpec>,
    pub tree: TreeConfig,
    pub grammars: crate::spec::GrammarConfig,
    /// Non-fatal complaints, in file order.
    pub warnings: Vec<Warning>,
}

/// Parser state: the source text (for line numbers) and the config being built.
struct Parser<'a> {
    source: &'a str,
    config: Config,
}

impl Config {
    /// Parses a configuration document.
    pub fn parse(source: &str) -> Result<Config> {
        let doc = KdlDocument::parse(source).map_err(ConfigError::from)?;
        let mut parser = Parser {
            source,
            config: Config::default(),
        };
        parser.document(&doc);
        Ok(parser.config)
    }

    /// The keymap spec for `name`, if the file defined one.
    pub fn keymap(&self, name: &str) -> Option<&KeymapSpec> {
        self.keymaps.iter().find(|k| k.name == name)
    }

    /// The theme spec for `name`, if the file defined one.
    /// Folds themes read from elsewhere — the `themes/` directory — into this
    /// configuration, letting a `theme` block in the main file still override
    /// one of them face by face.
    pub fn merge_themes(&mut self, themes: Vec<ThemeSpec>) {
        for theme in themes {
            match self.themes.iter_mut().find(|t| t.name == theme.name) {
                // The main file was read first, so it wins: its faces are
                // laid back over the ones the file supplied.
                Some(existing) => {
                    let mut merged = theme;
                    merged.merge(existing);
                    *existing = merged;
                }
                None => self.themes.push(theme),
            }
        }
    }

    pub fn theme(&self, name: &str) -> Option<&ThemeSpec> {
        self.themes.iter().find(|t| t.name == name)
    }

    /// The language server configured for `language`, if any.
    pub fn lsp_for(&self, language: &str) -> Option<&LspSpec> {
        self.lsp.iter().find(|l| l.language == language)
    }

    /// Layers `other` on top of this configuration, as a project-local file is
    /// layered over the user's. Later definitions win.
    pub fn merge(&mut self, other: &Config) {
        self.settings = other.settings.clone();
        self.tree = other.tree.clone();
        for spec in &other.keymaps {
            match self.keymaps.iter_mut().find(|k| k.name == spec.name) {
                Some(existing) => {
                    existing.bindings.extend(spec.bindings.iter().cloned());
                    existing.unbound.extend(spec.unbound.iter().cloned());
                }
                None => self.keymaps.push(spec.clone()),
            }
        }
        for theme in &other.themes {
            match self.themes.iter_mut().find(|t| t.name == theme.name) {
                Some(existing) => existing.merge(theme),
                None => self.themes.push(theme.clone()),
            }
        }
        for spec in &other.lsp {
            match self.lsp.iter_mut().find(|l| l.language == spec.language) {
                Some(existing) => *existing = spec.clone(),
                None => self.lsp.push(spec.clone()),
            }
        }
        self.warnings.extend(other.warnings.iter().cloned());
    }
}

// ---- value accessors ---------------------------------------------------

/// Positional arguments of a node, in order.
fn args(node: &KdlNode) -> Vec<&KdlValue> {
    node.entries()
        .iter()
        .filter(|e| e.name().is_none())
        .map(|e| e.value())
        .collect()
}

/// Positional arguments coerced to strings, skipping any that are not.
fn string_args(node: &KdlNode) -> Vec<String> {
    args(node)
        .iter()
        .filter_map(|v| v.as_string().map(str::to_string))
        .collect()
}

/// A named property, if present.
fn prop<'a>(node: &'a KdlNode, key: &str) -> Option<&'a KdlValue> {
    node.entries()
        .iter()
        .find(|e| e.name().is_some_and(|n| n.value() == key))
        .map(|e| e.value())
}

impl<'a> Parser<'a> {
    fn line(&self, node: &KdlNode) -> usize {
        line_of(self.source, node.span().offset())
    }

    fn warn(&mut self, node: &KdlNode, message: impl Into<String>) {
        let line = self.line(node);
        self.config.warnings.push(Warning::new(line, message));
    }

    /// The node's first positional argument as a string, warning when absent.
    fn required_name(&mut self, node: &KdlNode, what: &str) -> Option<String> {
        match args(node).first().and_then(|v| v.as_string()) {
            Some(s) => Some(s.to_string()),
            None => {
                self.warn(
                    node,
                    format!(
                        "`{}` needs a {what} as its first argument",
                        node.name().value()
                    ),
                );
                None
            }
        }
    }

    fn bool_value(&mut self, node: &KdlNode, key: &str, value: &KdlValue) -> Option<bool> {
        match value {
            KdlValue::Bool(b) => Some(*b),
            other => {
                self.warn(
                    node,
                    format!("`{key}` expects #true or #false, found {other}"),
                );
                None
            }
        }
    }

    fn usize_value(&mut self, node: &KdlNode, key: &str, value: &KdlValue) -> Option<usize> {
        match value {
            KdlValue::Integer(i) if *i >= 0 => Some(*i as usize),
            KdlValue::Integer(_) => {
                self.warn(node, format!("`{key}` must not be negative"));
                None
            }
            other => {
                self.warn(node, format!("`{key}` expects an integer, found {other}"));
                None
            }
        }
    }

    fn string_value(&mut self, node: &KdlNode, key: &str, value: &KdlValue) -> Option<String> {
        match value.as_string() {
            Some(s) => Some(s.to_string()),
            None => {
                self.warn(node, format!("`{key}` expects a string, found {value}"));
                None
            }
        }
    }

    // ---- top level -----------------------------------------------------

    fn document(&mut self, doc: &KdlDocument) {
        for node in doc.nodes() {
            match node.name().value() {
                "set" => self.set_node(node),
                "keymap" => self.keymap_node(node),
                "theme" => self.theme_node(node),
                "lsp" => self.lsp_node(node),
                "tree" => self.tree_node(node),
                "grammars" => self.grammars_node(node),
                "grammar" => self.grammar_node(node),
                other => self.warn(node, format!("unknown node `{other}`, ignored")),
            }
        }
    }

    /// `set key=value …`
    fn set_node(&mut self, node: &KdlNode) {
        let props: Vec<(String, KdlValue)> = node
            .entries()
            .iter()
            .filter_map(|e| e.name().map(|n| (n.value().to_string(), e.value().clone())))
            .collect();
        if props.is_empty() {
            self.warn(node, "`set` needs at least one `key=value` property");
            return;
        }
        for (key, value) in props {
            self.apply_setting(node, &key, &value);
        }
    }

    fn apply_setting(&mut self, node: &KdlNode, key: &str, value: &KdlValue) {
        match key {
            "tab-width" => {
                if let Some(n) = self.usize_value(node, key, value) {
                    if n == 0 {
                        self.warn(node, "`tab-width` must be at least 1");
                    } else {
                        self.config.settings.tab_width = n;
                    }
                }
            }
            "indent-with-tabs" => {
                if let Some(b) = self.bool_value(node, key, value) {
                    self.config.settings.indent_with_tabs = b;
                }
            }
            "theme" => {
                if let Some(s) = self.string_value(node, key, value) {
                    self.config.settings.theme = s;
                }
            }
            "line-numbers" => {
                if let Some(b) = self.bool_value(node, key, value) {
                    self.config.settings.line_numbers = b;
                }
            }
            "truncate-lines" => {
                if let Some(b) = self.bool_value(node, key, value) {
                    self.config.settings.truncate_lines = b;
                }
            }
            "scroll-margin" => {
                if let Some(n) = self.usize_value(node, key, value) {
                    self.config.settings.scroll_margin = n;
                }
            }
            "fill-column" => {
                if let Some(n) = self.usize_value(node, key, value) {
                    self.config.settings.fill_column = n;
                }
            }
            "kill-ring-max" => {
                if let Some(n) = self.usize_value(node, key, value) {
                    self.config.settings.kill_ring_max = n.max(1);
                }
            }
            "case-fold-search" => match value {
                // `null` restores the smart-case heuristic.
                KdlValue::Null => self.config.settings.case_fold_search = None,
                _ => {
                    if let Some(b) = self.bool_value(node, key, value) {
                        self.config.settings.case_fold_search = Some(b);
                    }
                }
            },
            "require-final-newline" => {
                if let Some(b) = self.bool_value(node, key, value) {
                    self.config.settings.require_final_newline = b;
                }
            }
            "delete-trailing-whitespace" => {
                if let Some(b) = self.bool_value(node, key, value) {
                    self.config.settings.delete_trailing_whitespace = b;
                }
            }
            "backup-files" => {
                if let Some(b) = self.bool_value(node, key, value) {
                    self.config.settings.backup_files = b;
                }
            }
            "syntax-highlighting" => {
                if let Some(b) = self.bool_value(node, key, value) {
                    self.config.settings.syntax_highlighting = b;
                }
            }
            "lsp-enabled" => {
                if let Some(b) = self.bool_value(node, key, value) {
                    self.config.settings.lsp_enabled = b;
                }
            }
            "idle-delay-ms" => {
                if let Some(n) = self.usize_value(node, key, value) {
                    self.config.settings.idle_delay_ms = n as u64;
                }
            }
            "fill-column-indicator" => {
                if let Some(b) = self.bool_value(node, key, value) {
                    self.config.settings.fill_column_indicator = b;
                }
            }
            "nerd-font-icons" => {
                if let Some(b) = self.bool_value(node, key, value) {
                    self.config.settings.nerd_font_icons = b;
                }
            }
            "panel-tree" => {
                if let Some(b) = self.bool_value(node, key, value) {
                    self.config.settings.panel_tree = b;
                }
            }
            "panel-symbols" => {
                if let Some(b) = self.bool_value(node, key, value) {
                    self.config.settings.panel_symbols = b;
                }
            }
            "panel-buffers" => {
                if let Some(b) = self.bool_value(node, key, value) {
                    self.config.settings.panel_buffers = b;
                }
            }
            "panel-at-startup" => {
                if let Some(b) = self.bool_value(node, key, value) {
                    self.config.settings.panel_at_startup = b;
                }
            }
            "panel-symbols-height" => {
                if let Some(n) = self.usize_value(node, key, value) {
                    self.config.settings.panel_symbols_height = n;
                }
            }
            "panel-buffers-height" => {
                if let Some(n) = self.usize_value(node, key, value) {
                    self.config.settings.panel_buffers_height = n;
                }
            }
            "shell" => {
                if let Some(text) = self.string_value(node, key, value) {
                    self.config.settings.shell = Some(text);
                }
            }
            "beacon" => {
                if let Some(on) = self.bool_value(node, key, value) {
                    self.config.settings.beacon = on;
                }
            }
            "beacon-size" => {
                if let Some(n) = self.usize_value(node, key, value) {
                    self.config.settings.beacon_size = n.clamp(1, 500);
                }
            }
            "beacon-blink-delay-ms" => {
                if let Some(n) = self.usize_value(node, key, value) {
                    self.config.settings.beacon_blink_delay_ms = n.min(10_000);
                }
            }
            "beacon-blink-duration-ms" => {
                if let Some(n) = self.usize_value(node, key, value) {
                    self.config.settings.beacon_blink_duration_ms = n.clamp(1, 10_000);
                }
            }
            "beacon-color" => {
                if let Some(text) = self.string_value(node, key, value) {
                    self.config.settings.beacon_color = text;
                }
            }
            "beacon-blink-when-buffer-changes" => {
                if let Some(on) = self.bool_value(node, key, value) {
                    self.config.settings.beacon_blink_when_buffer_changes = on;
                }
            }
            "beacon-blink-when-window-scrolls" => {
                if let Some(on) = self.bool_value(node, key, value) {
                    self.config.settings.beacon_blink_when_window_scrolls = on;
                }
            }
            "beacon-blink-when-window-changes" => {
                if let Some(on) = self.bool_value(node, key, value) {
                    self.config.settings.beacon_blink_when_window_changes = on;
                }
            }
            "beacon-blink-when-point-moves-vertically" => {
                if let Some(n) = self.usize_value(node, key, value) {
                    self.config
                        .settings
                        .beacon_blink_when_point_moves_vertically = n;
                }
            }
            "session" => {
                if let Some(on) = self.bool_value(node, key, value) {
                    self.config.settings.session = on;
                }
            }
            "gui-font" => {
                if let Some(text) = self.string_value(node, key, value) {
                    self.config.settings.gui_font = text;
                }
            }
            "gui-font-size" => {
                if let Some(n) = self.usize_value(node, key, value) {
                    self.config.settings.gui_font_size = n.clamp(6, 96);
                }
            }
            "autocomplete" => {
                if let Some(b) = self.bool_value(node, key, value) {
                    self.config.settings.autocomplete = b;
                }
            }
            "autocomplete-min-chars" => {
                if let Some(n) = self.usize_value(node, key, value) {
                    self.config.settings.autocomplete_min_chars = n.clamp(1, 10);
                }
            }
            "lsp-doc" => {
                if let Some(b) = self.bool_value(node, key, value) {
                    self.config.settings.lsp_doc = b;
                }
            }
            "which-key" => {
                if let Some(b) = self.bool_value(node, key, value) {
                    self.config.settings.which_key = b;
                }
            }
            "which-key-delay-ms" => {
                if let Some(n) = self.usize_value(node, key, value) {
                    self.config.settings.which_key_delay_ms = n.min(10_000);
                }
            }
            "mouse-wheel-lines" => {
                if let Some(n) = self.usize_value(node, key, value) {
                    self.config.settings.mouse_wheel_lines = n.clamp(1, 50);
                }
            }
            "smooth-scroll-ms" => {
                if let Some(n) = self.usize_value(node, key, value) {
                    // Past a second it is no longer scrolling, it is an
                    // animation being watched.
                    self.config.settings.smooth_scroll_ms = n.min(1000);
                }
            }
            "blink-cursor" => {
                if let Some(b) = self.bool_value(node, key, value) {
                    self.config.settings.blink_cursor = b;
                }
            }
            "echo-keystrokes-ms" => {
                if let Some(n) = self.usize_value(node, key, value) {
                    self.config.settings.echo_keystrokes_ms = n as u64;
                }
            }
            other => {
                let hint = closest_setting(other)
                    .map(|c| format!(", did you mean `{c}`?"))
                    .unwrap_or_default();
                self.warn(node, format!("unknown setting `{other}`{hint}"));
            }
        }
    }

    /// `keymap "name" { bind "keys" "command"; unbind "keys" }`
    fn keymap_node(&mut self, node: &KdlNode) {
        let Some(name) = self.required_name(node, "keymap name") else {
            return;
        };
        let Some(children) = node.children() else {
            self.warn(node, format!("`keymap \"{name}\"` has no bindings"));
            return;
        };
        let mut spec = KeymapSpec::new(name);
        for child in children.nodes() {
            match child.name().value() {
                "bind" => self.bind_node(child, &mut spec),
                "unbind" => self.unbind_node(child, &mut spec),
                other => self.warn(
                    child,
                    format!("unknown node `{other}` inside `keymap`, ignored"),
                ),
            }
        }
        // Merge into an earlier block of the same name rather than shadowing.
        match self.config.keymaps.iter_mut().find(|k| k.name == spec.name) {
            Some(existing) => {
                existing.bindings.extend(spec.bindings);
                existing.unbound.extend(spec.unbound);
            }
            None => self.config.keymaps.push(spec),
        }
    }

    fn bind_node(&mut self, node: &KdlNode, spec: &mut KeymapSpec) {
        let a = string_args(node);
        let [keys, command] = a.as_slice() else {
            self.warn(
                node,
                "`bind` takes exactly two strings: a key sequence and a command",
            );
            return;
        };
        match KeySequence::parse(keys) {
            Ok(seq) if seq.is_empty() => self.warn(node, "`bind` was given an empty key sequence"),
            Ok(seq) => spec.bindings.push((seq, command.clone())),
            Err(e) => self.warn(node, format!("cannot parse key sequence `{keys}`: {e}")),
        }
    }

    fn unbind_node(&mut self, node: &KdlNode, spec: &mut KeymapSpec) {
        let a = string_args(node);
        if a.is_empty() {
            self.warn(node, "`unbind` takes one or more key sequences");
            return;
        }
        for keys in a {
            match KeySequence::parse(&keys) {
                Ok(seq) if !seq.is_empty() => spec.unbound.push(seq),
                Ok(_) => self.warn(node, "`unbind` was given an empty key sequence"),
                Err(e) => self.warn(node, format!("cannot parse key sequence `{keys}`: {e}")),
            }
        }
    }

    /// `theme "name" { face "name" fg="…" bold=#true }`
    fn theme_node(&mut self, node: &KdlNode) {
        let Some(name) = self.required_name(node, "theme name") else {
            return;
        };
        let Some(children) = node.children() else {
            self.warn(node, format!("`theme \"{name}\"` has no faces"));
            return;
        };
        let mut spec = ThemeSpec::new(name);
        // `base=` lets a theme file stand on its own: a light theme has to
        // start from the light built-in or every face it omits comes out dark.
        if let Some(value) = prop(node, "base").cloned() {
            spec.base = self.string_value(node, "base", &value);
        }
        for child in children.nodes() {
            match child.name().value() {
                "face" => self.face_node(child, &mut spec),
                other => self.warn(
                    child,
                    format!("unknown node `{other}` inside `theme`, ignored"),
                ),
            }
        }
        match self.config.themes.iter_mut().find(|t| t.name == spec.name) {
            Some(existing) => existing.merge(&spec),
            None => self.config.themes.push(spec),
        }
    }

    fn face_node(&mut self, node: &KdlNode, theme: &mut ThemeSpec) {
        let Some(name) = self.required_name(node, "face name") else {
            return;
        };
        let mut face = FaceSpec::new(&name);
        // Recorded so whoever knows which faces exist can point at this line.
        face.line = self.line(node);
        for entry in node.entries() {
            let Some(key) = entry.name().map(|n| n.value().to_string()) else {
                continue;
            };
            let value = entry.value().clone();
            match key.as_str() {
                // `fg`/`bg` are the short spellings; the long ones also work.
                "fg" | "foreground" => face.foreground = self.string_value(node, &key, &value),
                "bg" | "background" => face.background = self.string_value(node, &key, &value),
                "inherit" => face.inherit = self.string_value(node, &key, &value),
                "bold" => face.bold = self.bool_value(node, &key, &value),
                "italic" => face.italic = self.bool_value(node, &key, &value),
                "underline" => face.underline = self.bool_value(node, &key, &value),
                "reverse" => face.reverse = self.bool_value(node, &key, &value),
                "dim" => face.dim = self.bool_value(node, &key, &value),
                "strikethrough" => face.strikethrough = self.bool_value(node, &key, &value),
                other => self.warn(node, format!("unknown face attribute `{other}`, ignored")),
            }
        }
        if face.is_empty() {
            self.warn(node, format!("`face \"{name}\"` sets no attributes"));
            return;
        }
        match theme.faces.iter_mut().find(|f| f.name == face.name) {
            Some(existing) => existing.overlay(&face),
            None => theme.faces.push(face),
        }
    }

    /// `lsp "language" command="…" { args "…"; root-markers "…" }`
    fn lsp_node(&mut self, node: &KdlNode) {
        let Some(language) = self.required_name(node, "language name") else {
            return;
        };
        let Some(command) = prop(node, "command").cloned() else {
            self.warn(
                node,
                format!("`lsp \"{language}\"` needs a `command=` property"),
            );
            return;
        };
        let Some(command) = self.string_value(node, "command", &command) else {
            return;
        };
        let mut spec = LspSpec::new(&language, command);

        // A single argument may be given inline as `args="--stdio"`.
        if let Some(v) = prop(node, "args").cloned()
            && let Some(s) = self.string_value(node, "args", &v)
        {
            spec.args.push(s);
        }
        if let Some(children) = node.children() {
            for child in children.nodes() {
                match child.name().value() {
                    "args" => spec.args.extend(string_args(child)),
                    "root-markers" => spec.root_markers.extend(string_args(child)),
                    other => {
                        self.warn(
                            child,
                            format!("unknown node `{other}` inside `lsp`, ignored"),
                        );
                    }
                }
            }
        }
        match self.config.lsp.iter_mut().find(|l| l.language == language) {
            Some(existing) => *existing = spec,
            None => self.config.lsp.push(spec),
        }
    }

    /// `tree { show-hidden #true; ignore "a" "b"; width 40 }`
    /// `grammars { search "…" …; queries "…" … }`
    fn grammars_node(&mut self, node: &KdlNode) {
        let Some(children) = node.children() else {
            self.warn(node, "`grammars` has no directories in it");
            return;
        };
        for child in children.nodes() {
            let key = child.name().value().to_string();
            let paths = string_args(child);
            match key.as_str() {
                "search" | "queries" if paths.is_empty() => {
                    self.warn(child, format!("`{key}` takes one or more directories"));
                }
                "search" => self
                    .config
                    .grammars
                    .search
                    .extend(paths.into_iter().map(std::path::PathBuf::from)),
                "queries" => self
                    .config
                    .grammars
                    .queries
                    .extend(paths.into_iter().map(std::path::PathBuf::from)),
                other => self.warn(child, format!("unknown `grammars` node `{other}`, ignored")),
            }
        }
    }

    /// `grammar "go" library="/usr/lib/libtree-sitter-go.so" queries="…"`
    fn grammar_node(&mut self, node: &KdlNode) {
        let Some(language) = string_args(node).first().cloned() else {
            self.warn(node, "`grammar` needs a language, e.g. `grammar \"go\"`");
            return;
        };
        let Some(library) = self.string_property(node, "library") else {
            self.warn(
                node,
                format!("`grammar \"{language}\"` needs `library=\"…\"`"),
            );
            return;
        };
        let queries = self.string_property(node, "queries");
        self.config.grammars.named.push(crate::spec::NamedGrammar {
            language,
            library: library.into(),
            queries: queries.map(std::path::PathBuf::from),
        });
    }

    /// A `key="value"` property on a node, as a string.
    fn string_property(&mut self, node: &KdlNode, key: &str) -> Option<String> {
        let value = node
            .entries()
            .iter()
            .find(|e| e.name().is_some_and(|n| n.value() == key))?
            .value()
            .clone();
        self.string_value(node, key, &value)
    }

    fn tree_node(&mut self, node: &KdlNode) {
        let Some(children) = node.children() else {
            self.warn(node, "`tree` has no settings");
            return;
        };
        for child in children.nodes() {
            let key = child.name().value().to_string();
            let first = args(child).first().cloned().cloned();
            match key.as_str() {
                "ignore" => {
                    let names = string_args(child);
                    if names.is_empty() {
                        self.warn(child, "`ignore` takes one or more names");
                    } else {
                        self.config.tree.ignore = names;
                    }
                }
                "show-hidden" | "follow" | "directories-first" | "git-status" => {
                    // A bare node with no argument reads as "on".
                    let value = first.unwrap_or(KdlValue::Bool(true));
                    if let Some(b) = self.bool_value(child, &key, &value) {
                        match key.as_str() {
                            "show-hidden" => self.config.tree.show_hidden = b,
                            "follow" => self.config.tree.follow = b,
                            "directories-first" => self.config.tree.directories_first = b,
                            _ => self.config.tree.git_status = b,
                        }
                    }
                }
                "width" => match first {
                    Some(v) => {
                        if let Some(n) = self.usize_value(child, &key, &v) {
                            if n < 8 {
                                self.warn(child, "`width` must be at least 8 columns");
                            } else {
                                self.config.tree.width = n;
                            }
                        }
                    }
                    None => self.warn(child, "`width` takes a column count"),
                },
                other => self.warn(
                    child,
                    format!("unknown node `{other}` inside `tree`, ignored"),
                ),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str) -> Config {
        Config::parse(src).expect("valid KDL")
    }

    fn clean(src: &str) -> Config {
        let c = parse(src);
        assert!(
            c.warnings.is_empty(),
            "unexpected warnings: {:?}",
            c.warnings
        );
        c
    }

    #[test]
    fn an_empty_document_yields_the_defaults() {
        let c = clean("");
        assert_eq!(c.settings, Settings::default());
        assert_eq!(c.tree, TreeConfig::default());
        assert!(c.keymaps.is_empty());
    }

    #[test]
    fn a_syntax_error_fails_the_whole_load() {
        assert!(Config::parse("keymap \"unclosed").is_err());
    }

    #[test]
    fn settings_parse_from_set_nodes() {
        let c = clean(
            r##"
            set tab-width=8 indent-with-tabs=#true
            set theme="solarized" line-numbers=#true
            set fill-column=100 scroll-margin=3
            "##,
        );
        assert_eq!(c.settings.tab_width, 8);
        assert!(c.settings.indent_with_tabs);
        assert_eq!(c.settings.theme, "solarized");
        assert!(c.settings.line_numbers);
        assert_eq!(c.settings.fill_column, 100);
        assert_eq!(c.settings.scroll_margin, 3);
    }

    #[test]
    fn case_fold_search_accepts_a_tri_state() {
        assert_eq!(
            clean("set case-fold-search=#true")
                .settings
                .case_fold_search,
            Some(true)
        );
        assert_eq!(
            clean("set case-fold-search=#false")
                .settings
                .case_fold_search,
            Some(false)
        );
        assert_eq!(
            clean("set case-fold-search=#null")
                .settings
                .case_fold_search,
            None
        );
    }

    #[test]
    fn a_misspelled_setting_warns_with_a_suggestion() {
        let c = parse("set tab-widht=8");
        assert_eq!(c.warnings.len(), 1);
        assert!(c.warnings[0].message.contains("did you mean `tab-width`"));
        assert_eq!(c.settings.tab_width, 4, "the default is kept");
    }

    #[test]
    fn a_setting_of_the_wrong_type_warns_and_keeps_the_default() {
        let c = parse(r##"set tab-width="eight""##);
        assert_eq!(c.warnings.len(), 1);
        assert!(c.warnings[0].message.contains("expects an integer"));
        assert_eq!(c.settings.tab_width, 4);
    }

    #[test]
    fn out_of_range_settings_are_rejected() {
        let c = parse("set tab-width=0");
        assert!(c.warnings[0].message.contains("at least 1"));
        assert_eq!(c.settings.tab_width, 4);

        let c = parse("set scroll-margin=-2");
        assert!(c.warnings[0].message.contains("must not be negative"));
    }

    #[test]
    fn warnings_carry_the_line_they_came_from() {
        let c = parse("set tab-width=4\n\nset nonsense=1\n");
        assert_eq!(c.warnings.len(), 1);
        assert_eq!(c.warnings[0].line, 3);
    }

    #[test]
    fn unknown_top_level_nodes_are_skipped_not_fatal() {
        let c = parse("future-feature \"x\"\nset tab-width=2");
        assert_eq!(c.settings.tab_width, 2, "later nodes still parse");
        assert_eq!(c.warnings.len(), 1);
        assert!(
            c.warnings[0]
                .message
                .contains("unknown node `future-feature`")
        );
    }

    #[test]
    fn keymaps_parse_into_specs() {
        let c = clean(
            r##"
            keymap "global" {
                bind "C-x C-f" "find-file"
                bind "M-x" "execute-extended-command"
                unbind "C-z"
            }
            "##,
        );
        let k = c.keymap("global").unwrap();
        assert_eq!(k.bindings.len(), 2);
        assert_eq!(k.bindings[0].0.notation(), "C-x C-f");
        assert_eq!(k.bindings[0].1, "find-file");
        assert_eq!(k.unbound.len(), 1);
        assert_eq!(k.unbound[0].notation(), "C-z");
    }

    #[test]
    fn repeated_keymap_blocks_merge() {
        let c = clean(
            r##"
            keymap "global" { bind "C-a" "one" }
            keymap "global" { bind "C-b" "two" }
            "##,
        );
        assert_eq!(c.keymaps.len(), 1);
        assert_eq!(c.keymap("global").unwrap().bindings.len(), 2);
    }

    #[test]
    fn a_bad_key_sequence_warns_and_the_rest_of_the_map_survives() {
        let c = parse(
            r##"
            keymap "global" {
                bind "C-" "broken"
                bind "C-a" "fine"
            }
            "##,
        );
        assert_eq!(c.warnings.len(), 1);
        assert!(c.warnings[0].message.contains("cannot parse key sequence"));
        assert_eq!(c.keymap("global").unwrap().bindings.len(), 1);
    }

    #[test]
    fn bind_requires_exactly_two_arguments() {
        let c = parse(r##"keymap "global" { bind "C-a" }"##);
        assert!(c.warnings[0].message.contains("exactly two strings"));
        assert!(c.keymap("global").unwrap().bindings.is_empty());
    }

    #[test]
    fn unbind_accepts_several_sequences_at_once() {
        let c = clean(r##"keymap "global" { unbind "C-z" "C-x C-z" }"##);
        assert_eq!(c.keymap("global").unwrap().unbound.len(), 2);
    }

    #[test]
    fn a_keymap_without_a_name_or_body_warns() {
        let c = parse("keymap { bind \"C-a\" \"x\" }");
        assert!(c.warnings[0].message.contains("needs a keymap name"));
        let c = parse(r##"keymap "empty""##);
        assert!(c.warnings[0].message.contains("has no bindings"));
    }

    #[test]
    fn themes_and_faces_parse() {
        let c = clean(
            r##"
            theme "maxgus-dark" {
                face "default" fg="#c5c8c6" bg="#1d1f21"
                face "font-lock-keyword" fg="#b294bb" bold=#true
                face "region" background="#373b41"
                face "error" inherit="default" underline=#true
            }
            "##,
        );
        let t = c.theme("maxgus-dark").unwrap();
        assert_eq!(t.faces.len(), 4);
        assert_eq!(
            t.face("default").unwrap().foreground.as_deref(),
            Some("#c5c8c6")
        );
        assert_eq!(t.face("font-lock-keyword").unwrap().bold, Some(true));
        assert_eq!(
            t.face("region").unwrap().background.as_deref(),
            Some("#373b41")
        );
        assert_eq!(t.face("error").unwrap().inherit.as_deref(), Some("default"));
    }

    #[test]
    fn repeated_faces_within_a_theme_overlay() {
        let c = clean(
            r##"
            theme "t" {
                face "default" fg="#111111" bold=#true
                face "default" fg="#222222"
            }
            "##,
        );
        let f = c.theme("t").unwrap().face("default").unwrap();
        assert_eq!(f.foreground.as_deref(), Some("#222222"));
        assert_eq!(f.bold, Some(true), "unspecified attributes survive");
    }

    #[test]
    fn an_unknown_face_attribute_warns_but_keeps_the_rest() {
        let c = parse(r##"theme "t" { face "default" fg="#fff" sparkle=#true }"##);
        assert_eq!(c.warnings.len(), 1);
        assert!(
            c.warnings[0]
                .message
                .contains("unknown face attribute `sparkle`")
        );
        assert_eq!(
            c.theme("t")
                .unwrap()
                .face("default")
                .unwrap()
                .foreground
                .as_deref(),
            Some("#fff")
        );
    }

    #[test]
    fn a_face_that_sets_nothing_warns() {
        let c = parse(r##"theme "t" { face "default" }"##);
        assert!(c.warnings[0].message.contains("sets no attributes"));
        assert!(c.theme("t").unwrap().faces.is_empty());
    }

    #[test]
    fn lsp_servers_parse_inline_and_with_a_body() {
        let c = clean(
            r##"
            lsp "rust" command="rust-analyzer"
            lsp "python" command="pyright-langserver" args="--stdio"
            lsp "go" command="gopls" {
                args "serve" "-rpc.trace"
                root-markers "go.mod" "go.work"
            }
            "##,
        );
        assert_eq!(c.lsp.len(), 3);
        assert!(c.lsp_for("rust").unwrap().args.is_empty());
        assert_eq!(c.lsp_for("python").unwrap().args, vec!["--stdio"]);
        let go = c.lsp_for("go").unwrap();
        assert_eq!(go.args, vec!["serve", "-rpc.trace"]);
        assert_eq!(go.root_markers, vec!["go.mod", "go.work"]);
    }

    #[test]
    fn an_lsp_entry_without_a_command_is_rejected() {
        let c = parse(r##"lsp "rust""##);
        assert!(
            c.warnings[0]
                .message
                .contains("needs a `command=` property")
        );
        assert!(c.lsp.is_empty());
    }

    #[test]
    fn a_later_lsp_entry_replaces_an_earlier_one() {
        let c = clean(
            r##"
            lsp "rust" command="rls"
            lsp "rust" command="rust-analyzer"
            "##,
        );
        assert_eq!(c.lsp.len(), 1);
        assert_eq!(c.lsp_for("rust").unwrap().command, "rust-analyzer");
    }

    #[test]
    fn the_tree_block_parses() {
        let c = clean(
            r##"
            tree {
                show-hidden #true
                ignore ".git" "target"
                width 40
                follow #false
                directories-first #true
                git-status #false
            }
            "##,
        );
        assert!(c.tree.show_hidden);
        assert_eq!(c.tree.ignore, vec![".git", "target"]);
        assert_eq!(c.tree.width, 40);
        assert!(!c.tree.follow);
        assert!(c.tree.directories_first);
        assert!(!c.tree.git_status);
    }

    #[test]
    fn a_bare_tree_flag_reads_as_enabled() {
        let c = clean("tree { show-hidden }");
        assert!(c.tree.show_hidden);
    }

    #[test]
    fn a_too_narrow_tree_width_is_rejected() {
        let c = parse("tree { width 3 }");
        assert!(c.warnings[0].message.contains("at least 8 columns"));
        assert_eq!(c.tree.width, TreeConfig::default().width);
    }

    #[test]
    fn an_empty_ignore_list_warns() {
        let c = parse("tree { ignore }");
        assert!(c.warnings[0].message.contains("one or more names"));
        assert_eq!(
            c.tree.ignore,
            TreeConfig::default().ignore,
            "the default list is kept"
        );
    }

    #[test]
    fn comments_are_ignored() {
        let c = clean("// a comment\nset tab-width=2 // trailing\n/* block */\n");
        assert_eq!(c.settings.tab_width, 2);
    }

    #[test]
    fn merging_layers_a_project_config_over_the_user_config() {
        let mut user = clean(
            r##"
            set tab-width=4
            keymap "global" { bind "C-a" "one" }
            theme "t" { face "default" fg="#111" bold=#true }
            lsp "rust" command="rls"
            "##,
        );
        let project = clean(
            r##"
            set tab-width=2
            keymap "global" { bind "C-b" "two" }
            theme "t" { face "default" fg="#222" }
            lsp "rust" command="rust-analyzer"
            "##,
        );
        user.merge(&project);
        assert_eq!(user.settings.tab_width, 2);
        assert_eq!(user.keymap("global").unwrap().bindings.len(), 2);
        let face = user.theme("t").unwrap().face("default").unwrap();
        assert_eq!(face.foreground.as_deref(), Some("#222"));
        assert_eq!(face.bold, Some(true));
        assert_eq!(user.lsp_for("rust").unwrap().command, "rust-analyzer");
    }

    #[test]
    fn every_kdl_block_in_the_readme_parses() {
        // The README is the first thing anyone copies from. A snippet in it
        // that does not parse is documentation that lies, which is the whole
        // reason `config.example.kdl` is checked as well.
        let readme = include_str!("../../../README.md");
        let mut blocks = 0;
        let mut rest = readme;
        while let Some(open) = rest.find("```kdl\n") {
            let body = &rest[open + 7..];
            let close = body
                .find("```")
                .expect("an unterminated kdl block in the README");
            let source = &body[..close];
            let config = Config::parse(source)
                .unwrap_or_else(|e| panic!("README kdl block {blocks} is not valid KDL: {e}"));
            assert!(
                config.warnings.is_empty(),
                "README kdl block {blocks} warns: {:?}",
                config.warnings
            );
            blocks += 1;
            rest = &body[close..];
        }
        assert!(blocks >= 1, "no kdl blocks found in the README");
    }

    #[test]
    fn the_shipped_example_configuration_parses_without_complaint() {
        // The example is documentation people copy; it must be correct.
        let source = include_str!("../../../docs/config.example.kdl");
        let config = Config::parse(source).expect("the example is valid KDL");
        assert!(
            config.warnings.is_empty(),
            "the example warns: {:?}",
            config.warnings
        );
        assert_eq!(config.settings.tab_width, 4);
        assert_eq!(config.settings.theme, "maxgus-dark");
        assert_eq!(config.lsp.len(), 4);
        assert_eq!(config.tree.width, 32);
        assert!(config.keymap("global").is_some());
        assert!(config.theme("maxgus-dark").is_some());
        // The `args` child node takes more than the `args=` property can.
        assert_eq!(
            config
                .lsp_for("typescript")
                .expect("the typescript entry")
                .args,
            ["--stdio", "--log-level", "2"]
        );
    }

    #[test]
    fn grammar_directories_are_read_and_nothing_is_assumed() {
        let config = Config::parse(
            r#"
            grammars {
                search "/usr/lib" "/usr/local/lib"
                queries "/usr/share/tree-sitter/queries"
                queries "/usr/share/nvim/runtime/queries"
            }
            grammar "go" library="/opt/ts/go.so" queries="/opt/ts/go/highlights.scm"
            grammar "zig" library="/opt/ts/zig.so"
            "#,
        )
        .unwrap();
        assert!(config.warnings.is_empty(), "{:?}", config.warnings);
        assert_eq!(config.grammars.search.len(), 2);
        assert_eq!(config.grammars.queries.len(), 2, "two nodes, both kept");
        assert_eq!(config.grammars.named.len(), 2);
        assert_eq!(config.grammars.named[0].language, "go");
        assert_eq!(
            config.grammars.named[0].queries.as_deref(),
            Some(std::path::Path::new("/opt/ts/go/highlights.scm"))
        );
        assert_eq!(
            config.grammars.named[1].queries, None,
            "without a query of its own it falls back to the search"
        );
    }

    #[test]
    fn a_file_that_says_nothing_looks_for_no_grammars() {
        // Loading one means loading a shared library, so it never happens
        // by default.
        let config = Config::parse("set theme=\"maxgus-dark\"\n").unwrap();
        assert!(config.grammars.search.is_empty());
        assert!(config.grammars.queries.is_empty());
        assert!(config.grammars.named.is_empty());
    }

    #[test]
    fn a_grammar_without_a_library_is_reported_rather_than_half_taken() {
        let config = Config::parse("grammar \"go\"\n").unwrap();
        assert!(config.grammars.named.is_empty());
        assert!(
            config
                .warnings
                .iter()
                .any(|w| w.to_string().contains("library")),
            "{:?}",
            config.warnings
        );
    }

    #[test]
    fn the_example_shows_every_setting_that_can_be_configured() {
        // The example is the whole of the user-facing documentation for the
        // configuration language, and it has drifted behind the parser before:
        // `kill-ring-max`, `blink-cursor` and `echo-keystrokes-ms` all worked
        // and appeared nowhere, so nobody could have known to write them.
        let source = include_str!("../../../docs/config.example.kdl");
        let missing: Vec<&str> = crate::settings::SETTING_NAMES
            .iter()
            .copied()
            .filter(|name| !source.contains(&format!("{name}=")))
            .collect();
        assert!(
            missing.is_empty(),
            "settings the example never shows: {missing:?}"
        );
    }

    #[test]
    fn the_reference_documents_every_setting() {
        // The example shows how to write them; the reference is where the
        // type, the default and the clamping are written down. Only the
        // example was checked, so the reference could — and did — fall
        // behind it.
        let source = include_str!("../../../docs/configuration-reference.md");
        let missing: Vec<&str> = crate::settings::SETTING_NAMES
            .iter()
            .copied()
            .filter(|name| !source.contains(&format!("| `{name}` |")))
            .collect();
        assert!(
            missing.is_empty(),
            "settings the reference never documents: {missing:?}"
        );
    }

    #[test]
    fn the_example_shows_every_face_attribute() {
        let source = include_str!("../../../docs/config.example.kdl");
        let missing: Vec<&str> = crate::settings::FACE_ATTRIBUTE_NAMES
            .iter()
            .copied()
            .filter(|name| !source.contains(&format!("{name}=")))
            .collect();
        assert!(
            missing.is_empty(),
            "face attributes the example never shows: {missing:?}"
        );
    }

    #[test]
    fn every_face_attribute_named_is_one_the_parser_takes() {
        // The other half of the pair: the list is only worth checking the
        // example against if the parser really accepts everything on it.
        for attribute in crate::settings::FACE_ATTRIBUTE_NAMES {
            let value = match *attribute {
                "fg" | "bg" | "foreground" | "background" => "\"#123456\"",
                "inherit" => "\"default\"",
                _ => "#true",
            };
            let source = format!("theme \"t\" {{\n    face \"region\" {attribute}={value}\n}}\n");
            let config = Config::parse(&source).expect("valid KDL");
            assert!(
                config.warnings.is_empty(),
                "`{attribute}` is on the list but the parser complains: {:?}",
                config.warnings
            );
            let face = config
                .theme("t")
                .expect("the theme")
                .face("region")
                .expect("the face");
            assert!(
                !face.is_empty(),
                "`{attribute}` parsed but set nothing on the face"
            );
        }
    }

    #[test]
    fn a_realistic_config_parses_without_complaint() {
        let c = clean(
            r##"
            set tab-width=4 indent-with-tabs=#false
            set theme="maxgus-dark"
            set line-numbers=#true scroll-margin=2

            keymap "global" {
                bind "C-x C-f" "find-file"
                bind "C-x C-s" "save-buffer"
                bind "C-x b"   "switch-to-buffer"
                bind "M-x"     "execute-extended-command"
                bind "C-x t t" "treefile-toggle"
            }

            keymap "rust-mode" {
                bind "C-c C-c" "lsp-code-action"
            }

            theme "maxgus-dark" {
                face "default"           fg="#c5c8c6" bg="#1d1f21"
                face "font-lock-keyword" fg="#b294bb" bold=#true
                face "font-lock-string"  fg="#b5bd68"
                face "region"            bg="#373b41"
                face "mode-line"         fg="#1d1f21" bg="#c5c8c6"
            }

            lsp "rust" command="rust-analyzer" {
                root-markers "Cargo.toml"
            }

            tree {
                show-hidden #false
                ignore ".git" "target" "node_modules"
                width 32
            }
            "##,
        );
        assert_eq!(c.keymaps.len(), 2);
        assert_eq!(c.themes.len(), 1);
        assert_eq!(c.lsp.len(), 1);
        assert_eq!(c.keymap("global").unwrap().bindings.len(), 5);
    }
}
