//! Darwin helper capability and receipt boundary.
//!
//! Availability is not active isolation: the runtime remains detached until its finite
//! payload-release gate and owned-child reaper can use the helper's launch protocol.
//! In particular, a helper wrapper's exit is never evidence of payload exit or retirement.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context as _, Result, ensure};
pub use fractal_mac_custody::{Receipt, Status};
use fractal_mac_custody::{SCHEMA, SLOT};
use serde::{Deserialize, Serialize};

pub const HELPER_ENV: &str = "FRACTAL_MAC_CUSTODY";

pub(crate) fn degradation_report(available: bool) -> &'static str {
    if available {
        "Darwin custody helper is available but runtime launch integration is unavailable; tasks remain detached."
    } else {
        "helperUnavailable: Darwin custody helper is unavailable; tasks remain detached."
    }
}

#[derive(Clone, Debug)]
pub struct Helper {
    path: PathBuf,
}

impl Helper {
    /// Probe the exact sudoers command without running a privileged helper operation.
    /// No PATH fallback is permitted: activation must supply the pinned absolute helper.
    pub fn detect_in(environment: &BTreeMap<String, String>) -> Option<Self> {
        let path = configured_path(environment)?;
        if !cfg!(target_os = "macos") {
            return None;
        }
        let allowed = Command::new("/usr/bin/sudo")
            .args(["-n", "-l"])
            .arg(&path)
            .args(["leftovers", SLOT])
            .env_clear()
            .envs(environment)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status().ok()?.success();
        allowed.then_some(Self { path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The caller must admit only an `Empty` receipt; members and unknown states retain custody.
    pub fn leftovers(&self) -> Result<Receipt> {
        self.invoke("leftovers")
    }

    /// Darwin RSS samples are observations, never memory enforcement or peak measurements.
    pub fn sample(&self) -> Result<Receipt> {
        self.invoke("sample")
    }

    /// Both natural exit and cancellation require this operation before retirement is claimed.
    /// The guard must still bind the caller to the current slot operation.
    pub fn cancel(&self, guard: &dyn Fn() -> Result<()>) -> Result<Receipt> {
        guard()?;
        self.invoke("cancel")
    }

    fn invoke(&self, verb: &str) -> Result<Receipt> {
        ensure!(cfg!(target_os = "macos"), "Darwin custody requires macOS");
        let output = Command::new("/usr/bin/sudo")
            .arg("-n").arg(&self.path).args([verb, SLOT])
            .stdin(Stdio::null()).output()
            .with_context(|| format!("invoke Darwin custody {verb}"))?;
        // A valid failure receipt is still useful evidence (notably retirementUnknown).
        // Never replace it with the wrapper's exit status or infer successful retirement.
        decode_receipt(&output.stdout)
            .with_context(|| format!("Darwin custody {verb} returned {}", output.status))
    }
}

fn configured_path(environment: &BTreeMap<String, String>) -> Option<PathBuf> {
    let path = PathBuf::from(environment.get(HELPER_ENV)?);
    path.is_absolute().then_some(path)
}

/// Decode the shared helper wire contract, rejecting unknown schema versions.
pub fn decode_receipt(bytes: &[u8]) -> Result<Receipt> {
    let receipt: Receipt = serde_json::from_slice(bytes).context("decode Darwin custody receipt")?;
    ensure!(receipt.schema == SCHEMA, "unsupported Darwin custody receipt schema");
    Ok(receipt)
}

/// An operation-bound empty-slot receipt, not a root-process exit observation.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RetirementProof {
    schema: String,
    slot: String,
    sweeps: u64,
    signals: u64,
}

impl RetirementProof {
    /// Only the helper's completed root sweep can produce whole-slot retirement evidence.
    pub fn from_receipt(slot: &str, receipt: &Receipt) -> Result<Self> {
        ensure!(slot == SLOT && receipt.schema == SCHEMA, "Darwin retirement changed its bound slot or schema");
        let Status::Retired { sweeps, signals } = &receipt.status else {
            anyhow::bail!("Darwin slot retirement is unproven: {:?}", receipt.status);
        };
        Ok(Self { schema: receipt.schema.clone(), slot: slot.into(), sweeps: *sweeps, signals: *signals })
    }

    pub fn validate(&self, expected_slot: &str) -> Result<()> {
        ensure!(self.schema == SCHEMA && self.slot == SLOT && self.slot == expected_slot,
            "Darwin retirement changed its bound slot or schema");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unset_or_relative_helper_is_unavailable() {
        assert!(Helper::detect_in(&BTreeMap::new()).is_none());
        assert!(configured_path(&BTreeMap::new()).is_none());
        assert!(configured_path(&BTreeMap::from([(HELPER_ENV.into(), "helper".into())])).is_none());
    }

    #[test]
    fn unset_helper_reports_helper_unavailable() {
        let available = Helper::detect_in(&BTreeMap::new()).is_some();
        assert!(degradation_report(available).starts_with("helperUnavailable:"));
    }

    #[test]
    fn only_retired_receipts_prove_retirement() {
        let retired = decode_receipt(br#"{"schema":"fractal.darwin-custody.v2","status":{"status":"retired","sweeps":2,"signals":3}}"#).unwrap();
        let proof = RetirementProof::from_receipt(SLOT, &retired).unwrap();
        proof.validate(SLOT).unwrap();
        assert!(proof.validate("other").is_err());
        let bytes = serde_json::to_vec(&proof).unwrap();
        serde_json::from_slice::<RetirementProof>(&bytes).unwrap().validate(SLOT).unwrap();
        let unknown = decode_receipt(br#"{"schema":"fractal.darwin-custody.v2","status":{"status":"retirementUnknown","members":[123],"reason":"permission denied"}}"#).unwrap();
        assert!(RetirementProof::from_receipt(SLOT, &unknown).is_err());
        let empty = decode_receipt(br#"{"schema":"fractal.darwin-custody.v2","status":{"status":"empty"}}"#).unwrap();
        assert!(RetirementProof::from_receipt(SLOT, &empty).is_err());
        assert!(decode_receipt(br#"{"schema":"future","status":{"status":"empty"}}"#).is_err());
    }
}
