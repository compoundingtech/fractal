//! Test-support scheduling gate: delay a real mutation before it acquires the writer.
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use crate::model::St3Error;
pub struct Gate {
    pub root: PathBuf,
    pub grant_subject: Option<String>,
}
pub static GATE: OnceLock<Gate> = OnceLock::new();

pub fn install(root: &Path, grant_subject: Option<String>) -> Result<(), St3Error> {
    std::fs::create_dir_all(root).map_err(super::internal)?;
    GATE.set(Gate { root: root.to_owned(), grant_subject })
        .map_err(|_| St3Error::new("internal", "mutation gate already installed"))
}

