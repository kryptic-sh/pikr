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
    config: Config,
    poisoned: bool,
}

impl Default for Matcher {
    fn default() -> Self {
        Self::new()
    }
}

impl Matcher {
    pub fn new() -> Self {
        Self::with_case_sensitive(false)
    }

    pub fn with_case_sensitive(case_sensitive: bool) -> Self {
        let mut config = Config::DEFAULT;
        config.ignore_case = !case_sensitive;
        Self {
            inner: NucleoMatcher::new(config.clone()),
            config,
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
            self.inner = NucleoMatcher::new(self.config.clone());
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
        // `nucleo_dead` is set once any row panics nucleo. The internal
        // slab is corrupted after the panic, so the rest of the pass
        // walks substring_match instead of try_match. Subsequent rank
        // calls rebuild via the `poisoned` flag at the top of the fn.
        let mut nucleo_dead = false;
        for (i, (label, description)) in entries.iter().enumerate() {
            let label_hit = self.match_field(label, needle, query, &mut nucleo_dead, i, "label");
            let desc_hit = description.and_then(|d| {
                self.match_field(d, needle, query, &mut nucleo_dead, i, "description")
            });

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
        if nucleo_dead {
            self.poisoned = true;
        }

        // Restore the user-installed panic hook.
        std::panic::set_hook(prev_hook);

        out.sort_by(|a, b| b.score.cmp(&a.score).then(a.index.cmp(&b.index)));
        out
    }

    /// Match `haystack` against `needle` for one row, falling back to a
    /// substring scan when nucleo panics (or has been disabled for the
    /// remainder of the pass). `nucleo_dead` is set on first panic.
    fn match_field(
        &mut self,
        haystack: &str,
        needle: Utf32Str<'_>,
        query: &str,
        nucleo_dead: &mut bool,
        row: usize,
        field: &'static str,
    ) -> Option<(u16, Vec<u32>)> {
        if *nucleo_dead {
            return substring_match(haystack, query, self.config.ignore_case);
        }
        let mut panicked = false;
        let hit = self.try_match(haystack, needle, &mut panicked);
        if panicked {
            tracing::warn!(
                index = row,
                field,
                haystack = %haystack,
                query = %query,
                "nucleo panicked; falling back to substring for the rest of this rank pass"
            );
            *nucleo_dead = true;
            // nucleo had a chance and gave up — try substring instead. For
            // queries like \"Thunder\" against \"Thunderbird\" this still
            // yields the obvious match the user expects.
            return substring_match(haystack, query, self.config.ignore_case);
        }
        hit
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

/// Substring scan used as a fallback when nucleo panics inside its prefilter.
/// Emits codepoint positions covering the matched span; arbitrary score is high
/// enough to outrank typical fuzzy hits so direct-substring queries surface near
/// the top.
fn substring_match(haystack: &str, query: &str, ignore_case: bool) -> Option<(u16, Vec<u32>)> {
    if haystack.is_empty() || query.is_empty() {
        return None;
    }
    let (cp_start, cp_count) = if ignore_case {
        let h_lower = haystack.to_lowercase();
        let q_lower = query.to_lowercase();
        let byte_idx = h_lower.find(&q_lower)?;
        (h_lower[..byte_idx].chars().count(), q_lower.chars().count())
    } else {
        let byte_idx = haystack.find(query)?;
        (haystack[..byte_idx].chars().count(), query.chars().count())
    };
    let positions: Vec<u32> = (cp_start..cp_start + cp_count).map(|i| i as u32).collect();
    // 200 is comfortably above typical nucleo scores (~100-180 range for
    // short queries). Substring matches are high-quality by definition.
    Some((200, positions))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_desc<'a>(labels: &'a [&'a str]) -> Vec<(&'a str, Option<&'a str>)> {
        labels.iter().map(|l| (*l, None)).collect()
    }

    #[test]
    fn case_insensitive_matcher_matches_different_case() {
        let mut matcher = Matcher::with_case_sensitive(false);
        let pairs = no_desc(&["Foo"]);

        assert_eq!(matcher.rank(&pairs, "foo").len(), 1);
    }

    #[test]
    fn case_sensitive_matcher_rejects_different_case() {
        let mut matcher = Matcher::with_case_sensitive(true);
        let pairs = no_desc(&["Foo"]);

        assert!(matcher.rank(&pairs, "foo").is_empty());
        assert_eq!(matcher.rank(&pairs, "Foo").len(), 1);
    }

    #[test]
    fn case_sensitive_fallback_rejects_different_case() {
        let mut matcher = Matcher::with_case_sensitive(true);
        let mut needle_buf = Vec::new();
        let needle = Utf32Str::new("Thunder", &mut needle_buf);
        let mut nucleo_dead = true;

        assert!(
            matcher
                .match_field(
                    "thunderbird",
                    needle,
                    "Thunder",
                    &mut nucleo_dead,
                    0,
                    "label",
                )
                .is_none()
        );
    }

    #[test]
    fn poisoned_case_sensitive_matcher_preserves_configuration() {
        let mut matcher = Matcher::with_case_sensitive(true);
        matcher.poisoned = true;
        let pairs = no_desc(&["Foo"]);

        assert!(matcher.rank(&pairs, "foo").is_empty());
        assert!(!matcher.poisoned);
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

    /// Regression for "Thunder" vs "Thunderbird": the substring query that
    /// stops exactly at a word boundary inside the haystack must match, just
    /// like any one-character-shorter or one-character-longer query around
    /// that boundary does. Caught a real-world bug where typing "Thunderb"
    /// or "Thunde" matched but typing "Thunder" did not.
    #[test]
    fn substring_query_at_word_boundary_matches() {
        let mut m = Matcher::new();
        let pairs = no_desc(&["Thunderbird"]);
        for q in ["Thunde", "Thunder", "Thunderb", "Thunderbird"] {
            let out = m.rank(&pairs, q);
            assert_eq!(
                out.len(),
                1,
                "query {q:?} against \"Thunderbird\" must match; got {} results",
                out.len(),
            );
        }
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
