//! Registers (`C-x r`).
//!
//! Emacs registers are a single namespace keyed by one character, holding
//! heterogeneous values: text, a buffer position, a rectangle, or a number.

use crate::{Result, TextError, position::Position};
use std::collections::BTreeMap;

/// A value stored under a register key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Register {
    /// `copy-to-register` / `insert-register`.
    Text(String),
    /// `point-to-register` / `jump-to-register`.
    Position { buffer: String, position: Position, offset: usize },
    /// `copy-rectangle-to-register`, stored as one string per row.
    Rectangle(Vec<String>),
    /// `number-to-register`, incremented by `increment-register`.
    Number(i64),
}

/// The register table.
#[derive(Debug, Clone, Default)]
pub struct Registers {
    slots: BTreeMap<char, Register>,
}

impl Registers {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&mut self, key: char, value: Register) {
        self.slots.insert(key, value);
    }

    pub fn get(&self, key: char) -> Option<&Register> {
        self.slots.get(&key)
    }

    pub fn remove(&mut self, key: char) -> Option<Register> {
        self.slots.remove(&key)
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    pub fn len(&self) -> usize {
        self.slots.len()
    }

    /// Registers in key order, for the `C-x r` completion listing.
    pub fn iter(&self) -> impl Iterator<Item = (char, &Register)> {
        self.slots.iter().map(|(k, v)| (*k, v))
    }

    /// The text `insert-register` would insert, joining rectangle rows with
    /// newlines and rendering numbers in decimal.
    pub fn text_of(&self, key: char) -> Result<String> {
        match self.slots.get(&key) {
            Some(Register::Text(t)) => Ok(t.clone()),
            Some(Register::Rectangle(rows)) => Ok(rows.join("\n")),
            Some(Register::Number(n)) => Ok(n.to_string()),
            Some(Register::Position { .. }) | None => Err(TextError::EmptyRegister(key)),
        }
    }

    /// `increment-register`: adds `by` to a number register, treating an unset
    /// register as zero.
    pub fn increment(&mut self, key: char, by: i64) -> Result<i64> {
        let next = match self.slots.get(&key) {
            Some(Register::Number(n)) => n + by,
            None => by,
            Some(_) => return Err(TextError::EmptyRegister(key)),
        };
        self.slots.insert(key, Register::Number(next));
        Ok(next)
    }

    /// A one-line summary for the register listing.
    pub fn describe(&self, key: char) -> Option<String> {
        let value = self.slots.get(&key)?;
        Some(match value {
            Register::Text(t) => {
                let preview: String = t.chars().take(60).collect();
                format!("{key}: text \"{}\"", preview.replace('\n', "\\n"))
            }
            Register::Position { buffer, position, .. } => {
                format!("{key}: position {position} in {buffer}")
            }
            Register::Rectangle(rows) => format!("{key}: rectangle, {} rows", rows.len()),
            Register::Number(n) => format!("{key}: number {n}"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_registers_round_trip() {
        let mut r = Registers::new();
        r.set('a', Register::Text("hello".into()));
        assert_eq!(r.text_of('a').unwrap(), "hello");
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn reading_an_unset_register_errors() {
        let r = Registers::new();
        assert!(matches!(r.text_of('z'), Err(TextError::EmptyRegister('z'))));
    }

    #[test]
    fn position_registers_are_not_insertable_as_text() {
        let mut r = Registers::new();
        r.set(
            'p',
            Register::Position { buffer: "main.rs".into(), position: Position::new(2, 4), offset: 30 },
        );
        assert!(r.text_of('p').is_err());
        assert_eq!(r.describe('p').unwrap(), "p: position 3:5 in main.rs");
    }

    #[test]
    fn rectangle_registers_join_rows_with_newlines() {
        let mut r = Registers::new();
        r.set('r', Register::Rectangle(vec!["ab".into(), "cd".into()]));
        assert_eq!(r.text_of('r').unwrap(), "ab\ncd");
    }

    #[test]
    fn increment_starts_from_zero_and_accumulates() {
        let mut r = Registers::new();
        assert_eq!(r.increment('n', 5).unwrap(), 5);
        assert_eq!(r.increment('n', 3).unwrap(), 8);
        assert_eq!(r.text_of('n').unwrap(), "8");
    }

    #[test]
    fn increment_refuses_non_numeric_registers() {
        let mut r = Registers::new();
        r.set('t', Register::Text("x".into()));
        assert!(r.increment('t', 1).is_err());
    }

    #[test]
    fn registers_iterate_in_key_order() {
        let mut r = Registers::new();
        r.set('c', Register::Number(1));
        r.set('a', Register::Number(2));
        r.set('b', Register::Number(3));
        let keys: Vec<char> = r.iter().map(|(k, _)| k).collect();
        assert_eq!(keys, vec!['a', 'b', 'c']);
    }

    #[test]
    fn removing_a_register_clears_it() {
        let mut r = Registers::new();
        r.set('a', Register::Number(1));
        assert!(r.remove('a').is_some());
        assert!(r.is_empty());
        assert!(r.describe('a').is_none());
    }

    #[test]
    fn text_preview_escapes_newlines_and_truncates() {
        let mut r = Registers::new();
        r.set('a', Register::Text("line\nnext".into()));
        assert_eq!(r.describe('a').unwrap(), r#"a: text "line\nnext""#);
    }
}
