//! drun mode — XDG .desktop application launcher.

use super::{Entry, Mode};
use anyhow::Result;

pub struct Drun;

impl Mode for Drun {
    fn name(&self) -> &'static str {
        "drun"
    }

    fn entries(&self) -> Result<Vec<Entry>> {
        // TODO: scan $XDG_DATA_DIRS/applications/*.desktop, parse Exec=.
        Ok(Vec::new())
    }

    fn select(&self, _entry: &Entry) -> Result<()> {
        // TODO: spawn detached process from Exec field.
        Ok(())
    }
}
