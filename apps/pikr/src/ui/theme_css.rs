//! The stylesheet source: `default.css` with the theme substituted in.
//!
//! This is plain text, independent of the GUI toolkit that parses and applies
//! it, and it is the stylesheet contract a backend must honour.

use crate::config::Theme;

/// `default.css` with every `{placeholder}` replaced from `theme`. Computed
/// colours (`ex_bg`, `status_bg`, `count_bg`) mirror the `blend()` calls that
/// were inline in `ui/view.rs` before the CSS migration.
pub fn stylesheet_source(theme: &Theme) -> String {
    let accent = parse_hex(&theme.accent);
    let muted = parse_hex(&theme.muted);
    let selected_bg = parse_hex(&theme.selected_bg);
    // The ex bar and status bar share the same derived background.
    let ex_bg = blend_hex(muted, selected_bg, 0.15);
    let status_bg = ex_bg;
    let count_bg = blend_hex(accent, selected_bg, 0.25);

    include_str!("styles/default.css")
        .replace("{bg}", &theme.bg)
        .replace("{fg}", &theme.fg)
        .replace("{accent}", &theme.accent)
        .replace("{muted}", &theme.muted)
        .replace("{selected_bg}", &theme.selected_bg)
        .replace("{ex_bg}", &hex(ex_bg))
        .replace("{status_bg}", &hex(status_bg))
        .replace("{count_bg}", &hex(count_bg))
        .replace("{font_family}", &theme.font)
        .replace("{font_size}", &theme.font_size.to_string())
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Any `{name}` left in the output is a placeholder `default.css` uses
    /// and `stylesheet_source` does not fill, which a parser would reject or
    /// silently drop. CSS block braces are always followed by whitespace or a
    /// newline, so a brace directly followed by a letter is a placeholder.
    #[test]
    fn every_placeholder_is_filled() {
        let css = stylesheet_source(&Theme::default());
        let leftover: Vec<&str> = css
            .match_indices('{')
            .map(|(i, _)| &css[i..])
            .filter(|rest| rest[1..].starts_with(|c: char| c.is_ascii_alphabetic()))
            .map(|rest| rest.find('}').map_or(rest, |end| &rest[..=end]))
            .collect();
        assert!(leftover.is_empty(), "unfilled placeholders: {leftover:?}");
    }

    #[test]
    fn theme_values_reach_the_output() {
        let theme = Theme::default();
        let css = stylesheet_source(&theme);
        assert!(css.contains(&theme.bg), "bg {} missing", theme.bg);
        assert!(css.contains(&theme.font), "font {} missing", theme.font);
    }
}
