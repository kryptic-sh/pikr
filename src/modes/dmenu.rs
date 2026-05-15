//! dmenu mode — stdin in, stdout out.

use super::{Entry, Mode, Payload};
use anyhow::Result;

pub struct Dmenu;

impl Mode for Dmenu {
    fn name(&self) -> &'static str {
        "dmenu"
    }

    fn entries(&self) -> Result<Vec<Entry>> {
        // TODO: read stdin lines into entries.
        Ok(Vec::new())
    }

    fn select(&self, entry: &Entry) -> Result<()> {
        if let Payload::Stdout(s) = &entry.payload {
            println!("{s}");
        }
        Ok(())
    }
}
