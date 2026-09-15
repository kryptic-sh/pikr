//! Key specs for `--kb-custom` — `Shift+Delete`, `Ctrl+d`, `Alt+Right`, `F2`,
//! optionally with a confirm prompt: `Right=Forget?`.

use std::fmt;
use std::str::FromStr;

use floem::ui_events::keyboard::{Key, Modifiers, NamedKey};

/// A single key chord: a key plus the exact set of modifiers held with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeySpec {
    key: SpecKey,
    ctrl: bool,
    shift: bool,
    alt: bool,
    meta: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SpecKey {
    Named(NamedKey),
    /// Stored lowercase; matched case-insensitively so `Shift+d` still fires
    /// when the compositor delivers the shifted `D`.
    Char(char),
}

impl KeySpec {
    /// True when this spec's key is `named`, whatever the modifiers.
    pub fn is_named(&self, named: NamedKey) -> bool {
        self.key == SpecKey::Named(named)
    }

    /// True when `key` pressed with `mods` is this chord. Modifiers must match
    /// exactly, so `Delete` does not fire on `Shift+Delete` and vice versa.
    /// The one exception is Shift on a non-alphabetic character (see
    /// [`KeySpec::shift_is_optional`]).
    pub fn matches(&self, key: &Key, mods: Modifiers) -> bool {
        let shift_ok = if self.shift_is_optional() {
            mods.shift() || !self.shift
        } else {
            mods.shift() == self.shift
        };
        if !shift_ok || (self.ctrl, self.alt, self.meta) != (mods.ctrl(), mods.alt(), mods.meta()) {
            return false;
        }
        match (&self.key, key) {
            (SpecKey::Named(want), Key::Named(got)) => want == got,
            (SpecKey::Char(want), Key::Character(got)) => {
                let mut chars = got.chars();
                matches!(
                    (chars.next(), chars.next()),
                    (Some(c), None) if c.to_lowercase().eq(want.to_lowercase())
                )
            }
            _ => false,
        }
    }

    /// Shift is part of how a symbol such as `?` or `+` is typed on many
    /// layouts (US `?` is Shift+`/`), so the press arrives with Shift held
    /// and an exact comparison would never fire. For a non-alphabetic
    /// character Shift is therefore ignored unless the spec names it, in
    /// which case it is required. Letters keep exact matching, so `d` and
    /// `Shift+d` stay distinct.
    fn shift_is_optional(&self) -> bool {
        matches!(self.key, SpecKey::Char(c) if !c.is_alphabetic())
    }

    /// The built-in key a binding would take over, if any: bare Escape
    /// (cancel), bare Enter (accept), or a printable character without
    /// Ctrl, Alt or Super (typing, and Normal-mode commands such as `g`).
    /// Shift alone doesn't help a character: `Shift+x` types `X`.
    fn shadowed_builtin(&self) -> Option<&'static str> {
        if self.ctrl || self.alt || self.meta {
            return None;
        }
        match self.key {
            SpecKey::Char(c) if !c.is_control() => Some("typing into the query"),
            SpecKey::Named(NamedKey::Escape) if !self.shift => Some("cancel"),
            SpecKey::Named(NamedKey::Enter) if !self.shift => Some("accept"),
            _ => None,
        }
    }
}

impl FromStr for KeySpec {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut spec = KeySpec {
            key: SpecKey::Char(' '),
            ctrl: false,
            shift: false,
            alt: false,
            meta: false,
        };

        // Split on `+`, but let a trailing `+` be the key itself (`+`, `Ctrl++`).
        let (mods, key) = if s == "+" {
            (None, "+")
        } else if let Some(head) = s.strip_suffix("++") {
            (Some(head), "+")
        } else {
            match s.rsplit_once('+') {
                Some((head, key)) => (Some(head), key),
                None => (None, s),
            }
        };

        for m in mods.into_iter().flat_map(|mods| mods.split('+')) {
            // `Ctrl++d` and `+d` are typos, not `Ctrl+d` and `d`.
            if m.is_empty() {
                return Err(format!("empty modifier in `{s}`"));
            }
            let flag = match m.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => &mut spec.ctrl,
                "shift" => &mut spec.shift,
                "alt" => &mut spec.alt,
                "super" | "meta" | "mod4" => &mut spec.meta,
                _ => return Err(format!("unknown modifier `{m}` in `{s}`")),
            };
            if std::mem::replace(flag, true) {
                return Err(format!("modifier `{m}` repeated in `{s}`"));
            }
        }

        spec.key = parse_key(key).ok_or_else(|| format!("unknown key `{key}` in `{s}`"))?;
        Ok(spec)
    }
}

fn parse_key(name: &str) -> Option<SpecKey> {
    let mut chars = name.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        return Some(SpecKey::Char(c.to_lowercase().next().unwrap_or(c)));
    }
    let named = match name.to_ascii_lowercase().as_str() {
        "delete" | "del" => NamedKey::Delete,
        "backspace" => NamedKey::Backspace,
        "insert" => NamedKey::Insert,
        "return" | "enter" => NamedKey::Enter,
        "tab" => NamedKey::Tab,
        "escape" | "esc" => NamedKey::Escape,
        "space" => return Some(SpecKey::Char(' ')),
        "left" | "arrowleft" => NamedKey::ArrowLeft,
        "right" | "arrowright" => NamedKey::ArrowRight,
        "up" | "arrowup" => NamedKey::ArrowUp,
        "down" | "arrowdown" => NamedKey::ArrowDown,
        "home" => NamedKey::Home,
        "end" => NamedKey::End,
        "pageup" => NamedKey::PageUp,
        "pagedown" => NamedKey::PageDown,
        f => match f.strip_prefix('f').and_then(|n| n.parse::<u8>().ok()) {
            Some(1) => NamedKey::F1,
            Some(2) => NamedKey::F2,
            Some(3) => NamedKey::F3,
            Some(4) => NamedKey::F4,
            Some(5) => NamedKey::F5,
            Some(6) => NamedKey::F6,
            Some(7) => NamedKey::F7,
            Some(8) => NamedKey::F8,
            Some(9) => NamedKey::F9,
            Some(10) => NamedKey::F10,
            Some(11) => NamedKey::F11,
            Some(12) => NamedKey::F12,
            _ => return None,
        },
    };
    Some(SpecKey::Named(named))
}

/// One `--kb-custom` binding: `KEY` or `KEY=PROMPT`. With a prompt the key
/// shows a confirm card on the highlighted row instead of accepting at once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KbCustom {
    pub key: KeySpec,
    pub confirm: Option<String>,
}

impl FromStr for KbCustom {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // The first `=` that isn't the key itself separates KEY from PROMPT:
        // `==Delete?` binds `=`, and `Ctrl+==Delete?` binds `Ctrl+=`.
        let split = s
            .char_indices()
            .find(|&(i, c)| c == '=' && i > 0 && !s[..i].ends_with('+'));
        let (key, confirm) = match split {
            Some((i, _)) => (&s[..i], Some(&s[i + 1..])),
            None => (s, None),
        };
        let confirm = match confirm {
            Some(p) if p.trim().is_empty() => {
                return Err(format!("empty confirm prompt in `{s}`"));
            }
            other => other.map(str::to_owned),
        };
        let key: KeySpec = key.parse()?;
        if let Some(builtin) = key.shadowed_builtin() {
            return Err(format!(
                "`{key}` would shadow {builtin}; add a modifier such as `Alt+{key}`"
            ));
        }
        Ok(KbCustom { key, confirm })
    }
}

impl fmt::Display for KeySpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (on, name) in [
            (self.ctrl, "Ctrl+"),
            (self.shift, "Shift+"),
            (self.alt, "Alt+"),
            (self.meta, "Super+"),
        ] {
            if on {
                f.write_str(name)?;
            }
        }
        match &self.key {
            SpecKey::Named(k) => write!(f, "{k:?}"),
            SpecKey::Char(' ') => f.write_str("Space"),
            SpecKey::Char(c) => write!(f, "{c}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mods(ctrl: bool, shift: bool, alt: bool, meta: bool) -> Modifiers {
        let mut m = Modifiers::empty();
        m.set(Modifiers::CONTROL, ctrl);
        m.set(Modifiers::SHIFT, shift);
        m.set(Modifiers::ALT, alt);
        m.set(Modifiers::META, meta);
        m
    }

    const NONE: (bool, bool, bool, bool) = (false, false, false, false);

    fn m(t: (bool, bool, bool, bool)) -> Modifiers {
        mods(t.0, t.1, t.2, t.3)
    }

    #[test]
    fn named_key_with_modifier() {
        let spec: KeySpec = "Shift+Delete".parse().unwrap();
        let del = Key::Named(NamedKey::Delete);
        assert!(spec.matches(&del, mods(false, true, false, false)));
        // Exact modifiers: bare Delete and Ctrl+Shift+Delete don't fire.
        assert!(!spec.matches(&del, m(NONE)));
        assert!(!spec.matches(&del, mods(true, true, false, false)));
    }

    #[test]
    fn bare_named_key_ignores_modified_press() {
        let spec: KeySpec = "Right".parse().unwrap();
        let right = Key::Named(NamedKey::ArrowRight);
        assert!(spec.matches(&right, m(NONE)));
        assert!(!spec.matches(&right, mods(false, true, false, false)));
    }

    #[test]
    fn char_key_is_case_insensitive() {
        let spec: KeySpec = "Ctrl+D".parse().unwrap();
        let ctrl = mods(true, false, false, false);
        assert!(spec.matches(&Key::Character("d".into()), ctrl));
        assert!(spec.matches(&Key::Character("D".into()), ctrl));
        assert!(!spec.matches(&Key::Character("e".into()), ctrl));
    }

    #[test]
    fn shifted_char_delivered_uppercase() {
        let spec: KeySpec = "Shift+x".parse().unwrap();
        assert!(spec.matches(&Key::Character("X".into()), mods(false, true, false, false)));
    }

    #[test]
    fn modifier_names_and_case() {
        let a: KeySpec = "control+alt+super+k".parse().unwrap();
        let b: KeySpec = "CTRL+ALT+META+K".parse().unwrap();
        assert_eq!(a, b);
        assert!(a.matches(&Key::Character("k".into()), mods(true, false, true, true)));
    }

    #[test]
    fn function_keys() {
        let f2: KeySpec = "F2".parse().unwrap();
        assert!(f2.matches(&Key::Named(NamedKey::F2), m(NONE)));
        assert!("F13".parse::<KeySpec>().is_err());
        assert!("F0".parse::<KeySpec>().is_err());
    }

    #[test]
    fn plus_as_the_key() {
        // A US keyboard types `+` as Shift+`=`, so the press carries Shift.
        let plus = Key::Character("+".into());
        let spec: KeySpec = "Ctrl++".parse().unwrap();
        assert!(spec.matches(&plus, mods(true, true, false, false)));
        let bare: KeySpec = "+".parse().unwrap();
        assert!(bare.matches(&plus, mods(false, true, false, false)));
    }

    #[test]
    fn shifted_symbol_ignores_shift_unless_named() {
        let question = Key::Character("?".into());
        let ctrl = mods(true, false, false, false);
        let ctrl_shift = mods(true, true, false, false);

        let spec: KeySpec = "Ctrl+?".parse().unwrap();
        assert!(spec.matches(&question, ctrl_shift), "US layout: Shift+/");
        assert!(spec.matches(&question, ctrl), "layout with an unshifted ?");

        let explicit: KeySpec = "Ctrl+Shift+?".parse().unwrap();
        assert!(explicit.matches(&question, ctrl_shift));
        assert!(
            !explicit.matches(&question, ctrl),
            "named Shift is required"
        );

        // Other modifiers stay exact.
        assert!(!spec.matches(&question, mods(true, true, true, false)));
    }

    #[test]
    fn letters_keep_exact_shift() {
        let spec: KeySpec = "Ctrl+d".parse().unwrap();
        assert!(!spec.matches(&Key::Character("D".into()), mods(true, true, false, false)));
    }

    #[test]
    fn rejects_empty_modifier_segments() {
        for s in ["Ctrl++d", "+d", "++", "Ctrl+++", "Ctrl+", "Ctrl++Alt+d"] {
            assert!(s.parse::<KeySpec>().is_err(), "{s} must be rejected");
        }
        let err = "Ctrl++d".parse::<KeySpec>().unwrap_err();
        assert!(err.contains("empty modifier"), "{err}");
        // `+` as the key itself still parses.
        assert!("Ctrl++".parse::<KeySpec>().is_ok());
        assert!("+".parse::<KeySpec>().is_ok());
    }

    #[test]
    fn space_alias() {
        let spec: KeySpec = "Alt+Space".parse().unwrap();
        assert!(spec.matches(&Key::Character(" ".into()), mods(false, false, true, false)));
    }

    #[test]
    fn rejects_unknown_key_and_modifier() {
        assert!("Shift+Nope".parse::<KeySpec>().is_err());
        assert!("Hyper+d".parse::<KeySpec>().is_err());
        assert!("".parse::<KeySpec>().is_err());
    }

    #[test]
    fn rejects_repeated_modifier() {
        assert!("Ctrl+Ctrl+d".parse::<KeySpec>().is_err());
    }

    #[test]
    fn kb_custom_without_prompt() {
        let b: KbCustom = "Shift+Delete".parse().unwrap();
        assert_eq!(b.key, "Shift+Delete".parse().unwrap());
        assert_eq!(b.confirm, None);
    }

    #[test]
    fn kb_custom_with_prompt() {
        let b: KbCustom = "Right=Forget?".parse().unwrap();
        assert!(b.key.is_named(NamedKey::ArrowRight));
        assert_eq!(b.confirm.as_deref(), Some("Forget?"));
    }

    #[test]
    fn kb_custom_prompt_may_contain_equals() {
        let b: KbCustom = "F2=Set a=b?".parse().unwrap();
        assert_eq!(b.confirm.as_deref(), Some("Set a=b?"));
    }

    #[test]
    fn kb_custom_equals_as_the_key() {
        // A bare `=` would shadow typing; the split must still find the key.
        let bare = "==Delete?".parse::<KbCustom>().unwrap_err();
        assert!(bare.contains("`=` would shadow typing"), "{bare}");
        let ctrl: KbCustom = "Ctrl+==Delete?".parse().unwrap();
        assert!(
            ctrl.key
                .matches(&Key::Character("=".into()), mods(true, false, false, false))
        );
        assert_eq!(ctrl.confirm.as_deref(), Some("Delete?"));
        let no_prompt: KbCustom = "Ctrl+=".parse().unwrap();
        assert_eq!(no_prompt.confirm, None);
    }

    #[test]
    fn kb_custom_rejects_keys_that_shadow_builtins() {
        for (s, builtin) in [
            ("Escape", "cancel"),
            ("Esc=Sure?", "cancel"),
            ("Return", "accept"),
            ("Enter", "accept"),
            ("g", "typing"),
            ("?", "typing"),
            ("Space", "typing"),
            ("Shift+x", "typing"),
        ] {
            let err = s.parse::<KbCustom>().unwrap_err();
            assert!(err.contains(builtin), "{s}: {err}");
        }
        for s in [
            "Alt+1",
            "Ctrl+g",
            "Super+Space",
            "Shift+Return",
            "Ctrl+Escape",
            "Shift+Delete",
            "Right",
            "F2",
        ] {
            assert!(s.parse::<KbCustom>().is_ok(), "{s} must stay allowed");
        }
    }

    #[test]
    fn kb_custom_rejects_empty_prompt() {
        assert!("Right=".parse::<KbCustom>().is_err());
        assert!("Right=  ".parse::<KbCustom>().is_err());
    }

    #[test]
    fn display_round_trips_through_parse() {
        for s in [
            "Shift+Delete",
            "Ctrl+Alt+d",
            "Super+F5",
            "Alt+Space",
            "Right",
        ] {
            let spec: KeySpec = s.parse().unwrap();
            let again: KeySpec = spec.to_string().parse().unwrap();
            assert_eq!(spec, again, "{s} -> {spec}");
        }
    }
}
