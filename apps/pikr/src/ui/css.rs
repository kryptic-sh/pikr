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

/// Build the runtime stylesheet by substituting theme colours into the
/// `default.css` template. Computed colours (`ex_bg`, `status_bg`,
/// `count_bg`) mirror the `blend()` calls that were inline in
/// `ui/view.rs` before the CSS migration.
pub fn build_stylesheet(theme: &Theme) -> Stylesheet {
    let accent = parse_hex(&theme.accent);
    let muted = parse_hex(&theme.muted);
    let selected_bg = parse_hex(&theme.selected_bg);
    // The ex bar and status bar share the same derived background.
    let ex_bg = blend_hex(muted, selected_bg, 0.15);
    let status_bg = ex_bg;
    let count_bg = blend_hex(accent, selected_bg, 0.25);

    let css = include_str!("styles/default.css")
        .replace("{bg}", &theme.bg)
        .replace("{fg}", &theme.fg)
        .replace("{accent}", &theme.accent)
        .replace("{muted}", &theme.muted)
        .replace("{selected_bg}", &theme.selected_bg)
        .replace("{ex_bg}", &hex(ex_bg))
        .replace("{status_bg}", &hex(status_bg))
        .replace("{count_bg}", &hex(count_bg))
        .replace("{font_family}", &theme.font)
        .replace("{font_size}", &theme.font_size.to_string());

    parse(&css).unwrap_or_else(|err| {
        tracing::error!(?err, "default.css failed to parse — using empty stylesheet");
        Stylesheet { rules: Vec::new() }
    })
}

/// Apply every matching rule's declarations to `s`, in source order.
/// Specificity is honoured: higher-specificity rules override lower
/// when both target the same property.
pub fn apply(s: Style, sheet: &Stylesheet, element: &str, classes: &[&str]) -> Style {
    let node = Node { element, classes };
    // Collect matching rules with their specificity. Iterate in source order
    // so equal-specificity rules cascade later-wins; sort_by_key ensures the
    // *order* is (specificity asc, source order asc) by relying on
    // `sort_by_key` being stable.
    let mut matches: Vec<(u32, usize, &hjkl_css::Rule)> = sheet
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
                .map(|sp| (sp, idx, rule))
        })
        .collect();
    matches.sort_by_key(|(sp, idx, _)| (*sp, *idx));

    let mut out = s;
    for (_, _, rule) in matches {
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

fn parse_hex(s: &str) -> u32 {
    let s = s.trim_start_matches('#');
    u32::from_str_radix(s, 16).unwrap_or(0)
}

fn hex(rgb: u32) -> String {
    format!("#{:06x}", rgb & 0xff_ffff)
}

fn blend_hex(over: u32, under: u32, t: f32) -> u32 {
    let t = t.clamp(0.0, 1.0);
    let or = ((over >> 16) & 0xff) as f32;
    let og = ((over >> 8) & 0xff) as f32;
    let ob = (over & 0xff) as f32;
    let ur = ((under >> 16) & 0xff) as f32;
    let ug = ((under >> 8) & 0xff) as f32;
    let ub = (under & 0xff) as f32;
    let r = (or * t + ur * (1.0 - t)) as u32;
    let g = (og * t + ug * (1.0 - t)) as u32;
    let b = (ob * t + ub * (1.0 - t)) as u32;
    (r << 16) | (g << 8) | b
}
