//! Picker — selection state, vim keymap, fuzzy matcher.

pub mod frecency;
pub mod history;
pub mod icons;
pub mod keys;
pub mod matcher;
pub mod state;

use std::io::Write;
use std::path::Path;

/// Write `text` to `path` with owner-only permissions (0600), creating or
/// truncating. Used for persisted query history and frecency keys — both can
/// contain typed queries / program arguments a launcher user may not want
/// world-readable on a multi-user host. `std::fs::write` would create the
/// file 0644 under a normal umask.
///
/// Unix-only: the state dirs (`history.rs` / `frecency.rs` `state_file_path`)
/// are unix-only, so on other targets nothing ever calls this.
#[cfg(unix)]
pub(crate) fn write_private_state(path: &Path, text: &str) -> std::io::Result<()> {
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
