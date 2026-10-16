//! Translation from crcbl's keyboard events to pikr's own input types.
//!
//! This is the only place the picker's key handling meets the toolkit's key
//! types; everything past it works on [`crate::picker::input`].
//!
//! A typed character reaches the window twice: as a `Key` event naming the
//! physical key, and as the `TextCommit` the layout and input method made of
//! it. Named keys are read from the former and characters from the latter,
//! so each press becomes one [`Key`]. The exception is a character typed
//! with Ctrl, Alt or Super held: the platform commits no text for a chord,
//! so it is read from the key's symbol instead.

use crcbl_shell::{KeyCode, Keysym, Modifiers as CrcblModifiers};

use crate::picker::input::{Key, Modifiers, NamedKey};

pub fn modifiers_from_crcbl(mods: CrcblModifiers) -> Modifiers {
    let mods = mods.chord();
    Modifiers {
        ctrl: mods.contains(CrcblModifiers::CTRL),
        shift: mods.contains(CrcblModifiers::SHIFT),
        alt: mods.contains(CrcblModifiers::ALT),
        meta: mods.contains(CrcblModifiers::SUPER),
    }
}

/// Whether `mods` is a chord, for which the platform commits no text.
fn is_chord(mods: Modifiers) -> bool {
    mods.ctrl || mods.alt || mods.meta
}

/// The [`Key`] a key press stands for, or `None` when the press is a typed
/// character that arrives as a text commit instead.
pub fn key_from_press(key_code: Option<KeyCode>, keysym: Keysym, mods: Modifiers) -> Option<Key> {
    if let Some(named) = key_code.and_then(named_from_crcbl) {
        return Some(Key::Named(named));
    }
    if is_chord(mods) {
        return keysym.to_char().map(|c| Key::Character(c.to_string()));
    }
    None
}

/// The [`Key`] a text commit stands for, or `None` when it belongs to a chord
/// already read from its key press.
pub fn key_from_commit(text: &str, mods: Modifiers) -> Option<Key> {
    (!is_chord(mods)).then(|| Key::Character(text.to_owned()))
}

fn named_from_crcbl(code: KeyCode) -> Option<NamedKey> {
    Some(match code {
        KeyCode::ArrowDown => NamedKey::ArrowDown,
        KeyCode::ArrowLeft => NamedKey::ArrowLeft,
        KeyCode::ArrowRight => NamedKey::ArrowRight,
        KeyCode::ArrowUp => NamedKey::ArrowUp,
        KeyCode::Backspace => NamedKey::Backspace,
        KeyCode::Delete => NamedKey::Delete,
        KeyCode::End => NamedKey::End,
        KeyCode::Enter | KeyCode::NumpadEnter => NamedKey::Enter,
        KeyCode::Escape => NamedKey::Escape,
        KeyCode::Home => NamedKey::Home,
        KeyCode::Insert => NamedKey::Insert,
        KeyCode::PageDown => NamedKey::PageDown,
        KeyCode::PageUp => NamedKey::PageUp,
        KeyCode::Tab => NamedKey::Tab,
        KeyCode::F1 => NamedKey::F1,
        KeyCode::F2 => NamedKey::F2,
        KeyCode::F3 => NamedKey::F3,
        KeyCode::F4 => NamedKey::F4,
        KeyCode::F5 => NamedKey::F5,
        KeyCode::F6 => NamedKey::F6,
        KeyCode::F7 => NamedKey::F7,
        KeyCode::F8 => NamedKey::F8,
        KeyCode::F9 => NamedKey::F9,
        KeyCode::F10 => NamedKey::F10,
        KeyCode::F11 => NamedKey::F11,
        KeyCode::F12 => NamedKey::F12,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctrl() -> Modifiers {
        Modifiers {
            ctrl: true,
            ..Default::default()
        }
    }

    #[test]
    fn named_keys_map_and_numpad_enter_is_enter() {
        let none = Modifiers::default();
        assert_eq!(
            key_from_press(Some(KeyCode::F12), Keysym::NONE, none),
            Some(Key::Named(NamedKey::F12))
        );
        assert_eq!(
            key_from_press(Some(KeyCode::NumpadEnter), Keysym::NONE, none),
            Some(Key::Named(NamedKey::Enter))
        );
    }

    #[test]
    fn plain_characters_come_from_the_commit_not_the_press() {
        let none = Modifiers::default();
        let a = Keysym::from_char('a');
        assert_eq!(key_from_press(Some(KeyCode::KeyA), a, none), None);
        assert_eq!(key_from_commit("é", none), Some(Key::Character("é".into())));
    }

    #[test]
    fn chords_come_from_the_press_not_the_commit() {
        let w = Keysym::from_char('w');
        assert_eq!(
            key_from_press(Some(KeyCode::KeyW), w, ctrl()),
            Some(Key::Character("w".into()))
        );
        assert_eq!(key_from_commit("w", ctrl()), None);
    }

    #[test]
    fn modifiers_map_field_by_field_and_drop_locks() {
        let mods = modifiers_from_crcbl(
            CrcblModifiers::CTRL | CrcblModifiers::SUPER | CrcblModifiers::CAPS_LOCK,
        );
        assert_eq!(
            mods,
            Modifiers {
                ctrl: true,
                meta: true,
                ..Default::default()
            }
        );
    }
}
