//! Query-text helpers: caret math over char indices, and the text pikr
//! shows in place of the query (password masking, the empty-state hint).

/// Char-index → byte-offset into `s`. Clamps past-the-end to `s.len()`.
/// All caret math runs in char indices (codepoint-aware); String operations
/// need byte offsets.
pub(crate) fn char_idx_to_byte(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map(|(b, _)| b)
        .unwrap_or(s.len())
}

/// Ctrl-W word boundary: walk back from `cur` (char index), skip trailing
/// whitespace, then skip the run of non-whitespace before it. Returns the
/// new caret position.
pub(crate) fn word_boundary_back(s: &str, cur: usize) -> usize {
    let chars: Vec<char> = s.chars().collect();
    let mut i = cur.min(chars.len());
    while i > 0 && chars[i - 1].is_whitespace() {
        i -= 1;
    }
    while i > 0 && !chars[i - 1].is_whitespace() {
        i -= 1;
    }
    i
}

/// Replace every character in `text` with ● (U+25CF) when `enabled` is true.
/// The returned `String` has the same codepoint count as `text` so cursor
/// positioning stays correct.
pub(crate) fn mask_password(enabled: bool, text: &str) -> String {
    if enabled {
        "\u{25CF}".repeat(text.chars().count())
    } else {
        text.to_owned()
    }
}

/// Empty-state hint for a query with no matching rows. `--password` draws
/// no list, so this never echoes a secret.
pub(crate) fn empty_state_text(query: &str) -> String {
    if query.is_empty() {
        "No entries.".to_owned()
    } else {
        format!("No results for \u{201C}{query}\u{201D}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn char_idx_to_byte_ascii() {
        let s = "hello";
        assert_eq!(char_idx_to_byte(s, 0), 0);
        assert_eq!(char_idx_to_byte(s, 3), 3);
        assert_eq!(char_idx_to_byte(s, 5), 5); // past-the-end
        assert_eq!(char_idx_to_byte(s, 99), 5); // clamps
    }

    #[test]
    fn char_idx_to_byte_multibyte() {
        // "héllo" — é is 2 bytes in UTF-8.
        let s = "héllo";
        assert_eq!(char_idx_to_byte(s, 0), 0);
        assert_eq!(char_idx_to_byte(s, 1), 1); // before é
        assert_eq!(char_idx_to_byte(s, 2), 3); // after é (skips 2 bytes)
        assert_eq!(char_idx_to_byte(s, 5), 6);
    }

    #[test]
    fn word_boundary_back_skips_trailing_space() {
        // cursor at end of "foo bar  " — Ctrl-W should land at end of "foo "
        // (skip trailing ws, then skip "bar").
        let s = "foo bar  ";
        assert_eq!(word_boundary_back(s, 9), 4);
    }

    #[test]
    fn word_boundary_back_inside_word() {
        // cursor at "fo|o bar" → Ctrl-W deletes "fo", lands at 0.
        let s = "foo bar";
        assert_eq!(word_boundary_back(s, 2), 0);
    }

    #[test]
    fn word_boundary_back_at_start_noop() {
        let s = "foo";
        assert_eq!(word_boundary_back(s, 0), 0);
    }

    #[test]
    fn word_boundary_back_only_whitespace() {
        // " | " → walks back past all whitespace to 0.
        let s = "   ";
        assert_eq!(word_boundary_back(s, 3), 0);
    }

    // ── empty_state_text tests ────────────────────────────────────────────

    #[test]
    fn empty_state_no_query() {
        assert_eq!(empty_state_text(""), "No entries.");
    }

    #[test]
    fn empty_state_echoes_query() {
        assert_eq!(
            empty_state_text("fire"),
            "No results for \u{201C}fire\u{201D}"
        );
    }

    // ── mask_password tests ───────────────────────────────────────────────

    #[test]
    fn mask_password_disabled_returns_original() {
        assert_eq!(mask_password(false, "hello"), "hello");
        assert_eq!(mask_password(false, ""), "");
    }

    #[test]
    fn mask_password_enabled_replaces_with_bullets() {
        assert_eq!(mask_password(true, "abc"), "●●●");
        assert_eq!(mask_password(true, ""), "");
    }

    #[test]
    fn mask_password_preserves_char_count_for_multibyte() {
        // Each multibyte char must produce exactly one ● so cursor index
        // arithmetic in with_cursor stays correct.
        let s = "héllo"; // 5 codepoints, 6 bytes
        let masked = mask_password(true, s);
        assert_eq!(masked.chars().count(), s.chars().count());
        assert_eq!(masked, "●●●●●");
    }

    #[test]
    fn mask_password_single_char() {
        assert_eq!(mask_password(true, "x"), "●");
    }
}
