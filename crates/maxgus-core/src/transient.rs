//! Transients: the popups magit is driven by.
//!
//! A transient is a menu that appears when a prefix key is pressed and lists
//! what that prefix can do, with the switches that change how it does it. It
//! is the reason magit is usable without being memorised: `P` shows what
//! pushing means here and now, including whether `--force-with-lease` is on,
//! rather than requiring the whole of `git push` to be held in the head.
//!
//! The menus themselves are data — a table of key, label and what to do — so
//! adding one is a table entry rather than a screenful of code, and the whole
//! set can be checked by a test that walks it.

use std::collections::BTreeSet;

/// What pressing a key in a transient does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Runs a command and closes the menu.
    Command(&'static str),
    /// Turns an argument on or off, leaving the menu up.
    Switch(&'static str),
    /// Opens another menu, which `C-g` comes back from.
    Prefix(&'static str),
}

/// One line of a menu.
#[derive(Debug, Clone, Copy)]
pub struct Item {
    pub key: &'static str,
    pub label: &'static str,
    pub action: Action,
}

const fn command(key: &'static str, label: &'static str, name: &'static str) -> Item {
    Item {
        key,
        label,
        action: Action::Command(name),
    }
}

const fn switch(key: &'static str, label: &'static str, flag: &'static str) -> Item {
    Item {
        key,
        label,
        action: Action::Switch(flag),
    }
}

const fn prefix(key: &'static str, label: &'static str, name: &'static str) -> Item {
    Item {
        key,
        label,
        action: Action::Prefix(name),
    }
}

/// A column of the menu, with a heading.
#[derive(Debug, Clone, Copy)]
pub struct Group {
    pub title: &'static str,
    pub items: &'static [Item],
}

/// One menu.
#[derive(Debug, Clone, Copy)]
pub struct Transient {
    pub name: &'static str,
    pub title: &'static str,
    pub groups: &'static [Group],
}

/// The menu named, if there is one.
pub fn find(name: &str) -> Option<&'static Transient> {
    TRANSIENTS.iter().find(|transient| transient.name == name)
}

/// What has become of a key: something to do, more to type, or nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Press {
    Do(Action),
    /// The key so far is the start of a binding, so the menu waits. Magit
    /// spells a switch `-f`, which is two keystrokes.
    More,
    /// Nothing in this menu begins with it.
    Unknown,
}

/// A menu that is up, and what has been switched on in it.
#[derive(Debug, Clone, Default)]
pub struct Active {
    /// The menu showing, with whatever it was opened from beneath it.
    stack: Vec<&'static str>,
    /// Arguments switched on, which the command that runs will be given.
    switches: BTreeSet<&'static str>,
    /// Keys pressed so far that are not yet a whole binding. Magit spells a
    /// switch `-f`, which is two keystrokes, so a key may be the start of
    /// something rather than the whole of it.
    pending: String,
}

impl Active {
    pub fn new(name: &'static str) -> Active {
        Active {
            stack: vec![name],
            switches: BTreeSet::new(),
            pending: String::new(),
        }
    }

    /// The keys pressed so far that have not yet made a binding.
    pub fn pending(&self) -> &str {
        &self.pending
    }

    pub fn current(&self) -> Option<&'static Transient> {
        self.stack.last().and_then(|name| find(name))
    }

    /// Opens `name` on top of what is showing.
    pub fn push(&mut self, name: &'static str) {
        self.stack.push(name);
        self.pending.clear();
    }

    /// Goes back one menu, saying whether anything is left.
    ///
    /// A half-typed key is abandoned first: `C-g` after `-` should take the
    /// `-` back rather than leaving the menu with it still pending.
    pub fn pop(&mut self) -> bool {
        if !self.pending.is_empty() {
            self.pending.clear();
            return true;
        }
        self.stack.pop();
        !self.stack.is_empty()
    }

    pub fn toggle(&mut self, flag: &'static str) {
        if !self.switches.insert(flag) {
            self.switches.remove(flag);
        }
    }

    pub fn is_on(&self, flag: &str) -> bool {
        self.switches.contains(flag)
    }

    /// The switches, as arguments to pass git.
    pub fn arguments(&self) -> Vec<String> {
        self.switches.iter().map(|flag| flag.to_string()).collect()
    }

    /// Presses `key`, which may complete a binding, begin one, or neither.
    pub fn press(&mut self, key: &str) -> Press {
        let Some(transient) = self.current() else {
            return Press::Unknown;
        };
        let candidate = format!("{}{key}", self.pending);
        let items = || transient.groups.iter().flat_map(|group| group.items);

        if let Some(item) = items().find(|item| item.key == candidate) {
            self.pending.clear();
            return Press::Do(item.action);
        }
        if items().any(|item| item.key.starts_with(&candidate)) {
            self.pending = candidate;
            return Press::More;
        }
        self.pending.clear();
        Press::Unknown
    }

    /// What a whole key would do, without pressing it.
    pub fn lookup(&self, key: &str) -> Option<Action> {
        let transient = self.current()?;
        transient
            .groups
            .iter()
            .flat_map(|group| group.items)
            .find(|item| item.key == key)
            .map(|item| item.action)
    }
}

// ---- the menus ----------------------------------------------------------

/// The top-level menu, which every other one hangs off.
static DISPATCH: &[Group] = &[
    Group {
        title: "Inspect",
        items: &[
            prefix("d", "Diff", "diff"),
            prefix("l", "Log", "log"),
            command("y", "Show refs", "magit-show-refs"),
            command("$", "Git output", "magit-process-buffer"),
        ],
    },
    Group {
        title: "Manipulate",
        items: &[
            prefix("c", "Commit", "commit"),
            prefix("b", "Branch", "branch"),
            prefix("m", "Merge", "merge"),
            prefix("r", "Rebase", "rebase"),
            prefix("z", "Stash", "stash"),
            prefix("t", "Tag", "tag"),
            prefix("X", "Reset", "reset"),
            prefix("A", "Cherry-pick", "cherry-pick"),
            prefix("V", "Revert", "revert"),
        ],
    },
    Group {
        title: "Transfer",
        items: &[
            prefix("f", "Fetch", "fetch"),
            prefix("F", "Pull", "pull"),
            prefix("P", "Push", "push"),
            prefix("M", "Remote", "remote"),
        ],
    },
    Group {
        title: "Apply",
        items: &[
            command("s", "Stage", "magit-stage"),
            command("u", "Unstage", "magit-unstage"),
            command("k", "Discard", "magit-discard"),
            command("!", "Run git", "magit-run"),
            command("i", "Ignore", "magit-gitignore"),
        ],
    },
];

static COMMIT: &[Group] = &[
    Group {
        title: "Arguments",
        items: &[
            switch("-a", "Stage all modified files", "--all"),
            switch("-e", "Allow empty commit", "--allow-empty"),
            switch("-s", "Add Signed-off-by", "--signoff"),
            switch("-n", "Skip hooks", "--no-verify"),
        ],
    },
    Group {
        title: "Create",
        items: &[
            command("c", "Commit", "magit-commit"),
            command("a", "Amend", "magit-commit-amend"),
            command("e", "Extend", "magit-commit-extend"),
            command("w", "Reword", "magit-commit-reword"),
            command("f", "Fixup", "magit-commit-fixup"),
        ],
    },
];

static DIFF: &[Group] = &[
    Group {
        title: "Arguments",
        items: &[
            switch("-s", "Show stat only", "--stat"),
            switch("-w", "Ignore whitespace", "--ignore-all-space"),
            switch("-b", "Ignore space change", "--ignore-space-change"),
        ],
    },
    Group {
        title: "Diff",
        items: &[
            command("u", "Unstaged", "magit-diff-unstaged"),
            command("s", "Staged", "magit-diff-staged"),
            command("w", "Worktree", "magit-diff-worktree"),
            command("r", "Range", "magit-diff-range"),
        ],
    },
];

static LOG: &[Group] = &[
    Group {
        title: "Arguments",
        items: &[
            switch("-g", "Show graph", "--graph"),
            switch("-a", "All branches", "--all"),
            switch("-p", "Show patches", "--patch"),
        ],
    },
    Group {
        title: "Log",
        items: &[
            command("l", "Current branch", "magit-log-current"),
            command("h", "HEAD", "magit-log-head"),
            command("o", "Other branch", "magit-log-other"),
            command("f", "This file", "magit-log-file"),
        ],
    },
];

static BRANCH: &[Group] = &[Group {
    title: "Branch",
    items: &[
        command("b", "Checkout", "magit-checkout"),
        command("c", "Create and checkout", "magit-branch-create"),
        command("n", "Create", "magit-branch-new"),
        command("k", "Delete", "magit-branch-delete"),
        command("m", "Rename", "magit-branch-rename"),
    ],
}];

static MERGE: &[Group] = &[
    Group {
        title: "Arguments",
        items: &[
            switch("-s", "Squash", "--squash"),
            switch("-n", "No fast-forward", "--no-ff"),
        ],
    },
    Group {
        title: "Merge",
        items: &[
            command("m", "Merge", "magit-merge"),
            command("a", "Abort", "magit-merge-abort"),
        ],
    },
];

static REBASE: &[Group] = &[
    Group {
        title: "Rebase",
        items: &[
            command("u", "Onto upstream", "magit-rebase-upstream"),
            command("e", "Onto elsewhere", "magit-rebase-elsewhere"),
        ],
    },
    Group {
        title: "In progress",
        items: &[
            command("c", "Continue", "magit-rebase-continue"),
            command("s", "Skip", "magit-rebase-skip"),
            command("a", "Abort", "magit-rebase-abort"),
        ],
    },
];

static RESET: &[Group] = &[Group {
    title: "Reset",
    items: &[
        command("m", "Mixed (keep the tree)", "magit-reset-mixed"),
        command("s", "Soft (keep the index)", "magit-reset-soft"),
        command("h", "Hard (throw it all away)", "magit-reset-hard"),
    ],
}];

static STASH: &[Group] = &[
    Group {
        title: "Arguments",
        items: &[
            switch("-u", "Include untracked", "--include-untracked"),
            switch("-k", "Keep the index", "--keep-index"),
        ],
    },
    Group {
        title: "Stash",
        items: &[
            command("z", "Stash", "magit-stash"),
            command("p", "Pop", "magit-stash-pop"),
            command("a", "Apply", "magit-stash-apply"),
            command("k", "Drop", "magit-stash-drop"),
        ],
    },
];

static TAG: &[Group] = &[Group {
    title: "Tag",
    items: &[
        command("t", "Create", "magit-tag-create"),
        command("k", "Delete", "magit-tag-delete"),
    ],
}];

static PUSH: &[Group] = &[
    Group {
        title: "Arguments",
        items: &[
            switch("-f", "Force with lease", "--force-with-lease"),
            switch("-u", "Set upstream", "--set-upstream"),
            switch("-n", "Dry run", "--dry-run"),
        ],
    },
    Group {
        title: "Push",
        items: &[
            command("p", "To upstream", "magit-push"),
            command("t", "Tags", "magit-push-tags"),
            command("e", "Elsewhere", "magit-push-elsewhere"),
        ],
    },
];

static PULL: &[Group] = &[
    Group {
        title: "Arguments",
        items: &[switch("-r", "Rebase rather than merge", "--rebase")],
    },
    Group {
        title: "Pull",
        items: &[command("p", "From upstream", "magit-pull")],
    },
];

static FETCH: &[Group] = &[
    Group {
        title: "Arguments",
        items: &[
            switch("-p", "Prune", "--prune"),
            switch("-t", "Fetch tags", "--tags"),
        ],
    },
    Group {
        title: "Fetch",
        items: &[
            command("f", "From upstream", "magit-fetch"),
            command("a", "From all remotes", "magit-fetch-all"),
        ],
    },
];

static REMOTE: &[Group] = &[Group {
    title: "Remote",
    items: &[
        command("a", "Add", "magit-remote-add"),
        command("k", "Remove", "magit-remote-remove"),
    ],
}];

static CHERRY_PICK: &[Group] = &[Group {
    title: "Cherry-pick",
    items: &[
        command("A", "Pick the commit here", "magit-cherry-pick"),
        command("a", "Abort", "magit-sequencer-abort"),
        command("c", "Continue", "magit-sequencer-continue"),
    ],
}];

static REVERT: &[Group] = &[Group {
    title: "Revert",
    items: &[
        command("V", "Revert the commit here", "magit-revert"),
        command("a", "Abort", "magit-sequencer-abort"),
    ],
}];

/// Every menu there is.
pub static TRANSIENTS: &[Transient] = &[
    Transient {
        name: "dispatch",
        title: "Git",
        groups: DISPATCH,
    },
    Transient {
        name: "commit",
        title: "Commit",
        groups: COMMIT,
    },
    Transient {
        name: "diff",
        title: "Diff",
        groups: DIFF,
    },
    Transient {
        name: "log",
        title: "Log",
        groups: LOG,
    },
    Transient {
        name: "branch",
        title: "Branch",
        groups: BRANCH,
    },
    Transient {
        name: "merge",
        title: "Merge",
        groups: MERGE,
    },
    Transient {
        name: "rebase",
        title: "Rebase",
        groups: REBASE,
    },
    Transient {
        name: "reset",
        title: "Reset",
        groups: RESET,
    },
    Transient {
        name: "stash",
        title: "Stash",
        groups: STASH,
    },
    Transient {
        name: "tag",
        title: "Tag",
        groups: TAG,
    },
    Transient {
        name: "push",
        title: "Push",
        groups: PUSH,
    },
    Transient {
        name: "pull",
        title: "Pull",
        groups: PULL,
    },
    Transient {
        name: "fetch",
        title: "Fetch",
        groups: FETCH,
    },
    Transient {
        name: "remote",
        title: "Remote",
        groups: REMOTE,
    },
    Transient {
        name: "cherry-pick",
        title: "Cherry-pick",
        groups: CHERRY_PICK,
    },
    Transient {
        name: "revert",
        title: "Revert",
        groups: REVERT,
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_menu_a_key_opens_is_a_menu_that_exists() {
        // A prefix naming a menu that is not there is a key that does
        // nothing, and there is no way to notice by reading the table.
        for transient in TRANSIENTS {
            for item in transient.groups.iter().flat_map(|group| group.items) {
                if let Action::Prefix(name) = item.action {
                    assert!(
                        find(name).is_some(),
                        "`{}` in {} opens `{name}`, which does not exist",
                        item.key,
                        transient.name
                    );
                }
            }
        }
    }

    #[test]
    fn no_menu_binds_the_same_key_twice() {
        for transient in TRANSIENTS {
            let mut seen = std::collections::BTreeSet::new();
            for item in transient.groups.iter().flat_map(|group| group.items) {
                assert!(
                    seen.insert(item.key),
                    "`{}` appears twice in the {} menu",
                    item.key,
                    transient.name
                );
            }
        }
    }

    #[test]
    fn every_menu_can_be_reached_from_the_top_one() {
        // A menu nothing opens is a menu nobody will find.
        let mut reachable = std::collections::BTreeSet::from(["dispatch"]);
        let mut frontier = vec!["dispatch"];
        while let Some(name) = frontier.pop() {
            let Some(transient) = find(name) else {
                continue;
            };
            for item in transient.groups.iter().flat_map(|group| group.items) {
                if let Action::Prefix(next) = item.action
                    && reachable.insert(next)
                {
                    frontier.push(next);
                }
            }
        }
        for transient in TRANSIENTS {
            assert!(
                reachable.contains(transient.name),
                "the {} menu cannot be reached from the top one",
                transient.name
            );
        }
    }

    #[test]
    fn a_key_finds_what_it_does_in_the_menu_showing() {
        let mut active = Active::new("dispatch");
        assert_eq!(active.lookup("P"), Some(Action::Prefix("push")));
        assert_eq!(active.lookup("zzz"), None);

        active.push("push");
        assert_eq!(active.lookup("p"), Some(Action::Command("magit-push")));
        // `P` means push at the top level and nothing inside the push menu.
        assert_eq!(
            active.lookup("P"),
            None,
            "the menu underneath is still being read"
        );
    }

    #[test]
    fn a_switch_stays_on_until_it_is_turned_off() {
        let mut active = Active::new("push");
        assert!(active.arguments().is_empty());
        active.toggle("--force-with-lease");
        assert!(active.is_on("--force-with-lease"));
        assert_eq!(active.arguments(), ["--force-with-lease"]);
        active.toggle("--force-with-lease");
        assert!(
            active.arguments().is_empty(),
            "toggling twice should turn it off"
        );
    }

    #[test]
    fn going_back_returns_to_the_menu_underneath() {
        let mut active = Active::new("dispatch");
        active.push("push");
        assert_eq!(active.current().map(|t| t.name), Some("push"));
        assert!(active.pop(), "the top menu is still there");
        assert_eq!(active.current().map(|t| t.name), Some("dispatch"));
        assert!(!active.pop(), "nothing is left");
    }

    #[test]
    fn switches_survive_moving_between_menus() {
        // They belong to the invocation, not to the menu: turning on
        // `--force-with-lease` and then stepping back and in again should not
        // quietly forget it.
        let mut active = Active::new("dispatch");
        active.push("push");
        active.toggle("--force-with-lease");
        active.pop();
        active.push("push");
        assert!(active.is_on("--force-with-lease"));
    }
}

#[cfg(test)]
mod press_tests {
    use super::*;

    #[test]
    fn a_switch_spelled_with_two_keys_waits_for_the_second() {
        // Magit spells a switch `-f`. Acting on the `-` alone would toggle
        // whatever happened to be first.
        let mut active = Active::new("push");
        assert_eq!(active.press("-"), Press::More);
        assert_eq!(active.pending(), "-");
        assert_eq!(
            active.press("f"),
            Press::Do(Action::Switch("--force-with-lease"))
        );
        assert_eq!(active.pending(), "", "the pending key was not cleared");
    }

    #[test]
    fn a_half_typed_key_that_goes_nowhere_is_dropped() {
        let mut active = Active::new("push");
        active.press("-");
        assert_eq!(
            active.press("z"),
            Press::Unknown,
            "`-z` is not a switch here"
        );
        assert_eq!(active.pending(), "", "the `-` was left pending");
        // And the menu still works afterwards.
        assert_eq!(active.press("p"), Press::Do(Action::Command("magit-push")));
    }

    #[test]
    fn going_back_abandons_a_half_typed_key_before_the_menu() {
        // `C-g` after `-` should take the `-` back, not the whole menu.
        let mut active = Active::new("dispatch");
        active.push("push");
        active.press("-");
        assert!(active.pop(), "the menu should still be up");
        assert_eq!(active.pending(), "");
        assert_eq!(active.current().map(|t| t.name), Some("push"));
        assert!(active.pop());
        assert_eq!(active.current().map(|t| t.name), Some("dispatch"));
    }
}
