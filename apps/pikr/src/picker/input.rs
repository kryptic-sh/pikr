//! Pikr-owned keyboard input types.
//!
//! The keymap (`keys.rs`) and `--kb-custom` specs (`keyspec.rs`) work on these
//! rather than on a GUI toolkit's event types, so the picker logic does not
//! change when the toolkit does. The UI layer translates its key events into
//! [`Key`] and [`Modifiers`] at the boundary.

/// A key press as the picker sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Key {
    /// A key with a name rather than a printed character.
    Named(NamedKey),
    /// The text the key produces under the current layout and modifiers.
    /// Space arrives here as `" "`.
    Character(String),
    /// A key pikr binds nothing to. It still reaches the handlers so that a
    /// modal state (a confirm card, the ex prompt) can swallow it.
    Other,
}

/// The named keys pikr binds, in the keymap or through `--kb-custom`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NamedKey {
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    ArrowUp,
    Backspace,
    Delete,
    End,
    Enter,
    Escape,
    Home,
    Insert,
    PageDown,
    PageUp,
    Tab,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
}

/// The modifier keys held during a press.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    /// Super / Windows / Command.
    pub meta: bool,
}
