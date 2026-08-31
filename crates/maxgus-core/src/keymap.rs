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
    #[cfg(feature = "full")]
    ("M-g n", "next-error"),
    #[cfg(feature = "full")]
    ("M-g M-n", "next-error"),
    #[cfg(feature = "full")]
    ("M-g p", "previous-error"),
    #[cfg(feature = "full")]
    ("M-g M-p", "previous-error"),
    // ---- insertion and deletion ----
    ("RET", "newline"),
    ("C-j", "electric-newline-and-maybe-indent"),
    ("C-o", "open-line"),
    ("C-M-o", "split-line"),
    ("TAB", "indent-for-tab-command"),
    ("DEL", "delete-backward-char"),
    // `C-d` duplicates rather than deletes, as this configuration binds it.
    // `<delete>` is still the forward delete.
    ("C-d", "duplicate-line-or-region"),
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
    #[cfg(feature = "full")]
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
    #[cfg(feature = "full")]
    ("M-s g", "project-grep"),
    #[cfg(feature = "full")]
    ("M-s G", "project-grep-literal"),
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
    ("C-x K", "kill-buffer-in-all-windows"),
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
    // Shift with them resizes, which is what this configuration binds.
    ("C-S-<up>", "shrink-window"),
    ("C-S-<down>", "enlarge-window"),
    ("C-S-<left>", "shrink-window-horizontally"),
    ("C-S-<right>", "enlarge-window-horizontally"),
    // The panel and its sections, on the super key.
    ("C-s-a", "treefile-toggle"),
    #[cfg(feature = "full")]
    ("C-s-o", "panel-toggle-symbols-section"),
    ("C-s-i", "panel-toggle-buffers-section"),
    ("C-s-p", "panel-toggle-tree-section"),
    #[cfg(feature = "full")]
    ("s-t", "terminal-toggle"),
    #[cfg(feature = "full")]
    ("C-<tab>", "lsp-describe-thing-at-point"),
    // Doom's own: the tree on a function key.
    ("<f9>", "treefile-toggle"),
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
    ("C-x d", "dired"),
    // Emacs' `list-directory` slot, which this editor has nothing else for.
    ("C-x C-d", "browse-files"),
    ("C-x u", "undo"),
    ("S-TAB", "snippet-previous-field"),
    // Several cursors, spelled as `multiple-cursors` spells them.
    ("C->", "mark-next-like-this"),
    ("C-<", "mark-previous-like-this"),
    ("C-c C-<", "mark-all-like-this"),
    ("C-c C->", "unmark-cursor"),
    // The visualiser beside `undo` rather than over it: `C-x u` is undo in
    // every Emacs that has not loaded undo-tree, and that is muscle memory
    // worth more than matching a package's own binding.
    ("C-x U", "undo-tree-visualize"),
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
    #[cfg(feature = "full")]
    ("C-x g", "magit-status"),
    ("C-x t t", "treefile-toggle"),
    #[cfg(feature = "full")]
    ("C-x t v", "terminal-toggle"),
    #[cfg(feature = "full")]
    ("C-x t s", "terminal-select"),
    ("C-x t 1", "treefile-select"),
    ("C-x t 2", "panel-select-symbols"),
    ("C-x t 3", "panel-select-buffers"),
    ("C-x t d", "treefile-select-directory"),
    // ---- language server ----
    #[cfg(feature = "full")]
    ("M-.", "lsp-find-definition"),
    ("M-,", "pop-mark"),
    #[cfg(feature = "full")]
    ("M-?", "lsp-find-references"),
    #[cfg(feature = "full")]
    ("C-M-i", "completion-at-point"),
    #[cfg(feature = "full")]
    ("C-'", "lsp-document-symbols"),
    // ---- the leader ----
    //
    // Doom's non-evil leader is `C-c`, with `C-c l` left to the localleader
    // and `C-c e` to eval, so nothing here takes either. The language server
    // lives under `C-c c`, which is Doom's code map, and not under `C-c l`.

    // `C-c c` — code.
    #[cfg(feature = "full")]
    ("C-c c d", "lsp-find-definition"),
    #[cfg(feature = "full")]
    ("C-c c D", "lsp-find-references"),
    #[cfg(feature = "full")]
    ("C-c c f", "lsp-format-buffer"),
    #[cfg(feature = "full")]
    ("C-c c r", "lsp-rename"),
    #[cfg(feature = "full")]
    ("C-c c a", "lsp-code-action"),
    #[cfg(feature = "full")]
    ("C-c c k", "lsp-describe-thing-at-point"),
    #[cfg(feature = "full")]
    ("C-c c j", "lsp-workspace-symbol"),
    #[cfg(feature = "full")]
    ("C-c c h", "lsp-signature-help"),
    #[cfg(feature = "full")]
    ("C-c c x", "next-error"),
    #[cfg(feature = "full")]
    ("C-c c R", "lsp-restart-server"),
    ("C-c c w", "delete-trailing-whitespace"),
    // `C-c f` — files.
    // The workspace: a named set of directories for the tree. `p` for
    // project, the way Doom's leader spells it.
    ("C-c p p", "workspace-switch"),
    ("C-c p s", "workspace-save"),
    ("C-c p d", "workspace-delete"),
    ("C-c f f", "find-file"),
    ("C-c f b", "browse-files"),
    ("C-c f d", "dired"),
    ("C-c f D", "delete-this-file"),
    ("C-c f m", "move-this-file"),
    ("C-c f C", "copy-this-file"),
    ("C-c f y", "yank-buffer-path"),
    ("C-c f Y", "yank-buffer-path-relative-to-project"),
    ("C-c f p", "edit-configuration"),
    // `C-c s` — search.
    #[cfg(feature = "full")]
    ("C-c s p", "project-grep"),
    #[cfg(feature = "full")]
    ("C-c s .", "project-grep-literal"),
    ("C-c s b", "occur"),
    ("C-c s s", "occur"),
    #[cfg(feature = "full")]
    ("C-c s i", "lsp-document-symbols"),
    #[cfg(feature = "full")]
    ("C-c s I", "lsp-workspace-symbol"),
    // `C-c o` — open.
    #[cfg(feature = "full")]
    ("C-c o t", "terminal-toggle"),
    ("C-c o p", "treefile-toggle"),
    ("C-c o -", "dired"),
    ("C-c o b", "open-externally"),
    // `C-c t` — toggle.
    ("C-c t l", "toggle-line-numbers"),
    ("C-c t r", "read-only-mode"),
    ("C-c t c", "toggle-fill-column-indicator"),
    ("C-c t I", "toggle-indent-style"),
    ("C-c t w", "toggle-truncate-lines"),
    // `C-c v` — versioning.
    #[cfg(feature = "full")]
    ("C-c v g", "magit-status"),
    #[cfg(feature = "full")]
    ("C-c v /", "magit-dispatch"),
    // `C-c m` — several cursors.
    ("C-c m n", "mark-next-like-this"),
    ("C-c m N", "unmark-cursor"),
    ("C-c m p", "mark-previous-like-this"),
    ("C-c m t", "mark-all-like-this"),
    ("C-c m <down>", "cursor-at-next-line"),
    ("C-c m <up>", "cursor-at-previous-line"),
    // `C-c i` — insert, and `C-c &` — snippets.
    ("C-c i s", "insert-snippet"),
    ("C-c i y", "yank-pop"),
    ("C-c & i", "insert-snippet"),
    // `C-c q` — quitting, and the session.
    ("C-c q q", "save-buffers-kill-terminal"),
    ("C-c q s", "save-session"),
    ("C-c q l", "restore-session"),
    // `C-c w` — windows, where Doom also keeps the session.
    ("C-c w s", "save-session"),
    ("C-c w l", "restore-session"),
    // ---- help ----
    ("C-h k", "describe-key"),
    ("C-h f", "describe-function"),
    ("C-h v", "describe-variable"),
    ("C-h b", "describe-bindings"),
    ("C-h m", "describe-mode"),
    ("C-h w", "where-is"),
    #[cfg(feature = "full")]
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
pub const DIGIT_ARGUMENT_KEYS: [&str; 10] = [
    "M-0", "M-1", "M-2", "M-3", "M-4", "M-5", "M-6", "M-7", "M-8", "M-9",
];

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
/// The keys the file browser answers to.
///
/// Everything else goes into the filter, by way of the map's default
/// binding — which is what makes it a thing you type at rather than a thing
/// you drive. So the keys it does keep are the ones that are not letters:
/// the arrows, `RET`, `C-g`, and the `C-n`/`C-p` an Emacs hand reaches for
/// without thinking.
pub const BROWSE_BINDINGS: &[(&str, &str)] = &[
    ("<down>", "browse-files-next"),
    ("<up>", "browse-files-previous"),
    ("C-n", "browse-files-next"),
    ("C-p", "browse-files-previous"),
    ("<next>", "browse-files-next"),
    ("<prior>", "browse-files-previous"),
    ("M-<", "browse-files-first"),
    ("M->", "browse-files-last"),
    // Right goes in, left comes out, which is the whole of walking a tree
    // with one hand.
    ("<right>", "browse-files-enter"),
    ("<left>", "browse-files-up"),
    // Wider, which is what `C-s` means everywhere else in the editor.
    ("C-s", "browse-files-search"),
    ("RET", "browse-files-open"),
    ("DEL", "browse-files-rub-out"),
    ("<backspace>", "browse-files-rub-out"),
    ("C-g", "browse-files-quit"),
    ("<escape>", "browse-files-quit"),
];

pub fn browse_keymap() -> Result<Keymap> {
    let mut map = Keymap::new("browse-files-mode");
    for (keys, command) in BROWSE_BINDINGS {
        map.define_str(keys, *command)?;
    }
    map.set_default_binding(Some("browse-files-self-insert".to_string()));
    Ok(map)
}

pub fn minibuffer_keymap() -> Result<Keymap> {
    let mut map = Keymap::new("minibuffer-mode");
    for (keys, command) in MINIBUFFER_BINDINGS {
        map.define_str(keys, *command)?;
    }
    map.set_default_binding(Some("minibuffer-self-insert".to_string()));
    Ok(map)
}

#[cfg(feature = "full")]
/// The keys that are *not* sent to the shell.
///
/// Everything else is, by way of the map's default binding. The prefix is
/// `C-c`, which is what vterm uses and what a shell uses least — and `C-c c`
/// sends a real interrupt, so the one key the prefix takes away is given
/// straight back.
pub const TERMINAL_BINDINGS: &[(&str, &str)] = &[
    ("C-c C-t", "terminal-copy-mode"),
    ("C-c t", "terminal-new-tab"),
    ("C-c n", "terminal-next-tab"),
    ("C-c p", "terminal-previous-tab"),
    ("C-c k", "terminal-close-tab"),
    // The four keys the editor keeps, given straight back under the prefix.
    ("C-c c", "terminal-send-control"),
    ("C-c x", "terminal-send-control"),
    ("C-c g", "terminal-send-control"),
    ("C-c h", "terminal-send-control"),
    ("C-c C-y", "terminal-paste"),
    ("C-c C-v", "terminal-paste"),
    ("C-c 1", "terminal-select-tab"),
    ("C-c 2", "terminal-select-tab"),
    ("C-c 3", "terminal-select-tab"),
    ("C-c 4", "terminal-select-tab"),
    ("C-c 5", "terminal-select-tab"),
    ("C-c 6", "terminal-select-tab"),
    ("C-c 7", "terminal-select-tab"),
    ("C-c 8", "terminal-select-tab"),
    ("C-c 9", "terminal-select-tab"),
    // Scrolling back is worth having without leaving the shell, since it is
    // the one thing a reader wants that typing cannot give.
    ("S-<prior>", "terminal-scroll-up"),
    ("S-<next>", "terminal-scroll-down"),
];

#[cfg(feature = "full")]
/// Reading mode: keys move a cursor over the output instead of reaching the
/// shell, so a selection can be made without a mouse.
pub const TERMINAL_COPY_BINDINGS: &[(&str, &str)] = &[
    ("C-g", "terminal-copy-mode-quit"),
    ("q", "terminal-copy-mode-quit"),
    ("C-c C-t", "terminal-copy-mode-quit"),
    ("C-SPC", "terminal-set-mark"),
    ("C-x SPC", "terminal-set-block-mark"),
    ("V", "terminal-set-line-mark"),
    ("M-w", "terminal-copy"),
    ("w", "terminal-copy"),
    ("n", "terminal-next-line"),
    ("p", "terminal-previous-line"),
    ("C-n", "terminal-next-line"),
    ("C-p", "terminal-previous-line"),
    ("C-f", "terminal-forward-char"),
    ("C-b", "terminal-backward-char"),
    ("<down>", "terminal-next-line"),
    ("<up>", "terminal-previous-line"),
    ("<right>", "terminal-forward-char"),
    ("<left>", "terminal-backward-char"),
    ("C-a", "terminal-beginning-of-line"),
    ("C-e", "terminal-end-of-line"),
    ("<home>", "terminal-beginning-of-line"),
    ("<end>", "terminal-end-of-line"),
    ("<prior>", "terminal-scroll-up"),
    ("<next>", "terminal-scroll-down"),
    ("M-<", "terminal-goto-first"),
    ("M->", "terminal-goto-last"),
];

#[cfg(feature = "full")]
/// The map for typing at a shell: a few commands, and everything else sent.
pub fn terminal_keymap() -> Result<Keymap> {
    let mut map = Keymap::new(crate::commands::terminal::TERMINAL_MODE);
    for (keys, command) in TERMINAL_BINDINGS {
        map.define_str(keys, *command)?;
    }
    // The reason a terminal is a terminal: an unbound key is not an error, it
    // is a keystroke, and it belongs to the program running inside.
    map.set_default_binding(Some("terminal-send-key".to_string()));
    // Except these four, which stay the editor's or there would be no way to
    // leave the terminal, ask for help, run a command, or stop what is
    // happening. `C-c x`, `C-c g` and `C-c h` send them for real.
    let keep: Vec<maxgus_keys::Key> = ["C-x", "M-x", "C-h", "C-g"]
        .iter()
        .filter_map(|k| maxgus_keys::KeySequence::parse(k).ok())
        .filter_map(|sequence| sequence.keys().first().copied())
        .collect();
    map.set_default_catches_all(&keep);
    Ok(map)
}

#[cfg(feature = "full")]
/// The map for reading a terminal's output.
pub fn terminal_copy_keymap() -> Result<Keymap> {
    let mut map = Keymap::new(crate::commands::terminal::TERMINAL_COPY_MODE);
    for (keys, command) in TERMINAL_COPY_BINDINGS {
        map.define_str(keys, *command)?;
    }
    Ok(map)
}

#[cfg(feature = "full")]
/// Magit's own keymap.
///
/// The single letters are *menus*, not prefixes: `c` shows what committing
/// can mean here and what is switched on, and the second key chooses. That is
/// how magit is used, and it is why magit is usable without being memorised.
pub const MAGIT_BINDINGS: &[(&str, &str)] = &[
    // ---- moving ----
    // `n` and `p` move by section, skipping the lines inside a hunk; the
    // ordinary motion keys still move by line.
    ("n", "magit-next-section"),
    ("p", "magit-previous-section"),
    ("M-n", "magit-next-sibling"),
    ("M-p", "magit-previous-sibling"),
    ("^", "magit-parent-section"),
    ("C-n", "next-line"),
    ("C-p", "previous-line"),
    ("<down>", "next-line"),
    ("<up>", "previous-line"),
    ("M-<", "beginning-of-buffer"),
    ("M->", "end-of-buffer"),
    ("SPC", "scroll-up-command"),
    ("DEL", "scroll-down-command"),
    // ---- folding ----
    ("TAB", "magit-toggle"),
    ("S-TAB", "magit-toggle-all"),
    ("RET", "magit-visit"),
    // ---- the index, which is what the status view is for ----
    ("s", "magit-stage"),
    ("S", "magit-stage-all"),
    ("u", "magit-unstage"),
    ("U", "magit-unstage-all"),
    ("k", "magit-discard"),
    // ---- the menus ----
    ("?", "magit-dispatch"),
    ("h", "magit-dispatch"),
    ("c", "magit-commit-menu"),
    ("d", "magit-diff-menu"),
    ("l", "magit-log-menu"),
    ("b", "magit-branch-menu"),
    ("m", "magit-merge-menu"),
    ("r", "magit-rebase-menu"),
    ("X", "magit-reset-menu"),
    ("z", "magit-stash-menu"),
    ("t", "magit-tag-menu"),
    ("P", "magit-push-menu"),
    ("F", "magit-pull-menu"),
    ("f", "magit-fetch-menu"),
    ("M", "magit-remote-menu"),
    ("A", "magit-cherry-pick-menu"),
    ("V", "magit-revert-menu"),
    // ---- the other views ----
    ("y", "magit-show-refs"),
    ("$", "magit-process-buffer"),
    ("!", "magit-run"),
    ("i", "magit-gitignore"),
    // ---- the view itself ----
    ("g", "magit-refresh"),
    ("q", "magit-quit"),
];

#[cfg(feature = "full")]
/// Writing a commit message: the editor's own keys, plus two.
pub const COMMIT_BINDINGS: &[(&str, &str)] = &[
    ("C-c C-c", "magit-commit-finish"),
    ("C-c C-k", "magit-commit-cancel"),
];

#[cfg(feature = "full")]
pub fn magit_keymap() -> Result<Keymap> {
    let mut map = Keymap::new(crate::commands::git::GIT_MODE);
    for (keys, command) in MAGIT_BINDINGS {
        map.define_str(keys, *command)?;
    }
    Ok(map)
}

#[cfg(feature = "full")]
pub fn commit_keymap() -> Result<Keymap> {
    let mut map = Keymap::new(crate::commands::git::COMMIT_MODE);
    for (keys, command) in COMMIT_BINDINGS {
        map.define_str(keys, *command)?;
    }
    Ok(map)
}

#[cfg(feature = "full")]
/// The map a menu takes the keyboard with.
///
/// Every key, with no exceptions: a menu that let some keys through would be
/// competing with whatever they mean underneath, and `C-g` is handled by the
/// dispatch itself so there is always a way out.
pub fn transient_keymap() -> Result<Keymap> {
    let mut map = Keymap::new(crate::commands::transient::TRANSIENT_MODE);
    map.set_default_binding(Some("transient-dispatch".to_string()));
    map.set_default_catches_all(&[]);
    Ok(map)
}

/// The symbol outline's keys.
///
/// A short map: the outline is read and jumped from, so it needs moving,
/// folding and going, and nothing else.
pub const SYMBOLS_BINDINGS: &[(&str, &str)] = &[
    ("n", "next-line"),
    ("p", "previous-line"),
    ("TAB", "panel-toggle-symbol"),
    ("<left>", "panel-collapse-symbol"),
    ("<right>", "panel-expand-symbol"),
    ("RET", "panel-goto-symbol"),
    ("g", "panel-refresh-symbols"),
    ("q", "panel-quit"),
    ("t r", "panel-toggle-tree-section"),
    ("t s", "panel-toggle-symbols-section"),
    ("t b", "panel-toggle-buffers-section"),
];

/// The buffer list's keys.
pub const BUFFERS_BINDINGS: &[(&str, &str)] = &[
    ("n", "next-line"),
    ("p", "previous-line"),
    ("RET", "panel-switch-to-buffer"),
    ("k", "panel-kill-buffer"),
    ("d", "panel-kill-buffer"),
    ("q", "panel-quit"),
    ("t r", "panel-toggle-tree-section"),
    ("t s", "panel-toggle-symbols-section"),
    ("t b", "panel-toggle-buffers-section"),
];

/// The results of a project search: read like a list, edited like a buffer.
#[cfg(feature = "full")]
pub const GREP_BINDINGS: &[(&str, &str)] = &[
    ("n", "grep-next"),
    ("p", "grep-previous"),
    ("<down>", "grep-next"),
    ("<up>", "grep-previous"),
    ("RET", "grep-visit"),
    ("o", "grep-visit-other-window"),
    ("g", "grep-refresh"),
    ("q", "grep-quit"),
    ("C-c C-p", "grep-edit"),
    ("C-c C-e", "grep-edit"),
    ("C-c C-c", "grep-apply"),
    ("C-c C-k", "grep-abandon"),
];

/// The results while they are being written into: what the reading map binds
/// to letters is left to `self-insert-command`, and only the two keys that
/// finish the edit remain.
#[cfg(feature = "full")]
pub const GREP_EDIT_BINDINGS: &[(&str, &str)] =
    &[("C-c C-c", "grep-apply"), ("C-c C-k", "grep-abandon")];

/// The keys the suggestion list takes while it is on screen.
///
/// Everything else falls through to whatever it normally does — typing a
/// letter types it, and the list narrows to what is now written. Only the
/// keys that mean something to a list are taken.
#[cfg(feature = "full")]
pub const AUTOCOMPLETE_BINDINGS: &[(&str, &str)] = &[
    ("C-n", "autocomplete-next"),
    ("<down>", "autocomplete-next"),
    ("C-p", "autocomplete-previous"),
    ("<up>", "autocomplete-previous"),
    ("RET", "autocomplete-accept"),
    ("TAB", "autocomplete-accept"),
    ("C-g", "autocomplete-abort"),
    ("ESC", "autocomplete-abort"),
];

#[cfg(feature = "full")]
pub fn autocomplete_keymap() -> Result<Keymap> {
    let mut map = Keymap::new("autocomplete-mode");
    for (keys, command) in AUTOCOMPLETE_BINDINGS {
        map.define_str(keys, *command)?;
    }
    Ok(map)
}

#[cfg(feature = "full")]
pub fn grep_edit_keymap() -> Result<Keymap> {
    let mut map = Keymap::new(crate::commands::grep::GREP_EDIT_MODE);
    for (keys, command) in GREP_EDIT_BINDINGS {
        map.define_str(keys, *command)?;
    }
    Ok(map)
}

#[cfg(feature = "full")]
pub fn grep_keymap() -> Result<Keymap> {
    let mut map = Keymap::new(crate::commands::grep::GREP_MODE);
    for (keys, command) in GREP_BINDINGS {
        map.define_str(keys, *command)?;
    }
    Ok(map)
}

/// The visualiser: moving in it moves the buffer.
pub const UNDO_TREE_BINDINGS: &[(&str, &str)] = &[
    ("p", "undo-tree-undo"),
    ("<up>", "undo-tree-undo"),
    ("n", "undo-tree-redo"),
    ("<down>", "undo-tree-redo"),
    ("b", "undo-tree-switch-branch"),
    ("<left>", "undo-tree-switch-branch"),
    ("<right>", "undo-tree-switch-branch"),
    ("q", "undo-tree-quit"),
    ("RET", "undo-tree-quit"),
];

pub fn undo_tree_keymap() -> Result<Keymap> {
    let mut map = Keymap::new(crate::commands::undo_tree::VISUALIZER_MODE);
    for (keys, command) in UNDO_TREE_BINDINGS {
        map.define_str(keys, *command)?;
    }
    Ok(map)
}

/// Dired's own keys, spelled as Emacs spells them.
pub const DIRED_BINDINGS: &[(&str, &str)] = &[
    ("n", "dired-next"),
    ("p", "dired-previous"),
    ("<down>", "dired-next"),
    ("<up>", "dired-previous"),
    ("RET", "dired-visit"),
    ("f", "dired-visit"),
    ("^", "dired-up"),
    ("g", "dired-refresh"),
    ("m", "dired-mark"),
    ("u", "dired-unmark"),
    ("U", "dired-unmark-all"),
    ("t", "dired-toggle-marks"),
    ("d", "dired-flag-deletion"),
    ("x", "dired-do-flagged-delete"),
    ("D", "dired-do-delete"),
    ("C", "dired-do-copy"),
    ("R", "dired-do-rename"),
    ("+", "dired-create-directory"),
    ("!", "dired-do-shell-command"),
    ("q", "dired-quit"),
];

pub fn dired_keymap() -> Result<Keymap> {
    let mut map = Keymap::new(crate::commands::dired::DIRED_MODE);
    for (keys, command) in DIRED_BINDINGS {
        map.define_str(keys, *command)?;
    }
    Ok(map)
}

pub fn symbols_keymap() -> Result<Keymap> {
    let mut map = Keymap::new(crate::commands::tree::SYMBOLS_MODE);
    for (keys, command) in SYMBOLS_BINDINGS {
        map.define_str(keys, *command)?;
    }
    Ok(map)
}

pub fn buffers_keymap() -> Result<Keymap> {
    let mut map = Keymap::new(crate::commands::tree::BUFFERS_MODE);
    for (keys, command) in BUFFERS_BINDINGS {
        map.define_str(keys, *command)?;
    }
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
        assert_eq!(
            keys.len(),
            before,
            "a key sequence appears twice in the table"
        );
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
        assert_eq!(
            map.lookup(&seq("C-x r j")).command(),
            Some("jump-to-register")
        );
    }

    #[test]
    fn every_multi_key_prefix_resolves() {
        let map = global_keymap().unwrap();
        // Doom's leader is `C-c`, with a map under each of these letters.
        #[cfg(feature = "full")]
        let prefixes = [
            "C-x", "C-h", "C-c", "M-g", "M-s", "C-x r", "C-x 4", "C-x t", "C-x RET", "C-c c",
            "C-c f", "C-c s", "C-c o", "C-c t", "C-c m", "C-c i", "C-c q", "C-c w",
        ];
        #[cfg(not(feature = "full"))]
        let prefixes = [
            "C-x", "C-h", "C-c", "M-g", "M-s", "C-x r", "C-x 4", "C-x t", "C-x RET", "C-c f",
            "C-c o", "C-c t", "C-c m", "C-c i", "C-c q", "C-c w",
        ];
        for prefix in prefixes {
            assert!(
                map.lookup(&seq(prefix)).is_prefix(),
                "`{prefix}` should be a prefix"
            );
        }
        // `C-c l` is Doom's localleader and `C-c e` its eval key. Nothing
        // global may take either, or a mode's own bindings have nowhere to
        // live and a habit from Doom stops working.
        for reserved in ["C-c l", "C-c e"] {
            assert!(
                !map.lookup(&seq(reserved)).is_prefix()
                    && map.lookup(&seq(reserved)).command().is_none(),
                "`{reserved}` is Doom's to bind, and is taken"
            );
        }
    }

    #[test]
    fn digit_arguments_are_bound_for_every_digit() {
        let map = global_keymap().unwrap();
        for keys in DIGIT_ARGUMENT_KEYS {
            assert_eq!(
                map.lookup(&seq(keys)).command(),
                Some("digit-argument"),
                "`{keys}`"
            );
        }
        assert_eq!(map.lookup(&seq("M--")).command(), Some("negative-argument"));
        assert_eq!(
            map.lookup(&seq("C-u")).command(),
            Some("universal-argument")
        );
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
        assert_eq!(
            map.lookup(&rewritten).command(),
            Some("execute-extended-command")
        );
    }

    #[test]
    fn the_isearch_map_shadows_printing_characters() {
        let map = isearch_keymap().unwrap();
        assert_eq!(
            map.lookup(&seq("a")).command(),
            Some("isearch-printing-char")
        );
        assert_eq!(
            map.lookup(&seq("C-s")).command(),
            Some("isearch-repeat-forward")
        );
        assert_eq!(
            map.lookup(&seq("C-r")).command(),
            Some("isearch-repeat-backward")
        );
        assert_eq!(map.lookup(&seq("C-g")).command(), Some("isearch-abort"));
        assert_eq!(map.lookup(&seq("RET")).command(), Some("isearch-exit"));
    }

    #[test]
    fn the_minibuffer_map_shadows_printing_characters() {
        let map = minibuffer_keymap().unwrap();
        assert_eq!(
            map.lookup(&seq("a")).command(),
            Some("minibuffer-self-insert")
        );
        assert_eq!(
            map.lookup(&seq("TAB")).command(),
            Some("minibuffer-complete")
        );
        assert_eq!(
            map.lookup(&seq("RET")).command(),
            Some("minibuffer-complete-and-exit")
        );
        assert_eq!(
            map.lookup(&seq("C-g")).command(),
            Some("minibuffer-keyboard-quit")
        );
        assert_eq!(
            map.lookup(&seq("M-p")).command(),
            Some("minibuffer-previous-history")
        );
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

    #[cfg(feature = "full")]
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
        assert_eq!(
            map.lookup(&seq("TAB")).command(),
            Some("indent-for-tab-command")
        );
        assert_eq!(
            map.lookup(&seq("C-i")).command(),
            Some("indent-for-tab-command")
        );
        // And RET and `C-m`.
        assert_eq!(map.lookup(&seq("RET")).command(), Some("newline"));
        assert_eq!(map.lookup(&seq("C-m")).command(), Some("newline"));
    }

    #[test]
    fn command_names_use_the_conventional_spelling() {
        // Emacs command names are lowercase words joined by hyphens; anything
        // else is a typo waiting to be discovered at run time.
        for (keys, command) in GLOBAL_BINDINGS
            .iter()
            .chain(ISEARCH_BINDINGS)
            .chain(MINIBUFFER_BINDINGS)
        {
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
        assert_eq!(
            map.lookup(&seq("C-c p")).command(),
            Some("my-project-command")
        );
        assert_eq!(
            map.lookup(&seq("C-x C-s")).command(),
            Some("save-buffer"),
            "untouched"
        );
        assert_eq!(
            map.lookup(&seq("q")).command(),
            Some("self-insert-command"),
            "fallback kept"
        );
    }
}
