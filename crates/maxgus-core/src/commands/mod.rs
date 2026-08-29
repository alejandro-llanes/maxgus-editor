//! Command implementations.
//!
//! Each module registers one family of commands. [`standard_registry`] gathers
//! them, and a test asserts that everything the default keymaps bind is
//! actually registered — so a binding can never point at nothing.

pub mod buffer;
pub mod dired;
pub mod edit;
pub mod file;
#[cfg(feature = "git")]
pub mod git;
#[cfg(feature = "grep")]
pub mod grep;
pub mod help;
#[cfg(feature = "lsp")]
pub mod lsp;
pub mod minibuffer;
pub mod misc;
pub mod motion;
pub mod multi;
pub mod panel;
pub mod register;
pub mod search;
pub mod snippet;
#[cfg(feature = "terminal")]
pub mod terminal;
pub mod text;
#[cfg(feature = "git")]
pub mod transient;
pub mod tree;
pub mod undo_tree;
pub mod window;

use crate::command::Registry;

/// Every command the editor ships with.
pub fn standard_registry() -> Registry {
    let mut registry = Registry::new();
    motion::register(&mut registry);
    dired::register(&mut registry);
    edit::register(&mut registry);
    minibuffer::register(&mut registry);
    multi::register(&mut registry);
    window::register(&mut registry);
    buffer::register(&mut registry);
    file::register(&mut registry);
    search::register(&mut registry);
    snippet::register(&mut registry);
    panel::register(&mut registry);
    #[cfg(feature = "git")]
    git::register(&mut registry);
    #[cfg(feature = "grep")]
    grep::register(&mut registry);
    #[cfg(feature = "terminal")]
    terminal::register(&mut registry);
    #[cfg(feature = "git")]
    transient::register(&mut registry);
    tree::register(&mut registry);
    undo_tree::register(&mut registry);
    misc::register(&mut registry);
    help::register(&mut registry);
    register::register(&mut registry);
    text::register(&mut registry);
    #[cfg(feature = "syntax")]
    text::register_syntax(&mut registry);
    #[cfg(feature = "lsp")]
    lsp::register(&mut registry);
    registry
}
