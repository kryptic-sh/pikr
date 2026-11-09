//! Which icon themes entry icons are looked up in, before
//! [`FALLBACK_THEMES`](super::icons) and `hicolor`: pikr's `theme.icon_theme`,
//! then the theme KDE is set to, then the one GTK is set to, then, when no
//! settings file names one, GNOME's from GSettings. Each is tried in
//! turn, so a theme that is not installed, or lacks an icon, gives way to the
//! next.

/// The GSettings schema and key GNOME keeps its icon theme in, which GTK
/// also reads on Wayland.
const GNOME_ICON_THEME: [&str; 2] = ["org.gnome.desktop.interface", "icon-theme"];

/// The themes to search first, highest priority first: `configured`, then
/// KDE's (`kdeglobals`, `[Icons] Theme`), then GTK 3's and GTK 4's
/// (`settings.ini`, `[Settings] gtk-icon-theme-name`), each read from the
/// highest-priority XDG config file that sets it. When none of those files
/// names one, GNOME's ([`GNOME_ICON_THEME`], asked of `gsettings`): GNOME
/// keeps it there and writes no `settings.ini`. A name already listed is not
/// repeated.
///
/// It may start a process, so call it off the start-up path.
pub fn chosen(configured: Option<&str>) -> Vec<String> {
    let themes = from_sources(configured, desktop_setting, gnome_setting);
    tracing::debug!(?themes, "icon themes chosen");
    themes
}

/// [`chosen`] with the desktop settings read through `setting(file, group,
/// key)`, and GNOME's through `gnome`.
fn from_sources(
    configured: Option<&str>,
    setting: impl Fn(&str, &str, &str) -> Option<String>,
    gnome: impl FnOnce() -> Option<String>,
) -> Vec<String> {
    let desktop = [
        setting("kdeglobals", "Icons", "Theme"),
        setting("gtk-3.0/settings.ini", "Settings", "gtk-icon-theme-name"),
        setting("gtk-4.0/settings.ini", "Settings", "gtk-icon-theme-name"),
    ];
    let gnome = if desktop.iter().all(Option::is_none) {
        gnome()
    } else {
        None
    };
    let sources = std::iter::once(configured.map(str::to_owned))
        .chain(desktop)
        .chain([gnome]);
    let mut themes: Vec<String> = Vec::new();
    for theme in sources.flatten() {
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

/// GNOME's icon theme, as `gsettings get` prints it. `None` when `gsettings`
/// is missing or fails (no such schema, say) or prints no string.
#[cfg(unix)]
fn gnome_setting() -> Option<String> {
    let output = std::process::Command::new("gsettings")
        .arg("get")
        .args(GNOME_ICON_THEME)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .inspect_err(|e| tracing::debug!("gsettings not run: {e}"))
        .ok()?;
    if !output.status.success() {
        tracing::debug!(status = %output.status, "gsettings gave no icon theme");
        return None;
    }
    let printed = String::from_utf8_lossy(&output.stdout);
    let theme = gvariant_string(printed.trim());
    if theme.is_none() {
        tracing::debug!(%printed, "gsettings printed no icon theme name");
    }
    theme
}

/// No GSettings off the XDG platforms.
#[cfg(not(unix))]
fn gnome_setting() -> Option<String> {
    None
}

/// The string a GVariant in text form holds, as `g_variant_print` writes one:
/// in single quotes, or in double quotes when it holds a single quote and no
/// double one, with the quote and backslash escaped by a backslash. `None`
/// for anything else (another type, or an escape a theme name has no use
/// for, such as `\n` or `\u…`), and for an empty string.
#[cfg(unix)]
fn gvariant_string(text: &str) -> Option<String> {
    let quote = text.chars().next().filter(|c| matches!(c, '\'' | '"'))?;
    let inner = text[1..].strip_suffix(quote)?;
    let mut value = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next()? {
                escaped @ ('\\' | '\'' | '"') => value.push(escaped),
                _ => return None,
            },
            c if c == quote => return None,
            c => value.push(c),
        }
    }
    (!value.is_empty()).then_some(value)
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
            from_sources(Some("Mine"), setting, no_gnome),
            ["Mine", "breeze-dark", "Papirus-Dark"]
        );
        assert_eq!(
            from_sources(None, setting, no_gnome),
            ["breeze-dark", "Papirus-Dark"]
        );
    }

    #[test]
    fn unset_and_blank_sources_are_skipped() {
        let gtk_only = |file: &str, _: &str, _: &str| {
            (file == "gtk-4.0/settings.ini").then(|| "Adwaita".to_owned())
        };
        assert_eq!(from_sources(Some("  "), gtk_only, no_gnome), ["Adwaita"]);
        assert!(from_sources(None, |_, _, _| None, no_gnome).is_empty());
    }

    /// A GSettings source for tests where it holds nothing.
    fn no_gnome() -> Option<String> {
        None
    }

    #[test]
    fn gnome_is_asked_only_when_no_settings_file_names_a_theme() {
        let gnome = || Some("Papirus-Dark".to_owned());
        assert_eq!(
            from_sources(Some("Mine"), |_, _, _| None, gnome),
            ["Mine", "Papirus-Dark"]
        );
        let kde =
            |file: &str, _: &str, _: &str| (file == "kdeglobals").then(|| "breeze-dark".to_owned());
        let asked = std::cell::Cell::new(false);
        let gnome = || {
            asked.set(true);
            Some("Adwaita".to_owned())
        };
        assert_eq!(from_sources(None, kde, gnome), ["breeze-dark"]);
        assert!(
            !asked.get(),
            "gsettings was run though kdeglobals named a theme"
        );
    }

    #[cfg(unix)]
    #[test]
    fn gsettings_strings_are_unquoted() {
        assert_eq!(
            gvariant_string("'Papirus-Dark'").as_deref(),
            Some("Papirus-Dark")
        );
        assert_eq!(gvariant_string(r#""it's""#).as_deref(), Some("it's"));
        assert_eq!(gvariant_string(r"'a\'b\\c'").as_deref(), Some(r"a'b\c"));
        for refused in [
            "''",
            "'",
            "Adwaita",
            "'open",
            "'a'b'",
            r"'a\nb'",
            "uint32 3",
            "@ms nothing",
        ] {
            assert_eq!(gvariant_string(refused), None, "{refused}");
        }
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
