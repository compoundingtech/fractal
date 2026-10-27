//! The part of `store::assistant_write_gate` that moved up from fractal-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use std::{path::{Path, PathBuf}, sync::OnceLock, time::{Duration, Instant}};
use crate::model::St3Error;
pub use fractal_store::store::assistant_write_gate::*;
pub use fractal_store::store::assistant_write_gate::{Gate, install};

pub struct Completion(&'static Path);

impl Drop for Completion {
    fn drop(&mut self) { let _ = std::fs::write(self.0.join("done"), b"done"); }
}

pub fn pause(grant_subject: &str) -> Result<Option<Completion>, St3Error> {
    let Some(gate) = GATE.get().filter(|gate|
        gate.root.join("hold").exists()
            && gate.grant_subject.as_deref().is_none_or(|target| target == grant_subject)
    ) else { return Ok(None) };
    let root = gate.root.as_path();
    let completion = Completion(root);
    std::fs::write(root.join("entered"), b"entered").map_err(super::internal)?;
    let deadline = Instant::now() + Duration::from_secs(30);
    while !root.join("release").exists() {
        if Instant::now() >= deadline { return Err(St3Error::new("unavailable", "test mutation gate release deadline exceeded")); }
        std::thread::sleep(Duration::from_millis(5));
    }
    Ok(Some(completion))
}
