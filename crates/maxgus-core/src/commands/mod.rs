//! Command implementations.
//!
//! Each module registers one family of commands. [`standard_registry`] gathers
//! them, and a test asserts that everything the default keymaps bind is
//! actually registered — so a binding can never point at nothing.

pub mod buffer;
pub mod edit;
pub mod file;
pub mod help;
pub mod lsp;
pub mod minibuffer;
pub mod misc;
pub mod motion;
pub mod register;
pub mod search;
pub mod text;
pub mod tree;
pub mod window;

use crate::command::Registry;

/// Every command the editor ships with.
pub fn standard_registry() -> Registry {
    let mut registry = Registry::new();
    motion::register(&mut registry);
    edit::register(&mut registry);
    minibuffer::register(&mut registry);
    window::register(&mut registry);
    buffer::register(&mut registry);
    file::register(&mut registry);
    search::register(&mut registry);
    tree::register(&mut registry);
    misc::register(&mut registry);
    help::register(&mut registry);
    register::register(&mut registry);
    text::register(&mut registry);
    text::register_syntax(&mut registry);
    lsp::register(&mut registry);
    registry
}
