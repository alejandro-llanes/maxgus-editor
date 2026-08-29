//! Turning a window system's key events into the editor's own keys.
//!
//! The terminal front end gets `C-x` as a single control byte; a window system
//! gives a key and a set of modifiers and leaves the interpretation to the
//! application. That difference is the whole of this module: everything below
//! it — keymaps, prefixes, `C-h k` — is shared.

use maxgus_keys::{Key, KeyCode, Modifiers};
use winit::event::ElementState;
use winit::keyboard::{Key as WinitKey, ModifiersState, NamedKey};

/// The editor's key for a window-system key press, or `None` for a press that
/// means nothing on its own — a modifier being held down, a dead key.
///
/// Takes the parts of the event it reads rather than the event: winit's
/// `KeyEvent` carries a platform-private field and cannot be built outside
/// winit, which would leave this untestable.
pub fn translate(
    state: ElementState,
    logical: &WinitKey,
    modifiers: ModifiersState,
) -> Option<Key> {
    if state != ElementState::Pressed {
        return None;
    }
    let mut mods = Modifiers::NONE;
    if modifiers.control_key() {
        mods = mods.insert(Modifiers::CONTROL);
    }
    // Alt is Emacs' meta, which is what every Emacs user has their Alt set to.
    if modifiers.alt_key() {
        mods = mods.insert(Modifiers::META);
    }
    if modifiers.super_key() {
        mods = mods.insert(Modifiers::SUPER);
    }

    let code = match logical {
        WinitKey::Named(named) => named_code(*named)?,
        WinitKey::Character(text) => {
            let mut chars = text.chars();
            let first = chars.next()?;
            // A key that produced more than one character is composed text
            // rather than a key press, and belongs to the insertion path.
            if chars.next().is_some() {
                return None;
            }
            KeyCode::Char(first)
        }
        _ => return None,
    };

    // Shift is part of the character the keyboard layout already produced —
    // `A` rather than `S-a` — so it is only a modifier for keys that have no
    // character of their own. Saying both would mean `C-S-a` never matching a
    // binding written `C-A`.
    if modifiers.shift_key() && !matches!(code, KeyCode::Char(_)) {
        mods = mods.insert(Modifiers::SHIFT);
    }
    Some(Key::new(code, mods))
}

fn named_code(named: NamedKey) -> Option<KeyCode> {
    Some(match named {
        NamedKey::Enter => KeyCode::Enter,
        NamedKey::Tab => KeyCode::Tab,
        NamedKey::Space => KeyCode::Char(' '),
        NamedKey::Backspace => KeyCode::Backspace,
        NamedKey::Delete => KeyCode::Delete,
        NamedKey::Escape => KeyCode::Escape,
        NamedKey::Insert => KeyCode::Insert,
        NamedKey::Home => KeyCode::Home,
        NamedKey::End => KeyCode::End,
        NamedKey::PageUp => KeyCode::PageUp,
        NamedKey::PageDown => KeyCode::PageDown,
        NamedKey::ArrowUp => KeyCode::Up,
        NamedKey::ArrowDown => KeyCode::Down,
        NamedKey::ArrowLeft => KeyCode::Left,
        NamedKey::ArrowRight => KeyCode::Right,
        NamedKey::F1 => KeyCode::F(1),
        NamedKey::F2 => KeyCode::F(2),
        NamedKey::F3 => KeyCode::F(3),
        NamedKey::F4 => KeyCode::F(4),
        NamedKey::F5 => KeyCode::F(5),
        NamedKey::F6 => KeyCode::F(6),
        NamedKey::F7 => KeyCode::F(7),
        NamedKey::F8 => KeyCode::F(8),
        NamedKey::F9 => KeyCode::F(9),
        NamedKey::F10 => KeyCode::F(10),
        NamedKey::F11 => KeyCode::F(11),
        NamedKey::F12 => KeyCode::F(12),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(key: WinitKey, modifiers: ModifiersState) -> Option<Key> {
        translate(ElementState::Pressed, &key, modifiers)
    }

    fn character(c: &str) -> WinitKey {
        WinitKey::Character(c.into())
    }

    #[test]
    fn a_plain_letter_is_that_letter() {
        let key = press(character("a"), ModifiersState::empty()).expect("a key");
        assert_eq!(key.notation(), "a");
    }

    #[test]
    fn control_and_a_letter_is_what_emacs_writes() {
        let key = press(character("x"), ModifiersState::CONTROL).expect("a key");
        assert_eq!(key.notation(), "C-x");
    }

    #[test]
    fn alt_is_meta() {
        let key = press(character("x"), ModifiersState::ALT).expect("a key");
        assert_eq!(key.notation(), "M-x");
    }

    #[test]
    fn shift_is_in_the_character_rather_than_beside_it() {
        // The layout already produced a capital, so the key is `A` and not
        // `S-a`: saying both would stop a binding written `A` being reached.
        let key = press(character("A"), ModifiersState::SHIFT).expect("a key");
        assert_eq!(key.notation(), "A");
    }

    #[test]
    fn a_shifted_control_letter_is_the_same_key_as_the_unshifted_one() {
        // As in a terminal, where `C-A` and `C-a` are one byte. A binding
        // written `C-a` has to be reached with caps lock on.
        let key = press(
            character("A"),
            ModifiersState::SHIFT | ModifiersState::CONTROL,
        )
        .expect("a key");
        assert_eq!(key.notation(), "C-a");
    }

    #[test]
    fn shift_is_a_modifier_for_keys_with_no_character() {
        let key = press(WinitKey::Named(NamedKey::PageUp), ModifiersState::SHIFT).expect("a key");
        assert_eq!(key.notation(), "S-<prior>");
    }

    #[test]
    fn the_named_keys_all_arrive() {
        for (named, expected) in [
            (NamedKey::Enter, "RET"),
            (NamedKey::Tab, "TAB"),
            (NamedKey::Space, "SPC"),
            (NamedKey::Backspace, "DEL"),
            (NamedKey::Escape, "ESC"),
            (NamedKey::ArrowUp, "<up>"),
            (NamedKey::PageDown, "<next>"),
            (NamedKey::F5, "<f5>"),
        ] {
            let key = press(WinitKey::Named(named), ModifiersState::empty())
                .unwrap_or_else(|| panic!("{named:?} produced nothing"));
            assert_eq!(key.notation(), expected);
        }
    }

    #[test]
    fn a_release_is_not_a_press() {
        assert_eq!(
            translate(
                ElementState::Released,
                &character("a"),
                ModifiersState::empty()
            ),
            None
        );
    }

    #[test]
    fn composed_text_is_left_to_the_insertion_path() {
        assert_eq!(press(character("ab"), ModifiersState::empty()), None);
    }
}
