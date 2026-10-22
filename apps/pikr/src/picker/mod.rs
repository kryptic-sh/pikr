//! Picker — selection state, vim keymap, fuzzy matcher.

pub mod app_state;
pub mod frecency;
pub mod history;
pub mod icon_theme;
pub mod icons;
pub mod input;
pub mod keys;
pub mod keyspec;
pub mod matcher;
pub mod query_text;
pub mod session;
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

/// `path`'s TOML, or the default when there is none. A file that does not
/// parse is moved aside to `<name>.unreadable`, with a warning, so the next
/// save does not replace a whole history or frecency table that one bad value
/// made unreadable; the user can still recover it from there.
pub(crate) fn read_state_toml<T: serde::de::DeserializeOwned + Default>(path: &Path) -> T {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return T::default(),
        Err(e) => {
            tracing::warn!(error = %e, path = %path.display(), "cannot read state file");
            return T::default();
        }
    };
    match toml::from_str(&text) {
        Ok(state) => state,
        Err(e) => {
            let mut aside = path.as_os_str().to_owned();
            aside.push(".unreadable");
            let aside = PathBuf::from(aside);
            match std::fs::rename(path, &aside) {
                Ok(()) => tracing::warn!(
                    error = %e,
                    path = %aside.display(),
                    "state file does not parse; moved aside and starting empty"
                ),
                Err(move_error) => tracing::warn!(
                    error = %e,
                    move_error = %move_error,
                    path = %path.display(),
                    "state file does not parse and could not be moved aside"
                ),
            }
            T::default()
        }
    }
}

/// Runs `update` holding an exclusive lock on `<path>.lock`, so pikrs saving
/// `path` at once take turns: each one's read of `path`, merge and rename
/// finish before the next one reads. A sibling, since the rename replaces
/// `path` itself. The lock file is left in place: deleting it would let a
/// third pikr lock a new one while the second still holds the old.
///
/// Where no lock can be had (a file system without locking, a lock file that
/// cannot be created) `update` runs unlocked: a rare lost update beats a
/// lost save.
pub(crate) fn with_state_lock(path: &Path, update: impl FnOnce()) {
    let mut name = path.as_os_str().to_owned();
    name.push(".lock");
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let lock = options.open(PathBuf::from(name)).and_then(|file| {
        file.lock()?;
        Ok(file)
    });
    if let Err(e) = &lock {
        tracing::warn!(error = %e, path = %path.display(), "state file lock; saving unlocked");
    }
    update();
    // Closing the file releases the lock.
    drop(lock);
}

/// Replace `path` with `bytes`, readable by the owner only (0600) on unix.
/// Used for persisted query history, frecency keys and the drun caches —
/// typed queries and program arguments a launcher user may not want
/// world-readable on a multi-user host. `std::fs::write` would create the
/// file 0644 under a normal umask.
///
/// The bytes go to a fresh temporary sibling that is then renamed over
/// `path`, so a reader, another pikr saving at the same moment, or the next
/// launch after pikr was killed mid-save sees the old file or the new one,
/// never part of one. The rename also replaces a file an older pikr left
/// 0644. The bytes are synced to disk before the rename and the directory
/// after it (see [`sync_dir`]), so a power cut after a save returns does
/// not lose it.
///
/// Off unix there is no mode to set. On Windows [`state_file_path`] lands
/// under `%LOCALAPPDATA%`, whose inherited ACL already grants only the user,
/// SYSTEM and Administrators, and the rename fails while another process has
/// `path` open without delete sharing.
pub(crate) fn write_private_state(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);

    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(
        ".{}.{}.tmp",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let tmp = PathBuf::from(tmp);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let written = options
        .open(&tmp)
        .and_then(|mut f| f.write_all(bytes).and_then(|()| f.sync_all()))
        .and_then(|()| std::fs::rename(&tmp, path));
    if written.is_err() {
        // The error being returned is the one worth reporting; a temporary
        // file that is already gone, or never opened, needs no cleanup.
        let _ = std::fs::remove_file(&tmp);
        return written;
    }
    // The new file is in place either way; a directory that cannot be
    // synced (some network and FUSE filesystems refuse) only leaves the
    // rename to the filesystem's own schedule.
    if let Some(dir) = path.parent()
        && let Err(e) = sync_dir(dir)
    {
        tracing::warn!(
            "saved {} but could not sync its directory: {e}",
            path.display()
        );
    }
    Ok(())
}

/// Flushes `dir`'s entries to disk, so a rename into it survives a power
/// cut. A directory is opened for writing with `FILE_FLAG_BACKUP_SEMANTICS`
/// on Windows, where `FlushFileBuffers` (`sync_all`) needs a write handle,
/// and read-only elsewhere, where `fsync` takes any descriptor.
fn sync_dir(dir: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    let dir = {
        use std::os::windows::fs::OpenOptionsExt;
        use windows::Win32::Storage::FileSystem::FILE_FLAG_BACKUP_SEMANTICS;
        std::fs::OpenOptions::new()
            .write(true)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0)
            .open(dir)?
    };
    #[cfg(not(windows))]
    let dir = std::fs::File::open(dir)?;
    dir.sync_all()
}

#[cfg(test)]
mod state_path_tests {
    use super::{state_file_path, sync_dir, write_private_state};

    /// Every shipped platform must persist state; a `None` here silently
    /// turns history and frecency into per-session memory.
    #[test]
    fn state_file_path_resolves_under_a_pikr_dir() {
        let path = state_file_path("usage.toml").expect("state dir must resolve");
        assert!(path.is_absolute(), "got {}", path.display());
        assert!(path.ends_with("pikr/usage.toml"), "got {}", path.display());
    }

    /// Concurrent writers of one file must never expose a partial file to a
    /// reader.
    #[test]
    fn write_private_state_never_exposes_partial_file() {
        const LEN: usize = 256 * 1024;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.toml");
        write_private_state(&path, &vec![0u8; LEN]).unwrap();

        std::thread::scope(|s| {
            for w in 0..4u8 {
                let path = &path;
                s.spawn(move || {
                    for _ in 0..25 {
                        // Windows refuses the rename while a reader has the
                        // file open; that save is lost, never half-made.
                        let _ = write_private_state(path, &vec![w; LEN]);
                    }
                });
            }
            for _ in 0..4 {
                let path = &path;
                s.spawn(move || {
                    for _ in 0..100 {
                        // A read can lose to a rename in progress on Windows;
                        // only a successful read is checked for completeness.
                        if let Ok(bytes) = std::fs::read(path) {
                            assert_eq!(bytes.len(), LEN, "reader saw a partial file");
                        }
                    }
                });
            }
        });
    }

    /// A state directory can be synced on every OS CI runs, so saves do not
    /// warn on an ordinary disk.
    #[test]
    fn a_state_directory_can_be_synced() {
        let dir = tempfile::tempdir().unwrap();
        sync_dir(dir.path()).unwrap();
    }

    /// A write leaves only the target behind, and a failed one removes its
    /// temporary file.
    #[test]
    fn write_private_state_leaves_no_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.toml");
        write_private_state(&path, b"one").unwrap();
        write_private_state(&path, b"two").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"two");

        // A directory in the way makes the rename fail.
        let blocked = dir.path().join("blocked");
        std::fs::create_dir(&blocked).unwrap();
        write_private_state(&blocked, b"x").unwrap_err();

        let mut names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        names.sort();
        assert_eq!(names, ["blocked", "state.toml"]);
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
        write_private_state(&path, b"hello").unwrap();
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

        write_private_state(&path, b"new").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
        assert_eq!(mode_of(&path), 0o600);
    }

    #[test]
    fn rewrite_truncates() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.toml");
        write_private_state(&path, "a".repeat(100).as_bytes()).unwrap();
        write_private_state(&path, b"b").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "b");
        assert_eq!(mode_of(&path), 0o600);
    }
}
