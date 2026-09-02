//! The editor core: windows, the minibuffer, prefix arguments, the command
//! registry and the keymaps that reach them.
//!
//! Commands here are synchronous functions over editor state. Work that has to
//! touch the filesystem, a language server or a subprocess is expressed as a
//! [`task::Task`] that the event loop runs on tokio and delivers back as a
//! [`task::TaskResult`]. That keeps every command deterministically testable
//! while all real input and output stays asynchronous.

pub mod autocomplete;
pub mod beacon;
pub mod browser;
pub mod buffers;
pub mod clipboard;
pub mod command;
pub mod commands;
pub mod dired;
pub mod dispatch;
pub mod editor;
pub mod frontend;
pub mod fuzzy;
#[cfg(feature = "full")]
pub mod git;
#[cfg(feature = "full")]
pub mod grep;
pub mod icons;
pub mod keymap;
pub mod markup;
pub mod minibuffer;
pub mod multi;
pub mod panel;
pub mod picture;
#[cfg(feature = "full")]
pub mod position;
pub mod prefix;
pub mod render;
pub mod session;
pub mod snippet;
pub mod task;
#[cfg(feature = "full")]
pub mod terminal;
#[cfg(feature = "full")]
pub mod transient;
pub mod undo_tree;
pub mod which_key;
pub mod window;
pub mod wrap;

pub mod workspace;

pub use buffers::{BufferList, SCRATCH_NAME};
pub use command::{Args, Command, Registry};
pub use commands::standard_registry;
pub use dispatch::{Dispatch, Dispatcher};
pub use editor::{Editor, build_theme, mode_display_name};
pub use keymap::{global_keymap, isearch_keymap, minibuffer_keymap};
pub use minibuffer::{Completion, Minibuffer, MinibufferKind};
#[cfg(feature = "full")]
pub use position::{offset_of_position, position_of_offset};
pub use prefix::Prefix;
pub use render::{draw, draw_background, draw_floating, edge_row, edge_rows, text_area};

/// What a language server said about a symbol, and where it was.
#[cfg(feature = "full")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Doc {
    pub text: String,
    /// The buffer line the symbol is on, so the box can sit beside it
    /// rather than over it.
    pub line: usize,
    pub window: window::WindowId,
}
#[cfg(feature = "full")]
pub use task::LspQuery;
pub use task::{Task, TaskQueue, TaskResult, TreeAction, WriteGuard};
pub use window::{Direction, Window, WindowId, WindowTree};

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    // Passed through as it is: "Buffer *dired* is read-only" is the whole
    // of what the echo area should say about it.
    #[error("{0}")]
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

/// A duration written the way a startup time is read: milliseconds until
/// they stop being informative, then seconds.
pub fn human_duration(elapsed: std::time::Duration) -> String {
    let millis = elapsed.as_secs_f64() * 1000.0;
    if millis < 1.0 {
        format!("{:.2}ms", millis)
    } else if millis < 10.0 {
        format!("{:.1}ms", millis)
    } else if millis < 1000.0 {
        format!("{:.0}ms", millis)
    } else {
        format!("{:.2}s", elapsed.as_secs_f64())
    }
}

/// `n` of `noun`, as a person would write it: "1 file", "2 files",
/// "3 matches", "4 directories". The echo area is read, and "1 file(s)"
/// is written by something that could not be bothered to count.
pub fn count(n: usize, noun: &str) -> String {
    if n == 1 {
        return format!("1 {noun}");
    }
    let plural = if let Some(stem) = noun.strip_suffix('y')
        && !stem.ends_with(['a', 'e', 'i', 'o', 'u'])
    {
        format!("{stem}ies")
    } else if noun.ends_with(['s', 'x', 'z']) || noun.ends_with("ch") || noun.ends_with("sh") {
        format!("{noun}es")
    } else {
        format!("{noun}s")
    };
    format!("{n} {plural}")
}

#[cfg(test)]
mod count_tests {
    use super::count;

    #[test]
    fn one_is_singular_and_the_rest_are_not() {
        assert_eq!(count(1, "file"), "1 file");
        assert_eq!(count(0, "file"), "0 files");
        assert_eq!(count(2, "line"), "2 lines");
    }

    #[test]
    fn the_irregular_endings_are_spelt_out() {
        assert_eq!(count(3, "match"), "3 matches");
        assert_eq!(count(2, "directory"), "2 directories");
        assert_eq!(count(2, "day"), "2 days");
        assert_eq!(count(2, "change"), "2 changes");
        assert_eq!(count(1, "directory"), "1 directory");
    }
}

#[cfg(test)]
mod duration_tests {
    use std::time::Duration;

    #[test]
    fn a_startup_time_is_written_at_a_useful_precision() {
        let says = |micros| super::human_duration(Duration::from_micros(micros));
        assert_eq!(says(400), "0.40ms");
        assert_eq!(says(4_200), "4.2ms");
        assert_eq!(says(42_000), "42ms");
        assert_eq!(says(999_400), "999ms");
        assert_eq!(says(1_500_000), "1.50s");
    }
}

/// The command that hands a path to whatever the desktop opens it with.
///
/// `xdg-open` on a free desktop, `open` on macOS, `start` on Windows. Which
/// program that turns out to be is the desktop's business — an image viewer
/// for an image, a reader for a PDF — and asking it is what keeps the editor
/// from having to be all of them.
pub fn desktop_open_command(path: &str) -> String {
    let quoted = shell_quote(path);
    if cfg!(target_os = "macos") {
        format!("open {quoted}")
    } else if cfg!(target_os = "windows") {
        // `start` is a shell builtin and takes a window title first, which is
        // why the empty string is there rather than being an oversight.
        format!("cmd /c start \"\" {quoted}")
    } else {
        format!("xdg-open {quoted}")
    }
}

/// Wraps `text` in single quotes for a shell, escaping any it contains.
pub fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

#[cfg(test)]
mod desktop_tests {
    #[test]
    fn a_path_is_quoted_so_a_space_or_a_quote_cannot_escape_it() {
        assert_eq!(super::shell_quote("plain.txt"), "'plain.txt'");
        assert_eq!(super::shell_quote("two words.txt"), "'two words.txt'");
        assert_eq!(super::shell_quote("it's here.txt"), r"'it'\''s here.txt'");
    }

    #[test]
    fn the_command_names_the_desktops_own_opener() {
        let command = super::desktop_open_command("/tmp/a.pdf");
        let expected = if cfg!(target_os = "macos") {
            "open"
        } else if cfg!(target_os = "windows") {
            "cmd"
        } else {
            "xdg-open"
        };
        assert!(
            command.starts_with(expected),
            "`{command}` does not call `{expected}`"
        );
        assert!(command.contains("'/tmp/a.pdf'"), "the path is not quoted");
    }
}
