//! The editor core: windows, the minibuffer, prefix arguments, the command
//! registry and the keymaps that reach them.
//!
//! Commands here are synchronous functions over editor state. Work that has to
//! touch the filesystem, a language server or a subprocess is expressed as a
//! [`task::Task`] that the event loop runs on tokio and delivers back as a
//! [`task::TaskResult`]. That keeps every command deterministically testable
//! while all real input and output stays asynchronous.

pub mod buffers;
pub mod command;
pub mod commands;
pub mod dispatch;
pub mod editor;
pub mod fuzzy;
pub mod icons;
pub mod keymap;
pub mod minibuffer;
pub mod position;
pub mod prefix;
pub mod render;
pub mod task;
pub mod window;

pub use command::{Args, Command, Registry};
pub use commands::standard_registry;
pub use buffers::{BufferList, SCRATCH_NAME};
pub use dispatch::{Dispatch, Dispatcher};
pub use editor::{Editor, build_theme};
pub use keymap::{global_keymap, isearch_keymap, minibuffer_keymap};
pub use minibuffer::{Minibuffer, MinibufferKind, Completion};
pub use position::{offset_of_position, position_of_offset};
pub use prefix::Prefix;
pub use render::draw;
pub use task::{LspQuery, Task, TaskQueue, TaskResult, TreeAction, WriteGuard};
pub use window::{Direction, Window, WindowId, WindowTree};

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("text error: {0}")]
    Text(#[from] maxgus_text::TextError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("no such window")]
    NoSuchWindow,
    #[error("no such buffer")]
    NoSuchBuffer,
    #[error("unknown command `{0}`")]
    UnknownCommand(String),
    #[error("cannot kill the last buffer")]
    LastBuffer,
    #[error("cannot delete the only window")]
    OnlyWindow,
    #[error("window is too small to split")]
    TooSmallToSplit,
    #[error("{0}")]
    Message(String),
}

pub type Result<T> = std::result::Result<T, CoreError>;
