//! Fuzzy matcher facade over `nucleo`.
//!
//! Each call to [`Matcher::rank`] returns a `Vec<Match>` ordered by score
//! descending. A `Match` carries the original entry index plus the codepoint
//! positions in the label / description that matched the query — the picker
//! UI uses those to highlight the matched spans independently.

use nucleo::{Config, Matcher as NucleoMatcher, Utf32Str};

#[derive(Debug, Clone)]
pub struct Match {
    pub index: usize,
    pub score: u16,
    /// Codepoint indices in `entry.label` that matched the query (empty if
    /// the match came from the description only).
    pub positions: Vec<u32>,
    /// Codepoint indices in `entry.description` (when present) that matched
    /// the query. Empty when the label alone explains the hit.
    pub desc_positions: Vec<u32>,
}

pub struct Matcher {
    inner: NucleoMatcher,
    poisoned: bool,
}

impl Default for Matcher {
    fn default() -> Self {
        Self::new()
    }
}

impl Matcher {
    pub fn new() -> Self {
        Self {
            inner: NucleoMatcher::new(Config::DEFAULT),
            poisoned: false,
        }
    }

    /// Rank `entries` (label + optional description) against `query`.
    ///
    /// Empty query returns every entry in original order with score 0 and
    /// no positions. For a non-empty query each entry is matched against
    /// label and description independently — both fields score equally
    /// (raw nucleo score, no field-specific bonus). An entry survives if
    /// either field hits; if both hit, the scores sum.
    pub fn rank(&mut self, entries: &[(&str, Option<&str>)], query: &str) -> Vec<Match> {
        // Lazy-rebuild a poisoned matcher (post-panic recovery). Doing this
        // here rather than inside the catch_unwind avoids dropping the
        // corrupted matcher while its slab allocator is mid-mutation.
        if self.poisoned {
            self.inner = NucleoMatcher::new(Config::DEFAULT);
            self.poisoned = false;
        }
        if query.is_empty() {
            return entries
                .iter()
                .enumerate()
                .map(|(i, _)| Match {
                    index: i,
                    score: 0,
                    positions: Vec::new(),
                    desc_positions: Vec::new(),
                })
                .collect();
        }

        let mut needle_buf = Vec::new();
        let needle = Utf32Str::new(query, &mut needle_buf);

        // Install a no-op panic hook ONCE for the whole rank pass instead of
        // per row. `set_hook` / `take_hook` lock a global mutex; doing it
        // 1800× per keystroke (emoji mode) was the dominant cost and froze
        // the UI thread.
        let prev_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));

        let mut out: Vec<Match> = Vec::with_capacity(entries.len());
        let mut panicked = false;
        'rows: for (i, (label, description)) in entries.iter().enumerate() {
            // Try label and description independently.
            let label_hit = self.try_match(label, needle, &mut panicked);
            if panicked {
                tracing::warn!(index = i, label = %label, "nucleo panicked; aborting rank");
                break 'rows;
            }
            let desc_hit = description.and_then(|d| {
                let h = self.try_match(d, needle, &mut panicked);
                if panicked {
                    // Record but keep this iteration — we still want
                    // label_hit reported; the outer loop bails next round.
                    None
                } else {
                    h
                }
            });
            if panicked {
                tracing::warn!(index = i, "nucleo panicked on description; aborting rank");
                break 'rows;
            }

            match (label_hit, desc_hit) {
                (None, None) => {}
                (Some((ls, lp)), None) => out.push(Match {
                    index: i,
                    score: ls,
                    positions: lp,
                    desc_positions: Vec::new(),
                }),
                (None, Some((ds, dp))) => out.push(Match {
                    index: i,
                    score: ds,
                    positions: Vec::new(),
                    desc_positions: dp,
                }),
                (Some((ls, lp)), Some((ds, dp))) => out.push(Match {
                    index: i,
                    score: ls.saturating_add(ds),
                    positions: lp,
                    desc_positions: dp,
                }),
            }
        }
        if panicked {
            self.poisoned = true;
        }

        // Restore the user-installed panic hook.
        std::panic::set_hook(prev_hook);

        out.sort_by(|a, b| b.score.cmp(&a.score).then(a.index.cmp(&b.index)));
        out
    }

    /// Inner fuzzy match helper. Returns `Some((score, positions))` if the
    /// haystack matched, `None` otherwise. Sets `*panicked` if the nucleo
    /// prefilter assert fired so the caller can short-circuit the rest of
    /// the pass and mark the matcher poisoned.
    fn try_match(
        &mut self,
        haystack: &str,
        needle: Utf32Str<'_>,
        panicked: &mut bool,
    ) -> Option<(u16, Vec<u32>)> {
        if haystack.is_empty() {
            return None;
        }
        let mut hay_buf = Vec::new();
        let hay = Utf32Str::new(haystack, &mut hay_buf);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut positions = Vec::new();
            let score = self.inner.fuzzy_indices(hay, needle, &mut positions);
            (score, positions)
        }));
        match result {
            Ok((Some(score), positions)) => Some((score, positions)),
            Ok((None, _)) => None,
            Err(_) => {
                *panicked = true;
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_desc<'a>(labels: &'a [&'a str]) -> Vec<(&'a str, Option<&'a str>)> {
        labels.iter().map(|l| (*l, None)).collect()
    }

    #[test]
    fn empty_query_returns_all_in_order() {
        let mut m = Matcher::new();
        let pairs = no_desc(&["a", "b", "c"]);
        let out = m.rank(&pairs, "");
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].index, 0);
        assert_eq!(out[2].index, 2);
    }

    #[test]
    fn ranks_by_match_quality() {
        let mut m = Matcher::new();
        let pairs = no_desc(&["Firefox", "Files", "Filezilla"]);
        let out = m.rank(&pairs, "fir");
        assert!(!out.is_empty());
        assert_eq!(out[0].index, 0); // "Firefox" — closest match
    }

    #[test]
    fn drops_non_matches() {
        let mut m = Matcher::new();
        let pairs = no_desc(&["foo", "bar"]);
        let out = m.rank(&pairs, "xyz");
        assert!(out.is_empty());
    }

    #[test]
    fn rofi_does_not_match_fonts_or_tf2() {
        let mut m = Matcher::new();
        let pairs = no_desc(&["Fonts", "Team Fortress 2"]);
        let out = m.rank(&pairs, "rofi");
        assert!(
            out.is_empty(),
            "expected no matches, got: {:?}",
            out.iter().map(|x| x.index).collect::<Vec<_>>()
        );
    }

    #[test]
    fn match_positions_populated() {
        let mut m = Matcher::new();
        let pairs = no_desc(&["Firefox"]);
        let out = m.rank(&pairs, "fox");
        assert_eq!(out.len(), 1);
        assert!(!out[0].positions.is_empty());
        assert!(
            out[0].desc_positions.is_empty(),
            "label-only match must not populate desc_positions"
        );
    }

    /// Regression for the nucleo 0.5 `should have been caught by prefilter`
    /// hang: after a panic the matcher is marked poisoned and the *next*
    /// call must rebuild and still return correct results.
    #[test]
    fn poisoned_matcher_rebuilds_on_next_rank() {
        let mut m = Matcher::new();
        m.poisoned = true;
        let pairs = no_desc(&["Firefox", "Files"]);
        let out = m.rank(&pairs, "fir");
        assert!(!m.poisoned, "rank() must clear the poison flag");
        assert!(!out.is_empty(), "rebuilt matcher must still match");
        assert_eq!(out[0].index, 0);
    }

    /// Empty-query path runs before the poison check used to be wired —
    /// guard against regressing the rebuild order, since a poisoned matcher
    /// might be reused on the next non-empty query.
    #[test]
    fn poisoned_matcher_clears_even_on_empty_query() {
        let mut m = Matcher::new();
        m.poisoned = true;
        let pairs = no_desc(&["a", "b"]);
        let _ = m.rank(&pairs, "");
        assert!(
            !m.poisoned,
            "even the empty-query fast-path must reset the matcher"
        );
    }

    /// Entry with a label miss but a description hit must still appear.
    /// Caught the matcher's "label OR description" union semantics.
    #[test]
    fn matches_via_description_when_label_misses() {
        let mut m = Matcher::new();
        let pairs: Vec<(&str, Option<&str>)> = vec![
            ("Firefox", Some("Web Browser")),
            ("KCalc", Some("Scientific calculator")),
        ];
        let out = m.rank(&pairs, "browser");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].index, 0);
        assert!(
            out[0].positions.is_empty(),
            "label didn't match, so label positions must be empty"
        );
        assert!(
            !out[0].desc_positions.is_empty(),
            "description positions must be populated for desc-only hits"
        );
    }

    /// Label and description score equally. Two entries that share the
    /// query in different fields must report the same score — only
    /// nucleo's intrinsic scoring (substring length, word boundaries, etc.)
    /// is allowed to break the tie.
    #[test]
    fn label_and_desc_score_equally() {
        let mut m = Matcher::new();
        // Same surrounding string in both fields so nucleo gives the same
        // raw score for both hits; one has it in the label, the other in
        // the description.
        let pairs: Vec<(&str, Option<&str>)> = vec![
            ("audio settings", Some("blank")),
            ("blank", Some("audio settings")),
        ];
        let out = m.rank(&pairs, "audio");
        assert_eq!(out.len(), 2);
        assert_eq!(
            out[0].score, out[1].score,
            "label vs description hit must produce identical scores; got {:?}",
            out
        );
    }

    /// Both fields hit → both position vectors populated, score is the sum
    /// (weighted label + raw desc).
    #[test]
    fn both_fields_can_match_together() {
        let mut m = Matcher::new();
        let pairs: Vec<(&str, Option<&str>)> = vec![("Editor", Some("text editor app"))];
        let out = m.rank(&pairs, "edit");
        assert_eq!(out.len(), 1);
        assert!(!out[0].positions.is_empty());
        assert!(!out[0].desc_positions.is_empty());
    }
}
