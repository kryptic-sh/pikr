//! Translation from floem's keyboard events to pikr's own input types.
//!
//! This is the only place the picker's key handling meets the toolkit's key
//! types; everything past it works on [`crate::picker::input`].

use floem::ui_events::keyboard::{
    Key as FloemKey, Modifiers as FloemModifiers, NamedKey as FloemNamedKey,
};

use crate::picker::input::{Key, Modifiers, NamedKey};

pub fn key_from_floem(key: &FloemKey) -> Key {
    match key {
        FloemKey::Character(s) => Key::Character(s.to_string()),
        FloemKey::Named(named) => named_from_floem(*named).map_or(Key::Other, Key::Named),
    }
}

pub fn modifiers_from_floem(mods: FloemModifiers) -> Modifiers {
    Modifiers {
        ctrl: mods.ctrl(),
        shift: mods.shift(),
        alt: mods.alt(),
        meta: mods.meta(),
    }
}

fn named_from_floem(named: FloemNamedKey) -> Option<NamedKey> {
    Some(match named {
        FloemNamedKey::ArrowDown => NamedKey::ArrowDown,
        FloemNamedKey::ArrowLeft => NamedKey::ArrowLeft,
        FloemNamedKey::ArrowRight => NamedKey::ArrowRight,
        FloemNamedKey::ArrowUp => NamedKey::ArrowUp,
        FloemNamedKey::Backspace => NamedKey::Backspace,
        FloemNamedKey::Delete => NamedKey::Delete,
        FloemNamedKey::End => NamedKey::End,
        FloemNamedKey::Enter => NamedKey::Enter,
        FloemNamedKey::Escape => NamedKey::Escape,
        FloemNamedKey::Home => NamedKey::Home,
        FloemNamedKey::Insert => NamedKey::Insert,
        FloemNamedKey::PageDown => NamedKey::PageDown,
        FloemNamedKey::PageUp => NamedKey::PageUp,
        FloemNamedKey::Tab => NamedKey::Tab,
        FloemNamedKey::F1 => NamedKey::F1,
        FloemNamedKey::F2 => NamedKey::F2,
        FloemNamedKey::F3 => NamedKey::F3,
        FloemNamedKey::F4 => NamedKey::F4,
        FloemNamedKey::F5 => NamedKey::F5,
        FloemNamedKey::F6 => NamedKey::F6,
        FloemNamedKey::F7 => NamedKey::F7,
        FloemNamedKey::F8 => NamedKey::F8,
        FloemNamedKey::F9 => NamedKey::F9,
        FloemNamedKey::F10 => NamedKey::F10,
        FloemNamedKey::F11 => NamedKey::F11,
        FloemNamedKey::F12 => NamedKey::F12,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_keys_map_and_unbound_ones_become_other() {
        assert_eq!(
            key_from_floem(&FloemKey::Named(FloemNamedKey::F12)),
            Key::Named(NamedKey::F12)
        );
        assert_eq!(
            key_from_floem(&FloemKey::Named(FloemNamedKey::CapsLock)),
            Key::Other
        );
    }

    #[test]
    fn characters_carry_their_text() {
        assert_eq!(
            key_from_floem(&FloemKey::Character("é".into())),
            Key::Character("é".into())
        );
    }

    #[test]
    fn modifiers_map_field_by_field() {
        let mods = modifiers_from_floem(FloemModifiers::CONTROL | FloemModifiers::META);
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
