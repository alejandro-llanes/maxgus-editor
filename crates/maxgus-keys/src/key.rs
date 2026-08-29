//! A single key press.

use crate::{KeyError, Result};
use serde::{Deserialize, Serialize};

/// Modifier bits. Emacs' hyper and super are accepted so bindings copied from
/// an Emacs config parse, even though terminals rarely deliver them.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
pub struct Modifiers(u8);

impl Modifiers {
    pub const NONE: Modifiers = Modifiers(0);
    pub const CONTROL: Modifiers = Modifiers(1 << 0);
    pub const META: Modifiers = Modifiers(1 << 1);
    pub const SHIFT: Modifiers = Modifiers(1 << 2);
    pub const SUPER: Modifiers = Modifiers(1 << 3);
    pub const HYPER: Modifiers = Modifiers(1 << 4);

    pub fn contains(self, other: Modifiers) -> bool {
        self.0 & other.0 == other.0
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub fn insert(self, other: Modifiers) -> Modifiers {
        Modifiers(self.0 | other.0)
    }

    pub fn remove(self, other: Modifiers) -> Modifiers {
        Modifiers(self.0 & !other.0)
    }

    /// The prefix Emacs writes for these modifiers, in its canonical order
    /// (`A-C-H-M-S-s-`, reduced here to the ones we support).
    pub fn notation(self) -> String {
        let mut s = String::new();
        if self.contains(Modifiers::CONTROL) {
            s.push_str("C-");
        }
        if self.contains(Modifiers::HYPER) {
            s.push_str("H-");
        }
        if self.contains(Modifiers::META) {
            s.push_str("M-");
        }
        if self.contains(Modifiers::SHIFT) {
            s.push_str("S-");
        }
        if self.contains(Modifiers::SUPER) {
            s.push_str("s-");
        }
        s
    }
}

impl std::ops::BitOr for Modifiers {
    type Output = Modifiers;
    fn bitor(self, rhs: Modifiers) -> Modifiers {
        Modifiers(self.0 | rhs.0)
    }
}

/// The key itself, independent of modifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum KeyCode {
    Char(char),
    Enter,
    Tab,
    Backspace,
    Delete,
    Escape,
    Insert,
    Home,
    End,
    PageUp,
    PageDown,
    Up,
    Down,
    Left,
    Right,
    F(u8),
}

impl KeyCode {
    /// The name Emacs uses for this key in `C-h k` output.
    pub fn notation(self) -> String {
        match self {
            KeyCode::Char(' ') => "SPC".into(),
            KeyCode::Char(c) => c.to_string(),
            KeyCode::Enter => "RET".into(),
            KeyCode::Tab => "TAB".into(),
            KeyCode::Backspace => "DEL".into(),
            KeyCode::Delete => "<delete>".into(),
            KeyCode::Escape => "ESC".into(),
            KeyCode::Insert => "<insert>".into(),
            KeyCode::Home => "<home>".into(),
            KeyCode::End => "<end>".into(),
            KeyCode::PageUp => "<prior>".into(),
            KeyCode::PageDown => "<next>".into(),
            KeyCode::Up => "<up>".into(),
            KeyCode::Down => "<down>".into(),
            KeyCode::Left => "<left>".into(),
            KeyCode::Right => "<right>".into(),
            KeyCode::F(n) => format!("<f{n}>"),
        }
    }

    /// Parses a bare key name: a single character, an Emacs mnemonic such as
    /// `RET`, or an angle-bracketed name such as `<f5>`.
    pub fn parse(name: &str) -> Result<KeyCode> {
        if name.is_empty() {
            return Err(KeyError::Empty);
        }
        let mut chars = name.chars();
        if let (Some(c), None) = (chars.next(), chars.next()) {
            return Ok(KeyCode::Char(c));
        }
        let code = match name {
            "SPC" | "<space>" => KeyCode::Char(' '),
            "RET" | "<return>" | "<enter>" => KeyCode::Enter,
            "TAB" | "<tab>" => KeyCode::Tab,
            "DEL" | "<backspace>" => KeyCode::Backspace,
            "<delete>" | "<deletechar>" => KeyCode::Delete,
            "ESC" | "<escape>" => KeyCode::Escape,
            "<insert>" => KeyCode::Insert,
            "<home>" => KeyCode::Home,
            "<end>" => KeyCode::End,
            "<prior>" | "<pageup>" => KeyCode::PageUp,
            "<next>" | "<pagedown>" => KeyCode::PageDown,
            "<up>" => KeyCode::Up,
            "<down>" => KeyCode::Down,
            "<left>" => KeyCode::Left,
            "<right>" => KeyCode::Right,
            other => {
                // `<fN>` function keys.
                let n = other
                    .strip_prefix("<f")
                    .and_then(|r| r.strip_suffix('>'))
                    .and_then(|n| n.parse::<u8>().ok())
                    .ok_or_else(|| KeyError::UnknownKey(other.to_string()))?;
                KeyCode::F(n)
            }
        };
        Ok(code)
    }
}

/// A key press: a code plus its modifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Key {
    pub code: KeyCode,
    pub modifiers: Modifiers,
}

impl Key {
    pub fn new(code: KeyCode, modifiers: Modifiers) -> Self {
        Self { code, modifiers }.normalized()
    }

    pub fn plain(code: KeyCode) -> Self {
        Self::new(code, Modifiers::NONE)
    }

    pub fn char(c: char) -> Self {
        Self::new(KeyCode::Char(c), Modifiers::NONE)
    }

    pub fn ctrl(c: char) -> Self {
        Self::new(KeyCode::Char(c), Modifiers::CONTROL)
    }

    pub fn meta(c: char) -> Self {
        Self::new(KeyCode::Char(c), Modifiers::META)
    }

    /// Canonicalises the key so equal presses compare equal.
    ///
    /// Shift is folded into the character itself (`S-a` is just `A`), because
    /// terminals report the shifted character rather than a shift bit. Control
    /// characters are folded to lowercase, since `C-A` and `C-a` are one key.
    ///
    /// The ASCII control aliases are folded too: on a terminal `C-i` and TAB
    /// are the same byte, so they must be the same key here. Doing it in the
    /// constructor is what makes a binding written `C-M-i` match the `M-TAB`
    /// a terminal actually delivers — folding only on the way in would leave
    /// the two spellings unable to meet.
    fn normalized(mut self) -> Self {
        if let KeyCode::Char(c) = self.code {
            if self.modifiers.contains(Modifiers::SHIFT) && c.is_alphabetic() {
                let upper = c.to_ascii_uppercase();
                self.code = KeyCode::Char(upper);
                self.modifiers = self.modifiers.remove(Modifiers::SHIFT);
            }
            if self.modifiers.contains(Modifiers::CONTROL) && c.is_ascii_uppercase() {
                self.code = KeyCode::Char(c.to_ascii_lowercase());
            }
        }
        self.fold_control_aliases()
    }

    /// Rewrites the control characters that share a byte with a named key.
    fn fold_control_aliases(mut self) -> Self {
        if !self.modifiers.contains(Modifiers::CONTROL) {
            return self;
        }
        let without_control = self.modifiers.remove(Modifiers::CONTROL);
        match self.code {
            KeyCode::Char('m') => {
                self.code = KeyCode::Enter;
                self.modifiers = without_control;
            }
            KeyCode::Char('i') => {
                self.code = KeyCode::Tab;
                self.modifiers = without_control;
            }
            KeyCode::Char('[') => {
                self.code = KeyCode::Escape;
                self.modifiers = without_control;
            }
            KeyCode::Char('?') => {
                self.code = KeyCode::Backspace;
                self.modifiers = without_control;
            }
            // `C-@` is the other spelling of `C-SPC`; the control bit stays.
            KeyCode::Char('@') => self.code = KeyCode::Char(' '),
            _ => {}
        }
        self
    }

    /// True when this key inserts itself: a plain printable character.
    pub fn is_self_inserting(&self) -> bool {
        matches!(self.code, KeyCode::Char(c) if !c.is_control())
            && self.modifiers.remove(Modifiers::SHIFT).is_empty()
    }

    /// The character this key inserts, if any.
    pub fn as_char(&self) -> Option<char> {
        match self.code {
            KeyCode::Char(c) if self.modifiers.remove(Modifiers::SHIFT).is_empty() => Some(c),
            _ => None,
        }
    }

    /// Parses one key such as `C-M-x` or `<f5>`.
    pub fn parse(text: &str) -> Result<Key> {
        if text.is_empty() {
            return Err(KeyError::Empty);
        }
        let mut modifiers = Modifiers::NONE;
        let mut rest = text;
        // A trailing `-` is the key itself, as in `C--`, so the loop stops as
        // soon as the head is not a modifier name.
        while let Some((head, tail)) = rest.split_once('-') {
            let m = match head {
                "C" => Modifiers::CONTROL,
                "M" | "A" => Modifiers::META,
                "S" => Modifiers::SHIFT,
                "s" => Modifiers::SUPER,
                "H" => Modifiers::HYPER,
                _ => break,
            };
            if tail.is_empty() {
                // `C-` names a modifier with no key to apply it to.
                return Err(KeyError::DanglingModifier(text.to_string()));
            }
            modifiers = modifiers.insert(m);
            rest = tail;
        }
        if rest.is_empty() {
            return Err(KeyError::DanglingModifier(text.to_string()));
        }
        Ok(Key::new(KeyCode::parse(rest)?, modifiers))
    }

    /// Renders the key in Emacs notation.
    pub fn notation(&self) -> String {
        format!("{}{}", self.modifiers.notation(), self.code.notation())
    }
}

impl std::fmt::Display for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.notation())
    }
}

impl std::str::FromStr for Key {
    type Err = KeyError;
    fn from_str(s: &str) -> Result<Key> {
        Key::parse(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_characters_parse_and_render() {
        assert_eq!(Key::parse("a").unwrap(), Key::char('a'));
        assert_eq!(Key::char('a').notation(), "a");
        assert_eq!(Key::parse("SPC").unwrap(), Key::char(' '));
        assert_eq!(Key::char(' ').notation(), "SPC");
    }

    #[test]
    fn modifiers_parse_in_any_order_and_render_canonically() {
        let a = Key::parse("C-M-s").unwrap();
        let b = Key::parse("M-C-s").unwrap();
        assert_eq!(a, b);
        assert_eq!(a.notation(), "C-M-s", "canonical order is C then M");
    }

    #[test]
    fn alt_is_accepted_as_a_spelling_of_meta() {
        assert_eq!(Key::parse("A-x").unwrap(), Key::meta('x'));
    }

    #[test]
    fn named_keys_round_trip() {
        for name in [
            "RET", "TAB", "DEL", "ESC", "<f5>", "<up>", "<prior>", "<delete>",
        ] {
            let k = Key::parse(name).unwrap();
            assert_eq!(k.notation(), name, "`{name}` should round-trip");
        }
    }

    #[test]
    fn alternative_key_spellings_are_accepted() {
        assert_eq!(Key::parse("<return>").unwrap(), Key::plain(KeyCode::Enter));
        assert_eq!(
            Key::parse("<backspace>").unwrap(),
            Key::plain(KeyCode::Backspace)
        );
        assert_eq!(Key::parse("<pageup>").unwrap(), Key::plain(KeyCode::PageUp));
        assert_eq!(Key::parse("<space>").unwrap(), Key::char(' '));
    }

    #[test]
    fn shift_folds_into_the_character() {
        assert_eq!(Key::parse("S-a").unwrap(), Key::char('A'));
        assert_eq!(Key::parse("A").unwrap(), Key::char('A'));
        // Shift on a non-character key is preserved.
        assert_eq!(Key::parse("S-<tab>").unwrap().notation(), "S-TAB");
    }

    #[test]
    fn control_keys_are_case_insensitive() {
        assert_eq!(Key::parse("C-A").unwrap(), Key::ctrl('a'));
        assert_eq!(Key::ctrl('a').notation(), "C-a");
    }

    #[test]
    fn the_control_aliases_fold_to_their_named_keys() {
        // A terminal cannot tell these apart, so neither can a keymap.
        assert_eq!(Key::parse("C-i").unwrap(), Key::plain(KeyCode::Tab));
        assert_eq!(Key::parse("C-m").unwrap(), Key::plain(KeyCode::Enter));
        assert_eq!(Key::parse("C-[").unwrap(), Key::plain(KeyCode::Escape));
        assert_eq!(Key::parse("C-?").unwrap(), Key::plain(KeyCode::Backspace));
        assert_eq!(
            Key::parse("C-@").unwrap(),
            Key::new(KeyCode::Char(' '), Modifiers::CONTROL)
        );
    }

    #[test]
    fn folding_keeps_the_other_modifiers() {
        // `C-M-i` is what a user writes; `M-TAB` is what arrives.
        assert_eq!(Key::parse("C-M-i").unwrap(), Key::parse("M-TAB").unwrap());
        assert_eq!(Key::parse("C-M-i").unwrap().notation(), "M-TAB");
        assert_eq!(Key::parse("C-M-m").unwrap(), Key::parse("M-RET").unwrap());
    }

    #[test]
    fn a_hyphen_can_be_the_key_itself() {
        let k = Key::parse("C--").unwrap();
        assert_eq!(k, Key::ctrl('-'));
        assert_eq!(Key::parse("-").unwrap(), Key::char('-'));
    }

    #[test]
    fn malformed_descriptions_are_rejected() {
        assert_eq!(Key::parse(""), Err(KeyError::Empty));
        assert_eq!(
            Key::parse("C-"),
            Err(KeyError::DanglingModifier("C-".into()))
        );
        assert!(matches!(Key::parse("<nope>"), Err(KeyError::UnknownKey(_))));
        assert!(matches!(Key::parse("<f300>"), Err(KeyError::UnknownKey(_))));
    }

    #[test]
    fn self_inserting_keys_are_unmodified_printables() {
        assert!(Key::char('x').is_self_inserting());
        assert!(Key::char(' ').is_self_inserting());
        assert!(!Key::ctrl('x').is_self_inserting());
        assert!(!Key::meta('x').is_self_inserting());
        assert!(!Key::plain(KeyCode::Enter).is_self_inserting());
        assert_eq!(Key::char('x').as_char(), Some('x'));
        assert_eq!(Key::ctrl('x').as_char(), None);
    }

    #[test]
    fn modifier_set_operations() {
        let m = Modifiers::CONTROL | Modifiers::META;
        assert!(m.contains(Modifiers::CONTROL));
        assert!(!m.contains(Modifiers::SHIFT));
        assert!(
            m.remove(Modifiers::CONTROL)
                .remove(Modifiers::META)
                .is_empty()
        );
        assert_eq!(m.notation(), "C-M-");
    }
}
