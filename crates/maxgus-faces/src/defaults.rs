//! The built-in themes.
//!
//! Three ship with the editor: `maxgus-dark` and `maxgus-light` use direct colour
//! and are derived from the Tomorrow palette, while `maxgus-term` names only the
//! sixteen ANSI colours so it follows whatever the terminal is themed with.

use crate::{color::Color, face::Face, names, theme::Theme};

/// Names of every built-in theme.
pub const BUILTIN_THEMES: &[&str] = &["maxgus-dark", "maxgus-light", "maxgus-term"];

/// The theme used when the configured one does not exist.
pub const FALLBACK_THEME: &str = "maxgus-dark";

/// A palette, so the three themes share one construction routine.
struct Palette {
    bg: Color,
    fg: Color,
    /// Slightly off the background: the mode line and gutter.
    surface: Color,
    /// The region and current-line highlight.
    selection: Color,
    comment: Color,
    red: Color,
    orange: Color,
    yellow: Color,
    green: Color,
    aqua: Color,
    blue: Color,
    purple: Color,
    /// Foreground for text drawn on `surface`.
    surface_fg: Color,
    /// Barely-there bands behind added and removed diff lines. A wall of
    /// green and red text is much harder to read than banded rows are.
    added_bg: Color,
    removed_bg: Color,
}

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::Rgb(r, g, b)
}

const fn idx(i: u8) -> Color {
    Color::Indexed(i)
}

/// Tomorrow Night.
const DARK: Palette = Palette {
    bg: rgb(0x1d, 0x1f, 0x21),
    fg: rgb(0xc5, 0xc8, 0xc6),
    surface: rgb(0x28, 0x2a, 0x2e),
    selection: rgb(0x37, 0x3b, 0x41),
    comment: rgb(0x96, 0x98, 0x96),
    red: rgb(0xcc, 0x66, 0x66),
    orange: rgb(0xde, 0x93, 0x5f),
    yellow: rgb(0xf0, 0xc6, 0x74),
    green: rgb(0xb5, 0xbd, 0x68),
    aqua: rgb(0x8a, 0xbe, 0xb7),
    blue: rgb(0x81, 0xa2, 0xbe),
    purple: rgb(0xb2, 0x94, 0xbb),
    surface_fg: rgb(0xc5, 0xc8, 0xc6),
    added_bg: rgb(0x1f, 0x2b, 0x1f),
    removed_bg: rgb(0x2e, 0x1f, 0x1f),
};

/// Tomorrow.
const LIGHT: Palette = Palette {
    bg: rgb(0xff, 0xff, 0xff),
    fg: rgb(0x4d, 0x4d, 0x4c),
    surface: rgb(0xef, 0xef, 0xef),
    selection: rgb(0xd6, 0xd6, 0xd6),
    comment: rgb(0x8e, 0x90, 0x8c),
    red: rgb(0xc8, 0x28, 0x29),
    orange: rgb(0xf5, 0x87, 0x1f),
    yellow: rgb(0xea, 0xb7, 0x00),
    green: rgb(0x71, 0x8c, 0x00),
    aqua: rgb(0x3e, 0x99, 0x9f),
    blue: rgb(0x42, 0x71, 0xae),
    purple: rgb(0x89, 0x59, 0xa8),
    surface_fg: rgb(0x4d, 0x4d, 0x4c),
    added_bg: rgb(0xe8, 0xf3, 0xdd),
    removed_bg: rgb(0xfa, 0xe6, 0xe6),
};

/// Terminal defaults: nothing but the sixteen ANSI colours, so the theme
/// follows the user's terminal configuration.
const TERM: Palette = Palette {
    bg: Color::Default,
    fg: Color::Default,
    surface: idx(0),
    selection: idx(8),
    comment: idx(8),
    red: idx(1),
    orange: idx(3),
    yellow: idx(11),
    green: idx(2),
    aqua: idx(6),
    blue: idx(4),
    purple: idx(5),
    surface_fg: idx(15),
    // The terminal theme keeps to the sixteen colours, and there is no dim
    // green to be had among them: the foreground carries the meaning here.
    added_bg: Color::Default,
    removed_bg: Color::Default,
};

/// Builds a theme from a palette.
fn build(name: &str, p: &Palette) -> Theme {
    let mut t = Theme::new(name);
    let mut set = |face_name: &str, face: Face| t.set(face_name, face);

    set(
        names::DEFAULT,
        Face {
            foreground: Some(p.fg),
            background: Some(p.bg),
            ..Default::default()
        },
    );

    // ---- interface ----
    set("cursor", Face::bg(p.fg).with_fg(p.bg));
    set("region", Face::bg(p.selection));
    set("highlight", Face::bg(p.selection));
    set("shadow", Face::fg(p.comment));
    // No background of their own: they take whatever `default` has. Naming
    // this palette's background explicitly would survive a drop-in theme that
    // changes `default`, and paint a stripe down the side of the text in the
    // colour of the theme that was replaced.
    set("fringe", Face::fg(p.comment));
    set("line-number", Face::fg(p.comment));
    set("line-number-current-line", Face::fg(p.yellow).bold());
    set("mode-line", Face::fg(p.surface_fg).with_bg(p.surface));
    set("mode-line-inactive", Face::fg(p.comment).with_bg(p.surface));
    set("mode-line-buffer-id", Face::fg(p.blue).bold());
    set("minibuffer-prompt", Face::fg(p.blue).bold());
    set("echo-area", Face::fg(p.fg));
    set("isearch", Face::fg(p.bg).with_bg(p.yellow).bold());
    set("isearch-fail", Face::fg(p.bg).with_bg(p.red).bold());
    set("lazy-highlight", Face::fg(p.bg).with_bg(p.aqua));
    set("match-paren", Face::fg(p.orange).bold());
    set("trailing-whitespace", Face::bg(p.red));
    set("fill-column-indicator", Face::fg(p.selection));
    set("completion-selected", Face::bg(p.selection).bold());
    set("completion-annotation", Face::fg(p.comment).italic());
    // The panel's section bands. A heading has to read as furniture rather
    // than as content, or the eye keeps mistaking it for a file.
    // A terminal's own colours come from the program running in it; this is
    // only what an unpainted cell falls back to.
    // Magit's own face names, so a theme written for magit ports straight
    // across. The diff faces carry a background as well as a foreground: a
    // wall of green and red text is much harder to read than banded rows.
    // The menus. A key has to stand out from its description or the menu is
    // a wall of words rather than something to read a key off.
    set("transient-key", Face::fg(p.aqua).bold());
    set("transient-heading", Face::fg(p.yellow).bold());
    set("transient-switch-on", Face::fg(p.green).bold());
    set("transient-switch-off", Face::fg(p.comment));
    set("magit-section-heading", Face::fg(p.yellow).bold());
    set("magit-section-highlight", Face::bg(p.selection));
    set("magit-diff-file-heading", Face::fg(p.fg).bold());
    set(
        "magit-diff-hunk-heading",
        Face::fg(p.comment).with_bg(p.selection),
    );
    set("magit-diff-added", Face::fg(p.green).with_bg(p.added_bg));
    set("magit-diff-removed", Face::fg(p.red).with_bg(p.removed_bg));
    set("magit-diff-context", Face::fg(p.comment));
    set("magit-hash", Face::fg(p.orange));
    set("magit-branch-local", Face::fg(p.aqua).bold());
    set("magit-branch-remote", Face::fg(p.green).bold());
    set("magit-tag", Face::fg(p.yellow).bold());
    set("terminal", Face::fg(p.fg));
    set("terminal-tab", Face::fg(p.comment).with_bg(p.selection));
    set(
        "terminal-tab-selected",
        Face::fg(p.bg).with_bg(p.blue).bold(),
    );
    set("terminal-exited", Face::fg(p.red).with_bg(p.selection));
    set("panel-header", Face::fg(p.blue).with_bg(p.selection).bold());
    set("panel-note", Face::fg(p.comment).italic());
    set("panel-current-buffer", Face::fg(p.yellow).bold());
    set("symbol-detail", Face::fg(p.comment));
    set("completion-border", Face::fg(p.selection));
    set("completion-key", Face::fg(p.aqua));
    set("completion-count", Face::fg(p.orange).bold());
    set("error", Face::fg(p.red).bold());
    set("warning", Face::fg(p.orange).bold());
    set("success", Face::fg(p.green).bold());

    // ---- syntax ----
    set("font-lock-keyword", Face::fg(p.purple));
    set("font-lock-builtin", Face::fg(p.aqua));
    set("font-lock-constant", Face::fg(p.orange));
    set("font-lock-string", Face::fg(p.green));
    set("font-lock-comment", Face::fg(p.comment).italic());
    set("font-lock-doc", Face::fg(p.comment).italic());
    set("font-lock-function-name", Face::fg(p.blue));
    set("font-lock-variable-name", Face::fg(p.red));
    set("font-lock-type", Face::fg(p.yellow));
    set("font-lock-property", Face::fg(p.aqua));
    set("font-lock-number", Face::fg(p.orange));
    set("font-lock-operator", Face::fg(p.aqua));
    set("font-lock-punctuation", Face::fg(p.fg));
    set("font-lock-preprocessor", Face::fg(p.purple));
    set("font-lock-escape", Face::fg(p.orange).bold());
    set("font-lock-label", Face::fg(p.red));
    set("font-lock-attribute", Face::fg(p.yellow));

    // ---- diagnostics ----
    set("diagnostic-error", Face::fg(p.red).underline());
    set("diagnostic-warning", Face::fg(p.orange).underline());
    set("diagnostic-info", Face::fg(p.blue).underline());
    set("diagnostic-hint", Face::fg(p.comment).underline());

    // ---- file tree ----
    set("tree-root", Face::fg(p.purple).bold());
    set("tree-directory", Face::fg(p.blue).bold());
    set("tree-file", Face::fg(p.fg));
    set("tree-symlink", Face::fg(p.aqua).italic());
    set("tree-selected", Face::bg(p.selection).bold());
    set("tree-arrow", Face::fg(p.comment));
    set("tree-git-modified", Face::fg(p.yellow));
    set("tree-git-added", Face::fg(p.green));
    set("tree-git-deleted", Face::fg(p.red));
    set("tree-git-untracked", Face::fg(p.orange));
    set("tree-git-ignored", Face::fg(p.comment).dim());
    set("tree-git-conflict", Face::fg(p.red).bold());

    t
}

/// The built-in theme called `name`.
pub fn builtin(name: &str) -> Option<Theme> {
    match name {
        "maxgus-dark" => Some(build("maxgus-dark", &DARK)),
        "maxgus-light" => Some(build("maxgus-light", &LIGHT)),
        "maxgus-term" => Some(build("maxgus-term", &TERM)),
        _ => None,
    }
}

/// The built-in theme called `name`, or [`FALLBACK_THEME`].
pub fn builtin_or_fallback(name: &str) -> Theme {
    builtin(name).unwrap_or_else(|| builtin(FALLBACK_THEME).expect("the fallback theme exists"))
}

/// Every built-in theme.
pub fn all() -> Vec<Theme> {
    BUILTIN_THEMES.iter().filter_map(|n| builtin(n)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::ColorDepth;

    /// True for colours a sixteen-colour terminal can show.
    fn is_ansi16(c: Color) -> bool {
        match c {
            Color::Default => true,
            Color::Indexed(i) => i < 16,
            Color::Rgb(..) => false,
        }
    }

    #[test]
    fn every_listed_theme_exists() {
        assert_eq!(all().len(), BUILTIN_THEMES.len());
        for name in BUILTIN_THEMES {
            let t = builtin(name).unwrap_or_else(|| panic!("`{name}` is missing"));
            assert_eq!(t.name(), *name);
        }
    }

    #[test]
    fn an_unknown_theme_falls_back() {
        assert!(builtin("nonexistent").is_none());
        assert_eq!(builtin_or_fallback("nonexistent").name(), FALLBACK_THEME);
        assert_eq!(builtin_or_fallback("maxgus-light").name(), "maxgus-light");
    }

    #[test]
    fn every_theme_defines_every_known_face() {
        for theme in all() {
            let defined = theme.face_names();
            for face in names::all() {
                assert!(
                    defined.contains(&face),
                    "theme `{}` is missing face `{face}`",
                    theme.name()
                );
            }
        }
    }

    #[test]
    fn a_theme_that_changes_the_background_changes_it_everywhere() {
        // A drop-in theme sets `default` and expects the editor to follow.
        // `line-number` and `fringe` used to carry the built-in background
        // explicitly, which outlived the theme that chose it and showed up as
        // a stripe beside the line numbers.
        let mut theme = builtin("maxgus-dark").unwrap();
        let replaced = theme.resolve("default").background.expect("a background");

        let mut spec = maxgus_config::ThemeSpec::new("under-test");
        let mut face = maxgus_config::FaceSpec::new("default");
        face.background = Some("#123456".to_string());
        spec.faces.push(face);
        theme.apply_spec(&spec).expect("applies");

        for name in names::all() {
            assert_ne!(
                theme.resolve(name).background,
                Some(replaced),
                "`{name}` kept the background of the theme it replaced"
            );
        }
    }

    #[test]
    fn every_theme_validates() {
        for theme in all() {
            theme
                .validate()
                .unwrap_or_else(|e| panic!("theme `{}`: {e}", theme.name()));
        }
    }

    #[test]
    fn the_dark_and_light_themes_are_what_they_claim() {
        assert!(builtin("maxgus-dark").unwrap().is_dark());
        assert!(!builtin("maxgus-light").unwrap().is_dark());
    }

    #[test]
    fn the_terminal_theme_uses_the_terminals_own_colours() {
        let t = builtin("maxgus-term").unwrap();
        let d = t.resolve("default");
        assert_eq!(d.foreground, Some(Color::Default));
        assert_eq!(d.background, Some(Color::Default));
        // Everything else stays within the sixteen ANSI colours.
        for name in names::FONT_LOCK_FACES {
            let f = t.resolve(name);
            for c in [f.foreground, f.background].into_iter().flatten() {
                assert!(is_ansi16(c), "`{name}` uses {c}, outside the ANSI range");
            }
        }
    }

    #[test]
    fn syntax_faces_are_visually_distinct_in_the_dark_theme() {
        let t = builtin("maxgus-dark").unwrap();
        let keyword = t.resolve("font-lock-keyword").foreground;
        let string = t.resolve("font-lock-string").foreground;
        let comment = t.resolve("font-lock-comment").foreground;
        assert_ne!(keyword, string);
        assert_ne!(string, comment);
        assert_ne!(keyword, comment);
    }

    #[test]
    fn comments_are_rendered_in_italics() {
        for theme in all() {
            assert_eq!(
                theme.resolve("font-lock-comment").attributes.italic,
                Some(true),
                "theme `{}`",
                theme.name()
            );
        }
    }

    #[test]
    fn diagnostics_are_underlined_in_every_theme() {
        for theme in all() {
            for face in names::DIAGNOSTIC_FACES {
                assert_eq!(
                    theme.resolve(face).attributes.underline,
                    Some(true),
                    "theme `{}`, face `{face}`",
                    theme.name()
                );
            }
        }
    }

    #[test]
    fn faces_inherit_the_default_background() {
        let t = builtin("maxgus-dark").unwrap();
        let bg = t.resolve("default").background;
        assert_eq!(t.resolve("font-lock-keyword").background, bg);
    }

    #[test]
    fn themes_degrade_cleanly_to_sixteen_colours() {
        let t = builtin("maxgus-dark").unwrap();
        for name in names::all() {
            let f = t.resolve_for(name, ColorDepth::Ansi16);
            for c in [f.foreground, f.background].into_iter().flatten() {
                assert!(is_ansi16(c), "`{name}` degraded to {c}");
            }
        }
    }

    #[test]
    fn the_cursor_inverts_the_default_face() {
        for theme in all() {
            let cursor = theme.resolve("cursor");
            let default = theme.resolve("default");
            assert_eq!(
                cursor.foreground,
                default.background,
                "theme `{}`",
                theme.name()
            );
            assert_eq!(
                cursor.background,
                default.foreground,
                "theme `{}`",
                theme.name()
            );
        }
    }
}
