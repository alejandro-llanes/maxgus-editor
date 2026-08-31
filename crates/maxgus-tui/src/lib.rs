//! Terminal rendering.
//!
//! The editor draws into an off-screen [`Surface`] of styled cells. Each frame
//! is diffed against the last, so only cells that actually changed are written
//! to the terminal — the difference between a usable console editor and one
//! that flickers. Input arrives asynchronously through crossterm's event
//! stream, so redisplay and the tokio runtime never block each other.

pub mod geometry;
pub mod job;
pub mod render;
pub mod surface;
pub mod terminal;

pub use geometry::{Rect, Size};
pub use job::Suspension;
pub use render::{Change, diff, render_to};
pub use surface::{Cell, Surface, char_width};
pub use terminal::{Terminal, TuiEvent};

#[derive(Debug, thiserror::Error)]
pub enum TuiError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("the terminal is too small: {0}x{1}, at least {2}x{3} is needed")]
    TooSmall(u16, u16, u16, u16),
}

pub type Result<T> = std::result::Result<T, TuiError>;
