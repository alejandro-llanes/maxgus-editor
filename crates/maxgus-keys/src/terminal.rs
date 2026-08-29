//! Translation from terminal events to [`Key`] values.
//!
//! Terminals are inconsistent about how they report Emacs-relevant keys: Meta
//! may arrive as an Alt bit or as a preceding ESC, `C-i` is indistinguishable
//! from TAB on most terminals, and `C-@`, `C-space` and NUL are all the same
//! byte. This module normalises those cases into the keys Emacs users expect.

use crate::key::{Key, KeyCode, Modifiers};
use crossterm::event::{KeyCode as CtCode, KeyEvent, KeyEventKind, KeyModifiers};

/// Converts a crossterm key event, returning `None` for events that carry no
/// key (releases and repeats we do not act on).
pub fn key_from_event(event: KeyEvent) -> Option<Key> {
    // Kitty-protocol terminals report releases; Emacs acts on press only.
    if event.kind == KeyEventKind::Release {
        return None;
    }
    let mut modifiers = Modifiers::NONE;
    if event.modifiers.contains(KeyModifiers::CONTROL) {
        modifiers = modifiers.insert(Modifiers::CONTROL);
    }
    if event.modifiers.contains(KeyModifiers::ALT) {
        modifiers = modifiers.insert(Modifiers::META);
    }
    if event.modifiers.contains(KeyModifiers::SHIFT) {
        modifiers = modifiers.insert(Modifiers::SHIFT);
    }
    if event.modifiers.contains(KeyModifiers::SUPER) {
        modifiers = modifiers.insert(Modifiers::SUPER);
    }
    if event.modifiers.contains(KeyModifiers::HYPER) {
        modifiers = modifiers.insert(Modifiers::HYPER);
    }

    let code = match event.code {
        CtCode::Char(c) => KeyCode::Char(c),
        CtCode::Enter => KeyCode::Enter,
        CtCode::Tab => KeyCode::Tab,
        CtCode::BackTab => {
            // Shift-Tab arrives as its own code; put the shift bit back.
            modifiers = modifiers.insert(Modifiers::SHIFT);
            KeyCode::Tab
        }
        CtCode::Backspace => KeyCode::Backspace,
        CtCode::Delete => KeyCode::Delete,
        CtCode::Esc => KeyCode::Escape,
        CtCode::Insert => KeyCode::Insert,
        CtCode::Home => KeyCode::Home,
        CtCode::End => KeyCode::End,
        CtCode::PageUp => KeyCode::PageUp,
        CtCode::PageDown => KeyCode::PageDown,
        CtCode::Up => KeyCode::Up,
        CtCode::Down => KeyCode::Down,
        CtCode::Left => KeyCode::Left,
        CtCode::Right => KeyCode::Right,
        CtCode::F(n) => KeyCode::F(n),
        // Modifier presses on their own, media keys and the rest carry no
        // command binding.
        _ => return None,
    };

    // `Key::new` folds the ASCII control aliases, so `C-i` arrives as TAB and
    // matches a binding written either way.
    Some(Key::new(code, modifiers))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(code: CtCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    #[test]
    fn plain_characters_pass_through() {
        let k = key_from_event(ev(CtCode::Char('a'), KeyModifiers::NONE)).unwrap();
        assert_eq!(k, Key::char('a'));
        assert!(k.is_self_inserting());
    }

    #[test]
    fn control_and_alt_map_to_control_and_meta() {
        assert_eq!(
            key_from_event(ev(CtCode::Char('x'), KeyModifiers::CONTROL)).unwrap(),
            Key::ctrl('x')
        );
        assert_eq!(
            key_from_event(ev(CtCode::Char('x'), KeyModifiers::ALT)).unwrap(),
            Key::meta('x')
        );
        let both = key_from_event(ev(
            CtCode::Char('s'),
            KeyModifiers::CONTROL | KeyModifiers::ALT,
        ))
        .unwrap();
        assert_eq!(both.notation(), "C-M-s");
    }

    #[test]
    fn control_m_i_and_bracket_fold_to_their_named_keys() {
        assert_eq!(
            key_from_event(ev(CtCode::Char('m'), KeyModifiers::CONTROL)).unwrap(),
            Key::plain(KeyCode::Enter)
        );
        assert_eq!(
            key_from_event(ev(CtCode::Char('i'), KeyModifiers::CONTROL)).unwrap(),
            Key::plain(KeyCode::Tab)
        );
        assert_eq!(
            key_from_event(ev(CtCode::Char('['), KeyModifiers::CONTROL)).unwrap(),
            Key::plain(KeyCode::Escape)
        );
    }

    #[test]
    fn meta_survives_the_control_fold() {
        let k = key_from_event(ev(
            CtCode::Char('m'),
            KeyModifiers::CONTROL | KeyModifiers::ALT,
        ))
        .unwrap();
        assert_eq!(k.notation(), "M-RET");
    }

    #[test]
    fn control_at_is_control_space() {
        let k = key_from_event(ev(CtCode::Char('@'), KeyModifiers::CONTROL)).unwrap();
        assert_eq!(k.notation(), "C-SPC");
    }

    #[test]
    fn control_question_mark_is_del() {
        let k = key_from_event(ev(CtCode::Char('?'), KeyModifiers::CONTROL)).unwrap();
        assert_eq!(k.notation(), "DEL");
    }

    #[test]
    fn backtab_restores_the_shift_bit() {
        let k = key_from_event(ev(CtCode::BackTab, KeyModifiers::NONE)).unwrap();
        assert_eq!(k.notation(), "S-TAB");
    }

    #[test]
    fn navigation_and_function_keys_convert() {
        assert_eq!(
            key_from_event(ev(CtCode::Up, KeyModifiers::NONE))
                .unwrap()
                .notation(),
            "<up>"
        );
        assert_eq!(
            key_from_event(ev(CtCode::F(5), KeyModifiers::NONE))
                .unwrap()
                .notation(),
            "<f5>"
        );
        assert_eq!(
            key_from_event(ev(CtCode::PageUp, KeyModifiers::NONE))
                .unwrap()
                .notation(),
            "<prior>"
        );
    }

    #[test]
    fn shifted_letters_normalise_to_the_uppercase_character() {
        let k = key_from_event(ev(CtCode::Char('A'), KeyModifiers::SHIFT)).unwrap();
        assert_eq!(k, Key::char('A'));
        assert!(k.is_self_inserting());
    }

    #[test]
    fn key_releases_are_ignored() {
        let mut e = ev(CtCode::Char('a'), KeyModifiers::NONE);
        e.kind = KeyEventKind::Release;
        assert!(key_from_event(e).is_none());
    }

    #[test]
    fn events_without_a_bindable_key_are_dropped() {
        assert!(key_from_event(ev(CtCode::Null, KeyModifiers::NONE)).is_none());
    }
}
