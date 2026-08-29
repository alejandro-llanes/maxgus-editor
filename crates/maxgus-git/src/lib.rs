//! Reading git, without running it.
//!
//! Every function here is given the output of a git command and returns a
//! value; nothing in this crate starts a process. That is what lets the
//! parsing of a status, a diff and a log be tested exhaustively against text,
//! and what keeps the one part that *does* run git — the executor — small
//! enough to check by hand.

pub mod diff;
pub mod log;
pub mod status;

pub use diff::{DiffLine, FileDiff, Hunk, LineKind};
pub use log::{Commit, RefKind, Reference, Stash};
pub use status::{Change, Entry, Status};
