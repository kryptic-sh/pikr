//! Modes — sources of selectable entries.

pub mod dmenu;
pub mod drun;
pub mod run;

/// One selectable row.
#[derive(Debug, Clone)]
pub struct Entry {
    pub label: String,
    pub payload: Payload,
}

#[derive(Debug, Clone)]
pub enum Payload {
    /// Print this string to stdout (dmenu).
    Stdout(String),
    /// Exec this command line.
    Exec(String),
}

pub trait Mode {
    fn name(&self) -> &'static str;
    fn entries(&self) -> anyhow::Result<Vec<Entry>>;
    fn select(&self, entry: &Entry) -> anyhow::Result<()>;
}
