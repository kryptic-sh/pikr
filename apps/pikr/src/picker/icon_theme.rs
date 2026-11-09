//! Which icon themes entry icons are looked up in, before
//! [`FALLBACK_THEMES`](super::icons) and `hicolor`: pikr's `theme.icon_theme`,
//! then the theme KDE is set to, then the one GTK is set to, then, when no
//! settings file names one, the desktop's from GSettings. Each is tried in
//! turn, so a theme that is not installed, or lacks an icon, gives way to the
//! next.

/// The GSettings key that holds the icon theme, in each schema
/// [`gsettings_schema`] names.
#[cfg(unix)]
const ICON_THEME_KEY: &str = "icon-theme";

/// The themes to search first, highest priority first: `configured`, then
/// KDE's (`kdeglobals`, `[Icons] Theme`), then GTK 3's and GTK 4's
/// (`settings.ini`, `[Settings] gtk-icon-theme-name`), each read from the
/// highest-priority XDG config file that sets it. When none of those files
/// names one, the running desktop's GSettings ([`gsettings_schema`], asked of
/// `gsettings`): GNOME, Cinnamon and MATE keep it there and write no
/// `settings.ini`, and GTK reads GNOME's on Wayland. A name already listed is
/// not repeated.
///
/// It may start a process, so call it off the start-up path.
pub fn chosen(configured: Option<&str>) -> Vec<String> {
    let themes = from_sources(configured, desktop_setting, gsettings_icon_theme);
    tracing::debug!(?themes, "icon themes chosen");
    themes
}

/// [`chosen`] with the desktop settings read through `setting(file, group,
/// key)`, and GSettings' through `gsettings`.
fn from_sources(
    configured: Option<&str>,
    setting: impl Fn(&str, &str, &str) -> Option<String>,
    gsettings: impl FnOnce() -> Option<String>,
) -> Vec<String> {
    let desktop = [
        setting("kdeglobals", "Icons", "Theme"),
        setting("gtk-3.0/settings.ini", "Settings", "gtk-icon-theme-name"),
        setting("gtk-4.0/settings.ini", "Settings", "gtk-icon-theme-name"),
    ];
    let gsettings = if desktop.iter().all(Option::is_none) {
        gsettings()
    } else {
        None
    };
    let sources = std::iter::once(configured.map(str::to_owned))
        .chain(desktop)
        .chain([gsettings]);
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

/// The running desktop's icon theme, as `gsettings get` prints it from the
/// desktop's schema. `None` when `gsettings` is missing or fails (no such
/// schema, say) or prints no string.
#[cfg(unix)]
fn gsettings_icon_theme() -> Option<String> {
    let desktops = freedesktop_desktop_entry::current_desktop().unwrap_or_default();
    let output = std::process::Command::new("gsettings")
        .args(["get", gsettings_schema(&desktops), ICON_THEME_KEY])
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
fn gsettings_icon_theme() -> Option<String> {
    None
}

/// The GSettings schema the desktop named in `desktops` (`XDG_CURRENT_DESKTOP`
/// lowercased and split, as `freedesktop_desktop_entry::current_desktop`
/// gives it) keeps its icon theme in: Cinnamon's and MATE's own, else
/// GNOME's, which GTK reads on every other desktop.
#[cfg(unix)]
fn gsettings_schema(desktops: &[String]) -> &'static str {
    desktops
        .iter()
        .find_map(|desktop| match desktop.as_str() {
            "x-cinnamon" | "cinnamon" => Some("org.cinnamon.desktop.interface"),
            "mate" => Some("org.mate.interface"),
            _ => None,
        })
        .unwrap_or("org.gnome.desktop.interface")
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
            from_sources(Some("Mine"), setting, no_gsettings),
            ["Mine", "breeze-dark", "Papirus-Dark"]
        );
        assert_eq!(
            from_sources(None, setting, no_gsettings),
            ["breeze-dark", "Papirus-Dark"]
        );
    }

    #[test]
    fn unset_and_blank_sources_are_skipped() {
        let gtk_only = |file: &str, _: &str, _: &str| {
            (file == "gtk-4.0/settings.ini").then(|| "Adwaita".to_owned())
        };
        assert_eq!(
            from_sources(Some("  "), gtk_only, no_gsettings),
            ["Adwaita"]
        );
        assert!(from_sources(None, |_, _, _| None, no_gsettings).is_empty());
    }

    /// A GSettings source for tests where it holds nothing.
    fn no_gsettings() -> Option<String> {
        None
    }

    #[test]
    fn gsettings_is_asked_only_when_no_settings_file_names_a_theme() {
        let gsettings = || Some("Papirus-Dark".to_owned());
        assert_eq!(
            from_sources(Some("Mine"), |_, _, _| None, gsettings),
            ["Mine", "Papirus-Dark"]
        );
        let kde =
            |file: &str, _: &str, _: &str| (file == "kdeglobals").then(|| "breeze-dark".to_owned());
        let asked = std::cell::Cell::new(false);
        let gsettings = || {
            asked.set(true);
            Some("Adwaita".to_owned())
        };
        assert_eq!(from_sources(None, kde, gsettings), ["breeze-dark"]);
        assert!(
            !asked.get(),
            "gsettings was run though kdeglobals named a theme"
        );
    }

    #[cfg(unix)]
    #[test]
    fn gsettings_schema_follows_the_desktop() {
        let schema = |desktop: &str| {
            let desktops: Vec<String> = desktop.split(':').map(str::to_owned).collect();
            gsettings_schema(&desktops)
        };
        assert_eq!(schema("x-cinnamon"), "org.cinnamon.desktop.interface");
        assert_eq!(schema("mate"), "org.mate.interface");
        assert_eq!(schema("ubuntu:gnome"), "org.gnome.desktop.interface");
        assert_eq!(schema("hyprland"), "org.gnome.desktop.interface");
        assert_eq!(gsettings_schema(&[]), "org.gnome.desktop.interface");
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
