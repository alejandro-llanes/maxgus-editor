//! What can follow a half-typed key sequence.
//!
//! `which-key` for Emacs: pause in the middle of `C-x` and a panel says what
//! the next key could be. Doom leans on it heavily — its whole leader scheme
//! is discoverable through it rather than memorised — and a leader scheme
//! without one is a scheme nobody can learn.
//!
//! This is the reading half, and it is a pure function of the keymaps that
//! are live: given the notation of the prefix already typed, it says what
//! continues it. Drawing it is [`crate::render`]'s job, and deciding when it
//! has waited long enough is the front end's.
//!
//! A [`Menu`] is the same panel asked for outright rather than by pausing.
//! `?` in the file tree opens one, the way treemacs' helpful hydra does, and
//! it is here rather than beside the tree because the panel a hesitation
//! opens and the panel a question mark opens should be the same panel — one
//! that has been read once is read the second time without being learnt
//! again.

use crate::editor::Editor;

/// One thing that can follow the prefix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Continuation {
    /// The single key that continues the sequence, in notation: `b`, `C-f`.
    pub key: String,
    /// The command it runs, or the name of the group it opens.
    pub label: String,
    /// True when this key opens another map rather than running something.
    pub group: bool,
}

/// A whole keymap laid out as a panel, in named columns.
///
/// treemacs' `?` shows every binding at once under headings — Navigation,
/// Nodes, Files — rather than the one level a half-typed prefix would show,
/// and it leaves the keys live underneath so the tree can be walked while
/// the map is being read. Both of those are the point of it: a panel you
/// have to dismiss before you can act on what it told you is a panel that
/// has to be opened twice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Menu {
    /// What the panel says it is, along the top border.
    pub title: String,
    pub sections: Vec<Section>,
}

/// One heading, and the keys under it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub title: String,
    /// The key, and the short phrase saying what it does.
    pub entries: Vec<(String, String)>,
}

impl Section {
    /// How many rows it occupies: the heading, and one per key.
    pub fn height(&self) -> usize {
        self.entries.len() + 1
    }

    /// The width the widest of its rows wants, given the gap between a key
    /// and what it does.
    pub fn width(&self, gap: usize) -> usize {
        let keys = self.entries.iter().map(|(key, _)| key.chars().count());
        let widest_key = keys.max().unwrap_or(0);
        let rows = self
            .entries
            .iter()
            .map(|(_, what)| widest_key + gap + what.chars().count())
            .max()
            .unwrap_or(0);
        rows.max(self.title.chars().count())
    }
}

impl Menu {
    /// The file tree's keymap, as treemacs' helpful hydra shows it.
    pub fn tree() -> Menu {
        Menu {
            title: "File tree".to_string(),
            sections: maxgus_tree::TREEMACS_HELP
                .iter()
                .map(|section| Section {
                    title: section.title.to_string(),
                    entries: section
                        .keys
                        .iter()
                        .map(|(key, what)| ((*key).to_string(), (*what).to_string()))
                        .collect(),
                })
                .collect(),
        }
    }
}

/// The names the leader's groups go by, so a panel can say `+file` rather
/// than `+prefix` seven times.
///
/// Doom's own names, because these are Doom's own maps. A prefix that is not
/// here is still shown; it is just shown as `+prefix`.
pub const GROUP_NAMES: &[(&str, &str)] = &[
    ("C-c c", "+code"),
    ("C-c f", "+file"),
    ("C-c s", "+search"),
    ("C-c o", "+open"),
    ("C-c t", "+toggle"),
    ("C-c v", "+versioning"),
    ("C-c m", "+multiple-cursors"),
    ("C-c i", "+insert"),
    ("C-c q", "+quit"),
    ("C-c w", "+window"),
    ("C-c &", "+snippets"),
    ("C-x 4", "+other-window"),
    ("C-x r", "+register"),
    ("C-x t", "+treefile"),
    ("C-x n", "+narrow"),
    ("C-x x", "+buffer"),
    ("C-h", "+help"),
    ("C-c", "+leader"),
    ("C-x", "+extend"),
];

/// The name a prefix goes by, or `+prefix` for one nobody has named.
fn group_name(sequence: &str) -> String {
    GROUP_NAMES
        .iter()
        .find(|(prefix, _)| *prefix == sequence)
        .map(|(_, name)| (*name).to_string())
        .unwrap_or_else(|| "+prefix".to_string())
}

/// Everything that can follow `prefix`, in the order a panel should show it.
///
/// `prefix` is notation — `"C-x"`, `"C-c f"` — which is what the front ends
/// already carry around for the echo area. An empty result means the prefix
/// leads nowhere, which should not happen for a live prefix and is not worth
/// a panel if it does.
pub fn continuations(editor: &Editor, prefix: &str) -> Vec<Continuation> {
    let lead = format!("{prefix} ");
    // Keyed by the next key, so `C-x 4 f` and `C-x 4 d` collapse into one
    // `4` that opens a group rather than two rows nobody can act on.
    let mut found: Vec<(String, Option<String>)> = Vec::new();
    for (sequence, command) in editor.keymaps.bindings() {
        let notation = sequence.notation();
        let Some(rest) = notation.strip_prefix(&lead) else {
            continue;
        };
        let mut parts = rest.splitn(2, ' ');
        let Some(key) = parts.next().filter(|key| !key.is_empty()) else {
            continue;
        };
        let leaf = match parts.next() {
            // More keys after this one: a group.
            Some(_) => None,
            None => Some(command),
        };
        match found.iter_mut().find(|(seen, _)| seen == key) {
            // A key that is both a command and a prefix cannot be bound, so
            // whichever was seen first is the whole truth about it — except
            // that a group beats a leaf, which is the direction a merge can
            // go wrong in.
            Some((_, existing)) => {
                if leaf.is_none() {
                    *existing = None;
                }
            }
            None => found.push((key.to_string(), leaf)),
        }
    }
    found.sort_by_key(|(key, _)| order(key));
    found
        .into_iter()
        .map(|(key, leaf)| match leaf {
            Some(command) => Continuation {
                key,
                label: command,
                group: false,
            },
            None => {
                let label = group_name(&format!("{prefix} {key}"));
                Continuation {
                    key,
                    label,
                    group: true,
                }
            }
        })
        .collect()
}

/// How the keys are sorted: plain keys first and alphabetically, then the
/// modified ones, then the named ones — which is roughly the order a hand
/// reaches for them.
fn order(key: &str) -> (u8, String) {
    let rank = match key {
        _ if key.len() == 1 => 0,
        _ if key.starts_with("C-") => 1,
        _ if key.starts_with("M-") => 2,
        _ => 3,
    };
    // Lower case before upper, so `b` and `B` sit together in that order
    // rather than every capital being herded to one end.
    (rank, format!("{}{key}", key.to_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use maxgus_keys::{Keymap, KeymapSet};

    /// An editor whose global map is only what a test defines.
    fn editor_with(bindings: &[(&str, &str)]) -> Editor {
        let mut map = Keymap::new("global");
        for (keys, command) in bindings {
            map.define_str(keys, *command).unwrap();
        }
        let mut editor = Editor::new(
            maxgus_config::Settings::default(),
            maxgus_faces::defaults::builtin("maxgus-dark").unwrap(),
            maxgus_tui::Rect::new(0, 0, 80, 24),
        );
        editor.keymaps = KeymapSet::new(map);
        editor
    }

    #[test]
    fn a_prefix_lists_the_keys_that_finish_it() {
        let editor = editor_with(&[
            ("C-x C-s", "save-buffer"),
            ("C-x C-f", "find-file"),
            ("C-x b", "switch-to-buffer"),
            ("C-s", "isearch-forward"),
        ]);
        let found = continuations(&editor, "C-x");
        let keys: Vec<&str> = found.iter().map(|c| c.key.as_str()).collect();
        assert_eq!(keys, ["b", "C-f", "C-s"], "got {found:?}");
        assert_eq!(found[0].label, "switch-to-buffer");
        assert!(!found[0].group);
    }

    #[test]
    fn a_key_that_opens_another_map_is_shown_as_a_group() {
        // Otherwise `C-x 4 f` and `C-x 4 d` would be two rows under `C-x`,
        // neither of which is a key anyone can press from there.
        let editor = editor_with(&[
            ("C-x 4 f", "find-file-other-window"),
            ("C-x 4 d", "dired-other-window"),
            ("C-x b", "switch-to-buffer"),
        ]);
        let found = continuations(&editor, "C-x");
        let four = found.iter().find(|c| c.key == "4").expect("the group");
        assert!(four.group, "a prefix was shown as a command");
        assert_eq!(four.label, "+other-window", "the group has a name");
        assert_eq!(found.len(), 2, "the group is one row, not two: {found:?}");
    }

    #[test]
    fn an_unnamed_group_still_says_it_is_one() {
        let editor = editor_with(&[("C-x z z", "repeat")]);
        let found = continuations(&editor, "C-x");
        assert_eq!(found[0].label, "+prefix");
        assert!(found[0].group);
    }

    #[test]
    fn a_prefix_that_leads_nowhere_lists_nothing() {
        let editor = editor_with(&[("C-x b", "switch-to-buffer")]);
        assert!(continuations(&editor, "C-q").is_empty());
        // And a whole binding is not a prefix of itself.
        assert!(continuations(&editor, "C-x b").is_empty());
    }

    #[test]
    fn a_deeper_prefix_lists_what_follows_it_rather_than_the_whole_map() {
        let editor = editor_with(&[
            ("C-c f f", "find-file"),
            ("C-c f d", "dired"),
            ("C-c c k", "lsp-describe-thing-at-point"),
        ]);
        let found = continuations(&editor, "C-c f");
        let keys: Vec<&str> = found.iter().map(|c| c.key.as_str()).collect();
        assert_eq!(keys, ["d", "f"]);
    }

    // The full map is what the names are for; a minimal build has no magit
    // to put under `C-c v`, and naming it there would prove nothing.
    #[cfg(feature = "full")]
    #[test]
    fn the_names_are_for_prefixes_that_are_really_bound() {
        // A group name for a sequence nothing is under would show a panel
        // entry that goes nowhere, so the table is checked against the map
        // the editor really starts with.
        let map = crate::keymap::global_keymap().expect("the global map");
        let bindings = map.bindings();
        for (prefix, name) in GROUP_NAMES {
            let lead = format!("{prefix} ");
            assert!(
                bindings
                    .iter()
                    .any(|(sequence, _)| sequence.notation().starts_with(&lead)),
                "`{prefix}` is named `{name}` and nothing is bound under it"
            );
        }
    }
}
