//! Fuzzy matcher facade over `nucleo`.
//!
//! Each call to [`Matcher::rank`] returns a `Vec<Match>` ordered by score
//! descending. A `Match` carries the original entry index plus the codepoint
//! positions in the label that matched the query — the picker UI uses those
//! to highlight the matched span.

use nucleo::{Config, Matcher as NucleoMatcher, Utf32Str};

#[derive(Debug, Clone)]
pub struct Match {
    pub index: usize,
    pub score: u16,
    pub positions: Vec<u32>,
}

pub struct Matcher {
    inner: NucleoMatcher,
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
        }
    }

    /// Rank `labels` against `query`. Empty query returns every entry in
    /// original order with score 0 and no positions. Non-matching rows are
    /// dropped.
    pub fn rank(&mut self, labels: &[&str], query: &str) -> Vec<Match> {
        if query.is_empty() {
            return labels
                .iter()
                .enumerate()
                .map(|(i, _)| Match {
                    index: i,
                    score: 0,
                    positions: Vec::new(),
                })
                .collect();
        }

        let mut needle_buf = Vec::new();
        let needle = Utf32Str::new(query, &mut needle_buf);

        let mut out: Vec<Match> = Vec::with_capacity(labels.len());
        for (i, label) in labels.iter().enumerate() {
            let mut hay_buf = Vec::new();
            let haystack = Utf32Str::new(label, &mut hay_buf);
            let mut positions = Vec::new();
            if let Some(score) = self.inner.fuzzy_indices(haystack, needle, &mut positions) {
                out.push(Match {
                    index: i,
                    score,
                    positions,
                });
            }
        }

        out.sort_by(|a, b| b.score.cmp(&a.score).then(a.index.cmp(&b.index)));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_query_returns_all_in_order() {
        let mut m = Matcher::new();
        let out = m.rank(&["a", "b", "c"], "");
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].index, 0);
        assert_eq!(out[2].index, 2);
    }

    #[test]
    fn ranks_by_match_quality() {
        let mut m = Matcher::new();
        let labels = ["Firefox", "Files", "Filezilla"];
        let out = m.rank(&labels, "fir");
        assert!(!out.is_empty());
        assert_eq!(out[0].index, 0); // "Firefox" — closest match
    }

    #[test]
    fn drops_non_matches() {
        let mut m = Matcher::new();
        let out = m.rank(&["foo", "bar"], "xyz");
        assert!(out.is_empty());
    }

    #[test]
    fn match_positions_populated() {
        let mut m = Matcher::new();
        let out = m.rank(&["Firefox"], "fox");
        assert_eq!(out.len(), 1);
        assert!(!out[0].positions.is_empty());
    }
}
