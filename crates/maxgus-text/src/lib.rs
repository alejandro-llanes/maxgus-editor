//! Text primitives for maxgus: rope-backed buffers, Emacs-style point/mark,
//! grouped undo, kill ring, registers, motions and search.

pub mod buffer;
pub mod edit;
pub mod kill_ring;
pub mod motion;
pub mod position;
pub mod registers;
pub mod search;
pub mod undo;

pub use buffer::{Buffer, BufferId, LineEnding};
pub use edit::{Edit, EditKind};
pub use kill_ring::KillRing;
pub use motion::{CharClass, Motion};
pub use position::{Position, Range};
pub use registers::{Register, Registers};
pub use search::{Match, SearchDirection, SearchKind, SearchQuery};
pub use undo::{UndoGroup, UndoStack};

/// Errors produced by the text layer.
#[derive(Debug, thiserror::Error)]
pub enum TextError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid regular expression: {0}")]
    Regex(#[from] regex::Error),
    #[error("position {0} is out of bounds (buffer holds {1} chars)")]
    OutOfBounds(usize, usize),
    #[error("no mark is set in this buffer")]
    NoMark,
    #[error("register `{0}` is empty")]
    EmptyRegister(char),
}

pub type Result<T> = std::result::Result<T, TextError>;
