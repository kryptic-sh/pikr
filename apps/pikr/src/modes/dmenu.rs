//! dmenu mode — stdin in, stdout out.

use super::{Entry, Mode};
use anyhow::Result;
use std::io::{self, BufRead, IsTerminal};

#[derive(Default)]
pub struct Dmenu;

impl Mode for Dmenu {
    fn name(&self) -> &'static str {
        "dmenu"
    }

    fn collect(&mut self) -> Result<Vec<Entry>> {
        let stdin = io::stdin();
        if stdin.is_terminal() {
            anyhow::bail!("dmenu mode requires entries on stdin");
        }
        let handle = stdin.lock();
        let mut entries = Vec::new();
        for line in handle.lines() {
            let line = line?;
            if line.is_empty() {
                continue;
            }
            entries.push(Entry::stdout(line));
        }
        Ok(entries)
    }
}
