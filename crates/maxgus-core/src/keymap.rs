//! The default global keymap.
//!
//! These are Emacs' own default bindings. Where `maxgus` has no equivalent of
//! an Emacs command the binding is left out rather than pointed at something
//! that only half works; where `maxgus` adds something Emacs has no default for
//! — the file tree, the language-server commands — the binding follows the
//! convention of the package everyone uses for it (`treemacs`, `lsp-mode`).

use maxgus_keys::{Keymap, Result};

/// Every binding in the global map, as (key sequence, command) pairs.
pub const GLOBAL_BINDINGS: &[(&str, &str)] = &[
    // ---- character and line motion ----
    ("C-f", "forward-char"),
    ("C-b", "backward-char"),
    ("C-n", "next-line"),
    ("C-p", "previous-line"),
    ("<right>", "forward-char"),
    ("<left>", "backward-char"),
    ("<down>", "next-line"),
    ("<up>", "previous-line"),
    ("C-a", "move-beginning-of-line"),
    ("C-e", "move-end-of-line"),
    ("<home>", "move-beginning-of-line"),
    ("<end>", "move-end-of-line"),
    ("M-m", "back-to-indentation"),
    // ---- word, sentence, paragraph, sexp ----
    ("M-f", "forward-word"),
    ("M-b", "backward-word"),
    ("M-e", "forward-sentence"),
    ("M-a", "backward-sentence"),
    ("M-}", "forward-paragraph"),
    ("M-{", "backward-paragraph"),
    ("C-M-f", "forward-sexp"),
    ("C-M-b", "backward-sexp"),
    ("C-M-a", "beginning-of-defun"),
    ("C-M-e", "end-of-defun"),
    // ---- buffer motion and scrolling ----
    ("M-<", "beginning-of-buffer"),
    ("M->", "end-of-buffer"),
    ("C-v", "scroll-up-command"),
    ("M-v", "scroll-down-command"),
    ("C-M-v", "scroll-other-window"),
    ("M-r", "move-to-window-line-top-bottom"),
    ("<next>", "scroll-up-command"),
    ("<prior>", "scroll-down-command"),
    ("C-l", "recenter-top-bottom"),
    ("M-g g", "goto-line"),
    ("M-g M-g", "goto-line"),
    ("M-g c", "goto-char"),
    ("M-g n", "next-error"),
    ("M-g M-n", "next-error"),
    ("M-g p", "previous-error"),
    ("M-g M-p", "previous-error"),
    // ---- insertion and deletion ----
    ("RET", "newline"),
    ("C-j", "electric-newline-and-maybe-indent"),
    ("C-o", "open-line"),
    ("C-M-o", "split-line"),
    ("TAB", "indent-for-tab-command"),
    ("DEL", "delete-backward-char"),
    ("C-d", "delete-char"),
    ("<delete>", "delete-char"),
    ("M-d", "kill-word"),
    ("M-DEL", "backward-kill-word"),
    ("C-k", "kill-line"),
    ("C-S-DEL", "kill-whole-line"),
    ("C-q", "quoted-insert"),
    ("M-\\", "delete-horizontal-space"),
    ("M-SPC", "just-one-space"),
    ("M-^", "delete-indentation"),
    ("M-z", "zap-to-char"),
    ("C-M-k", "kill-sexp"),
    ("M-k", "kill-sentence"),
    ("C-x DEL", "backward-kill-sentence"),
    // ---- mark, region and the kill ring ----
    ("C-SPC", "set-mark-command"),
    ("C-w", "kill-region"),
    ("M-w", "kill-ring-save"),
    ("C-y", "yank"),
    ("M-y", "yank-pop"),
    ("M-@", "mark-word"),
    ("C-M-@", "mark-sexp"),
    ("M-h", "mark-paragraph"),
    ("C-M-h", "mark-defun"),
    ("C-=", "expand-region"),
    // ---- transposition and case ----
    ("C-t", "transpose-chars"),
    ("M-t", "transpose-words"),
    ("M-u", "upcase-word"),
    ("M-l", "downcase-word"),
    ("M-c", "capitalize-word"),
    // ---- undo ----
    ("C-/", "undo"),
    ("C-_", "undo"),
    ("C-M-/", "undo-redo"),
    // ---- filling and comments ----
    ("M-q", "fill-paragraph"),
    ("M-;", "comment-dwim"),
    ("C-x C-;", "comment-line"),
    // ---- search and replace ----
    ("C-s", "isearch-forward"),
    ("C-r", "isearch-backward"),
    ("C-M-s", "isearch-forward-regexp"),
    ("C-M-r", "isearch-backward-regexp"),
    ("M-%", "query-replace"),
    ("C-M-%", "query-replace-regexp"),
    ("M-s o", "occur"),
    // ---- files ----
    ("C-x C-f", "find-file"),
    ("C-x C-v", "find-alternate-file"),
    ("C-x C-s", "save-buffer"),
    ("C-x C-w", "write-file"),
    ("C-x s", "save-some-buffers"),
    ("C-x i", "insert-file"),
    ("C-x C-c", "save-buffers-kill-terminal"),
    // ---- buffers ----
    ("C-x b", "switch-to-buffer"),
    ("C-x C-b", "list-buffers"),
    ("C-x k", "kill-buffer"),
    ("C-x <right>", "next-buffer"),
    ("C-x <left>", "previous-buffer"),
    ("C-x C-<right>", "next-buffer"),
    ("C-x C-<left>", "previous-buffer"),

    // Directional window movement. `C-x o` cycles in storage order, which
    // with a file tree open means guessing where you land; these say where to
    // go. Emacs puts windmove on `S-<arrow>` by default, but a terminal
    // often eats shifted arrows, and control-arrows are what was asked for.
    ("C-<left>", "windmove-left"),
    ("C-<right>", "windmove-right"),
    ("C-<up>", "windmove-up"),
    ("C-<down>", "windmove-down"),
    ("C-x C-q", "read-only-mode"),
    ("C-x RET f", "set-buffer-file-coding-system"),
    ("C-x x g", "revert-buffer"),
    ("C-x 4 b", "switch-to-buffer-other-window"),
    ("C-x 4 f", "find-file-other-window"),
    // ---- windows ----
    ("C-x 0", "delete-window"),
    ("C-x 1", "delete-other-windows"),
    ("C-x 2", "split-window-below"),
    ("C-x 3", "split-window-right"),
    ("C-x o", "other-window"),
    ("C-x 4 0", "kill-buffer-and-window"),
    ("C-x ^", "enlarge-window"),
    ("C-x }", "enlarge-window-horizontally"),
    ("C-x {", "shrink-window-horizontally"),
    ("C-x +", "balance-windows"),
    // ---- narrowing ----
    ("C-x n n", "narrow-to-region"),
    ("C-x n d", "narrow-to-defun"),
    ("C-x n w", "widen"),
    // ---- point, mark and the rest of C-x ----
    ("C-x C-x", "exchange-point-and-mark"),
    ("C-x h", "mark-whole-buffer"),
    ("C-x u", "undo"),
    ("C-x C-u", "upcase-region"),
    ("C-x C-l", "downcase-region"),
    ("C-x C-t", "transpose-lines"),
    ("C-x C-o", "delete-blank-lines"),
    ("C-x TAB", "indent-rigidly"),
    ("C-M-\\", "indent-region"),
    ("M-i", "tab-to-tab-stop"),
    ("C-x =", "what-cursor-position"),
    ("M-=", "count-words"),
    ("C-x z", "repeat"),
    // ---- registers ----
    ("C-x r SPC", "point-to-register"),
    ("C-x r j", "jump-to-register"),
    ("C-x r s", "copy-to-register"),
    ("C-x r i", "insert-register"),
    ("C-x r n", "number-to-register"),
    ("C-x r +", "increment-register"),
    ("C-x r r", "copy-rectangle-to-register"),
    // ---- keyboard macros ----
    ("C-x (", "kmacro-start-macro"),
    ("C-x )", "kmacro-end-macro"),
    ("C-x e", "kmacro-end-and-call-macro"),
    // ---- the file tree ----
    ("C-x t t", "treefile-toggle"),
    ("C-x t 1", "treefile-select"),
    ("C-x t d", "treefile-select-directory"),
    // ---- language server ----
    ("M-.", "lsp-find-definition"),
    ("M-,", "pop-mark"),
    ("M-?", "lsp-find-references"),
    ("C-M-i", "completion-at-point"),
    ("C-c l d", "lsp-describe-thing-at-point"),
    ("C-c l r", "lsp-rename"),
    ("C-c l f", "lsp-format-buffer"),
    ("C-c l a", "lsp-code-action"),
    ("C-c l s", "lsp-workspace-symbol"),
    ("C-c l h", "lsp-signature-help"),
    ("C-c l o", "lsp-document-symbols"),
    ("C-c l R", "lsp-restart-server"),
    // ---- help ----
    ("C-h k", "describe-key"),
    ("C-h f", "describe-function"),
    ("C-h v", "describe-variable"),
    ("C-h b", "describe-bindings"),
    ("C-h m", "describe-mode"),
    ("C-h w", "where-is"),
    ("C-h s", "describe-syntax-at-point"),
    ("C-h t", "help-with-tutorial"),
    ("<f1>", "describe-key"),
    // ---- shell ----
    ("M-!", "shell-command"),
    ("M-|", "shell-command-on-region"),
    // ---- prefix arguments and quitting ----
    ("C-u", "universal-argument"),
    ("M--", "negative-argument"),
    ("C-g", "keyboard-quit"),
    ("ESC ESC ESC", "keyboard-escape-quit"),
    ("M-x", "execute-extended-command"),
    ("C-z", "suspend-maxgus"),
];

/// `M-0` through `M-9`, which start or extend a numeric prefix argument.
pub const DIGIT_ARGUMENT_KEYS: [&str; 10] =
    ["M-0", "M-1", "M-2", "M-3", "M-4", "M-5", "M-6", "M-7", "M-8", "M-9"];

/// Builds the global keymap, with `self-insert-command` as the fallback for
/// any printable key that is not otherwise bound.
pub fn global_keymap() -> Result<Keymap> {
    let mut map = Keymap::new("global");
    for (keys, command) in GLOBAL_BINDINGS {
        map.define_str(keys, *command)?;
    }
    for keys in DIGIT_ARGUMENT_KEYS {
        map.define_str(keys, "digit-argument")?;
    }
    map.set_default_binding(Some("self-insert-command".to_string()));
    Ok(map)
}

/// The keymap active while an `isearch` is running. It shadows the global map
/// so ordinary characters extend the search string instead of self-inserting.
pub const ISEARCH_BINDINGS: &[(&str, &str)] = &[
    ("C-s", "isearch-repeat-forward"),
    ("C-r", "isearch-repeat-backward"),
    ("RET", "isearch-exit"),
    ("C-g", "isearch-abort"),
    ("DEL", "isearch-delete-char"),
    ("C-w", "isearch-yank-word"),
    ("C-y", "isearch-yank-line"),
    ("M-y", "isearch-yank-kill"),
    ("C-M-y", "isearch-yank-char"),
    ("ESC", "isearch-exit"),
    ("<up>", "isearch-repeat-backward"),
    ("<down>", "isearch-repeat-forward"),
];

/// Builds the isearch keymap.
pub fn isearch_keymap() -> Result<Keymap> {
    let mut map = Keymap::new("isearch-mode");
    for (keys, command) in ISEARCH_BINDINGS {
        map.define_str(keys, *command)?;
    }
    // Anything else extends the search string.
    map.set_default_binding(Some("isearch-printing-char".to_string()));
    Ok(map)
}

/// The keymap active while the minibuffer is prompting.
pub const MINIBUFFER_BINDINGS: &[(&str, &str)] = &[
    ("RET", "minibuffer-complete-and-exit"),
    ("C-g", "minibuffer-keyboard-quit"),
    ("TAB", "minibuffer-complete"),
    ("S-TAB", "minibuffer-complete-backward"),
    ("SPC", "minibuffer-complete-word"),
    ("DEL", "minibuffer-delete-backward-char"),
    ("C-d", "minibuffer-delete-char"),
    ("C-a", "minibuffer-beginning-of-line"),
    ("C-e", "minibuffer-end-of-line"),
    ("C-f", "minibuffer-forward-char"),
    ("C-b", "minibuffer-backward-char"),
    ("<left>", "minibuffer-backward-char"),
    ("<right>", "minibuffer-forward-char"),
    ("<home>", "minibuffer-beginning-of-line"),
    ("<end>", "minibuffer-end-of-line"),
    ("C-k", "minibuffer-kill-line"),
    ("M-DEL", "minibuffer-backward-kill-word"),
    ("M-p", "minibuffer-previous-history"),
    ("M-n", "minibuffer-next-history"),
    // The arrows walk the candidate list when a prompt is showing one, and
    // fall back to the history when it is not.
    ("<up>", "minibuffer-previous-candidate"),
    ("<down>", "minibuffer-next-candidate"),
    ("<prior>", "minibuffer-previous-candidate-page"),
    ("<next>", "minibuffer-next-candidate-page"),
    ("C-n", "minibuffer-next-candidate"),
    ("C-p", "minibuffer-previous-candidate"),
    ("C-y", "minibuffer-yank"),
];

/// Builds the minibuffer keymap.
pub fn minibuffer_keymap() -> Result<Keymap> {
    let mut map = Keymap::new("minibuffer-mode");
    for (keys, command) in MINIBUFFER_BINDINGS {
        map.define_str(keys, *command)?;
    }
    map.set_default_binding(Some("minibuffer-self-insert".to_string()));
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use maxgus_keys::KeySequence;

    fn seq(s: &str) -> KeySequence {
        KeySequence::parse(s).unwrap()
    }

    #[test]
    fn the_global_map_builds_without_prefix_conflicts() {
        let map = global_keymap().expect("no binding shadows a prefix");
        assert_eq!(map.name(), "global");
    }

    #[test]
    fn every_global_binding_is_reachable() {
        let map = global_keymap().unwrap();
        for (keys, command) in GLOBAL_BINDINGS {
            assert_eq!(
                map.lookup(&seq(keys)).command(),
                Some(*command),
                "`{keys}` should run `{command}`"
            );
        }
    }

    #[test]
    fn no_global_key_sequence_is_bound_twice() {
        let mut keys: Vec<&str> = GLOBAL_BINDINGS.iter().map(|(k, _)| *k).collect();
        keys.sort_unstable();
        let before = keys.len();
        keys.dedup();
        assert_eq!(keys.len(), before, "a key sequence appears twice in the table");
    }

    #[test]
    fn printable_keys_fall_through_to_self_insert() {
        let map = global_keymap().unwrap();
        assert_eq!(map.lookup(&seq("q")).command(), Some("self-insert-command"));
        assert_eq!(map.lookup(&seq("Z")).command(), Some("self-insert-command"));
        assert_eq!(map.lookup(&seq("(")).command(), Some("self-insert-command"));
        // A bound key is not shadowed by the fallback.
        assert_eq!(map.lookup(&seq("C-f")).command(), Some("forward-char"));
    }

    #[test]
    fn the_buffer_and_comment_keys_emacs_uses_are_all_present() {
        // These three commands were implemented and registered but had no key
        // at all, so they existed only through `M-x`. The sequences are the
        // ones Emacs itself uses, including the control-modified arrows it
        // binds alongside the plain ones.
        let map = global_keymap().expect("a keymap");
        for (keys, command) in [
            ("C-x <right>", "next-buffer"),
            ("C-x <left>", "previous-buffer"),
            ("C-x C-<right>", "next-buffer"),
            ("C-x C-<left>", "previous-buffer"),
            ("C-x C-;", "comment-line"),
        ] {
            assert_eq!(map.lookup(&seq(keys)).command(), Some(command), "`{keys}`");
        }
    }

    #[test]
    fn the_coding_system_key_is_reachable_through_the_return_prefix() {
        // `RET` in the middle of a sequence is the awkward case: it arrives as
        // a named key rather than a character, so a binding written with it
        // has to survive the same folding an ordinary key does.
        let map = global_keymap().expect("a keymap");
        assert!(map.lookup(&seq("C-x RET")).is_prefix());
        assert_eq!(
            map.lookup(&seq("C-x RET f")).command(),
            Some("set-buffer-file-coding-system")
        );
    }

    #[test]
    fn the_c_x_prefix_stays_live_until_completed() {
        let map = global_keymap().unwrap();
        assert!(map.lookup(&seq("C-x")).is_prefix());
        assert!(map.lookup(&seq("C-x r")).is_prefix());
        assert!(map.lookup(&seq("C-x 4")).is_prefix());
        assert!(map.lookup(&seq("C-x n")).is_prefix());
        assert!(map.lookup(&seq("C-x t")).is_prefix());
        assert_eq!(map.lookup(&seq("C-x r j")).command(), Some("jump-to-register"));
    }

    #[test]
    fn every_multi_key_prefix_resolves() {
        let map = global_keymap().unwrap();
        for prefix in
            ["C-x", "C-h", "C-c", "M-g", "M-s", "C-c l", "C-x r", "C-x 4", "C-x t", "C-x RET"]
        {
            assert!(map.lookup(&seq(prefix)).is_prefix(), "`{prefix}` should be a prefix");
        }
    }

    #[test]
    fn digit_arguments_are_bound_for_every_digit() {
        let map = global_keymap().unwrap();
        for keys in DIGIT_ARGUMENT_KEYS {
            assert_eq!(map.lookup(&seq(keys)).command(), Some("digit-argument"), "`{keys}`");
        }
        assert_eq!(map.lookup(&seq("M--")).command(), Some("negative-argument"));
        assert_eq!(map.lookup(&seq("C-u")).command(), Some("universal-argument"));
    }

    #[test]
    fn the_motion_commands_a_user_reaches_for_first_are_all_there() {
        let map = global_keymap().unwrap();
        let expected = [
            ("C-f", "forward-char"),
            ("C-b", "backward-char"),
            ("C-n", "next-line"),
            ("C-p", "previous-line"),
            ("C-a", "move-beginning-of-line"),
            ("C-e", "move-end-of-line"),
            ("M-f", "forward-word"),
            ("M-b", "backward-word"),
            ("M-<", "beginning-of-buffer"),
            ("M->", "end-of-buffer"),
            ("C-v", "scroll-up-command"),
            ("M-v", "scroll-down-command"),
        ];
        for (keys, command) in expected {
            assert_eq!(map.lookup(&seq(keys)).command(), Some(command), "`{keys}`");
        }
    }

    #[test]
    fn the_kill_ring_bindings_match_emacs() {
        let map = global_keymap().unwrap();
        let expected = [
            ("C-SPC", "set-mark-command"),
            ("C-w", "kill-region"),
            ("M-w", "kill-ring-save"),
            ("C-y", "yank"),
            ("M-y", "yank-pop"),
            ("C-k", "kill-line"),
            ("M-d", "kill-word"),
            ("M-DEL", "backward-kill-word"),
        ];
        for (keys, command) in expected {
            assert_eq!(map.lookup(&seq(keys)).command(), Some(command), "`{keys}`");
        }
    }

    #[test]
    fn the_file_and_window_bindings_match_emacs() {
        let map = global_keymap().unwrap();
        let expected = [
            ("C-x C-f", "find-file"),
            ("C-x C-s", "save-buffer"),
            ("C-x C-c", "save-buffers-kill-terminal"),
            ("C-x b", "switch-to-buffer"),
            ("C-x k", "kill-buffer"),
            ("C-x 0", "delete-window"),
            ("C-x 1", "delete-other-windows"),
            ("C-x 2", "split-window-below"),
            ("C-x 3", "split-window-right"),
            ("C-x o", "other-window"),
        ];
        for (keys, command) in expected {
            assert_eq!(map.lookup(&seq(keys)).command(), Some(command), "`{keys}`");
        }
    }

    #[test]
    fn undo_is_reachable_by_all_three_of_its_bindings() {
        let map = global_keymap().unwrap();
        let mut found: Vec<String> = map.where_is("undo").iter().map(|s| s.notation()).collect();
        found.sort();
        assert_eq!(found, vec!["C-/", "C-_", "C-x u"]);
    }

    #[test]
    fn goto_line_answers_to_both_of_its_sequences() {
        let map = global_keymap().unwrap();
        assert_eq!(map.where_is("goto-line").len(), 2);
    }

    #[test]
    fn the_escape_prefix_reaches_meta_bindings() {
        let map = global_keymap().unwrap();
        // A terminal that cannot send Meta sends ESC first; the key layer
        // rewrites it, and the result must find the same binding.
        let rewritten = seq("ESC x").canonicalize_escape_prefix();
        assert_eq!(rewritten.notation(), "M-x");
        assert_eq!(map.lookup(&rewritten).command(), Some("execute-extended-command"));
    }

    #[test]
    fn the_isearch_map_shadows_printing_characters() {
        let map = isearch_keymap().unwrap();
        assert_eq!(map.lookup(&seq("a")).command(), Some("isearch-printing-char"));
        assert_eq!(map.lookup(&seq("C-s")).command(), Some("isearch-repeat-forward"));
        assert_eq!(map.lookup(&seq("C-r")).command(), Some("isearch-repeat-backward"));
        assert_eq!(map.lookup(&seq("C-g")).command(), Some("isearch-abort"));
        assert_eq!(map.lookup(&seq("RET")).command(), Some("isearch-exit"));
    }

    #[test]
    fn the_minibuffer_map_shadows_printing_characters() {
        let map = minibuffer_keymap().unwrap();
        assert_eq!(map.lookup(&seq("a")).command(), Some("minibuffer-self-insert"));
        assert_eq!(map.lookup(&seq("TAB")).command(), Some("minibuffer-complete"));
        assert_eq!(map.lookup(&seq("RET")).command(), Some("minibuffer-complete-and-exit"));
        assert_eq!(map.lookup(&seq("C-g")).command(), Some("minibuffer-keyboard-quit"));
        assert_eq!(map.lookup(&seq("M-p")).command(), Some("minibuffer-previous-history"));
    }

    #[test]
    fn every_auxiliary_map_builds_without_conflicts() {
        assert!(isearch_keymap().is_ok());
        assert!(minibuffer_keymap().is_ok());
    }

    #[test]
    fn no_map_binds_the_same_key_twice_however_it_is_spelled() {
        // Comparing the written descriptions is not enough: `C-i` and `TAB`
        // are one key once the control aliases are folded, so two entries can
        // collide while looking different. Comparing parsed sequences is what
        // actually catches it.
        for (name, table) in [
            ("global", GLOBAL_BINDINGS),
            ("isearch", ISEARCH_BINDINGS),
            ("minibuffer", MINIBUFFER_BINDINGS),
        ] {
            let mut seen: std::collections::BTreeMap<String, &str> = Default::default();
            for (keys, command) in table {
                let canonical = seq(keys).notation();
                if let Some(existing) = seen.insert(canonical.clone(), command) {
                    panic!(
                        "`{name}` binds {canonical} twice: `{existing}` and `{command}`                          (written `{keys}`)"
                    );
                }
            }
        }
    }

    #[test]
    fn a_binding_written_with_a_control_alias_reaches_the_same_command() {
        let map = global_keymap().unwrap();
        // However the user writes it, `C-M-i` and `M-TAB` are one key.
        assert_eq!(
            map.lookup(&seq("C-M-i")).command(),
            Some("completion-at-point")
        );
        assert_eq!(
            map.lookup(&seq("M-TAB")).command(),
            Some("completion-at-point"),
            "the spelling a terminal actually delivers"
        );
        // Likewise TAB and `C-i`.
        assert_eq!(map.lookup(&seq("TAB")).command(), Some("indent-for-tab-command"));
        assert_eq!(map.lookup(&seq("C-i")).command(), Some("indent-for-tab-command"));
        // And RET and `C-m`.
        assert_eq!(map.lookup(&seq("RET")).command(), Some("newline"));
        assert_eq!(map.lookup(&seq("C-m")).command(), Some("newline"));
    }

    #[test]
    fn command_names_use_the_conventional_spelling() {
        // Emacs command names are lowercase words joined by hyphens; anything
        // else is a typo waiting to be discovered at run time.
        for (keys, command) in GLOBAL_BINDINGS.iter().chain(ISEARCH_BINDINGS).chain(MINIBUFFER_BINDINGS) {
            assert!(
                command.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
                "`{keys}` runs `{command}`, which is not a conventional command name"
            );
        }
    }

    #[test]
    fn a_user_binding_layers_over_the_defaults() {
        let mut map = global_keymap().unwrap();
        let mut user = maxgus_keys::Keymap::new("user");
        user.define_str("C-x C-f", "my-find-file").unwrap();
        user.define_str("C-c p", "my-project-command").unwrap();
        map.merge(&user);
        assert_eq!(map.lookup(&seq("C-x C-f")).command(), Some("my-find-file"));
        assert_eq!(map.lookup(&seq("C-c p")).command(), Some("my-project-command"));
        assert_eq!(map.lookup(&seq("C-x C-s")).command(), Some("save-buffer"), "untouched");
        assert_eq!(map.lookup(&seq("q")).command(), Some("self-insert-command"), "fallback kept");
    }
}
