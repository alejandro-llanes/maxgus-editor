//! Keymaps.
//!
//! A [`Keymap`] is a trie keyed by [`Key`]. Looking up a sequence yields a
//! command, a live prefix (`C-x` while waiting for the next key), or nothing.
//! [`KeymapSet`] stacks minor-mode, major-mode and global maps in Emacs'
//! precedence order.

use crate::{Key, KeyError, KeySequence, Result};
use std::collections::BTreeMap;

/// What a lookup found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lookup {
    /// A complete binding. The payload is the command name.
    Command(String),
    /// A live prefix: more keys are needed.
    Prefix,
    /// Nothing is bound, and no longer sequence could be.
    Undefined,
}

impl Lookup {
    pub fn command(&self) -> Option<&str> {
        match self {
            Lookup::Command(name) => Some(name),
            _ => None,
        }
    }

    pub fn is_prefix(&self) -> bool {
        matches!(self, Lookup::Prefix)
    }

    pub fn is_undefined(&self) -> bool {
        matches!(self, Lookup::Undefined)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Entry {
    Command(String),
    Prefix(Keymap),
}

/// A trie of key sequences to command names.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Keymap {
    name: String,
    entries: BTreeMap<Key, Entry>,
    /// Command run for any otherwise-unbound key, as `self-insert-command` is
    /// in the global map.
    default_binding: Option<String>,
    /// True when the default binding catches *every* unbound key rather than
    /// only the ones that insert a character. A terminal needs this: `C-a`
    /// and `<up>` belong to the program running inside it, not to the editor.
    default_catches_all: bool,
    /// Keys the catch-all does not take, which therefore fall through to the
    /// maps below. Without them a terminal would swallow `C-x` and there
    /// would be no way out of it.
    default_exceptions: std::collections::BTreeSet<Key>,
}

impl Keymap {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            entries: BTreeMap::new(),
            default_binding: None,
            default_catches_all: false,
            default_exceptions: std::collections::BTreeSet::new(),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// Sets the fallback command for keys with no explicit binding.
    /// Makes the default binding catch every unbound key except `except`.
    ///
    /// For a terminal, where an unbound key is not an error but a keystroke.
    /// The exceptions are the way back out: without them the editor's own
    /// prefix would be swallowed along with everything else.
    pub fn set_default_catches_all(&mut self, except: &[Key]) {
        self.default_catches_all = true;
        self.default_exceptions = except.iter().copied().collect();
    }

    pub fn set_default_binding(&mut self, command: Option<String>) {
        self.default_binding = command;
    }

    pub fn default_binding(&self) -> Option<&str> {
        self.default_binding.as_deref()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty() && self.default_binding.is_none()
    }

    /// `define-key`: binds `sequence` to `command`, creating prefix maps as
    /// needed. Rebinding an existing sequence replaces it; binding through a
    /// key that already holds a command is an error, since that would silently
    /// shadow the command.
    pub fn define(&mut self, sequence: &KeySequence, command: impl Into<String>) -> Result<()> {
        let command = command.into();
        let keys = sequence.keys();
        if keys.is_empty() {
            return Err(KeyError::Empty);
        }
        self.define_keys(keys, command, sequence)
    }

    fn define_keys(&mut self, keys: &[Key], command: String, full: &KeySequence) -> Result<()> {
        let (head, rest) = keys.split_first().expect("caller checked non-empty");
        if rest.is_empty() {
            self.entries.insert(*head, Entry::Command(command));
            return Ok(());
        }
        let child_name = format!("{} {}", self.name, head.notation());
        match self
            .entries
            .entry(*head)
            .or_insert_with(|| Entry::Prefix(Keymap::new(child_name)))
        {
            Entry::Prefix(map) => map.define_keys(rest, command, full),
            Entry::Command(existing) => {
                Err(KeyError::PrefixConflict(full.notation(), existing.clone()))
            }
        }
    }

    /// Convenience wrapper that parses the key description first.
    pub fn define_str(&mut self, keys: &str, command: impl Into<String>) -> Result<()> {
        self.define(&KeySequence::parse(keys)?, command)
    }

    /// `global-unset-key`: removes a binding, returning what was there.
    pub fn undefine(&mut self, sequence: &KeySequence) -> Option<String> {
        let (head, rest) = sequence.keys().split_first()?;
        if rest.is_empty() {
            return match self.entries.remove(head) {
                Some(Entry::Command(c)) => Some(c),
                Some(Entry::Prefix(map)) => {
                    // Put the prefix back: it holds bindings we were not asked
                    // to remove.
                    self.entries.insert(*head, Entry::Prefix(map));
                    None
                }
                None => None,
            };
        }
        match self.entries.get_mut(head)? {
            Entry::Prefix(map) => map.undefine(&KeySequence::new(rest.to_vec())),
            Entry::Command(_) => None,
        }
    }

    /// Looks up a whole sequence.
    pub fn lookup(&self, sequence: &KeySequence) -> Lookup {
        self.lookup_keys(sequence.keys())
    }

    fn lookup_keys(&self, keys: &[Key]) -> Lookup {
        let Some((head, rest)) = keys.split_first() else {
            // An empty sequence is the map itself, i.e. a prefix.
            return Lookup::Prefix;
        };
        match self.entries.get(head) {
            Some(Entry::Command(c)) if rest.is_empty() => Lookup::Command(c.clone()),
            // Extra keys after a complete binding cannot match anything.
            Some(Entry::Command(_)) => Lookup::Undefined,
            Some(Entry::Prefix(map)) => map.lookup_keys(rest),
            // The default binding stands in for `self-insert-command`, so it
            // only catches keys that actually insert a character: a function
            // or navigation key with no binding is undefined, not self-insert.
            None => {
                let catches = rest.is_empty()
                    && (head.is_self_inserting()
                        || (self.default_catches_all && !self.default_exceptions.contains(head)));
                match (&self.default_binding, catches) {
                    (Some(c), true) => Lookup::Command(c.clone()),
                    _ => Lookup::Undefined,
                }
            }
        }
    }

    /// Every binding in the map, sorted by key sequence — the data behind
    /// `describe-bindings`.
    pub fn bindings(&self) -> Vec<(KeySequence, String)> {
        let mut out = Vec::new();
        self.collect(&mut KeySequence::empty(), &mut out);
        out
    }

    fn collect(&self, prefix: &mut KeySequence, out: &mut Vec<(KeySequence, String)>) {
        for (key, entry) in &self.entries {
            prefix.push(*key);
            match entry {
                Entry::Command(c) => out.push((prefix.clone(), c.clone())),
                Entry::Prefix(map) => map.collect(prefix, out),
            }
            prefix.pop();
        }
    }

    /// Key sequences bound to `command`, for `where-is` and `M-x` hints.
    pub fn where_is(&self, command: &str) -> Vec<KeySequence> {
        self.bindings()
            .into_iter()
            .filter(|(_, c)| c == command)
            .map(|(seq, _)| seq)
            .collect()
    }

    /// Merges `other` into this map, with `other`'s bindings winning. Used to
    /// layer user configuration over the built-in defaults.
    pub fn merge(&mut self, other: &Keymap) {
        for (key, entry) in &other.entries {
            match (self.entries.get_mut(key), entry) {
                (Some(Entry::Prefix(mine)), Entry::Prefix(theirs)) => mine.merge(theirs),
                _ => {
                    self.entries.insert(*key, entry.clone());
                }
            }
        }
        if other.default_binding.is_some() {
            self.default_binding = other.default_binding.clone();
        }
    }
}

/// The stack of active keymaps, consulted in Emacs' precedence order:
/// minor modes first (most recently enabled wins), then the major mode, then
/// the global map.
#[derive(Debug, Clone, Default)]
pub struct KeymapSet {
    pub global: Keymap,
    pub major: Option<Keymap>,
    pub minor: Vec<Keymap>,
}

impl KeymapSet {
    pub fn new(global: Keymap) -> Self {
        Self {
            global,
            major: None,
            minor: Vec::new(),
        }
    }

    pub fn set_major(&mut self, map: Option<Keymap>) {
        self.major = map;
    }

    /// Enables a minor-mode map at the highest precedence.
    pub fn push_minor(&mut self, map: Keymap) {
        self.minor.insert(0, map);
    }

    /// Disables a minor-mode map by name.
    pub fn remove_minor(&mut self, name: &str) -> bool {
        let before = self.minor.len();
        self.minor.retain(|m| m.name() != name);
        self.minor.len() != before
    }

    fn maps(&self) -> impl Iterator<Item = &Keymap> {
        self.minor
            .iter()
            .chain(self.major.iter())
            .chain(std::iter::once(&self.global))
    }

    /// Looks the sequence up across the whole stack. A command found in a
    /// higher-precedence map wins outright; a prefix is only reported when no
    /// map binds the sequence to a command.
    pub fn lookup(&self, sequence: &KeySequence) -> Lookup {
        let mut prefix_seen = false;
        for map in self.maps() {
            match map.lookup(sequence) {
                // A map higher up the stack is still gathering a longer
                // sequence, so a command down here is shadowed rather than
                // run. `o` begins `o o` in the tree's map while the global
                // map answers any printable key with `self-insert-command`;
                // without this the fallback wins and the key types itself,
                // taking most of the treemacs keymap with it.
                Lookup::Command(_) if prefix_seen => {}
                Lookup::Command(c) => return Lookup::Command(c),
                Lookup::Prefix => prefix_seen = true,
                Lookup::Undefined => {}
            }
        }
        if prefix_seen {
            Lookup::Prefix
        } else {
            Lookup::Undefined
        }
    }

    /// All bindings across the stack, higher-precedence ones shadowing lower.
    pub fn bindings(&self) -> Vec<(KeySequence, String)> {
        let mut seen = BTreeMap::new();
        for map in self.maps() {
            for (seq, cmd) in map.bindings() {
                seen.entry(seq).or_insert(cmd);
            }
        }
        seen.into_iter().collect()
    }

    pub fn where_is(&self, command: &str) -> Vec<KeySequence> {
        self.bindings()
            .into_iter()
            .filter(|(_, c)| c == command)
            .map(|(seq, _)| seq)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seq(s: &str) -> KeySequence {
        KeySequence::parse(s).unwrap()
    }

    fn sample_map() -> Keymap {
        let mut m = Keymap::new("global");
        m.define_str("C-x C-f", "find-file").unwrap();
        m.define_str("C-x C-s", "save-buffer").unwrap();
        m.define_str("C-a", "move-beginning-of-line").unwrap();
        m
    }

    #[test]
    fn complete_sequences_resolve_to_commands() {
        let m = sample_map();
        assert_eq!(
            m.lookup(&seq("C-x C-f")),
            Lookup::Command("find-file".into())
        );
        assert_eq!(
            m.lookup(&seq("C-a")),
            Lookup::Command("move-beginning-of-line".into())
        );
    }

    #[test]
    fn partial_sequences_report_a_live_prefix() {
        let m = sample_map();
        assert_eq!(m.lookup(&seq("C-x")), Lookup::Prefix);
        assert!(m.lookup(&seq("C-x")).is_prefix());
    }

    #[test]
    fn unbound_sequences_are_undefined() {
        let m = sample_map();
        assert_eq!(m.lookup(&seq("C-q")), Lookup::Undefined);
        assert_eq!(m.lookup(&seq("C-x C-q")), Lookup::Undefined);
    }

    #[test]
    fn keys_after_a_complete_binding_cannot_match() {
        let m = sample_map();
        assert_eq!(m.lookup(&seq("C-a C-a")), Lookup::Undefined);
    }

    #[test]
    fn rebinding_replaces_the_previous_command() {
        let mut m = sample_map();
        m.define_str("C-a", "back-to-indentation").unwrap();
        assert_eq!(m.lookup(&seq("C-a")).command(), Some("back-to-indentation"));
    }

    #[test]
    fn binding_through_a_command_key_is_rejected() {
        let mut m = sample_map();
        let err = m.define_str("C-a C-b", "nope").unwrap_err();
        assert_eq!(
            err,
            KeyError::PrefixConflict("C-a C-b".into(), "move-beginning-of-line".into())
        );
    }

    #[test]
    fn an_empty_sequence_cannot_be_bound() {
        let mut m = Keymap::new("m");
        assert_eq!(m.define(&KeySequence::empty(), "x"), Err(KeyError::Empty));
    }

    #[test]
    fn undefine_removes_only_the_named_binding() {
        let mut m = sample_map();
        assert_eq!(m.undefine(&seq("C-x C-f")).as_deref(), Some("find-file"));
        assert_eq!(m.lookup(&seq("C-x C-f")), Lookup::Undefined);
        assert_eq!(m.lookup(&seq("C-x C-s")).command(), Some("save-buffer"));
    }

    #[test]
    fn undefine_refuses_to_delete_a_whole_prefix_map() {
        let mut m = sample_map();
        assert_eq!(m.undefine(&seq("C-x")), None);
        assert_eq!(m.lookup(&seq("C-x C-f")).command(), Some("find-file"));
    }

    #[test]
    fn undefine_on_a_missing_binding_is_harmless() {
        let mut m = sample_map();
        assert_eq!(m.undefine(&seq("C-z")), None);
        assert_eq!(m.undefine(&seq("C-z C-z")), None);
    }

    #[test]
    fn the_default_binding_catches_unbound_single_keys() {
        let mut m = Keymap::new("global");
        m.set_default_binding(Some("self-insert-command".into()));
        m.define_str("C-a", "move-beginning-of-line").unwrap();
        assert_eq!(m.lookup(&seq("q")).command(), Some("self-insert-command"));
        assert_eq!(
            m.lookup(&seq("C-a")).command(),
            Some("move-beginning-of-line")
        );
    }

    #[test]
    fn the_default_binding_only_catches_keys_that_insert_something() {
        let mut m = Keymap::new("global");
        m.set_default_binding(Some("self-insert-command".into()));
        assert_eq!(m.lookup(&seq("q")).command(), Some("self-insert-command"));
        assert_eq!(m.lookup(&seq("SPC")).command(), Some("self-insert-command"));
        // A function key inserts nothing, so it stays undefined.
        assert!(m.lookup(&seq("<f9>")).is_undefined());
        assert!(m.lookup(&seq("<up>")).is_undefined());
        assert!(m.lookup(&seq("C-q")).is_undefined());
        assert!(m.lookup(&seq("RET")).is_undefined());
    }

    #[test]
    fn the_default_binding_does_not_apply_mid_sequence() {
        let mut m = sample_map();
        m.set_default_binding(Some("self-insert-command".into()));
        assert_eq!(m.lookup(&seq("q q")), Lookup::Undefined);
    }

    #[test]
    fn bindings_are_listed_in_key_order() {
        let m = sample_map();
        let listed: Vec<String> = m.bindings().iter().map(|(s, _)| s.notation()).collect();
        assert_eq!(listed, vec!["C-a", "C-x C-f", "C-x C-s"]);
    }

    #[test]
    fn where_is_finds_every_binding_of_a_command() {
        let mut m = sample_map();
        m.define_str("<home>", "move-beginning-of-line").unwrap();
        let mut found: Vec<String> = m
            .where_is("move-beginning-of-line")
            .iter()
            .map(|s| s.notation())
            .collect();
        found.sort();
        assert_eq!(found, vec!["<home>", "C-a"]);
        assert!(m.where_is("nonexistent-command").is_empty());
    }

    #[test]
    fn merge_layers_user_bindings_over_defaults() {
        let mut base = sample_map();
        let mut user = Keymap::new("user");
        user.define_str("C-x C-f", "my-find-file").unwrap();
        user.define_str("C-x C-b", "ibuffer").unwrap();
        base.merge(&user);
        assert_eq!(base.lookup(&seq("C-x C-f")).command(), Some("my-find-file"));
        assert_eq!(base.lookup(&seq("C-x C-b")).command(), Some("ibuffer"));
        assert_eq!(
            base.lookup(&seq("C-x C-s")).command(),
            Some("save-buffer"),
            "untouched"
        );
    }

    #[test]
    fn minor_modes_take_precedence_over_the_major_mode_and_global_map() {
        let mut set = KeymapSet::new(sample_map());
        let mut major = Keymap::new("rust-mode");
        major.define_str("C-a", "rust-beginning-of-line").unwrap();
        set.set_major(Some(major));
        assert_eq!(
            set.lookup(&seq("C-a")).command(),
            Some("rust-beginning-of-line")
        );

        let mut minor = Keymap::new("flycheck-mode");
        minor.define_str("C-a", "flycheck-first-error").unwrap();
        set.push_minor(minor);
        assert_eq!(
            set.lookup(&seq("C-a")).command(),
            Some("flycheck-first-error")
        );
    }

    #[test]
    fn the_most_recently_enabled_minor_mode_wins() {
        let mut set = KeymapSet::new(sample_map());
        let mut first = Keymap::new("first");
        first.define_str("C-t", "first-command").unwrap();
        let mut second = Keymap::new("second");
        second.define_str("C-t", "second-command").unwrap();
        set.push_minor(first);
        set.push_minor(second);
        assert_eq!(set.lookup(&seq("C-t")).command(), Some("second-command"));
    }

    #[test]
    fn disabling_a_minor_mode_restores_the_shadowed_binding() {
        let mut set = KeymapSet::new(sample_map());
        let mut minor = Keymap::new("temp");
        minor.define_str("C-a", "temp-command").unwrap();
        set.push_minor(minor);
        assert!(set.remove_minor("temp"));
        assert_eq!(
            set.lookup(&seq("C-a")).command(),
            Some("move-beginning-of-line")
        );
        assert!(!set.remove_minor("temp"), "removing twice is a no-op");
    }

    #[test]
    fn a_prefix_in_a_higher_map_shadows_a_command_in_a_lower_one() {
        // This used to be the other way round, and it cost the file tree most
        // of its keymap: `o` begins `o o` there, while the global map answers
        // any printable key with `self-insert-command`. With the command
        // winning, `o` typed itself — into a read-only buffer, so it read as
        // the editor refusing to edit rather than as a binding gone missing.
        //
        // Emacs resolves it this way too: the first map with anything to say
        // about a key decides, and a prefix is something to say.
        let mut set = KeymapSet::new(sample_map());
        let mut minor = Keymap::new("minor");
        minor.define_str("C-a C-b", "deep").unwrap();
        set.push_minor(minor);

        assert!(
            set.lookup(&seq("C-a")).is_prefix(),
            "`C-a` should be waiting for more"
        );
        assert_eq!(set.lookup(&seq("C-a C-b")).command(), Some("deep"));

        // And the global binding comes back when the map holding the prefix
        // goes away.
        assert!(set.remove_minor("minor"));
        assert_eq!(
            set.lookup(&seq("C-a")).command(),
            Some("move-beginning-of-line")
        );
    }

    #[test]
    fn a_lower_maps_command_still_wins_when_nothing_above_claims_the_key() {
        // The shadowing above must not swallow keys no higher map mentions.
        let mut set = KeymapSet::new(sample_map());
        let mut minor = Keymap::new("minor");
        minor.define_str("C-x C-z", "deep").unwrap();
        set.push_minor(minor);
        assert_eq!(
            set.lookup(&seq("C-a")).command(),
            Some("move-beginning-of-line")
        );
    }

    #[test]
    fn a_prefix_anywhere_in_the_stack_keeps_the_sequence_alive() {
        let mut set = KeymapSet::new(Keymap::new("global"));
        let mut minor = Keymap::new("minor");
        minor.define_str("C-c C-c", "compile").unwrap();
        set.push_minor(minor);
        assert!(set.lookup(&seq("C-c")).is_prefix());
        assert!(set.lookup(&seq("C-q")).is_undefined());
    }

    #[test]
    fn stacked_bindings_shadow_by_precedence_when_listed() {
        let mut set = KeymapSet::new(sample_map());
        let mut major = Keymap::new("major");
        major.define_str("C-a", "major-command").unwrap();
        set.set_major(Some(major));
        let listed = set.bindings();
        let found = listed.iter().find(|(s, _)| s.notation() == "C-a").unwrap();
        assert_eq!(found.1, "major-command");
        assert_eq!(set.where_is("major-command").len(), 1);
    }
}
