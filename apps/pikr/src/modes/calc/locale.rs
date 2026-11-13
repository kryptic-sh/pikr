//! The separators the system's locale writes numbers with, and how the
//! config's `[calc]` keys override them.
//!
//! Windows asks `GetLocaleInfoEx` for the user's locale; unix asks
//! `localeconv()` under the environment's `LC_NUMERIC` (from `LC_ALL`,
//! `LC_NUMERIC` or `LANG`). Verified on Windows only; the unix path is
//! exercised by CI's Linux and macOS test runs, which check that it answers.
#![allow(unsafe_code)]

use super::number::Separators;

/// Characters calc reads as operators, digits' neighbours or brackets, so
/// none of them can be a separator.
const RESERVED: &[char] = &[
    '+', '-', '−', '*', '×', '·', '/', '÷', '^', '%', '!', '(', ')', ';', '√', '°',
];

/// The separators to use: the config's where it sets them, else the
/// system's, else `.` and `,`.
///
/// On unix this switches the process's `LC_NUMERIC` for the duration of the
/// query, which is not thread-safe: call it while no other thread uses the C
/// locale. pikr calls it before it starts any thread, and nothing else in
/// pikr (or its tests) touches the C locale.
pub fn separators(decimal: Option<char>, group: Option<char>) -> Separators {
    resolve(system(), decimal, group)
}

/// Checks a config override, so a bad one fails the config load with a
/// message rather than silently misreading numbers.
pub fn validate(decimal: Option<char>, group: Option<char>) -> Result<(), String> {
    if let Some(d) = decimal
        && d != '.'
        && d != ','
    {
        return Err(format!(
            "calc.decimal_separator must be \".\" or \",\", not {d:?}"
        ));
    }
    if let Some(g) = group
        && (g.is_alphanumeric() || RESERVED.contains(&g))
    {
        return Err(format!(
            "calc.thousands_separator {g:?} is a digit, letter or operator"
        ));
    }
    if let (Some(d), Some(g)) = (decimal, group)
        && d == g
    {
        return Err(format!(
            "calc.decimal_separator and calc.thousands_separator are both {d:?}"
        ));
    }
    Ok(())
}

/// What the system's locale answered: its decimal mark, and its group
/// separator when it has one (the C locale has none).
type System = Option<(char, Option<char>)>;

fn resolve(system: System, decimal: Option<char>, group: Option<char>) -> Separators {
    let system_decimal = system.map(|(d, _)| d).filter(|d| *d == '.' || *d == ',');
    let decimal = decimal.or(system_decimal).unwrap_or('.');
    let system_group = system
        .and_then(|(_, g)| g)
        .filter(|g| !g.is_alphanumeric() && !RESERVED.contains(g));
    let group = group
        .or(system_group)
        .filter(|g| *g != decimal)
        .unwrap_or(if decimal == ',' { '.' } else { ',' });
    Separators { decimal, group }
}

/// The text as a separator, when it is exactly one character.
fn one_char(text: &str) -> Option<char> {
    let mut chars = text.chars();
    let c = chars.next()?;
    chars.next().is_none().then_some(c)
}

#[cfg(windows)]
fn system() -> System {
    use windows::Win32::Globalization::{GetLocaleInfoEx, LOCALE_SDECIMAL, LOCALE_STHOUSAND};
    use windows::core::PCWSTR;

    let read = |kind: u32| -> Option<char> {
        // The documented limit for both values is four characters plus the
        // terminating NUL.
        let mut buffer = [0u16; 8];
        // SAFETY: a null locale name is `LOCALE_NAME_USER_DEFAULT`, and the
        // call writes at most `buffer.len()` UTF-16 units into the slice it is
        // given, returning how many it wrote (NUL included) or 0 on failure.
        let written = unsafe { GetLocaleInfoEx(PCWSTR::null(), kind, Some(&mut buffer)) };
        let len = usize::try_from(written).ok()?.checked_sub(1)?;
        one_char(&String::from_utf16(buffer.get(..len)?).ok()?)
    };
    Some((read(LOCALE_SDECIMAL)?, read(LOCALE_STHOUSAND)))
}

#[cfg(unix)]
fn system() -> System {
    use std::ffi::CStr;

    // SAFETY: `setlocale` with a null locale only reads the current one and
    // returns a pointer to a NUL-terminated string (or null), which is copied
    // before the next call can overwrite it. `localeconv` returns a pointer
    // to a static struct whose string fields are NUL-terminated and stay valid
    // until the next `setlocale`/`localeconv`; both are copied out before the
    // locale is put back. No other thread uses the C locale meanwhile (see
    // `separators`), which is what makes the process-wide switch sound.
    unsafe {
        let current = libc::setlocale(libc::LC_NUMERIC, std::ptr::null());
        let previous = (!current.is_null()).then(|| CStr::from_ptr(current).to_owned());
        if libc::setlocale(libc::LC_NUMERIC, c"".as_ptr()).is_null() {
            return None;
        }
        let conv = libc::localeconv();
        let read = |field: *const libc::c_char| -> Option<char> {
            if field.is_null() {
                return None;
            }
            one_char(CStr::from_ptr(field).to_str().ok()?)
        };
        let answer = if conv.is_null() {
            None
        } else {
            read((*conv).decimal_point).map(|d| (d, read((*conv).thousands_sep)))
        };
        if let Some(previous) = &previous {
            libc::setlocale(libc::LC_NUMERIC, previous.as_ptr());
        }
        answer
    }
}

#[cfg(not(any(unix, windows)))]
fn system() -> System {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seps(decimal: char, group: char) -> Separators {
        Separators { decimal, group }
    }

    #[test]
    fn the_system_answer_is_used_as_given() {
        assert_eq!(resolve(Some(('.', Some(','))), None, None), seps('.', ','));
        assert_eq!(resolve(Some((',', Some('.'))), None, None), seps(',', '.'));
        assert_eq!(
            resolve(Some((',', Some('\u{202f}'))), None, None),
            seps(',', '\u{202f}')
        );
    }

    #[test]
    fn a_missing_or_unusable_answer_falls_back() {
        assert_eq!(resolve(None, None, None), seps('.', ','));
        // The C locale has a decimal point and no grouping.
        assert_eq!(resolve(Some(('.', None)), None, None), seps('.', ','));
        assert_eq!(resolve(Some((',', None)), None, None), seps(',', '.'));
        // A decimal mark calc cannot read is replaced, not trusted.
        assert_eq!(resolve(Some(('٫', Some('٬'))), None, None), seps('.', '٬'));
    }

    #[test]
    fn the_config_wins_and_never_collides() {
        let german = Some((',', Some('.')));
        assert_eq!(resolve(german, Some('.'), None), seps('.', ','));
        assert_eq!(resolve(german, None, Some(' ')), seps(',', ' '));
        assert_eq!(resolve(None, Some(','), Some('\'')), seps(',', '\''));
    }

    #[test]
    fn bad_overrides_are_refused() {
        assert!(validate(Some('.'), Some(',')).is_ok());
        assert!(validate(None, None).is_ok());
        assert!(validate(Some(';'), None).is_err());
        assert!(validate(None, Some('5')).is_err());
        assert!(validate(None, Some('-')).is_err());
        assert!(validate(Some(','), Some(',')).is_err());
    }

    #[test]
    fn the_system_answers() {
        // Whatever this machine's locale is, the query must work and give
        // separators calc can use.
        let found = separators(None, None);
        assert_ne!(found.decimal, found.group);
        assert!(found.decimal == '.' || found.decimal == ',');
    }
}
