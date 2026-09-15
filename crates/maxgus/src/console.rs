//! The console a window is started with, on Windows.
//!
//! Windows gives a program built for the console a console window of its own
//! when it is started from Explorer or the Start menu rather than from a
//! terminal — and the `gui` build is one, because `maxgus -nw` has to be able
//! to take a console over. Started that way to open a window, it opened the
//! window and left an empty console beside it for as long as it ran.
//!
//! A console of its own is one no other process is attached to. A terminal
//! the editor was started from has the shell attached as well, and is kept,
//! so what the editor prints still reaches it; one of its own is let go of,
//! and closes, and the window is all there is.

use windows_sys::Win32::System::Console::{FreeConsole, GetConsoleProcessList};

/// Lets go of the console, when it was made for this process alone.
///
/// The two calls here are the only `unsafe` in this crate; there is no safe
/// way to ask Windows either question.
#[allow(unsafe_code)]
pub fn let_go_of_a_console_of_its_own() {
    // Room for two: whether a second process shares the console is all that
    // is being asked, and the count comes back however many there are.
    let mut attached = [0u32; 2];
    // SAFETY: the pointer and the length describe `attached`, which the call
    // writes at most that many process ids into. It returns how many
    // processes share the console — 0 when there is none — and writes
    // nothing when that is more than the buffer holds.
    let sharing = unsafe { GetConsoleProcessList(attached.as_mut_ptr(), attached.len() as u32) };
    if sharing == 1 {
        // SAFETY: takes nothing and detaches this process from its console,
        // whose handles nothing in a windowed editor goes on to use.
        unsafe {
            FreeConsole();
        }
    }
}
