//! A terminal emulator: the screen a program draws on, and what its escape
//! sequences mean.
//!
//! No process handling and no input decoding live here — this crate is given
//! bytes and answers with a grid, which is what lets the whole of it be
//! tested without a pty, a shell or a clock.

pub mod emulator;
pub mod grid;
pub mod keys;
pub mod selection;

pub use emulator::{Emulator, Modes};
pub use grid::{Cell, Cursor, Grid, Line};
pub use selection::{Mode as SelectionMode, Position, Selection};
