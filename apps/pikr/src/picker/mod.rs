//! Picker — selection state, vim keymap, fuzzy matcher.

pub mod frecency;
pub mod history;
pub mod icons;
pub mod keys;
pub mod keyspec;
pub mod matcher;
pub mod state;

use std::path::{Path, PathBuf};

/// Where pikr persists the state file `name` (query history, frecency):
/// `$XDG_STATE_HOME/pikr/<name>` on unix (macOS included), and
/// `%LOCALAPPDATA%\pikr\<name>` on Windows, beside the drun caches. `None`
/// when the platform has no such directory. The parent directory may not
/// exist yet; savers create it.
pub(crate) fn state_file_path(name: &str) -> Option<PathBuf> {
    #[cfg(unix)]
    {
        xdg::BaseDirectories::with_prefix("pikr")
            .place_state_file(name)
            .ok()
    }
    #[cfg(not(unix))]
    {
        dirs::data_local_dir().map(|dir| dir.join("pikr").join(name))
    }
}

/// Write `text` to `path` with owner-only permissions (0600), creating or
/// truncating. Used for persisted query history and frecency keys — both can
/// contain typed queries / program arguments a launcher user may not want
/// world-readable on a multi-user host. `std::fs::write` would create the
/// file 0644 under a normal umask.
///
/// Off unix there is no mode to set, so the fallback below is a plain write.
/// On Windows [`state_file_path`] lands under `%LOCALAPPDATA%`, whose
/// inherited ACL already grants only the user, SYSTEM and Administrators.
#[cfg(unix)]
pub(crate) fn write_private_state(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(text.as_bytes())?;
    // Re-apply for a pre-existing file: `mode` only takes effect at creation,
    // so a file left 0644 by an older pikr would otherwise keep its old
    // permissions.
    f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
pub(crate) fn write_private_state(path: &Path, text: &str) -> std::io::Result<()> {
    // No owner-only mode concept off unix; see the doc comment.
    std::fs::write(path, text)
}

#[cfg(test)]
mod state_path_tests {
    use super::state_file_path;

    /// Every shipped platform must persist state; a `None` here silently
    /// turns history and frecency into per-session memory.
    #[test]
    fn state_file_path_resolves_under_a_pikr_dir() {
        let path = state_file_path("usage.toml").expect("state dir must resolve");
        assert!(path.is_absolute(), "got {}", path.display());
        assert!(path.ends_with("pikr/usage.toml"), "got {}", path.display());
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::write_private_state;
    use std::os::unix::fs::PermissionsExt;

    fn mode_of(path: &std::path::Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn new_file_written_0600() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.toml");
        write_private_state(&path, "hello").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello");
        assert_eq!(mode_of(&path), 0o600);
    }

    #[test]
    fn preexisting_0644_file_upgraded_to_0600() {
        // A file written by an older pikr (or an explicitly relaxed umask)
        // must be re-chmod'd on save — `OpenOptions::mode` only applies at
        // creation.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.toml");
        std::fs::write(&path, b"old").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(mode_of(&path), 0o644);

        write_private_state(&path, "new").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
        assert_eq!(mode_of(&path), 0o600);
    }

    #[test]
    fn rewrite_truncates() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.toml");
        write_private_state(&path, &"a".repeat(100)).unwrap();
        write_private_state(&path, "b").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "b");
        assert_eq!(mode_of(&path), 0o600);
    }
}
