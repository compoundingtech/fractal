//! A wrapper's payload state survives loss of the daemon and precedes network reporting.
use std::fs;
use std::io::Write as _;
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::{Path, PathBuf};
use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

pub const PATH_ENV: &str = "FRACTAL_EXEC_PAYLOAD_RECEIPT";
pub const TRACK_ENV: &str = "FRACTAL_NATIVE_STEP_PAYLOAD";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(tag = "state", rename_all = "kebab-case", deny_unknown_fields)]
pub enum PayloadState {
    Running,
    Exited { at_unix_ms: u64 },
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Receipt { operation: String, payload: PayloadState }

pub(crate) fn path(root: &Path, id: &str, operation: &str) -> PathBuf {
    let digest = Sha256::digest(operation.as_bytes());
    root.join(format!("{id}.payload-{digest:x}.json"))
}

fn persist(path: &Path, operation: &str, payload: PayloadState) -> Result<()> {
    let temporary = path.with_extension(format!("tmp.{}", std::process::id()));
    let mut file = fs::OpenOptions::new().create_new(true).write(true).mode(0o600).open(&temporary)?;
    let result = (|| {
        serde_json::to_writer(&mut file, &Receipt { operation: operation.into(), payload })?;
        file.flush()?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        fs::File::open(path.parent().context("payload receipt has no parent")?)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() { let _ = fs::remove_file(&temporary); }
    result
}

/// Untracked/legacy wrappers do not fabricate evidence. Tracked wrappers must persist before
/// spawning a payload, and must persist completion before attempting delivery to the daemon.
pub fn publish_from_environment(payload: PayloadState) -> Result<()> {
    let Some(path) = std::env::var_os(PATH_ENV) else { return Ok(()); };
    let operation = std::env::var("ST3_START_OPERATION").context("tracked payload has no start operation")?;
    persist(Path::new(&path), &operation, payload)
}

pub(crate) fn read(path: &Path, operation: &str) -> Result<Option<PayloadState>> {
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let receipt: Receipt = serde_json::from_reader(file)?;
    anyhow::ensure!(receipt.operation == operation, "payload receipt operation changed");
    Ok(Some(receipt.payload))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receipt_path_uses_fixed_width_sha256() {
        assert_eq!(
            path(Path::new("/receipts"), "exec-example", "abc"),
            Path::new("/receipts").join(
                "exec-example.payload-ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad.json"
            )
        );
    }

    #[test]
    fn completion_survives_daemon_outage_and_is_operation_fenced() {
        let root = tempfile::tempdir().unwrap();
        let receipt = path(root.path(), "exec-example", "operation-one");
        persist(&receipt, "operation-one", PayloadState::Running).unwrap();
        // The wrapper remains alive retrying its claim, but no daemon/store is needed here.
        persist(&receipt, "operation-one", PayloadState::Exited { at_unix_ms: 100 }).unwrap();
        assert_eq!(read(&receipt, "operation-one").unwrap(), Some(PayloadState::Exited { at_unix_ms: 100 }));
        assert!(read(&receipt, "operation-two").is_err());
        assert_eq!(read(&path(root.path(), "exec-example", "operation-two"), "operation-two").unwrap(), None);
    }
}
