//! run mode — $PATH executable launcher.

use super::{Entry, Mode};
use anyhow::Result;

pub struct Run;

impl Mode for Run {
    fn name(&self) -> &'static str {
        "run"
    }

    fn entries(&self) -> Result<Vec<Entry>> {
        // TODO: walk $PATH, collect executable names.
        Ok(Vec::new())
    }

    fn select(&self, _entry: &Entry) -> Result<()> {
        // TODO: spawn detached.
        Ok(())
    }
}
