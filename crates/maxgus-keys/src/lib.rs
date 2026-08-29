//! Emacs key notation, key sequences and keymaps.
//!
//! Keys are written the way Emacs writes them — `C-x C-f`, `M-x`, `C-M-s`,
//! `<f5>`, `RET` — and parsed into [`Key`] values. Keymaps are tries over key
//! sequences, so a lookup returns either a bound command, an indication that
//! the sequence is a live prefix, or nothing.

pub mod key;
pub mod keymap;
pub mod sequence;
pub mod terminal;

pub use key::{Key, KeyCode, Modifiers};
pub use keymap::{Keymap, KeymapSet, Lookup};
pub use sequence::KeySequence;
pub use terminal::key_from_event;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum KeyError {
    #[error("empty key description")]
    Empty,
    #[error("unknown key name `{0}`")]
    UnknownKey(String),
    #[error("dangling modifier in `{0}`")]
    DanglingModifier(String),
    #[error("`{0}` binds a prefix that is already bound to command `{1}`")]
    PrefixConflict(String, String),
}

pub type Result<T> = std::result::Result<T, KeyError>;
