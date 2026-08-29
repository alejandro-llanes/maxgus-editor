//! Faces: named collections of colours and attributes.
//!
//! `maxgus` follows the Emacs model. Everything drawn on screen names a *face*
//! rather than a colour, themes bind faces to concrete attributes, and faces
//! inherit from one another so a theme only has to state what differs.

pub mod color;
pub mod defaults;
pub mod face;
pub mod names;
pub mod theme;

pub use color::{Color, ColorDepth, ColorError, rgb_to_ansi256, xterm_palette_rgb};
pub use face::{Attributes, Face};
pub use theme::{Theme, ThemeError};
