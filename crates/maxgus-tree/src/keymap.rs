//! The treemacs keymap.
//!
//! These are treemacs' own default bindings, with the command names adapted to
//! `maxgus`'s registry. Everything listed here is backed by a real command; the
//! bindings that operate on windows (`o v`, `o h`) are dispatched by
//! `maxgus-core`, which owns the window layout.

use maxgus_keys::{Keymap, Result};

/// Every binding in `treemacs-mode-map`, as (key sequence, command) pairs.
pub const TREEMACS_BINDINGS: &[(&str, &str)] = &[
    // ---- movement ----
    ("n", "treefile-next-line"),
    ("p", "treefile-previous-line"),
    ("<down>", "treefile-next-line"),
    ("<up>", "treefile-previous-line"),
    ("M-n", "treefile-next-neighbour"),
    ("M-p", "treefile-previous-neighbour"),
    ("u", "treefile-goto-parent"),
    ("M-<", "treefile-goto-first"),
    ("M->", "treefile-goto-last"),
    ("C-a", "treefile-goto-first"),
    ("C-e", "treefile-goto-last"),
    // ---- expansion ----
    ("TAB", "treefile-toggle-node"),
    ("RET", "treefile-visit-node"),
    ("<left>", "treefile-collapse-node"),
    ("<right>", "treefile-expand-node"),
    ("H", "treefile-collapse-parent"),
    ("C-c C-p", "treefile-expand-recursively"),
    // The root, under one prefix: down into the directory the cursor is
    // on, up one, or back to where the tree opened. treemacs calls the
    // first two `treemacs-root-down` and `treemacs-root-up` and leaves
    // them unbound; `<` and `>` are the width here, so they get their own
    // letter in the style of `c f`, `y a` and `t h`.
    ("r d", "treefile-root-down"),
    ("r u", "treefile-root-up"),
    ("r r", "treefile-root-reset"),
    // And the list of them. treemacs keeps these under `C-c C-p`, which is
    // already `treefile-expand-recursively` here; `r` is where the roots
    // live, so the two that add and remove one live there too.
    ("r a", "treefile-add-project"),
    ("r k", "treefile-remove-project"),
    // ---- visiting ----
    ("o o", "treefile-visit-node"),
    ("o v", "treefile-visit-node-vertical-split"),
    ("o h", "treefile-visit-node-horizontal-split"),
    ("o r", "treefile-visit-node-recent-window"),
    ("o x", "treefile-visit-node-external"),
    ("P", "treefile-peek"),
    // ---- file operations ----
    ("c f", "treefile-create-file"),
    ("c d", "treefile-create-dir"),
    ("R", "treefile-rename-file"),
    ("d", "treefile-delete-file"),
    ("m", "treefile-move-file"),
    ("!", "treefile-run-shell-command"),
    // ---- yanking paths ----
    ("y a", "treefile-copy-absolute-path"),
    ("y r", "treefile-copy-relative-path"),
    ("y p", "treefile-copy-project-path"),
    ("y f", "treefile-copy-file"),
    // ---- toggles ----
    ("t h", "treefile-toggle-show-dotfiles"),
    ("t w", "treefile-toggle-fixed-width"),
    ("t f", "treefile-toggle-follow-mode"),
    ("t g", "treefile-toggle-git-mode"),
    ("t d", "treefile-toggle-directories-first"),
    // ---- window ----
    ("w", "treefile-set-width"),
    ("<", "treefile-decrease-width"),
    (">", "treefile-increase-width"),
    // ---- the panel's other sections ----
    // `TAB` and `RET` are not here: they stay bound to the treemacs commands,
    // which ask what kind of row point is on and hand over. One `TAB` that
    // folds a heading, opens a directory, expands a symbol or visits a file
    // is what makes the three sections read as one panel.
    // The other two sections are windows of their own, so the ordinary
    // window keys — `C-<up>`, `C-<down>`, `C-x o` — reach them.
    ("t r", "panel-toggle-tree-section"),
    ("t s", "panel-toggle-symbols-section"),
    ("t b", "panel-toggle-buffers-section"),
    // ---- refresh and exit ----
    ("g s", "panel-refresh-symbols"),
    ("g r", "treefile-refresh"),
    ("g g", "treefile-goto-first"),
    ("s", "treefile-resort"),
    ("q", "treefile-quit"),
    ("Q", "treefile-kill"),
    ("?", "treefile-help"),
];

/// One column of the help panel: a heading, and the keys under it.
pub struct HelpSection {
    pub title: &'static str,
    /// The key, and the short phrase saying what it does.
    pub keys: &'static [(&'static str, &'static str)],
}

/// The keymap as `?` shows it, grouped the way treemacs' helpful hydra
/// groups it.
///
/// One row per *command* rather than per binding, which is treemacs' rule
/// too: `treemacs--find-keybind` asks what a function is bound to and shows
/// the one answer, so `n` is listed and the `<down>` that does the same
/// thing is not. A panel that listed both would be a third longer and no
/// more use — the second spelling is a convenience for the hand, not
/// something to be taught.
///
/// The phrases are short and lower case for the same reason treemacs' are:
/// these are columns, and a sentence in a column is a sentence that gets
/// cut. What each command does at length is what `M-x` says.
pub const TREEMACS_HELP: &[HelpSection] = &[
    HelpSection {
        title: "Navigation",
        keys: &[
            ("n", "next line"),
            ("p", "previous line"),
            ("M-n", "next neighbour"),
            ("M-p", "previous neighbour"),
            ("u", "goto parent"),
            ("M-<", "goto first"),
            ("M->", "goto last"),
        ],
    },
    HelpSection {
        title: "Nodes",
        keys: &[
            ("TAB", "toggle node"),
            ("RET", "visit node"),
            ("<right>", "expand"),
            ("<left>", "collapse"),
            ("H", "collapse parent"),
            ("C-c C-p", "expand recursively"),
        ],
    },
    HelpSection {
        title: "Opening",
        keys: &[
            ("o v", "vertical split"),
            ("o h", "horizontal split"),
            ("o r", "recent window"),
            ("o x", "external program"),
            ("P", "peek"),
        ],
    },
    HelpSection {
        title: "Files",
        keys: &[
            ("c f", "create file"),
            ("c d", "create directory"),
            ("R", "rename"),
            ("d", "delete"),
            ("m", "move"),
            ("!", "shell command"),
        ],
    },
    HelpSection {
        title: "Copying",
        keys: &[
            ("y a", "absolute path"),
            ("y r", "relative path"),
            ("y p", "project path"),
            ("y f", "the file itself"),
        ],
    },
    HelpSection {
        title: "Directories",
        keys: &[
            ("r a", "add one"),
            ("r k", "remove this one"),
            ("r d", "root down"),
            ("r u", "root up"),
            ("r r", "root back"),
        ],
    },
    HelpSection {
        title: "Toggles",
        keys: &[
            ("t h", "dotfiles"),
            ("t w", "fixed width"),
            ("t f", "follow mode"),
            ("t g", "git mode"),
            ("t d", "directories first"),
        ],
    },
    HelpSection {
        title: "Sections",
        keys: &[
            ("t r", "the tree"),
            ("t s", "the symbols"),
            ("t b", "the buffers"),
        ],
    },
    HelpSection {
        title: "Width",
        keys: &[("w", "set width"), ("<", "narrower"), (">", "wider")],
    },
    HelpSection {
        title: "Refreshing",
        keys: &[
            ("g r", "re-read the tree"),
            ("g s", "re-read the symbols"),
            ("s", "re-sort"),
        ],
    },
    HelpSection {
        title: "Leaving",
        keys: &[("q", "hide"), ("Q", "hide and forget"), ("?", "this panel")],
    },
];

/// Builds the treemacs keymap.
pub fn treemacs_keymap() -> Result<Keymap> {
    let mut map = Keymap::new("treefile-mode");
    for (keys, command) in TREEMACS_BINDINGS {
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
    fn the_keymap_builds_without_conflicts() {
        let map = treemacs_keymap().expect("no prefix collides with a command");
        assert_eq!(map.name(), "treefile-mode");
    }

    #[test]
    fn every_binding_is_reachable() {
        let map = treemacs_keymap().unwrap();
        for (keys, command) in TREEMACS_BINDINGS {
            assert_eq!(
                map.lookup(&seq(keys)).command(),
                Some(*command),
                "`{keys}` should run `{command}`"
            );
        }
    }

    #[test]
    fn the_help_describes_every_command_in_the_map_exactly_once() {
        // The failure this is here for is silent: a binding added to the map
        // and not to the help is a key nobody can find, and `?` is the only
        // place anyone would look for it.
        let mut described: Vec<&str> = Vec::new();
        for section in TREEMACS_HELP {
            for (keys, _) in section.keys {
                let command = TREEMACS_BINDINGS
                    .iter()
                    .find(|(bound, _)| bound == keys)
                    .map(|(_, command)| *command)
                    .unwrap_or_else(|| panic!("the help lists `{keys}`, which is not bound"));
                assert!(
                    !described.contains(&command),
                    "`{command}` is in the help twice, the second time as `{keys}`"
                );
                described.push(command);
            }
        }
        for (keys, command) in TREEMACS_BINDINGS {
            assert!(
                described.contains(command),
                "`{command}` is bound to `{keys}` and the help never says so"
            );
        }
    }

    #[test]
    fn the_help_says_the_key_that_reaches_a_command_soonest() {
        // `treefile-goto-first` answers to `M-<`, `C-a` and `g g`. Which of
        // them the panel shows is not arbitrary: the map lists them in the
        // order they are meant to be reached for, and the help should agree
        // rather than teaching whichever was typed last.
        for section in TREEMACS_HELP {
            for (keys, _) in section.keys {
                let command = TREEMACS_BINDINGS
                    .iter()
                    .find(|(bound, _)| bound == keys)
                    .map(|(_, command)| *command)
                    .expect("bound");
                let first = TREEMACS_BINDINGS
                    .iter()
                    .find(|(_, bound)| bound == &command)
                    .map(|(keys, _)| *keys)
                    .expect("bound");
                assert_eq!(
                    *keys, first,
                    "the help teaches `{keys}` for `{command}`, the map reaches for `{first}`"
                );
            }
        }
    }

    #[test]
    fn the_readme_counts_the_keymap_correctly() {
        // It said 47 and 41 while the map held 51 and 44, because the three
        // root bindings were added and the sentence describing them was
        // not. A number in prose that nothing checks is a number that goes
        // stale the first time the thing it counts is touched.
        let readme = include_str!("../../../README.md");
        let bindings = TREEMACS_BINDINGS.len();
        let commands = TREEMACS_HELP
            .iter()
            .map(|section| section.keys.len())
            .sum::<usize>();
        let claim = format!("**{bindings} bindings and {commands} commands**");
        assert!(readme.contains(&claim), "the README never says `{claim}`");
    }

    #[test]
    fn no_help_section_is_empty_or_nameless() {
        for section in TREEMACS_HELP {
            assert!(!section.title.is_empty(), "a section with no heading");
            assert!(
                !section.keys.is_empty(),
                "`{}` is a heading over nothing",
                section.title
            );
        }
    }

    #[test]
    fn the_multi_key_prefixes_are_live() {
        let map = treemacs_keymap().unwrap();
        for prefix in ["o", "c", "y", "t"] {
            assert!(
                map.lookup(&seq(prefix)).is_prefix(),
                "`{prefix}` should be a prefix"
            );
        }
    }

    #[test]
    fn no_key_sequence_is_bound_twice_however_it_is_spelled() {
        // Parsed sequences, not written ones: `C-i` and `TAB` are the same
        // key, so comparing the descriptions would miss a real collision.
        let mut seen: std::collections::BTreeMap<String, &str> = Default::default();
        for (keys, command) in TREEMACS_BINDINGS {
            let canonical = seq(keys).notation();
            if let Some(existing) = seen.insert(canonical.clone(), command) {
                panic!("{canonical} is bound twice: `{existing}` and `{command}`");
            }
        }
    }

    #[test]
    fn every_command_name_is_namespaced() {
        // The map covers the whole side panel: the tree's own commands and
        // the ones for the sections stacked with it. Anything else here would
        // be a global command bound by accident, which is what this catches.
        for (keys, command) in TREEMACS_BINDINGS {
            assert!(
                command.starts_with("treefile-") || command.starts_with("panel-"),
                "`{keys}` runs `{command}`, which is outside the panel's namespaces"
            );
        }
    }

    #[test]
    fn the_core_treemacs_bindings_are_present() {
        let map = treemacs_keymap().unwrap();
        // The handful a treemacs user reaches for without thinking.
        let expected = [
            ("RET", "treefile-visit-node"),
            ("TAB", "treefile-toggle-node"),
            ("n", "treefile-next-line"),
            ("p", "treefile-previous-line"),
            ("u", "treefile-goto-parent"),
            ("H", "treefile-collapse-parent"),
            ("q", "treefile-quit"),
            ("c f", "treefile-create-file"),
            ("c d", "treefile-create-dir"),
            ("t h", "treefile-toggle-show-dotfiles"),
            ("y a", "treefile-copy-absolute-path"),
            ("g r", "treefile-refresh"),
        ];
        for (keys, command) in expected {
            assert_eq!(map.lookup(&seq(keys)).command(), Some(command), "`{keys}`");
        }
    }

    #[test]
    fn where_is_finds_every_binding_of_a_command() {
        let map = treemacs_keymap().unwrap();
        let mut found: Vec<String> = map
            .where_is("treefile-next-line")
            .iter()
            .map(|s| s.notation())
            .collect();
        found.sort();
        assert_eq!(found, vec!["<down>", "n"]);
        assert_eq!(map.where_is("treefile-refresh").len(), 1);
    }
}
