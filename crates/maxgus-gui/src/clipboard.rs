//! The display server's clipboard, handed to the editor.
//!
//! A kill goes out here and a yank looks here first, so `C-w` in the editor
//! and `C-v` in a browser meet in the middle. On X11 and Wayland there is a
//! second selection too, the one the mouse makes: what is swept out with
//! the left button is offered there without disturbing the clipboard, and
//! the middle button pastes it back.

use maxgus_core::clipboard::Clipboard;

/// The clipboard behind the window, if the display server gave us one.
pub struct System {
    inner: Option<arboard::Clipboard>,
}

impl System {
    pub fn new() -> Self {
        Self {
            inner: arboard::Clipboard::new().ok(),
        }
    }
}

impl Default for System {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(all(
    unix,
    not(any(target_os = "macos", target_os = "android", target_os = "emscripten"))
))]
mod primary {
    use arboard::{GetExtLinux, LinuxClipboardKind, SetExtLinux};

    pub fn read(clipboard: &mut arboard::Clipboard) -> Option<String> {
        clipboard
            .get()
            .clipboard(LinuxClipboardKind::Primary)
            .text()
            .ok()
    }

    pub fn write(clipboard: &mut arboard::Clipboard, text: &str) {
        let _ = clipboard
            .set()
            .clipboard(LinuxClipboardKind::Primary)
            .text(text);
    }
}

#[cfg(not(all(
    unix,
    not(any(target_os = "macos", target_os = "android", target_os = "emscripten"))
)))]
mod primary {
    pub fn read(_: &mut arboard::Clipboard) -> Option<String> {
        None
    }

    pub fn write(_: &mut arboard::Clipboard, _: &str) {}
}

impl Clipboard for System {
    fn name(&self) -> &'static str {
        "system"
    }

    fn read(&mut self) -> Option<String> {
        self.inner.as_mut()?.get_text().ok()
    }

    fn write(&mut self, text: &str) {
        if let Some(clipboard) = self.inner.as_mut() {
            let _ = clipboard.set_text(text);
        }
    }

    fn read_primary(&mut self) -> Option<String> {
        primary::read(self.inner.as_mut()?)
    }

    fn write_primary(&mut self, text: &str) {
        if let Some(clipboard) = self.inner.as_mut() {
            primary::write(clipboard, text);
        }
    }
}
