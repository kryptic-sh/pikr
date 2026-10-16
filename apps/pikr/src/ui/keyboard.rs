//! Translation from crcbl's keyboard events to pikr's own input types.
//!
//! This is the only place the picker's key handling meets the toolkit's key
//! types; everything past it works on [`crate::picker::input`].
//!
//! A typed character reaches the window twice: as a `Key` event carrying the
//! key's symbol in the current keymap, and as the `TextCommit` the layout and
//! input method made of it. Named keys are read from the former and
//! characters from the latter, so each press becomes one [`Key`]. The
//! exception is a character typed with Ctrl, Alt or Super held: the platform
//! commits no text for a chord, so it is read from the key's symbol instead.
//!
//! Named keys come from the keysym, never the physical key code: a keymap
//! decides what a key is, and a virtual keyboard (wtype, an on-screen
//! keyboard) uploads its own keymap whose key codes stand for physical keys
//! they are not.

use crcbl_shell::{Keysym, Modifiers as CrcblModifiers};

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
pub fn key_from_press(keysym: Keysym, mods: Modifiers) -> Option<Key> {
    if let Some(named) = named_from_keysym(keysym) {
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

/// The named key `keysym` is. The values are X11's keysym numbering
/// (`xkbcommon-keysyms.h`), which crcbl reports on every backend.
fn named_from_keysym(keysym: Keysym) -> Option<NamedKey> {
    Some(match keysym.0 {
        0xFF08 => NamedKey::Backspace,
        // Shift+Tab is `ISO_Left_Tab` in XKB keymaps.
        0xFF09 | 0xFE20 => NamedKey::Tab,
        0xFF0D | 0xFF8D => NamedKey::Enter,
        0xFF1B => NamedKey::Escape,
        0xFF50 => NamedKey::Home,
        0xFF51 => NamedKey::ArrowLeft,
        0xFF52 => NamedKey::ArrowUp,
        0xFF53 => NamedKey::ArrowRight,
        0xFF54 => NamedKey::ArrowDown,
        0xFF55 => NamedKey::PageUp,
        0xFF56 => NamedKey::PageDown,
        0xFF57 => NamedKey::End,
        0xFF63 => NamedKey::Insert,
        0xFFBE => NamedKey::F1,
        0xFFBF => NamedKey::F2,
        0xFFC0 => NamedKey::F3,
        0xFFC1 => NamedKey::F4,
        0xFFC2 => NamedKey::F5,
        0xFFC3 => NamedKey::F6,
        0xFFC4 => NamedKey::F7,
        0xFFC5 => NamedKey::F8,
        0xFFC6 => NamedKey::F9,
        0xFFC7 => NamedKey::F10,
        0xFFC8 => NamedKey::F11,
        0xFFC9 => NamedKey::F12,
        0xFFFF => NamedKey::Delete,
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
    fn named_keys_come_from_the_keysym() {
        let none = Modifiers::default();
        // The keysyms crcbl's Win32 and AppKit backends report for these keys
        // (`crcbl_shell::keysym::named_keysym`), plus XKB's Shift+Tab.
        let cases = [
            (0xFF08, NamedKey::Backspace),
            (0xFF09, NamedKey::Tab),
            (0xFE20, NamedKey::Tab),
            (0xFF0D, NamedKey::Enter),
            (0xFF8D, NamedKey::Enter),
            (0xFF1B, NamedKey::Escape),
            (0xFF50, NamedKey::Home),
            (0xFF51, NamedKey::ArrowLeft),
            (0xFF52, NamedKey::ArrowUp),
            (0xFF53, NamedKey::ArrowRight),
            (0xFF54, NamedKey::ArrowDown),
            (0xFF55, NamedKey::PageUp),
            (0xFF56, NamedKey::PageDown),
            (0xFF57, NamedKey::End),
            (0xFF63, NamedKey::Insert),
            (0xFFFF, NamedKey::Delete),
            (0xFFBE, NamedKey::F1),
            (0xFFC9, NamedKey::F12),
        ];
        for (keysym, named) in cases {
            assert_eq!(
                key_from_press(Keysym(keysym), none),
                Some(Key::Named(named)),
                "keysym {keysym:#x}"
            );
        }
    }

    #[test]
    fn plain_characters_come_from_the_commit_not_the_press() {
        let none = Modifiers::default();
        let a = Keysym::from_char('a');
        assert_eq!(key_from_press(a, none), None);
        assert_eq!(key_from_commit("é", none), Some(Key::Character("é".into())));
    }

    #[test]
    fn chords_come_from_the_press_not_the_commit() {
        let w = Keysym::from_char('w');
        assert_eq!(key_from_press(w, ctrl()), Some(Key::Character("w".into())));
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
