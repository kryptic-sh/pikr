//! Fuzzy matcher facade over `nucleo`.
//!
//! Each call to [`Matcher::rank`] returns a `Vec<Match>` ordered by score
//! descending. A `Match` carries the original entry index plus the codepoint
//! positions in the label / description that matched the query — the picker
//! UI uses those to highlight the matched spans independently.

use std::rc::Rc;

use nucleo::pattern::{Atom, AtomKind, CaseMatching, Normalization};
use nucleo::{Config, Matcher as NucleoMatcher, Utf32Str};
use unicode_normalization::UnicodeNormalization;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Debug, Clone)]
pub struct Match {
    pub index: usize,
    pub score: u16,
    /// Codepoint indices in `entry.label` that matched the query (empty if
    /// the match came from the description only).
    pub positions: Rc<Vec<u32>>,
    /// Codepoint indices in `entry.description` (when present) that matched
    /// the query. Empty when the label alone explains the hit.
    pub desc_positions: Rc<Vec<u32>>,
}

pub struct Matcher {
    inner: NucleoMatcher,
    case_matching: CaseMatching,
    /// Scratch buffer for the rare grapheme→codepoint index conversion.
    scratch: Vec<u32>,
    /// Reused buffer for the per-field non-ASCII text reduction.
    text_buf: Vec<char>,
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
        Self {
            inner: NucleoMatcher::new(Config::DEFAULT),
            case_matching: if case_sensitive {
                CaseMatching::Respect
            } else {
                CaseMatching::Ignore
            },
            scratch: Vec::new(),
            text_buf: Vec::new(),
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
        if query.is_empty() {
            let empty: Rc<Vec<u32>> = Rc::new(Vec::new());
            return entries
                .iter()
                .enumerate()
                .map(|(i, _)| Match {
                    index: i,
                    score: 0,
                    positions: empty.clone(),
                    desc_positions: empty.clone(),
                })
                .collect();
        }

        let query: String = query.nfc().collect();
        let query_len = query.chars().count();
        let atom = Atom::new(
            &query,
            self.case_matching,
            Normalization::Smart,
            AtomKind::Fuzzy,
            false,
        );
        let empty: Rc<Vec<u32>> = Rc::new(Vec::new());
        let mut out: Vec<Match> = Vec::with_capacity(entries.len());
        // Reused across entries — cleared per entry so a survivor's positions
        // never bleed into the next entry. Survivors clone into their Rc; the
        // per-entry allocation churn is gone.
        let mut lp = Vec::with_capacity(query_len);
        let mut dp = Vec::with_capacity(query_len);
        for (i, (label, description)) in entries.iter().enumerate() {
            lp.clear();
            dp.clear();
            let label_score = self.match_field(&atom, label, &mut lp);
            let desc_score = description.and_then(|text| self.match_field(&atom, text, &mut dp));

            match (label_score, desc_score) {
                (None, None) => {}
                (Some(ls), None) => out.push(Match {
                    index: i,
                    score: ls,
                    positions: Rc::new(lp.clone()),
                    desc_positions: empty.clone(),
                }),
                (None, Some(ds)) => out.push(Match {
                    index: i,
                    score: ds,
                    positions: empty.clone(),
                    desc_positions: Rc::new(dp.clone()),
                }),
                (Some(ls), Some(ds)) => out.push(Match {
                    index: i,
                    score: ls.saturating_add(ds),
                    positions: Rc::new(lp.clone()),
                    desc_positions: Rc::new(dp.clone()),
                }),
            }
        }

        out.sort_by(|a, b| b.score.cmp(&a.score).then(a.index.cmp(&b.index)));
        out
    }

    fn match_field(&mut self, atom: &Atom, text: &str, positions: &mut Vec<u32>) -> Option<u16> {
        if text.is_empty() {
            return None;
        }
        let utf32 = if text.is_ascii() {
            Utf32Str::Ascii(text.as_bytes())
        } else {
            self.text_buf.clear();
            self.text_buf.extend(
                text.graphemes(true)
                    .map(|grapheme| grapheme.nfc().next().expect("graphemes must be non-empty")),
            );
            Utf32Str::Unicode(&self.text_buf)
        };
        positions.clear();
        let score = atom.indices(utf32, &mut self.inner, positions)?;
        if text.is_ascii() {
            Some(score)
        } else {
            self.scratch.clear();
            grapheme_positions_to_codepoints(text, positions, &mut self.scratch);
            std::mem::swap(positions, &mut self.scratch);
            Some(score)
        }
    }
}

fn grapheme_positions_to_codepoints(text: &str, positions: &[u32], out: &mut Vec<u32>) {
    let mut codepoint = 0_u32;
    for (grapheme_index, grapheme) in text.graphemes(true).enumerate() {
        let codepoint_count = grapheme.chars().count() as u32;
        if positions.contains(&(grapheme_index as u32)) {
            out.extend(codepoint..codepoint + codepoint_count);
        }
        codepoint += codepoint_count;
    }
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
    fn case_insensitive_unicode_query_matches_different_case() {
        let mut matcher = Matcher::with_case_sensitive(false);
        let pairs = no_desc(&["Äpfel"]);

        let matches = matcher.rank(&pairs, "äpfel");

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].positions.as_slice(), [0, 1, 2, 3, 4]);
    }

    #[test]
    fn canonical_equivalent_queries_have_identical_results() {
        let mut matcher = Matcher::new();
        let decomposed = "e\u{301}";
        let entries = [
            ("e🙂", Some("e🙂")),
            ("é🙂", Some(decomposed)),
            ("prefix é🙂", Some("prefix e\u{301}🙂")),
        ];

        let composed_matches = matcher.rank(&entries, "é");
        let decomposed_matches = matcher.rank(&entries, decomposed);
        let snapshot = |matches: &[Match]| {
            matches
                .iter()
                .map(|matched| {
                    (
                        matched.index,
                        matched.score,
                        matched.positions.clone(),
                        matched.desc_positions.clone(),
                    )
                })
                .collect::<Vec<_>>()
        };

        assert_eq!(snapshot(&composed_matches), snapshot(&decomposed_matches));
        let mut matched_indices: Vec<_> = composed_matches
            .iter()
            .map(|matched| matched.index)
            .collect();
        matched_indices.sort_unstable();
        assert_eq!(matched_indices, [1, 2]);

        let exact = composed_matches
            .iter()
            .find(|matched| matched.index == 1)
            .unwrap();
        assert_eq!(exact.positions.as_slice(), [0]);
        assert_eq!(exact.desc_positions.as_slice(), [0, 1]);
        let prefixed = composed_matches
            .iter()
            .find(|matched| matched.index == 2)
            .unwrap();
        assert_eq!(prefixed.positions.as_slice(), [7]);
        assert_eq!(prefixed.desc_positions.as_slice(), [7, 8]);
    }

    #[test]
    fn decomposed_unicode_matches_identical_label_and_description() {
        let mut matcher = Matcher::new();
        let decomposed = "e\u{301}";
        let pairs = [(decomposed, Some(decomposed))];

        let matches = matcher.rank(&pairs, decomposed);

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].positions.as_slice(), [0, 1]);
        assert_eq!(matches[0].desc_positions.as_slice(), [0, 1]);
    }

    #[test]
    fn grapheme_match_positions_convert_to_codepoints() {
        let mut matcher = Matcher::new();
        let family = "👨‍👩‍👧";
        let label = format!("{family}x");
        let pairs = [(label.as_str(), None)];

        let suffix = matcher.rank(&pairs, "x");
        assert_eq!(suffix[0].positions.as_slice(), [5]);

        let grapheme = matcher.rank(&pairs, family);
        assert_eq!(grapheme[0].positions.as_slice(), [0, 1, 2, 3, 4]);
    }

    #[test]
    fn case_sensitive_matcher_rejects_different_case() {
        let mut matcher = Matcher::with_case_sensitive(true);
        let pairs = no_desc(&["Foo"]);

        assert!(matcher.rank(&pairs, "foo").is_empty());
        assert_eq!(matcher.rank(&pairs, "Foo").len(), 1);
    }

    #[test]
    fn case_insensitive_uppercase_query_matches_both_cases() {
        let mut matcher = Matcher::with_case_sensitive(false);
        let pairs = no_desc(&["Thunderbird", "thunderbird"]);

        let matches = matcher.rank(&pairs, "Thunder");

        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].positions.as_slice(), [0, 1, 2, 3, 4, 5, 6]);
        assert_eq!(matches[1].positions.as_slice(), [0, 1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn query_is_one_literal_atom() {
        let mut matcher = Matcher::new();
        let pairs = no_desc(&["foo bar", "bar foo", "foo ^bar", "foo bar$"]);

        let spaced = matcher.rank(&pairs, "foo bar");
        let mut spaced_indices: Vec<_> = spaced.iter().map(|matched| matched.index).collect();
        spaced_indices.sort_unstable();
        assert_eq!(spaced_indices, [0, 2, 3]);

        let syntax = matcher.rank(&pairs, "^bar");
        assert_eq!(
            syntax
                .iter()
                .map(|matched| matched.index)
                .collect::<Vec<_>>(),
            [2]
        );
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

    #[test]
    fn uppercase_query_matches_lowercase_description() {
        let mut matcher = Matcher::with_case_sensitive(false);
        let pairs = [("Mail", Some("thunderbird client"))];

        let matches = matcher.rank(&pairs, "Thunder");

        assert_eq!(matches.len(), 1);
        assert!(matches[0].positions.is_empty());
        assert_eq!(matches[0].desc_positions.as_slice(), [0, 1, 2, 3, 4, 5, 6]);
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

    #[test]
    fn per_entry_positions_do_not_leak_across_entries() {
        // Scratch reuse must clear positions between entries: the second
        // entry's match must not inherit the first's positions.
        let mut m = Matcher::new();
        let pairs = no_desc(&["firefox", "prefix fox tail"]);
        let out = m.rank(&pairs, "fox");
        assert_eq!(out.len(), 2);
        let second = out
            .iter()
            .find(|m| m.index == 1)
            .expect("second entry must match");
        assert_eq!(second.positions.as_slice(), [7, 8, 9]);
    }
}
