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
    /// `grammar-auto-install`: when a file's language has no grammar, offer
    /// to fetch and build one for it.
    ///
    /// The offer names the language and the repository it would clone, and
    /// nothing is downloaded, compiled or loaded until it is answered `yes`.
    /// Turning this off does not take the feature away — `M-x
    /// install-grammar` still asks for one by name — it only stops the
    /// editor from asking first.
    pub grammar_auto_install: bool,
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
    /// `beacon`: shine a light beside the cursor after it jumps, so the eye
    /// finds it again. The names below are `beacon`'s own, so a setting
    /// copied from an Emacs configuration means what it meant there.
    pub beacon: bool,
    /// `beacon-size`: how many cells the light covers.
    pub beacon_size: usize,
    /// `beacon-blink-delay-ms`: how long it stays before it fades.
    pub beacon_blink_delay_ms: usize,
    /// `beacon-blink-duration-ms`: how long the fade takes.
    pub beacon_blink_duration_ms: usize,
    /// `beacon-color`: a colour, or a number from 0 to 1 meaning a grey
    /// chosen against the background — light on a dark theme, dark on a
    /// light one.
    pub beacon_color: String,
    /// `beacon-blink-when-buffer-changes`.
    pub beacon_blink_when_buffer_changes: bool,
    /// `beacon-blink-when-window-scrolls`.
    pub beacon_blink_when_window_scrolls: bool,
    /// `beacon-blink-when-window-changes`.
    pub beacon_blink_when_window_changes: bool,
    /// `beacon-blink-when-point-moves-vertically`: the lines point must move
    /// for a light, or `0` for never — which is `beacon`'s own default,
    /// because ordinary editing would otherwise light it constantly.
    pub beacon_blink_when_point_moves_vertically: usize,
    /// `session`: remember what is open, and open it again next time.
    pub session: bool,
    /// `gui-font`: the family the window draws with. Ignored by the terminal
    /// front end, which uses whatever the terminal is configured with.
    pub gui_font: String,
    /// `gui-font-size`: its size in pixels.
    pub gui_font_size: usize,
    /// `gui-line-spacing`: pixels added between one line of text and the
    /// next, over what the font asks for.
    pub gui_line_spacing: usize,
    /// `gui-padding`: pixels of margin between the window's edge and the
    /// text.
    pub gui_padding: usize,
    /// `gui-scroll-indicator`: a thin bar at a window's right edge, while
    /// it scrolls, saying where in the buffer it is.
    pub gui_scroll_indicator: bool,
    /// `gui-prose-font`: the family the window sets prose in — what a
    /// language server says about a symbol, which is read rather than
    /// lined up with anything. `sans-serif` is whatever the system has
    /// set up as its sans-serif.
    pub gui_prose_font: String,
    /// `gui-title-format`: what the window's title says.
    ///
    /// `%b` is the buffer's name, `%f` the file's path with home as `~`,
    /// `%p` the project's name, `%P` its path, `%m` a mark when the
    /// buffer is modified, `%a` the program's name, `%%` a per-cent sign.
    /// A taskbar full of windows called `maxgus` names nothing; the
    /// default names the file, the project and the program.
    pub gui_title_format: String,
    /// `mouse-wheel-lines`: how far one notch of the wheel moves the view.
    ///
    /// Three is what most programs do with a notch. A touchpad reports the
    /// pixels it moved and is not affected by this.
    pub mouse_wheel_lines: usize,
    /// `lsp-doc`: show what the language server knows about the symbol under
    /// point, in a box beside it, once the cursor has rested there.
    ///
    /// `lsp-ui-doc` for Emacs. `C-c c k` asks for it whatever this says.
    pub lsp_doc: bool,
    /// `autocomplete`: offer suggestions while typing, without being asked.
    ///
    /// `company-mode` for Emacs. `C-M-i` asks for them whatever this says.
    pub autocomplete: bool,
    /// `autocomplete-min-chars`: how much of a word has to be typed before
    /// suggestions are offered for it.
    ///
    /// One would offer them for every letter of every word, which is a list
    /// of everything the server knows and no help at all.
    pub autocomplete_min_chars: usize,
    /// `which-key`: after a pause mid-sequence, show what the next key can be.
    ///
    /// Doom's whole leader scheme is meant to be discovered this way rather
    /// than memorised, so it is on.
    pub which_key: bool,
    /// `which-key-delay-ms`: how long the pause is before the panel appears.
    ///
    /// Short enough to help someone who has stopped to think, long enough
    /// that a fluent `C-x C-s` never flashes it.
    pub which_key_delay_ms: usize,
    /// `smooth-scroll-ms`: roughly how long the view takes to come to rest
    /// after the wheel asks it to move.
    ///
    /// The window can draw a fraction of a line, so a wheel notch slides
    /// rather than jumping; this is how long the slide lasts. Lower is
    /// brisker, `0` turns it off and the view arrives at once. A terminal
    /// cannot draw a fraction of a line and ignores it.
    pub smooth_scroll_ms: usize,
    /// `scroll-animation-far-lines`: how much of a long jump is animated.
    ///
    /// A wheel notch is a line or two and slides in whole. A page down, or
    /// `M->` in a long file, is a hundred, and animating that in full is an
    /// animation being watched rather than a view being moved. So a jump
    /// longer than this is drawn as a slide of this many lines — the last
    /// of the journey, which is the part that says which way it went. `0`
    /// turns it off.
    pub scroll_animation_far_lines: usize,
    /// `cursor-animation-ms`: how long the cursor takes to arrive.
    ///
    /// The block slides to where point went instead of appearing there, so
    /// the eye follows it rather than looking for it. `0` turns it off and
    /// the cursor is simply where it is. A terminal cannot draw a block
    /// between two cells and ignores it.
    pub cursor_animation_ms: usize,
    /// `cursor-short-animation-ms`: how long a hop of a cell or two takes.
    ///
    /// The common case — a key typed, a character rubbed out — and giving it
    /// the same duration as a jump across the screen makes ordinary typing
    /// look like it is lagging behind the keyboard.
    pub cursor_short_animation_ms: usize,
    /// `cursor-trail`: how far the back of the cursor lags the front, as a
    /// percentage of the way.
    ///
    /// This is the smear. `0` moves the block rigidly; higher stretches it
    /// out behind itself while it travels and lets it catch up on arrival.
    /// In whole percent rather than a fraction, because this file has no
    /// floats in it.
    pub cursor_trail: usize,
    /// `cursor-vfx`: what the cursor leaves behind it.
    ///
    /// Six of them: `sonicboom`, `ripple` and `wireframe` mark where it
    /// landed; `railgun`, `torpedo` and `pixiedust` trail particles along
    /// the way it came. Empty — the default — is none of them, and nothing
    /// is drawn or computed at all.
    pub cursor_vfx: String,
    /// `cursor-vfx-opacity`: how solid the effect is at its strongest, in
    /// percent.
    pub cursor_vfx_opacity: usize,
    /// `cursor-vfx-particle-lifetime-ms`: how long a trailing particle
    /// lives.
    pub cursor_vfx_particle_lifetime_ms: usize,
    /// `cursor-vfx-highlight-lifetime-ms`: how long a mark at the
    /// destination takes to swell and fade.
    pub cursor_vfx_highlight_lifetime_ms: usize,
    /// `cursor-vfx-particle-density`: particles per cell travelled, in
    /// percent — so `70` is seven for every ten cells.
    pub cursor_vfx_particle_density: usize,
    /// `cursor-vfx-particle-speed`: how fast they fly away.
    pub cursor_vfx_particle_speed: usize,
    /// `cursor-vfx-particle-phase`: how far round the arc a railgun flings
    /// them, in percent.
    pub cursor_vfx_particle_phase: usize,
    /// `cursor-vfx-particle-curl`: how sharply a particle's flight turns as
    /// it goes, in percent.
    pub cursor_vfx_particle_curl: usize,
    /// `floating-blur`: blur what is behind a popup.
    ///
    /// The completion list, the doc box, the which-key panel and the rest.
    /// It costs the frame being drawn in two halves and three more passes
    /// over the popup's own area — and nothing at all while no popup is
    /// open, which is most of the time.
    pub floating_blur: bool,
    /// `floating-blur-radius`: how far the blur reaches, in pixels.
    pub floating_blur_radius: usize,
    /// `floating-opacity`: how solid a popup's own background is, in
    /// percent, over whatever was blurred behind it.
    ///
    /// Only has anything to show through when `floating-blur` is on; a
    /// popup over an unblurred buffer would just be hard to read.
    pub floating_opacity: usize,
    /// `ligatures`: let the font join characters that it draws as one.
    ///
    /// `!=` as one glyph rather than two, where the font has been made to
    /// do it — a font that has not is unaffected, because the shaping asks
    /// and takes the answer. Only the window: a terminal draws what its own
    /// font does and is not asked.
    pub ligatures: bool,
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
            grammar_auto_install: true,
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
            beacon: false,
            beacon_size: 40,
            beacon_blink_delay_ms: 300,
            beacon_blink_duration_ms: 300,
            beacon_color: "0.5".into(),
            beacon_blink_when_buffer_changes: true,
            beacon_blink_when_window_scrolls: true,
            beacon_blink_when_window_changes: true,
            beacon_blink_when_point_moves_vertically: 0,
            session: false,
            gui_font: "JetBrainsMono Nerd Font".into(),
            gui_font_size: 16,
            gui_line_spacing: 0,
            gui_padding: 0,
            gui_scroll_indicator: true,
            gui_prose_font: "sans-serif".into(),
            gui_title_format: "%m%b — %p — %a".into(),
            autocomplete: true,
            autocomplete_min_chars: 2,
            lsp_doc: true,
            which_key: true,
            which_key_delay_ms: 400,
            mouse_wheel_lines: 3,
            smooth_scroll_ms: 300,
            scroll_animation_far_lines: 1,
            cursor_animation_ms: 150,
            cursor_short_animation_ms: 40,
            cursor_trail: 70,
            cursor_vfx: String::new(),
            cursor_vfx_opacity: 78,
            cursor_vfx_particle_lifetime_ms: 500,
            cursor_vfx_highlight_lifetime_ms: 200,
            cursor_vfx_particle_density: 70,
            cursor_vfx_particle_speed: 10,
            cursor_vfx_particle_phase: 150,
            cursor_vfx_particle_curl: 100,
            floating_blur: true,
            floating_blur_radius: 8,
            floating_opacity: 82,
            ligatures: true,
            shell: None,
        }
    }
}

/// The settings a config file may name, used for the "did you mean" hint on a
/// misspelled key.
/// The cursor effects that can be named, which the window's `vfx` module
/// implements. Here as well so a configuration can be checked without a
/// window, which is where every other setting is checked too.
pub const CURSOR_VFX_NAMES: &[&str] = &[
    "sonicboom",
    "ripple",
    "wireframe",
    "railgun",
    "torpedo",
    "pixiedust",
];

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
    "grammar-auto-install",
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
    "beacon",
    "beacon-size",
    "beacon-blink-delay-ms",
    "beacon-blink-duration-ms",
    "beacon-color",
    "beacon-blink-when-buffer-changes",
    "beacon-blink-when-window-scrolls",
    "beacon-blink-when-window-changes",
    "beacon-blink-when-point-moves-vertically",
    "session",
    "gui-font",
    "gui-font-size",
    "gui-line-spacing",
    "gui-padding",
    "gui-scroll-indicator",
    "gui-prose-font",
    "gui-title-format",
    "autocomplete",
    "autocomplete-min-chars",
    "lsp-doc",
    "which-key",
    "which-key-delay-ms",
    "mouse-wheel-lines",
    "smooth-scroll-ms",
    "scroll-animation-far-lines",
    "cursor-animation-ms",
    "cursor-short-animation-ms",
    "cursor-trail",
    "cursor-vfx",
    "cursor-vfx-opacity",
    "cursor-vfx-particle-lifetime-ms",
    "cursor-vfx-highlight-lifetime-ms",
    "cursor-vfx-particle-density",
    "cursor-vfx-particle-speed",
    "cursor-vfx-particle-phase",
    "cursor-vfx-particle-curl",
    "floating-blur",
    "floating-blur-radius",
    "floating-opacity",
    "ligatures",
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
