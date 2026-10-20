//! Which icon themes entry icons are looked up in, before
//! [`FALLBACK_THEMES`](super::icons) and `hicolor`: pikr's `theme.icon_theme`,
//! then the theme KDE is set to, then the one GTK is set to. Each is tried in
//! turn, so a theme that is not installed, or lacks an icon, gives way to the
//! next.

/// The themes to search first, highest priority first: `configured`, then
/// KDE's (`kdeglobals`, `[Icons] Theme`), then GTK 3's and GTK 4's
/// (`settings.ini`, `[Settings] gtk-icon-theme-name`), each read from the
/// highest-priority XDG config file that sets it. A name already listed is
/// not repeated.
pub fn chosen(configured: Option<&str>) -> Vec<String> {
    from_sources(configured, desktop_setting)
}

/// [`chosen`] with the desktop settings read through `setting(file, group,
/// key)`.
fn from_sources(
    configured: Option<&str>,
    setting: impl Fn(&str, &str, &str) -> Option<String>,
) -> Vec<String> {
    let sources = [
        configured.map(str::to_owned),
        setting("kdeglobals", "Icons", "Theme"),
        setting("gtk-3.0/settings.ini", "Settings", "gtk-icon-theme-name"),
        setting("gtk-4.0/settings.ini", "Settings", "gtk-icon-theme-name"),
    ];
    let mut themes: Vec<String> = Vec::new();
    for theme in sources.into_iter().flatten() {
        let theme = theme.trim();
        if !theme.is_empty() && !themes.iter().any(|known| known == theme) {
            themes.push(theme.to_owned());
        }
    }
    themes
}

/// `key` in `[group]` of the highest-priority XDG config `file` that sets it
/// (`XDG_CONFIG_HOME` before `XDG_CONFIG_DIRS`).
#[cfg(unix)]
fn desktop_setting(file: &str, group: &str, key: &str) -> Option<String> {
    let mut files: Vec<_> = xdg::BaseDirectories::new()
        .find_config_files(file)
        .collect();
    // Listed lowest priority first.
    files.reverse();
    files.iter().find_map(|path| {
        std::fs::read_to_string(path)
            .inspect_err(|e| tracing::debug!(path = %path.display(), "not read: {e}"))
            .ok()
            .and_then(|text| key_file_value(&text, group, key))
    })
}

/// No desktop settings to follow off the XDG platforms.
#[cfg(not(unix))]
fn desktop_setting(_file: &str, _group: &str, _key: &str) -> Option<String> {
    None
}

/// `key`'s value in `[group]` of a key file as GLib's `GKeyFile` and KDE's
/// `KConfig` write them: `#` and `;` start comments, whitespace around `=` is
/// ignored, and KConfig's `[$…]` flags after a group or key name (`[$i]`,
/// `[$e]`) are not part of the name. Localised keys (`Theme[de]`) are other
/// keys. `None` when unset or empty.
#[cfg(unix)]
fn key_file_value(text: &str, group: &str, key: &str) -> Option<String> {
    let mut in_group = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with(['#', ';']) {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            in_group = without_flags(name) == group;
        } else if in_group
            && let Some((name, value)) = line.split_once('=')
            && without_flags(name.trim()) == key
        {
            let value = value.trim();
            return (!value.is_empty()).then(|| value.to_owned());
        }
    }
    None
}

/// `name` without a trailing KConfig flag such as `[$i]`; a group line's
/// brackets are already stripped, so its flag reads `Icons][$i`.
#[cfg(unix)]
fn without_flags(name: &str) -> &str {
    name.split_once("[$")
        .map_or(name, |(name, _)| name.trim_end_matches(']'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_then_kde_then_gtk_without_repeats() {
        let setting = |file: &str, _: &str, _: &str| match file {
            "kdeglobals" => Some("breeze-dark".to_owned()),
            "gtk-3.0/settings.ini" => Some("Papirus-Dark".to_owned()),
            "gtk-4.0/settings.ini" => Some("breeze-dark".to_owned()),
            _ => None,
        };
        assert_eq!(
            from_sources(Some("Mine"), setting),
            ["Mine", "breeze-dark", "Papirus-Dark"]
        );
        assert_eq!(from_sources(None, setting), ["breeze-dark", "Papirus-Dark"]);
    }

    #[test]
    fn unset_and_blank_sources_are_skipped() {
        let gtk_only = |file: &str, _: &str, _: &str| {
            (file == "gtk-4.0/settings.ini").then(|| "Adwaita".to_owned())
        };
        assert_eq!(from_sources(Some("  "), gtk_only), ["Adwaita"]);
        assert!(from_sources(None, |_, _, _| None).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn gtk_settings_are_read_with_spaces_and_comments() {
        let text = "# comment\n[Settings]\n; another\ngtk-theme-name=Adwaita\ngtk-icon-theme-name = Papirus-Dark \n";
        assert_eq!(
            key_file_value(text, "Settings", "gtk-icon-theme-name").as_deref(),
            Some("Papirus-Dark")
        );
    }

    #[cfg(unix)]
    #[test]
    fn kde_flags_and_other_groups_are_told_apart() {
        let text = "[General]\nTheme=wrong\n[Icons][$i]\nTheme[de]=lokal\nTheme[$e]=breeze-dark\n";
        assert_eq!(
            key_file_value(text, "Icons", "Theme").as_deref(),
            Some("breeze-dark")
        );
        assert_eq!(key_file_value("[Icons]\nTheme=\n", "Icons", "Theme"), None);
        assert_eq!(key_file_value("[Other]\nTheme=x\n", "Icons", "Theme"), None);
    }
}
