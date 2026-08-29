//! Turning a key back into the bytes a terminal program expects.
//!
//! The editor decodes what the terminal sends into a [`Key`]; a program
//! running inside a terminal window needs it encoded again. Going round that
//! loop rather than forwarding raw bytes is what lets the same key be a
//! binding in one window and a keystroke in another.
//!
//! The awkward part is that several keys have two spellings and the program
//! chooses which one it wants. `DECCKM` switches the arrows between `CSI A`
//! and `SS3 A`; a shell at a prompt asks for the second, and sending the first
//! makes the arrows print `^[[A` instead of moving.

use crate::emulator::Modes;
use maxgus_keys::{Key, KeyCode, Modifiers};

/// The bytes to send for `key`, or `None` when there are none to send.
pub fn encode(key: &Key, modes: Modes) -> Option<Vec<u8>> {
    let control = key.modifiers.contains(Modifiers::CONTROL);
    let meta = key.modifiers.contains(Modifiers::META);
    let shift = key.modifiers.contains(Modifiers::SHIFT);

    let mut bytes = match key.code {
        KeyCode::Char(c) if control => control_byte(c)?,
        KeyCode::Char(c) => c.to_string().into_bytes(),
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Tab if shift => b"\x1b[Z".to_vec(),
        KeyCode::Tab => vec![b'\t'],
        // The terminal convention that outlived the hardware: backspace sends
        // delete, and the delete key sends a sequence.
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Escape => vec![0x1b],
        KeyCode::Delete => tilde(3, key.modifiers),
        KeyCode::Insert => tilde(2, key.modifiers),
        KeyCode::PageUp => tilde(5, key.modifiers),
        KeyCode::PageDown => tilde(6, key.modifiers),
        KeyCode::Up => cursor(b'A', key.modifiers, modes),
        KeyCode::Down => cursor(b'B', key.modifiers, modes),
        KeyCode::Right => cursor(b'C', key.modifiers, modes),
        KeyCode::Left => cursor(b'D', key.modifiers, modes),
        KeyCode::Home => cursor(b'H', key.modifiers, modes),
        KeyCode::End => cursor(b'F', key.modifiers, modes),
        KeyCode::F(n) => function(n, key.modifiers)?,
    };

    // Meta is an escape ahead of whatever it modifies, which is how a
    // terminal has always carried it.
    if meta {
        let mut out = vec![0x1b];
        out.append(&mut bytes);
        return Some(out);
    }
    Some(bytes)
}

/// `C-a` is 1, `C-z` is 26, and the handful of others that have a byte.
fn control_byte(c: char) -> Option<Vec<u8>> {
    let byte = match c.to_ascii_lowercase() {
        c @ 'a'..='z' => c as u8 - b'a' + 1,
        '@' | ' ' => 0,
        '[' => 0x1b,
        '\\' => 0x1c,
        ']' => 0x1d,
        '^' => 0x1e,
        '_' | '?' => 0x1f,
        // Anything else with control held sends the character itself: the
        // shell can make of it what it likes, and swallowing it silently
        // would look like a dropped keystroke.
        other => return Some(other.to_string().into_bytes()),
    };
    Some(vec![byte])
}

/// The modifier code a terminal puts in a sequence: 1 plus a bitmask.
fn modifier_code(modifiers: Modifiers) -> Option<u8> {
    let mut code = 0;
    if modifiers.contains(Modifiers::SHIFT) {
        code |= 1;
    }
    if modifiers.contains(Modifiers::META) {
        code |= 2;
    }
    if modifiers.contains(Modifiers::CONTROL) {
        code |= 4;
    }
    (code != 0).then_some(code + 1)
}

/// An arrow or a home/end key, in whichever spelling the program asked for.
fn cursor(final_byte: u8, modifiers: Modifiers, modes: Modes) -> Vec<u8> {
    match modifier_code(modifiers) {
        // A modified arrow is always `CSI 1;n X`, application mode or not.
        Some(code) => format!("\x1b[1;{code}{}", final_byte as char).into_bytes(),
        None if modes.application_cursor => vec![0x1b, b'O', final_byte],
        None => vec![0x1b, b'[', final_byte],
    }
}

/// The `CSI n ~` family: insert, delete, page up and page down.
fn tilde(number: u8, modifiers: Modifiers) -> Vec<u8> {
    match modifier_code(modifiers) {
        Some(code) => format!("\x1b[{number};{code}~").into_bytes(),
        None => format!("\x1b[{number}~").into_bytes(),
    }
}

/// Function keys. The first four are `SS3`, the rest are `CSI n ~`, which is
/// an accident of history rather than a design.
fn function(n: u8, modifiers: Modifiers) -> Option<Vec<u8>> {
    let plain = match n {
        1..=4 => return Some(vec![0x1b, b'O', b'P' + (n - 1)]),
        5 => 15,
        6..=10 => 17 + (n - 6),
        11 => 23,
        12 => 24,
        _ => return None,
    };
    Some(tilde(plain, modifiers))
}

/// Wraps pasted text the way `bracketed paste` asks, when the program asked.
///
/// Without it a shell treats a pasted newline as `RET` and runs half of what
/// was pasted before the rest of it has arrived.
pub fn paste(text: &str, modes: Modes) -> Vec<u8> {
    let text = text.replace('\n', "\r");
    if !modes.bracketed_paste {
        return text.into_bytes();
    }
    let mut out = b"\x1b[200~".to_vec();
    out.extend_from_slice(text.as_bytes());
    out.extend_from_slice(b"\x1b[201~");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain_modes() -> Modes {
        Modes {
            cursor_visible: true,
            ..Modes::default()
        }
    }

    fn bytes(description: &str, modes: Modes) -> Vec<u8> {
        let sequence = maxgus_keys::KeySequence::parse(description).expect("a key");
        let key = sequence.keys().first().copied().expect("one key");
        encode(&key, modes).unwrap_or_default()
    }

    #[test]
    fn ordinary_characters_are_sent_as_themselves() {
        assert_eq!(bytes("a", plain_modes()), b"a");
        assert_eq!(bytes("SPC", plain_modes()), b" ");
        assert_eq!(bytes("RET", plain_modes()), b"\r");
        assert_eq!(bytes("TAB", plain_modes()), b"\t");
    }

    #[test]
    fn control_keys_become_the_bytes_they_have_always_been() {
        assert_eq!(bytes("C-a", plain_modes()), [1]);
        assert_eq!(bytes("C-c", plain_modes()), [3], "the one that interrupts");
        assert_eq!(
            bytes("C-d", plain_modes()),
            [4],
            "and the one that ends input"
        );
        assert_eq!(bytes("C-z", plain_modes()), [26]);
        assert_eq!(
            bytes("C-SPC", plain_modes()),
            [0],
            "a null, which is what emacs sends"
        );
    }

    #[test]
    fn backspace_sends_delete_as_terminals_have_always_done() {
        // Getting this backwards is the classic reason backspace prints `^H`
        // in a shell instead of erasing.
        assert_eq!(bytes("DEL", plain_modes()), [0x7f]);
        assert_eq!(bytes("<delete>", plain_modes()), b"\x1b[3~");
    }

    #[test]
    fn meta_is_an_escape_in_front() {
        assert_eq!(bytes("M-x", plain_modes()), b"\x1bx");
        assert_eq!(bytes("M-DEL", plain_modes()), [0x1b, 0x7f]);
    }

    #[test]
    fn the_arrows_change_spelling_when_the_program_asks() {
        // `DECCKM`. A shell at a prompt asks for the second spelling, and
        // sending the first makes the arrows print `^[[A` instead of moving.
        assert_eq!(bytes("<up>", plain_modes()), b"\x1b[A");
        let application = Modes {
            application_cursor: true,
            ..plain_modes()
        };
        assert_eq!(bytes("<up>", application), b"\x1bOA");
        assert_eq!(bytes("<left>", application), b"\x1bOD");
    }

    #[test]
    fn a_modified_arrow_is_always_the_long_spelling() {
        let application = Modes {
            application_cursor: true,
            ..plain_modes()
        };
        assert_eq!(bytes("C-<right>", plain_modes()), b"\x1b[1;5C");
        assert_eq!(
            bytes("C-<right>", application),
            b"\x1b[1;5C",
            "even in application mode"
        );
        assert_eq!(bytes("S-<left>", plain_modes()), b"\x1b[1;2D");
    }

    #[test]
    fn the_page_and_function_keys_have_their_own_shapes() {
        assert_eq!(bytes("<prior>", plain_modes()), b"\x1b[5~");
        assert_eq!(bytes("<next>", plain_modes()), b"\x1b[6~");
        assert_eq!(bytes("<f1>", plain_modes()), b"\x1bOP");
        assert_eq!(bytes("<f5>", plain_modes()), b"\x1b[15~");
        assert_eq!(bytes("<f12>", plain_modes()), b"\x1b[24~");
    }

    #[test]
    fn a_paste_is_bracketed_only_when_the_program_asked_for_it() {
        // Unbracketed, a shell reads the newlines as `RET` and runs half of
        // what was pasted before the rest arrives.
        assert_eq!(paste("ls -l\nwhoami", plain_modes()), b"ls -l\rwhoami");

        let bracketed = Modes {
            bracketed_paste: true,
            ..plain_modes()
        };
        assert_eq!(
            paste("ls -l\nwhoami", bracketed),
            b"\x1b[200~ls -l\rwhoami\x1b[201~".to_vec()
        );
    }

    #[test]
    fn a_newline_in_a_paste_is_sent_as_a_return() {
        // A terminal carries `\r` for the return key; sending `\n` gives a
        // line feed with no carriage return, and the shell sees nothing.
        assert!(!paste("a\nb", plain_modes()).contains(&b'\n'));
    }
}
