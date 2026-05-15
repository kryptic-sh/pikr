//! run mode — `$PATH` executable launcher.

use super::{Entry, Mode};
use anyhow::Result;
use std::collections::BTreeSet;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

#[derive(Default)]
pub struct Run;

impl Mode for Run {
    fn name(&self) -> &'static str {
        "run"
    }

    fn collect(&mut self) -> Result<Vec<Entry>> {
        let names = scan_path();
        Ok(names
            .into_iter()
            .map(|n| Entry::exec(n.clone(), n))
            .collect())
    }
}

fn scan_path() -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let Some(path) = std::env::var_os("PATH") else {
        return names;
    };
    for dir in std::env::split_paths(&path) {
        if dir.as_os_str().is_empty() {
            continue;
        }
        scan_dir(&dir, &mut names);
    }
    names
}

fn scan_dir(dir: &Path, names: &mut BTreeSet<String>) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in read.flatten() {
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        // Executable bit on any of user/group/other.
        if meta.permissions().mode() & 0o111 == 0 {
            continue;
        }
        if let Some(name) = entry.file_name().to_str() {
            names.insert(name.to_string());
        }
    }
}
