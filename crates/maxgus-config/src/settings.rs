//! Global editor settings.
//!
//! Each field corresponds to one `set` property in the config file and to an
//! Emacs variable of the same intent. Defaults are chosen to match Emacs where
//! that is sensible and to match modern expectations where it is not.

/// Editor-wide settings, all overridable from `config.kdl`.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// `tab-width`: columns a hard tab advances to.
    pub tab_width: usize,
    /// `indent-tabs-mode`: indent with hard tabs rather than spaces.
    pub indent_with_tabs: bool,
    /// Name of the theme to activate at startup.
    pub theme: String,
    /// `display-line-numbers-mode`.
    pub line_numbers: bool,
    /// `truncate-lines`: clip long lines instead of wrapping them.
    pub truncate_lines: bool,
    /// `scroll-margin`: lines of context kept above and below point.
    pub scroll_margin: usize,
    /// `fill-column`, used by `fill-paragraph` and the column indicator.
    pub fill_column: usize,
    /// `kill-ring-max`.
    pub kill_ring_max: usize,
    /// `case-fold-search`. `None` selects the smart-case heuristic: fold
    /// unless the search string contains an uppercase letter.
    pub case_fold_search: Option<bool>,
    /// `require-final-newline`: ensure a trailing newline on save.
    pub require_final_newline: bool,
    /// Strip trailing whitespace from every line on save.
    pub delete_trailing_whitespace: bool,
    /// `make-backup-files`: write `file~` before overwriting.
    pub backup_files: bool,
    /// Enable tree-sitter highlighting when a grammar is available.
    pub syntax_highlighting: bool,
    /// Start language servers for buffers whose language has one configured.
    pub lsp_enabled: bool,
    /// Milliseconds of idle time before the editor asks the server for
    /// diagnostics and re-parses the syntax tree.
    pub idle_delay_ms: u64,
    /// `display-fill-column-indicator-mode`: mark the fill column.
    pub fill_column_indicator: bool,
    /// `blink-cursor-mode`.
    pub blink_cursor: bool,
    /// Show the `C-x`-style prefix echo in the minibuffer after this delay.
    pub echo_keystrokes_ms: u64,
    /// Draw Nerd Font glyphs in the tree and the mode line. Turned off, both
    /// fall back to plain text — which is what a terminal without one of those
    /// fonts wants, since the glyphs would otherwise draw as boxes.
    pub nerd_font_icons: bool,
    /// Which of the side panel's three sections exist at all. A section that
    /// is off is not drawn, not headed and not navigable: the panel is short
    /// of rows, and a heading over something nobody wants is a wasted one.
    pub panel_tree: bool,
    pub panel_symbols: bool,
    pub panel_buffers: bool,
    /// How tall the outline and buffer-list windows are, in rows. The file
    /// tree takes whatever they leave.
    /// Open the side panel as soon as the editor starts.
    pub panel_at_startup: bool,
    pub panel_symbols_height: usize,
    pub panel_buffers_height: usize,
    /// The program a terminal tab starts. Unset means whatever `$SHELL` says,
    /// which is what a user has already chosen once.
    pub shell: Option<String>,
    /// `session`: remember what is open, and open it again next time.
    pub session: bool,
    /// `gui-font`: the family the window draws with. Ignored by the terminal
    /// front end, which uses whatever the terminal is configured with.
    pub gui_font: String,
    /// `gui-font-size`: its size in pixels.
    pub gui_font_size: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            tab_width: 4,
            indent_with_tabs: false,
            theme: "maxgus-dark".to_string(),
            line_numbers: false,
            truncate_lines: true,
            scroll_margin: 0,
            fill_column: 70,
            kill_ring_max: 120,
            case_fold_search: None,
            require_final_newline: true,
            delete_trailing_whitespace: false,
            backup_files: false,
            syntax_highlighting: true,
            lsp_enabled: true,
            idle_delay_ms: 150,
            fill_column_indicator: false,
            blink_cursor: false,
            echo_keystrokes_ms: 1000,
            nerd_font_icons: true,
            panel_tree: true,
            panel_symbols: true,
            panel_buffers: true,
            panel_at_startup: false,
            panel_symbols_height: 12,
            panel_buffers_height: 8,
            // A Nerd Font by default because the tree and the mode line draw
            // glyphs from one; the loader falls through to whatever monospace
            // font is installed when it is not there.
            session: false,
            gui_font: "JetBrainsMono Nerd Font".into(),
            gui_font_size: 16,
            shell: None,
        }
    }
}

/// The settings a config file may name, used for the "did you mean" hint on a
/// misspelled key.
pub const SETTING_NAMES: &[&str] = &[
    "tab-width",
    "indent-with-tabs",
    "theme",
    "line-numbers",
    "truncate-lines",
    "scroll-margin",
    "fill-column",
    "kill-ring-max",
    "case-fold-search",
    "require-final-newline",
    "delete-trailing-whitespace",
    "backup-files",
    "syntax-highlighting",
    "lsp-enabled",
    "idle-delay-ms",
    "fill-column-indicator",
    "blink-cursor",
    "echo-keystrokes-ms",
    "nerd-font-icons",
    "panel-tree",
    "panel-symbols",
    "panel-buffers",
    "panel-at-startup",
    "panel-symbols-height",
    "panel-buffers-height",
    "shell",
    "session",
    "gui-font",
    "gui-font-size",
];

/// Every attribute a `face` node may carry.
///
/// `fg`/`bg` are the short spellings and `foreground`/`background` the long
/// ones; both are listed because both are accepted. Kept here so the parser,
/// the shipped example and the tests that check they agree all read from one
/// place — the example had already drifted behind the parser once.
pub const FACE_ATTRIBUTE_NAMES: &[&str] = &[
    "fg",
    "bg",
    "foreground",
    "background",
    "inherit",
    "bold",
    "italic",
    "underline",
    "reverse",
    "dim",
    "strikethrough",
];

/// The setting name closest to `name`, when one is within edit distance two.
pub fn closest_setting(name: &str) -> Option<&'static str> {
    closest_among(name, SETTING_NAMES.iter().copied())
}

/// The candidate closest to `name`, when one is within edit distance two.
///
/// Shared so that a misspelled face name gets the same help a misspelled
/// setting does, rather than being silently ignored.
pub fn closest_among<'a>(name: &str, candidates: impl Iterator<Item = &'a str>) -> Option<&'a str> {
    candidates
        .map(|candidate| (candidate, edit_distance(name, candidate)))
        .filter(|(_, d)| *d <= 2)
        .min_by_key(|(_, d)| *d)
        .map(|(c, _)| c)
}

/// Levenshtein distance, used only for suggestions.
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            cur[j + 1] = (prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sensible() {
        let s = Settings::default();
        assert_eq!(s.tab_width, 4);
        assert!(!s.indent_with_tabs);
        assert_eq!(s.theme, "maxgus-dark");
        assert_eq!(s.case_fold_search, None, "smart case by default");
        assert!(s.syntax_highlighting);
        assert!(s.lsp_enabled);
    }

    #[test]
    fn every_setting_name_is_listed_once() {
        let mut sorted = SETTING_NAMES.to_vec();
        sorted.sort_unstable();
        let before = sorted.len();
        sorted.dedup();
        assert_eq!(sorted.len(), before, "duplicate entry in SETTING_NAMES");
    }

    #[test]
    fn edit_distance_matches_known_values() {
        assert_eq!(edit_distance("", ""), 0);
        assert_eq!(edit_distance("abc", "abc"), 0);
        assert_eq!(edit_distance("abc", "abd"), 1);
        assert_eq!(edit_distance("kitten", "sitting"), 3);
        assert_eq!(edit_distance("", "abc"), 3);
    }

    #[test]
    fn close_misspellings_get_a_suggestion() {
        assert_eq!(closest_setting("tab-widht"), Some("tab-width"));
        assert_eq!(closest_setting("theme"), Some("theme"));
        assert_eq!(closest_setting("fill-colum"), Some("fill-column"));
    }

    #[test]
    fn unrelated_names_get_no_suggestion() {
        assert_eq!(closest_setting("completely-different"), None);
    }
}
