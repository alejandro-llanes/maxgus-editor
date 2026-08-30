//! Canonical face names.
//!
//! Emacs' `font-lock-*-face` names are kept, because they are what a user
//! porting a theme will reach for. Tree-sitter capture names are mapped onto
//! them so a grammar's `@keyword.function` lands on `font-lock-keyword` unless
//! the theme defines something more specific.

/// The face every other face ultimately inherits from.
pub const DEFAULT: &str = "default";

/// Faces the editor itself draws with.
pub const UI_FACES: &[&str] = &[
    DEFAULT,
    "cursor",
    "region",
    "highlight",
    "shadow",
    "fringe",
    "line-number",
    "line-number-current-line",
    "mode-line",
    "mode-line-inactive",
    "mode-line-buffer-id",
    "minibuffer-prompt",
    "echo-area",
    "isearch",
    "isearch-fail",
    "lazy-highlight",
    "match-paren",
    "trailing-whitespace",
    "fill-column-indicator",
    "completion-selected",
    "completion-annotation",
    "transient-key",
    "transient-heading",
    "transient-switch-on",
    "transient-switch-off",
    "magit-section-heading",
    "magit-section-highlight",
    "magit-diff-file-heading",
    "magit-diff-hunk-heading",
    "magit-diff-added",
    "magit-diff-removed",
    "magit-diff-context",
    "magit-hash",
    "magit-branch-local",
    "magit-branch-remote",
    "magit-tag",
    "terminal",
    "terminal-tab",
    "terminal-tab-selected",
    "terminal-exited",
    "panel-header",
    "panel-note",
    "panel-current-buffer",
    "symbol-detail",
    "completion-border",
    "completion-key",
    "which-key-group",
    "completion-count",
    "error",
    "warning",
    "success",
];

/// Syntax-highlighting faces.
pub const FONT_LOCK_FACES: &[&str] = &[
    "font-lock-keyword",
    "font-lock-builtin",
    "font-lock-constant",
    "font-lock-string",
    "font-lock-comment",
    "font-lock-doc",
    "font-lock-function-name",
    "font-lock-variable-name",
    "font-lock-type",
    "font-lock-property",
    "font-lock-number",
    "font-lock-operator",
    "font-lock-punctuation",
    "font-lock-preprocessor",
    "font-lock-escape",
    "font-lock-label",
    "font-lock-attribute",
    // Markup, which arrived with markdown and XML: a heading is not a
    // keyword and a link is not a string, and a theme should be able to say
    // so.
    "font-lock-heading",
    "font-lock-link",
];

/// Faces for language-server diagnostics.
pub const DIAGNOSTIC_FACES: &[&str] = &[
    "diagnostic-error",
    "diagnostic-warning",
    "diagnostic-info",
    "diagnostic-hint",
];

/// Faces for the file tree.
pub const TREE_FACES: &[&str] = &[
    "tree-root",
    "tree-directory",
    "tree-file",
    "tree-symlink",
    "tree-selected",
    "tree-arrow",
    "tree-git-modified",
    "tree-git-added",
    "tree-git-deleted",
    "tree-git-untracked",
    "tree-git-ignored",
    "tree-git-conflict",
];

/// Every face the editor knows about.
pub fn all() -> Vec<&'static str> {
    UI_FACES
        .iter()
        .chain(FONT_LOCK_FACES)
        .chain(DIAGNOSTIC_FACES)
        .chain(TREE_FACES)
        .copied()
        .collect()
}

/// True when `name` is one of the built-in faces.
pub fn is_known(name: &str) -> bool {
    all().contains(&name)
}

/// The face name closest to `name`, for a "did you mean" on a misspelling.
pub fn closest(name: &str) -> Option<&'static str> {
    // `all()` builds a Vec, so the borrow has to outlive the search.
    let candidates = all();
    maxgus_config::settings::closest_among(name, candidates.iter().copied())
        .and_then(|found| all().into_iter().find(|c| *c == found))
}

/// Every face a theme block names that the editor has never heard of, with a
/// suggestion where one is close.
///
/// A misspelled face used to be accepted in silence and simply do nothing,
/// while a misspelled *setting* got told about — this closes that gap.
pub fn unknown_in(spec: &maxgus_config::ThemeSpec) -> Vec<(usize, String, Option<&'static str>)> {
    spec.faces
        .iter()
        .filter(|face| !is_known(&face.name))
        .map(|face| (face.line, face.name.clone(), closest(&face.name)))
        .collect()
}

/// The face a tree-sitter capture maps to.
///
/// Captures are dotted and increasingly specific (`function.method.builtin`).
/// The most specific mapping wins; failing that the capture is truncated at
/// each dot until something matches, which is how tree-sitter's own highlight
/// configuration resolves names.
pub fn face_for_capture(capture: &str) -> Option<&'static str> {
    let mut rest = capture;
    loop {
        if let Some(face) = exact_capture(rest) {
            return Some(face);
        }
        match rest.rsplit_once('.') {
            Some((head, _)) => rest = head,
            None => return None,
        }
    }
}

fn exact_capture(capture: &str) -> Option<&'static str> {
    Some(match capture {
        "keyword" | "conditional" | "repeat" | "include" | "exception" | "storageclass" => {
            "font-lock-keyword"
        }
        "function" | "method" | "constructor" => "font-lock-function-name",
        "function.builtin" | "function.macro" | "macro" => "font-lock-builtin",
        "variable" | "parameter" => "font-lock-variable-name",
        "variable.builtin" => "font-lock-builtin",
        "type" | "class" | "struct" | "enum" | "interface" => "font-lock-type",
        "type.builtin" => "font-lock-builtin",
        "constant" | "constant.builtin" | "boolean" => "font-lock-constant",
        "string" | "character" => "font-lock-string",
        "string.escape" | "escape" => "font-lock-escape",
        "number" | "float" | "integer" => "font-lock-number",
        "comment" => "font-lock-comment",
        "comment.documentation" | "doc" => "font-lock-doc",
        "operator" => "font-lock-operator",
        // `delimiter` is what the C grammar calls `.` and `;`, where every
        // other grammar here says `punctuation.delimiter`. Without it C was
        // the one language whose delimiters stayed uncoloured.
        "punctuation"
        | "punctuation.bracket"
        | "punctuation.delimiter"
        | "punctuation.special"
        | "delimiter" => "font-lock-punctuation",
        "property" | "field" | "tag.attribute" => "font-lock-property",
        "preproc" | "preprocessor" | "define" => "font-lock-preprocessor",
        "label" | "tag" => "font-lock-label",
        // Markup, in the vocabulary markdown and XML queries use. Both the
        // `text.*` spelling the older queries have and the `markup.*` one
        // that replaced it.
        "text.title" | "markup.heading" | "title" | "heading" => "font-lock-heading",
        "text.literal" | "markup.raw" | "literal" => "font-lock-string",
        "text.uri" | "markup.link" | "uri" | "link" => "font-lock-link",
        "text.reference" | "reference" => "font-lock-label",
        "text.emphasis" | "markup.italic" => "font-lock-doc",
        "text.strong" | "markup.bold" => "font-lock-heading",
        "markup.list" | "text.list" => "font-lock-punctuation",
        // `markup` on its own is XML's word for a tag's content, which is
        // ordinary text and wants no colour of its own — but it has to map
        // to something or it reads as a face nobody wrote.
        "markup" | "text" => "default",
        "attribute" | "annotation" | "decorator" => "font-lock-attribute",
        "error" => "error",
        "warning" => "warning",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    #[test]
    fn every_shipped_example_theme_loads_and_names_real_faces() {
        // The theme files under `docs/themes` are meant to be copied straight
        // into a config. One that does not parse, or that names a face the
        // editor does not have, would be documentation that quietly does
        // nothing — which is the failure this whole project keeps finding.
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/themes");
        let mut checked = 0;
        for entry in std::fs::read_dir(&dir).expect("docs/themes").flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "kdl") {
                continue;
            }
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            let source = std::fs::read_to_string(&path).expect("readable");
            let config = maxgus_config::Config::parse(&source)
                .unwrap_or_else(|e| panic!("{name} is not valid KDL: {e}"));
            assert!(
                config.warnings.is_empty(),
                "{name} warns: {:?}",
                config.warnings
            );

            let spec = config
                .themes
                .first()
                .unwrap_or_else(|| panic!("{name} defines no theme"));
            assert!(
                spec.faces.len() > 40,
                "{name} sets only {} faces; a whole theme is expected",
                spec.faces.len()
            );
            let unknown = unknown_in(spec);
            assert!(
                unknown.is_empty(),
                "{name} names faces that do not exist: {unknown:?}"
            );

            // A drop-in theme names itself and says which built-in it starts
            // from, so it can be copied into `themes/` and chosen by name.
            assert!(
                !crate::defaults::BUILTIN_THEMES.contains(&spec.name.as_str()),
                "{name} is named after a built-in; a drop-in theme needs its own name"
            );
            let base = spec.base.as_deref().unwrap_or_else(|| {
                panic!("{name} sets no `base=`, so it has nothing to start from")
            });
            let mut theme = crate::defaults::builtin(base)
                .unwrap_or_else(|| panic!("{name} starts from `{base}`, which is not built in"));
            theme
                .apply_spec(spec)
                .unwrap_or_else(|e| panic!("{name} will not apply: {e}"));
            checked += 1;
        }
        assert!(checked >= 4, "only {checked} example themes were checked");
    }

    #[test]
    fn a_misspelled_face_is_noticed_and_a_near_one_suggested() {
        let mut spec = maxgus_config::ThemeSpec::new("t");
        let mut face = maxgus_config::FaceSpec::new("font-lock-coment");
        face.line = 7;
        spec.faces.push(face);

        let found = unknown_in(&spec);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, 7, "the line it was written on");
        assert_eq!(found[0].1, "font-lock-coment");
        assert_eq!(found[0].2, Some("font-lock-comment"), "one letter away");
    }

    #[test]
    fn a_face_nothing_resembles_is_still_reported_without_a_guess() {
        let mut spec = maxgus_config::ThemeSpec::new("t");
        spec.faces.push(maxgus_config::FaceSpec::new("qqqqqqqqqq"));
        let found = unknown_in(&spec);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].2, None, "nothing is close enough to suggest");
    }

    #[test]
    fn every_real_face_passes_unremarked() {
        let mut spec = maxgus_config::ThemeSpec::new("t");
        for name in all() {
            spec.faces.push(maxgus_config::FaceSpec::new(name));
        }
        assert!(unknown_in(&spec).is_empty(), "got {:?}", unknown_in(&spec));
    }

    #[test]
    fn every_face_the_editor_draws_with_is_in_the_reference() {
        // The reference is where a theme author looks for the list. A face
        // that exists but is written down nowhere may as well not exist, and
        // adding one is exactly when documenting it is easiest to forget.
        let reference = include_str!("../../../docs/configuration-reference.md");
        let documented: Vec<&str> = UI_FACES
            .iter()
            .chain(FONT_LOCK_FACES)
            .chain(DIAGNOSTIC_FACES)
            .chain(TREE_FACES)
            .copied()
            .collect();
        for name in documented {
            assert!(
                reference.contains(&format!("`{name}`")),
                "the reference never names `{name}`"
            );
        }
    }

    #[test]
    fn every_face_the_example_configuration_names_is_real() {
        // The example is what people copy; a face in it that does not exist
        // would be a setting that silently never applies.
        let source = include_str!("../../../docs/config.example.kdl");
        let named: Vec<&str> = source
            .lines()
            .filter_map(|line| line.trim().strip_prefix("face \""))
            .filter_map(|rest| rest.split('"').next())
            .collect();
        assert!(
            named.len() >= 8,
            "the example should show a good few faces, saw {named:?}"
        );
        for name in named {
            assert!(
                is_known(name),
                "the example names `{name}`, which is not a face"
            );
        }
    }

    use super::*;

    #[test]
    fn every_face_name_is_unique() {
        let mut names = all();
        let before = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), before, "duplicate face name");
    }

    #[test]
    fn known_faces_are_recognised() {
        assert!(is_known("default"));
        assert!(is_known("font-lock-keyword"));
        assert!(is_known("tree-git-modified"));
        assert!(!is_known("font-lock-invented"));
    }

    #[test]
    fn exact_captures_map_directly() {
        assert_eq!(face_for_capture("keyword"), Some("font-lock-keyword"));
        assert_eq!(face_for_capture("string"), Some("font-lock-string"));
        assert_eq!(face_for_capture("comment"), Some("font-lock-comment"));
    }

    #[test]
    fn specific_captures_beat_their_prefix() {
        assert_eq!(
            face_for_capture("function"),
            Some("font-lock-function-name")
        );
        assert_eq!(
            face_for_capture("function.builtin"),
            Some("font-lock-builtin")
        );
        assert_eq!(face_for_capture("type.builtin"), Some("font-lock-builtin"));
    }

    #[test]
    fn unknown_suffixes_fall_back_to_the_prefix() {
        assert_eq!(
            face_for_capture("keyword.function"),
            Some("font-lock-keyword")
        );
        assert_eq!(
            face_for_capture("function.method.static"),
            Some("font-lock-function-name")
        );
        assert_eq!(
            face_for_capture("punctuation.bracket.unmatched"),
            Some("font-lock-punctuation")
        );
    }

    #[test]
    fn captures_with_no_mapping_at_all_yield_nothing() {
        assert_eq!(face_for_capture("spaceship"), None);
        assert_eq!(face_for_capture("spaceship.warp.core"), None);
        assert_eq!(face_for_capture(""), None);
    }

    #[test]
    fn every_capture_target_is_a_known_face() {
        let captures = [
            "keyword",
            "function",
            "function.builtin",
            "variable",
            "type",
            "constant",
            "string",
            "string.escape",
            "number",
            "comment",
            "comment.documentation",
            "operator",
            "punctuation",
            "property",
            "preproc",
            "label",
            "attribute",
            "error",
            "warning",
        ];
        for c in captures {
            let face = face_for_capture(c).unwrap_or_else(|| panic!("`{c}` has no face"));
            assert!(is_known(face), "`{c}` maps to unknown face `{face}`");
        }
    }
}
