//! The system clipboard, as the kill ring sees it.
//!
//! Emacs calls these `interprogram-cut-function` and
//! `interprogram-paste-function`: a kill goes to the clipboard as well as
//! the ring, and a yank takes what another program put there before what
//! the ring has. The editor does not know how to reach a clipboard — a
//! window asks the display server and a terminal has to be asked for it —
//! so a front end that has one hands it in here.

/// What a front end that can reach the clipboard provides.
pub trait Clipboard {
    /// A name for the debugging output, since the editor derives `Debug`.
    fn name(&self) -> &'static str {
        "clipboard"
    }
    /// What is on the clipboard, if it is text.
    fn read(&mut self) -> Option<String>;
    /// Puts `text` on the clipboard.
    fn write(&mut self, text: &str);
    /// The primary selection — what was last selected with the mouse — on
    /// the systems that have one. Nothing, elsewhere.
    fn read_primary(&mut self) -> Option<String> {
        None
    }
    /// Makes `text` the primary selection, where there is one.
    fn write_primary(&mut self, _text: &str) {}
}

/// A clipboard kept in memory, for tests and for a front end with none.
#[derive(Debug, Default)]
pub struct Local {
    pub text: Option<String>,
    pub primary: Option<String>,
}

impl Clipboard for Local {
    fn name(&self) -> &'static str {
        "local"
    }

    fn read(&mut self) -> Option<String> {
        self.text.clone()
    }

    fn write(&mut self, text: &str) {
        self.text = Some(text.to_string());
    }

    fn read_primary(&mut self) -> Option<String> {
        self.primary.clone()
    }

    fn write_primary(&mut self, text: &str) {
        self.primary = Some(text.to_string());
    }
}

impl std::fmt::Debug for dyn Clipboard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}
