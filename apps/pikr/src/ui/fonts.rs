//! Finding the theme's font file by family name.
//!
//! crcbl-ui draws only fonts it is handed, and has no system font lookup
//! yet; crcbl is adding one (see `docs/backlog.md`), and this module goes
//! when it lands. Until then: fontconfig's `fc-match` on unix, and the
//! registry's installed-font list on Windows. A family that can't be found
//! leaves the theme's text in crcbl's built-in sans-serif face.

use std::path::PathBuf;

/// The family [`find`] resolves to the system's colour emoji font:
/// fontconfig's generic `emoji` alias, or Windows' own emoji face.
#[cfg(not(windows))]
pub const EMOJI_FAMILY: &str = "emoji";
#[cfg(windows)]
pub const EMOJI_FAMILY: &str = "Segoe UI Emoji";

/// The file holding `family`'s regular face, if the system can say.
pub fn find(family: &str) -> Option<PathBuf> {
    let path = find_impl(family)?;
    tracing::debug!(family, path = %path.display(), "theme font found");
    Some(path)
}

/// fontconfig's best match for `family`. fontconfig always answers with
/// *some* font, as every fontconfig application gets for an unknown name.
#[cfg(unix)]
fn find_impl(family: &str) -> Option<PathBuf> {
    let out = std::process::Command::new("fc-match")
        .arg("--format=%{file}")
        .arg(family)
        .stderr(std::process::Stdio::null())
        .output()
        .inspect_err(|e| tracing::debug!("fc-match did not run: {e}"))
        .ok()?;
    let path = String::from_utf8(out.stdout).ok()?;
    (out.status.success() && !path.is_empty()).then(|| PathBuf::from(path))
}

/// The installed-fonts registry keys, per machine and per user. Their value
/// names are a face's full name plus its format, `"Cascadia Mono (TrueType)"`,
/// and their data a file name in the system font directory or a full path.
#[cfg(windows)]
fn find_impl(family: &str) -> Option<PathBuf> {
    const FONTS_KEY: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Fonts";
    let fonts_dir = std::env::var_os("WINDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
        .join("Fonts");
    ["HKCU", "HKLM"].into_iter().find_map(|hive| {
        let listing = reg_query(&format!(r"{hive}\{FONTS_KEY}"))?;
        let file = registry_font_file(&listing, family)?;
        Some(fonts_dir.join(file))
    })
}

#[cfg(not(any(unix, windows)))]
fn find_impl(_family: &str) -> Option<PathBuf> {
    None
}

/// `reg query KEY`'s output, without a console window flashing up in a
/// windowed-subsystem build.
#[cfg(windows)]
fn reg_query(key: &str) -> Option<String> {
    use std::os::windows::process::CommandExt;
    /// `CREATE_NO_WINDOW`, a documented Win32 process-creation flag.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let out = std::process::Command::new("reg")
        .args(["query", key])
        .creation_flags(CREATE_NO_WINDOW)
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The file of `family`'s regular face in a `reg query` listing of the fonts
/// key: lines of `NAME    REG_SZ    FILE`. A face named exactly `family`, or
/// `family` followed by `Regular`, matches, ignoring ASCII case.
#[cfg_attr(not(windows), allow(dead_code))]
fn registry_font_file(listing: &str, family: &str) -> Option<String> {
    let family = family.trim().to_ascii_lowercase();
    let regular = format!("{family} regular");
    listing.lines().find_map(|line| {
        let (name, file) = line.trim().split_once("REG_SZ")?;
        // Strip the trailing "(TrueType)" / "(OpenType)" format tag.
        let name = name.trim();
        let name = name.rsplit_once(" (").map_or(name, |(face, _)| face);
        let name = name.trim().to_ascii_lowercase();
        (name == family || name == regular).then(|| file.trim().to_owned())
    })
}

#[cfg(test)]
mod tests {
    use super::registry_font_file;

    const LISTING: &str = "
HKEY_LOCAL_MACHINE\\SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\Fonts
    Arial (TrueType)    REG_SZ    arial.ttf
    Cascadia Mono Bold (TrueType)    REG_SZ    CascadiaMono-Bold.ttf
    Cascadia Mono (TrueType)    REG_SZ    CascadiaMono.ttf
    Hack Nerd Font Mono Regular (TrueType)    REG_SZ    C:\\Users\\me\\Fonts\\HackNFM-Regular.ttf
";

    #[test]
    fn matches_the_family_and_its_regular_face_only() {
        assert_eq!(
            registry_font_file(LISTING, "Cascadia Mono").as_deref(),
            Some("CascadiaMono.ttf")
        );
        assert_eq!(
            registry_font_file(LISTING, "hack nerd font mono").as_deref(),
            Some("C:\\Users\\me\\Fonts\\HackNFM-Regular.ttf")
        );
        assert_eq!(registry_font_file(LISTING, "Cascadia"), None);
        assert_eq!(registry_font_file(LISTING, "Nope"), None);
    }
}
