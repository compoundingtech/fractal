//! Test-support scheduling gate: delay a real mutation before it acquires the writer.
use std::{path::{Path, PathBuf}, sync::OnceLock, time::{Duration, Instant}};
use crate::model::St3Error;
static ROOT: OnceLock<PathBuf> = OnceLock::new();

pub fn install(root: &Path) -> Result<(), St3Error> {
    std::fs::create_dir_all(root).map_err(super::internal)?;
    ROOT.set(root.to_owned()).map_err(|_| St3Error::new("internal", "mutation gate already installed"))
}

pub(crate) struct Completion(PathBuf);
impl Drop for Completion {
    fn drop(&mut self) { let _ = std::fs::write(self.0.join("done"), b"done"); }
}

pub(crate) fn pause() -> Result<Option<Completion>, St3Error> {
    let Some(root) = ROOT.get().filter(|root| root.join("hold").exists()) else { return Ok(None) };
    let completion = Completion(root.clone());
    std::fs::write(root.join("entered"), b"entered").map_err(super::internal)?;
    let deadline = Instant::now() + Duration::from_secs(30);
    while !root.join("release").exists() {
        if Instant::now() >= deadline { return Err(St3Error::new("unavailable", "test mutation gate release deadline exceeded")); }
        std::thread::sleep(Duration::from_millis(5));
    }
    Ok(Some(completion))
}
