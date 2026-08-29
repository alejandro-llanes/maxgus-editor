//! Terminal setup, teardown and asynchronous input.
//!
//! The terminal is put into raw mode on the alternate screen and restored on
//! drop, including on a panic, so a crash never leaves the user's shell in a
//! broken state. Input arrives through crossterm's async event stream, which
//! integrates with tokio rather than occupying a thread in a blocking read.

use crate::{Result, TuiError, geometry::Size};
use crossterm::event::{
    DisableBracketedPaste, DisableFocusChange, EnableBracketedPaste, EnableFocusChange, Event,
    EventStream, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags,
};
use crossterm::{
    cursor::{Hide, MoveTo, SetCursorStyle, Show},
    execute,
    terminal::{
        Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode,
        enable_raw_mode,
    },
};
use futures_util::StreamExt;
use maxgus_keys::Key;
use std::io::{Stdout, Write, stdout};

/// The smallest terminal the editor will run in: room for a text line, a mode
/// line and the echo area.
pub const MINIMUM_SIZE: Size = Size {
    width: 20,
    height: 3,
};

/// An input event, already translated out of crossterm's vocabulary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TuiEvent {
    Key(Key),
    Resize(Size),
    /// A bracketed paste. Inserted literally rather than interpreted as keys,
    /// which is what stops a pasted `C-x` from running a command.
    Paste(String),
    FocusGained,
    FocusLost,
}

impl TuiEvent {
    /// Translates a crossterm event, returning `None` for events the editor
    /// has no use for.
    pub fn from_crossterm(event: Event) -> Option<TuiEvent> {
        match event {
            Event::Key(key) => maxgus_keys::key_from_event(key).map(TuiEvent::Key),
            Event::Resize(width, height) => Some(TuiEvent::Resize(Size::new(width, height))),
            Event::Paste(text) => Some(TuiEvent::Paste(text)),
            Event::FocusGained => Some(TuiEvent::FocusGained),
            Event::FocusLost => Some(TuiEvent::FocusLost),
            // Mouse reporting is not enabled, so these should not arrive.
            Event::Mouse(_) => None,
        }
    }
}

/// Owns the terminal's mode and restores it on drop.
#[derive(Debug)]
pub struct Terminal {
    out: Stdout,
    size: Size,
    /// True while raw mode and the alternate screen are in effect.
    active: bool,
    /// True when the terminal accepted the keyboard enhancement protocol, so
    /// teardown knows whether to pop it.
    enhanced_keys: bool,
}

impl Terminal {
    /// Takes over the terminal: raw mode, alternate screen, hidden cursor,
    /// bracketed paste and focus reporting.
    pub fn new() -> Result<Terminal> {
        let size = Self::query_size()?;
        if size.width < MINIMUM_SIZE.width || size.height < MINIMUM_SIZE.height {
            return Err(TuiError::TooSmall(
                size.width,
                size.height,
                MINIMUM_SIZE.width,
                MINIMUM_SIZE.height,
            ));
        }
        let mut out = stdout();
        enable_raw_mode()?;
        execute!(
            out,
            EnterAlternateScreen,
            EnableBracketedPaste,
            EnableFocusChange,
            Hide
        )?;

        // Ask for disambiguated key events where the terminal supports them,
        // so `C-i` and TAB can be told apart. Terminals that do not understand
        // the sequence ignore it, so a failure here is not fatal.
        let enhanced_keys = execute!(
            out,
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        )
        .is_ok();

        Ok(Terminal {
            out,
            size,
            active: true,
            enhanced_keys,
        })
    }

    /// The terminal size right now.
    pub fn query_size() -> Result<Size> {
        let (width, height) = crossterm::terminal::size()?;
        Ok(Size::new(width, height))
    }

    /// The size as of the last resize event.
    pub fn size(&self) -> Size {
        self.size
    }

    /// Records a new size after a resize event.
    pub fn set_size(&mut self, size: Size) {
        self.size = size;
    }

    /// True when the terminal is at least [`MINIMUM_SIZE`].
    pub fn is_usable(&self) -> bool {
        self.size.width >= MINIMUM_SIZE.width && self.size.height >= MINIMUM_SIZE.height
    }

    /// The writer frames are rendered to.
    pub fn writer(&mut self) -> &mut Stdout {
        &mut self.out
    }

    /// Chooses whether the cursor blinks.
    ///
    /// Terminals that do not understand the sequence ignore it, so this is
    /// never fatal.
    pub fn set_cursor_blinking(&mut self, blinking: bool) -> Result<()> {
        let style = if blinking {
            SetCursorStyle::BlinkingBlock
        } else {
            SetCursorStyle::SteadyBlock
        };
        execute!(self.out, style).ok();
        Ok(())
    }

    /// Places the hardware cursor and shows it, which is how point is drawn.
    pub fn place_cursor(&mut self, x: u16, y: u16) -> Result<()> {
        execute!(self.out, MoveTo(x, y), Show)?;
        Ok(())
    }

    pub fn hide_cursor(&mut self) -> Result<()> {
        execute!(self.out, Hide)?;
        Ok(())
    }

    /// Clears the screen outright, for a forced full redraw.
    pub fn clear(&mut self) -> Result<()> {
        execute!(self.out, Clear(ClearType::All), MoveTo(0, 0))?;
        Ok(())
    }

    /// Restores the terminal. Idempotent, so calling it explicitly and then
    /// dropping is safe.
    pub fn restore(&mut self) -> Result<()> {
        if !self.active {
            return Ok(());
        }
        self.active = false;
        if self.enhanced_keys {
            execute!(self.out, PopKeyboardEnhancementFlags).ok();
        }
        execute!(
            self.out,
            DisableFocusChange,
            DisableBracketedPaste,
            Show,
            LeaveAlternateScreen
        )?;
        disable_raw_mode()?;
        self.out.flush()?;
        Ok(())
    }

    /// The asynchronous stream of input events.
    ///
    /// Events crossterm reports that the editor ignores — mouse events, key
    /// releases — are filtered out here, so the caller sees only actionable
    /// input.
    pub fn events() -> impl futures_util::Stream<Item = TuiEvent> + Unpin {
        // `ready` rather than an async block, so the stream stays `Unpin` and
        // can be polled from a `select!` without being boxed.
        EventStream::new().filter_map(|result| {
            futures_util::future::ready(match result {
                Ok(event) => TuiEvent::from_crossterm(event),
                // A read error means the terminal is gone; the stream ends
                // when crossterm stops yielding.
                Err(_) => None,
            })
        })
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        // The user's shell must come back intact even if the editor panicked.
        self.restore().ok();
    }
}

/// Installs a panic hook that restores the terminal before printing the panic.
///
/// Without this a panic inside raw mode leaves the shell without echo and with
/// the alternate screen still active.
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let mut out = stdout();
        execute!(out, PopKeyboardEnhancementFlags).ok();
        execute!(
            out,
            DisableFocusChange,
            DisableBracketedPaste,
            Show,
            LeaveAlternateScreen
        )
        .ok();
        disable_raw_mode().ok();
        previous(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{
        KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent, MouseEventKind,
    };
    use maxgus_keys::KeyCode as MaxgusCode;

    #[test]
    fn key_events_translate_into_editor_keys() {
        let event = Event::Key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL));
        assert_eq!(
            TuiEvent::from_crossterm(event),
            Some(TuiEvent::Key(Key::ctrl('x')))
        );
    }

    #[test]
    fn resize_events_carry_the_new_size() {
        assert_eq!(
            TuiEvent::from_crossterm(Event::Resize(120, 40)),
            Some(TuiEvent::Resize(Size::new(120, 40)))
        );
    }

    #[test]
    fn a_paste_stays_one_event_rather_than_becoming_keystrokes() {
        let event = Event::Paste("C-x C-f".into());
        assert_eq!(
            TuiEvent::from_crossterm(event),
            Some(TuiEvent::Paste("C-x C-f".into()))
        );
    }

    #[test]
    fn focus_events_translate() {
        assert_eq!(
            TuiEvent::from_crossterm(Event::FocusGained),
            Some(TuiEvent::FocusGained)
        );
        assert_eq!(
            TuiEvent::from_crossterm(Event::FocusLost),
            Some(TuiEvent::FocusLost)
        );
    }

    #[test]
    fn mouse_events_are_dropped() {
        let event = Event::Mouse(MouseEvent {
            kind: MouseEventKind::Moved,
            column: 1,
            row: 1,
            modifiers: KeyModifiers::NONE,
        });
        assert_eq!(TuiEvent::from_crossterm(event), None);
    }

    #[test]
    fn key_releases_are_dropped() {
        let mut key = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE);
        key.kind = KeyEventKind::Release;
        assert_eq!(TuiEvent::from_crossterm(Event::Key(key)), None);
    }

    #[test]
    fn the_terminal_key_folds_survive_the_translation() {
        // `C-m` reaching the editor as RET is the property that makes an
        // Emacs keymap work on a terminal at all.
        let event = Event::Key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::CONTROL));
        assert_eq!(
            TuiEvent::from_crossterm(event),
            Some(TuiEvent::Key(Key::plain(MaxgusCode::Enter)))
        );
    }

    #[test]
    fn the_minimum_size_leaves_room_for_the_mode_line_and_echo_area() {
        // A text row, a mode line and the echo area.
        let rows_needed = 3u16;
        assert!(MINIMUM_SIZE.height >= rows_needed);
        assert!(!MINIMUM_SIZE.is_empty());
    }

    #[test]
    fn usability_is_judged_against_the_minimum() {
        // Built without touching the real terminal.
        let mut terminal = Terminal {
            out: stdout(),
            size: Size::new(80, 24),
            active: false,
            enhanced_keys: false,
        };
        assert!(terminal.is_usable());
        terminal.set_size(Size::new(10, 24));
        assert!(!terminal.is_usable());
        terminal.set_size(Size::new(80, 2));
        assert!(!terminal.is_usable());
        assert_eq!(terminal.size(), Size::new(80, 2));
    }

    #[test]
    fn restoring_an_inactive_terminal_is_a_no_op() {
        let mut terminal = Terminal {
            out: stdout(),
            size: Size::new(80, 24),
            active: false,
            enhanced_keys: false,
        };
        assert!(terminal.restore().is_ok());
        assert!(terminal.restore().is_ok(), "restoring twice is safe");
    }
}
