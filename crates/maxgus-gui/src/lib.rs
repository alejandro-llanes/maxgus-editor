//! The desktop front end.
//!
//! The same editor the terminal front end drives, drawn into a window by the
//! GPU instead of into a terminal by escape sequences. Everything above the
//! drawing — commands, keymaps, buffers, redisplay — is shared; what is here
//! is a window, a font, and the arithmetic between pixels and cells.

pub mod cursor;
pub mod font;
pub mod keys;
pub mod mouse;
pub mod quads;
pub mod renderer;
pub mod scroll;
pub mod spring;
pub mod vfx;
pub mod window;

pub use window::{Settings, run};
