//! Thin floem-Style adapter over a parsed `hjkl_css::Stylesheet`.
//!
//! pikr ships its own adapter rather than depending on `hjkl-css-gui`
//! because the latter is built against floem 0.2.0 stable and breaks
//! under pikr's `[patch.crates-io]` redirect to current `lapce/floem`
//! main (renames: `Weight` → `FontWeight`, `text::Style` →
//! `text::FontStyle`, `Color::rgba8` → `Color::from_rgba8`, etc).
//! Swap back to `hjkl-css-gui` once floem main releases and the upstream
//! crate is patched.
//!
//! The adapter only handles the small subset of CSS pikr actually uses.
//! Reactive styling (per-row selected bg, vim-mode chip bg, match
//! highlight colours) stays inline in `ui/view.rs`.

use floem::peniko::Color;
use floem::style::Style;
use hjkl_css::{Color as CssColor, Length, Node, Stylesheet, Value, parse};

use crate::config::Theme;

/// A memo key: the (element, classes) pair a style evaluation is for.
type MemoKey = (String, Vec<String>);
/// Sorted (specificity, source-index) rule refs for one key.
type MatchedRules = Vec<(u32, usize)>;

/// A parsed stylesheet plus a per-(element, classes) memo of the sorted
/// matched rules. The sheet is immutable after build_stylesheet, so the
/// matched-rule set for a given (element, classes) pair is constant and
/// only needs computing once — previously apply() re-scanned and re-sorted
/// every rule on every style evaluation (per row, per keystroke).
pub struct Sheet {
    inner: Stylesheet,
    /// (element, classes) -> sorted (specificity, source-index) rule refs.
    matched: std::cell::RefCell<std::collections::HashMap<MemoKey, MatchedRules>>,
}

/// Build the runtime stylesheet from the theme-substituted `default.css`
/// (see [`crate::ui::theme_css::stylesheet_source`]).
pub fn build_stylesheet(theme: &Theme) -> Sheet {
    let css = crate::ui::theme_css::stylesheet_source(theme);

    Sheet {
        inner: parse(&css).unwrap_or_else(|err| {
            tracing::error!(?err, "default.css failed to parse — using empty stylesheet");
            Stylesheet { rules: Vec::new() }
        }),
        matched: std::cell::RefCell::new(std::collections::HashMap::new()),
    }
}

/// Apply every matching rule's declarations to `s`, in source order.
/// Specificity is honoured: higher-specificity rules override lower
/// when both target the same property. The matched-rule set for a given
/// (element, classes) pair is computed once and memoized in the sheet —
/// later calls walk the cached list instead of re-scanning and re-sorting
/// every rule.
pub fn apply(s: Style, sheet: &Sheet, element: &str, classes: &[&str]) -> Style {
    let node = Node { element, classes };
    let key = (
        element.to_string(),
        classes.iter().map(|c| c.to_string()).collect::<Vec<_>>(),
    );
    // Borrow once for the whole call: the memo lives alongside the sheet and
    // apply() never recurses into itself, so a single RefMut is safe.
    let mut memo = sheet.matched.borrow_mut();
    let matches = memo.entry(key).or_insert_with(|| {
        // Collect matching rules with their specificity. Iterate in source
        // order so equal-specificity rules cascade later-wins; sort_by_key
        // ensures the *order* is (specificity asc, source order asc) by
        // relying on `sort_by_key` being stable.
        let mut matches: Vec<(u32, usize)> = sheet
            .inner
            .rules
            .iter()
            .enumerate()
            .filter_map(|(idx, rule)| {
                rule.selectors
                    .iter()
                    .filter_map(|sel| {
                        if sel.matches(&node, &[], &[], None) {
                            Some(sel.specificity())
                        } else {
                            None
                        }
                    })
                    .max()
                    .map(|sp| (sp, idx))
            })
            .collect();
        matches.sort_by_key(|(sp, idx)| (*sp, *idx));
        matches
    });

    let mut out = s;
    for (_, idx) in matches.iter() {
        let rule = &sheet.inner.rules[*idx];
        for decl in &rule.declarations {
            out = apply_decl(out, decl);
        }
    }
    out
}

fn apply_decl(s: Style, decl: &hjkl_css::Declaration) -> Style {
    match decl.property.as_str() {
        "color" => match &decl.value {
            Value::Color(c) => s.color(to_floem_color(c)),
            _ => s,
        },
        "background-color" => match &decl.value {
            Value::Color(c) => s.background(to_floem_color(c)),
            _ => s,
        },
        "width" => match &decl.value {
            Value::Length(Length::Px(v)) => s.width(*v),
            Value::Length(Length::Percent(v)) => s.width_pct(*v),
            _ => s,
        },
        "height" => match &decl.value {
            Value::Length(Length::Px(v)) => s.height(*v),
            Value::Length(Length::Percent(v)) => s.height_pct(*v),
            _ => s,
        },
        "padding" => match &decl.value {
            Value::Length(l) => apply_padding_4(s, [*l, *l, *l, *l]),
            Value::LengthSet(set) => match hjkl_css::expand_sides(set) {
                Some(sides) => apply_padding_4(s, sides),
                None => s,
            },
            _ => s,
        },
        "margin" => match &decl.value {
            Value::Length(l) => apply_margin_4(s, [*l, *l, *l, *l]),
            Value::LengthSet(set) => match hjkl_css::expand_sides(set) {
                Some(sides) => apply_margin_4(s, sides),
                None => s,
            },
            _ => s,
        },
        "border" => match &decl.value {
            Value::Border { width, color } => {
                let width_px = width.as_px().unwrap_or(0.0);
                s.border(width_px).border_color(to_floem_color(color))
            }
            _ => s,
        },
        "border-radius" => match &decl.value {
            Value::Length(Length::Px(v)) => s.border_radius(*v),
            _ => s,
        },
        "border-color" => match &decl.value {
            Value::Color(c) => s.border_color(to_floem_color(c)),
            _ => s,
        },
        "align-items" => match &decl.value {
            Value::Keyword(k) => match k.as_str() {
                "center" => s.items_center(),
                "start" => s.items_start(),
                "end" => s.items_end(),
                _ => s,
            },
            _ => s,
        },
        "justify-content" => match &decl.value {
            Value::Keyword(k) => match k.as_str() {
                "center" => s.justify_center(),
                "start" => s.justify_start(),
                "end" => s.justify_end(),
                _ => s,
            },
            _ => s,
        },
        "font-family" => match &decl.value {
            Value::FontFamilyList(list) if !list.is_empty() => s.font_family(list.join(", ")),
            _ => s,
        },
        "font-size" => match &decl.value {
            Value::Length(Length::Px(v)) => s.font_size(*v as f32),
            _ => s,
        },
        _ => s,
    }
}

fn apply_padding_4(s: Style, sides: [Length; 4]) -> Style {
    let [t, r, b, l] = sides.map(|x| x.as_px().unwrap_or(0.0));
    s.padding_top(t)
        .padding_right(r)
        .padding_bottom(b)
        .padding_left(l)
}

fn apply_margin_4(s: Style, sides: [Length; 4]) -> Style {
    let [t, r, b, l] = sides.map(|x| x.as_px().unwrap_or(0.0));
    s.margin_top(t)
        .margin_right(r)
        .margin_bottom(b)
        .margin_left(l)
}

fn to_floem_color(c: &CssColor) -> Color {
    Color::from_rgba8(c.r, c.g, c.b, c.a)
}

#[cfg(test)]
mod tests {
    use super::*;

    // floem's Style is neither PartialEq nor publicly inspectable (its prop
    // map is pub(crate) to floem), so "equal Style values" can't be asserted
    // directly. Instead the test pins the observable contract of the memo:
    // the cached (specificity, source-index) list — the exact thing that
    // drives declaration application and cascade order.
    #[test]
    fn apply_memoizes_sorted_matches_per_key() {
        let sheet = build_stylesheet(&Theme::default());
        assert!(sheet.matched.borrow().is_empty());

        // First call computes and stores the matched-rule list for the key.
        let _ = apply(Style::new(), &sheet, "div", &["icon"]);
        assert_eq!(sheet.matched.borrow().len(), 1);
        let key = ("div".to_string(), vec!["icon".to_string()]);
        let first = sheet
            .matched
            .borrow()
            .get(&key)
            .cloned()
            .expect("memo entry for div.icon");
        // default.css has an `.icon` rule — apply() must really have matched
        // and walked it, not stored an empty list.
        assert!(!first.is_empty(), "div.icon should match the .icon rule");
        // Cascade order: (specificity asc, source index asc) — identical to
        // the pre-memo sort, so the applied declarations are unchanged.
        let mut sorted = first.clone();
        sorted.sort_by_key(|(sp, idx)| (*sp, *idx));
        assert_eq!(first, sorted);

        // Second call with the same key must hit the memo: no recompute, no
        // new entry, and it returns the identical stored list.
        let _ = apply(Style::new(), &sheet, "div", &["icon"]);
        assert_eq!(sheet.matched.borrow().len(), 1);
        let second = sheet
            .matched
            .borrow()
            .get(&key)
            .cloned()
            .expect("memo entry still present");
        assert_eq!(first, second);

        // A second sheet built from the same theme computes the identical
        // list cold — the memo result is deterministic per (sheet, key).
        let sheet2 = build_stylesheet(&Theme::default());
        let _ = apply(Style::new(), &sheet2, "div", &["icon"]);
        let other = sheet2
            .matched
            .borrow()
            .get(&key)
            .cloned()
            .expect("memo entry for second sheet");
        assert_eq!(first, other);

        // A different (element, classes) pair is a separate memo key.
        let _ = apply(Style::new(), &sheet, "label", &["mode-name"]);
        assert_eq!(sheet.matched.borrow().len(), 2);
    }
}
