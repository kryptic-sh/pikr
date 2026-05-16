//! emoji mode — searchable emoji picker via the `emojis` crate.

use super::{Entry, Mode, Payload};
use anyhow::Result;

#[derive(Default)]
pub struct Emoji;

impl Mode for Emoji {
    fn name(&self) -> &'static str {
        "emoji"
    }

    fn collect(&mut self) -> Result<Vec<Entry>> {
        let entries = emojis::iter()
            .map(|e| Entry {
                label: e.as_str().to_string(),
                description: Some(e.name().to_string()),
                payload: Payload::Stdout(e.as_str().to_string()),
            })
            .collect();
        Ok(entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iter_yields_entries() {
        let mut mode = Emoji;
        let entries = mode.collect().unwrap();
        assert!(
            entries.len() > 1000,
            "expected >1000 emoji entries, got {}",
            entries.len()
        );
        let first = &entries[0];
        // Must be a non-empty string containing at least one grapheme cluster.
        assert!(
            !first.label.is_empty(),
            "first emoji label must not be empty"
        );
    }
}
