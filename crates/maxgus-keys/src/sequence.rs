//! Key sequences: a whole `C-x C-f`, not just one press.

use crate::{Key, Result, key::{KeyCode, Modifiers}};
use serde::{Deserialize, Serialize};

/// An ordered run of key presses.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct KeySequence(Vec<Key>);

impl KeySequence {
    pub fn new(keys: Vec<Key>) -> Self {
        Self(keys)
    }

    pub fn empty() -> Self {
        Self(Vec::new())
    }

    /// Parses a whitespace-separated description such as `C-x C-f`.
    pub fn parse(text: &str) -> Result<KeySequence> {
        text.split_whitespace().map(Key::parse).collect::<Result<Vec<_>>>().map(KeySequence)
    }

    pub fn keys(&self) -> &[Key] {
        &self.0
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn push(&mut self, key: Key) {
        self.0.push(key);
    }

    pub fn pop(&mut self) -> Option<Key> {
        self.0.pop()
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }

    pub fn first(&self) -> Option<Key> {
        self.0.first().copied()
    }

    pub fn last(&self) -> Option<Key> {
        self.0.last().copied()
    }

    /// Rewrites `ESC x` into `M-x`, which is how Emacs treats the escape
    /// prefix. Terminals that cannot send a Meta bit send ESC instead.
    pub fn canonicalize_escape_prefix(&self) -> KeySequence {
        let mut out = Vec::with_capacity(self.0.len());
        let mut pending_escape = false;
        for key in &self.0 {
            if pending_escape {
                pending_escape = false;
                out.push(Key::new(key.code, key.modifiers.insert(Modifiers::META)));
                continue;
            }
            if key.code == KeyCode::Escape && key.modifiers.is_empty() {
                pending_escape = true;
                continue;
            }
            out.push(*key);
        }
        if pending_escape {
            // A trailing ESC with nothing after it stays a literal ESC.
            out.push(Key::plain(KeyCode::Escape));
        }
        KeySequence(out)
    }

    /// Emacs notation for the whole sequence.
    pub fn notation(&self) -> String {
        self.0.iter().map(Key::notation).collect::<Vec<_>>().join(" ")
    }
}

impl std::fmt::Display for KeySequence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.notation())
    }
}

impl std::str::FromStr for KeySequence {
    type Err = crate::KeyError;
    fn from_str(s: &str) -> Result<KeySequence> {
        KeySequence::parse(s)
    }
}

impl From<Key> for KeySequence {
    fn from(key: Key) -> Self {
        KeySequence(vec![key])
    }
}

impl FromIterator<Key> for KeySequence {
    fn from_iter<T: IntoIterator<Item = Key>>(iter: T) -> Self {
        KeySequence(iter.into_iter().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multi_key_sequences_round_trip() {
        let s = KeySequence::parse("C-x C-f").unwrap();
        assert_eq!(s.len(), 2);
        assert_eq!(s.keys()[0], Key::ctrl('x'));
        assert_eq!(s.keys()[1], Key::ctrl('f'));
        assert_eq!(s.notation(), "C-x C-f");
    }

    #[test]
    fn extra_whitespace_is_tolerated() {
        assert_eq!(KeySequence::parse("  C-x    C-s  ").unwrap().len(), 2);
        assert!(KeySequence::parse("   ").unwrap().is_empty());
    }

    #[test]
    fn escape_prefix_becomes_meta() {
        let s = KeySequence::parse("ESC x").unwrap().canonicalize_escape_prefix();
        assert_eq!(s.notation(), "M-x");
        let s = KeySequence::parse("ESC C-f").unwrap().canonicalize_escape_prefix();
        assert_eq!(s.notation(), "C-M-f");
    }

    #[test]
    fn a_trailing_escape_stays_literal() {
        let s = KeySequence::parse("C-x ESC").unwrap().canonicalize_escape_prefix();
        assert_eq!(s.notation(), "C-x ESC");
    }

    #[test]
    fn escape_canonicalisation_leaves_other_keys_untouched() {
        let s = KeySequence::parse("C-x C-f").unwrap().canonicalize_escape_prefix();
        assert_eq!(s.notation(), "C-x C-f");
    }

    #[test]
    fn push_and_pop_build_sequences_incrementally() {
        let mut s = KeySequence::empty();
        s.push(Key::ctrl('x'));
        s.push(Key::char('b'));
        assert_eq!(s.notation(), "C-x b");
        assert_eq!(s.last(), Some(Key::char('b')));
        assert_eq!(s.pop(), Some(Key::char('b')));
        assert_eq!(s.first(), Some(Key::ctrl('x')));
        s.clear();
        assert!(s.is_empty());
    }

    #[test]
    fn a_single_key_converts_into_a_sequence() {
        let s: KeySequence = Key::ctrl('g').into();
        assert_eq!(s.notation(), "C-g");
    }

    #[test]
    fn parse_errors_propagate_from_the_offending_key() {
        assert!(KeySequence::parse("C-x <bogus>").is_err());
    }
}
