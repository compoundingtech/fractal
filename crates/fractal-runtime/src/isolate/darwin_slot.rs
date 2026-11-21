//! Darwin slot launch, capability detection, and operation-fenced retirement receipts.
//! The foreground helper execs the payload with inherited stdin/stdout/stderr, preserving
//! the finite execution authorization gate. Wrapper exit alone never proves retirement.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::io::Write as _;
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

use anyhow::{Context as _, Result, ensure};
pub use fractal_mac_custody::{Receipt, Status};
use fractal_mac_custody::{Plan, SCHEMA};
use serde::{Deserialize, Serialize};

/// Explicit finite-job custody request. This never changes ambient PTY or exec isolation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Slot {
    Heavy1,
    Gate1,
    Gate2,
    Gate3,
    Eval1,
}

impl Slot {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Heavy1 => "heavy1",
            Self::Gate1 => "gate1",
            Self::Gate2 => "gate2",
            Self::Gate3 => "gate3",
            Self::Eval1 => "eval1",
        }
    }

    pub fn for_lane(lane: &str) -> Result<&'static [Self]> {
        match lane {
            "heavy" => Ok(&[Self::Heavy1]),
            "gate" => Ok(&[Self::Gate1, Self::Gate2, Self::Gate3]),
            "eval" => Ok(&[Self::Eval1]),
            _ => anyhow::bail!("darwin-custody-lane: unsupported lane {lane}"),
        }
    }

    pub fn root(self) -> &'static Path {
        Path::new(fractal_mac_custody::slot(self.name()).expect("runtime slot is compiled into helper").root)
    }
}

pub(crate) fn slot_for_unit(unit: &str) -> Option<Slot> {
    let suffix = unit.strip_prefix("st3-darwin-")?.strip_suffix(".scope")?;
    let (name, digest) = suffix.split_once('-')?;
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) { return None; }
    [Slot::Heavy1, Slot::Gate1, Slot::Gate2, Slot::Gate3, Slot::Eval1]
        .into_iter().find(|slot| slot.name() == name)
}

pub const HELPER_ENV: &str = "FRACTAL_MAC_CUSTODY";

static HELPER: OnceLock<Helper> = OnceLock::new();

/// Cached capability only; never selects a slot or changes default isolation.
pub fn available() -> bool {
    super::mode();
    HELPER.get().is_some()
}

pub(crate) fn current_helper() -> Result<&'static Helper> {
    if HELPER.get().is_none() {
        Helper::detect_in(&std::env::vars().collect())
            .context("helperUnavailable: requested Darwin slot has no authorized helper")?;
    }
    HELPER.get().context("helperUnavailable: requested Darwin slot has no authorized helper")
}

pub const HELPER_UNAVAILABLE: &str =
    "helperUnavailable: Darwin custody helper is unavailable; tasks remain detached.";

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
            .args(["leftovers", Slot::Heavy1.name()])
            .env_clear()
            .envs(environment)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status().ok()?.success();
        if !allowed { return None; }
        let helper = Self { path };
        let _ = HELPER.set(helper.clone());
        Some(helper)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The caller must admit only an `Empty` receipt; members and unknown states retain custody.
    pub fn leftovers(&self, slot: Slot) -> Result<Receipt> {
        self.invoke("leftovers", slot, None)
    }

    /// Select an unbound, empty slot. Empty cwd requests any slot in the lane;
    /// an explicit cwd must already lie beneath one of that lane's slot roots.
    pub fn select_slot(&self, lane: &str, cwd: &Path) -> Result<Slot> {
        let slots = Slot::for_lane(lane)?;
        let cwd = if cwd.as_os_str().is_empty() { None } else {
            Some(cwd.canonicalize().context("darwin-custody-cwd: canonicalize job cwd")?)
        };
        let mut contained = cwd.is_none();
        for &slot in slots {
            let root = slot.root().canonicalize().context("canonicalize Darwin slot root")?;
            if cwd.as_ref().is_some_and(|cwd| !cwd.starts_with(&root)) { continue; }
            contained = true;
            if matches!(self.leftovers(slot)?.status, Status::Empty) { return Ok(slot); }
        }
        ensure!(contained, "darwin-custody-cwd: cwd must lie beneath a {lane} slot root");
        anyhow::bail!("darwin-custody-slot: no empty {lane} slot is available")
    }

    /// Darwin RSS samples are observations, never memory enforcement or peak measurements.
    pub fn sample(&self, slot: Slot) -> Result<Receipt> {
        self.invoke("sample", slot, None)
    }

    /// Both natural exit and cancellation require this operation before retirement is claimed.
    /// The guard must still bind the caller to the current slot operation.
    pub fn cancel(&self, slot: Slot, job_id: &str, guard: &dyn Fn() -> Result<()>) -> Result<Receipt> {
        validate_job_id(job_id)?;
        guard()?;
        self.invoke("cancel", slot, Some(job_id))
    }

    fn invoke(&self, verb: &str, slot: Slot, job_id: Option<&str>) -> Result<Receipt> {
        ensure!(cfg!(target_os = "macos"), "Darwin custody requires macOS");
        let mut command = Command::new("/usr/bin/sudo");
        command.arg("-n").arg(&self.path).args([verb, slot.name()]);
        if let Some(job_id) = job_id {
            command.arg(job_id);
        }
        let output = command.stdin(Stdio::null()).output()
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
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RetirementProof {
    schema: String,
    slot: String,
    sweeps: u64,
    signals: u64,
    job_id: String,
}

impl RetirementProof {
    /// Only the helper's completed root sweep can produce whole-slot retirement evidence.
    pub fn from_receipt(slot: &str, job_id: &str, receipt: &Receipt) -> Result<Self> {
        validate_job_id(job_id)?;
        ensure!(fractal_mac_custody::slot(slot).is_some() && receipt.schema == SCHEMA, "Darwin retirement changed its bound slot or schema");
        let Status::Retired { sweeps, signals } = &receipt.status else {
            anyhow::bail!("Darwin slot retirement is unproven: {:?}", receipt.status);
        };
        Ok(Self { schema: receipt.schema.clone(), slot: slot.into(), job_id: job_id.into(), sweeps: *sweeps, signals: *signals })
    }

    pub fn validate(&self, expected_slot: &str, expected_job: &str) -> Result<()> {
        ensure!(self.schema == SCHEMA && fractal_mac_custody::slot(&self.slot).is_some() && self.slot == expected_slot
            && self.job_id == expected_job,
            "Darwin retirement changed its bound slot, job, or schema");
        validate_job_id(expected_job)?;
        Ok(())
    }
}

fn validate_job_id(job_id: &str) -> Result<()> {
    ensure!(!job_id.is_empty() && job_id.len() <= 128 && job_id.bytes().all(|byte|
        byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')),
        "invalid Darwin custody job ID");
    Ok(())
}

pub(crate) fn launch(
    helper: &Helper, unit: &str, program: &OsStr, arguments: &[&OsStr],
    cwd: &Path, environment: Option<&BTreeMap<String, String>>,
) -> Result<Command> {
    validate_job_id(unit)?;
    let slot = slot_for_unit(unit).context("invalid Darwin custody unit")?;
    let leftovers = helper.leftovers(slot)?;
    ensure!(matches!(&leftovers.status, Status::Empty),
        "Darwin slot is not empty: {:?}", leftovers.status);
    let root = slot.root().canonicalize()
        .context("canonicalize Darwin slot root")?;
    let cwd = cwd.canonicalize().context("canonicalize Darwin job cwd")?;
    ensure!(cwd.starts_with(&root), "Darwin job cwd must lie under {}", root.display());
    let mut env = environment.cloned().unwrap_or_else(|| std::env::vars().collect());
    env.insert("PWD".into(), cwd.to_str().context("Darwin cwd is not UTF-8")?.into());
    env.insert("FRACTAL_MAC_CUSTODY_STDIO".into(), "pipe".into());
    let program = crate::resolve_executable(program.to_str().context("Darwin program is not UTF-8")?, &env)?;
    let argv = std::iter::once(program.as_os_str()).chain(arguments.iter().copied())
        .map(|arg| arg.to_str().map(str::to_owned).context("Darwin argv is not UTF-8"))
        .collect::<Result<Vec<_>>>()?;
    let plan = Plan { job_id: unit.into(), argv, cwd, env };
    let directory = root.join(".fractal-plans");
    std::fs::create_dir_all(&directory).context("create Darwin plan directory")?;
    ensure!(directory.canonicalize()?.starts_with(&root), "Darwin plan directory escaped slot root");
    let path = directory.join(format!("{unit}.json"));
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600)
        .open(&path).context("create Darwin job plan")?;
    file.write_all(&serde_json::to_vec(&plan)?)?;
    file.sync_all()?;
    let mut command = Command::new("/usr/bin/sudo");
    command.arg("-n").arg(helper.path()).args(["run", slot.name()]).arg(path);
    Ok(command)
}

#[cfg(test)]
mod tests {
    use super::*;
    const SLOT: &str = "heavy1";

    #[test]
    fn lane_slots_and_unit_roundtrip() {
        assert_eq!(Slot::for_lane("heavy").unwrap(), &[Slot::Heavy1]);
        assert_eq!(Slot::for_lane("gate").unwrap(), &[Slot::Gate1, Slot::Gate2, Slot::Gate3]);
        assert_eq!(Slot::for_lane("eval").unwrap(), &[Slot::Eval1]);
        assert!(Slot::for_lane("other").unwrap_err().to_string().contains("darwin-custody-lane"));
        for lane in ["heavy", "gate", "eval"] {
            for &slot in Slot::for_lane(lane).unwrap() {
                let unit = format!("st3-darwin-{}-{}.scope", slot.name(), "a".repeat(64));
                assert_eq!(slot_for_unit(&unit), Some(slot));
                assert_eq!(slot.root(), Path::new(fractal_mac_custody::slot(slot.name()).unwrap().root));
            }
        }
        for unit in ["st3-darwin-other-aaaaaaaa.scope", "st3-darwin-gate1-short.scope"] {
            assert!(slot_for_unit(unit).is_none());
        }
    }

    #[test]
    fn unset_or_relative_helper_is_unavailable() {
        assert!(Helper::detect_in(&BTreeMap::new()).is_none());
        assert!(configured_path(&BTreeMap::new()).is_none());
        assert!(configured_path(&BTreeMap::from([(HELPER_ENV.into(), "helper".into())])).is_none());
    }

    #[test]
    fn unset_helper_reports_helper_unavailable() {
        assert!(Helper::detect_in(&BTreeMap::new()).is_none());
        assert!(HELPER_UNAVAILABLE.starts_with("helperUnavailable:"));
    }

    #[test]
    fn only_retired_receipts_prove_retirement() {
        let retired = decode_receipt(br#"{"schema":"fractal.darwin-custody.v2","status":{"status":"retired","sweeps":2,"signals":3}}"#).unwrap();
        let proof = RetirementProof::from_receipt(SLOT, "job", &retired).unwrap();
        proof.validate(SLOT, "job").unwrap();
        assert!(proof.validate("other", "job").is_err());
        assert!(proof.validate(SLOT, "replacement").is_err());
        let bytes = serde_json::to_vec(&proof).unwrap();
        serde_json::from_slice::<RetirementProof>(&bytes).unwrap().validate(SLOT, "job").unwrap();
        let unknown = decode_receipt(br#"{"schema":"fractal.darwin-custody.v2","status":{"status":"retirementUnknown","members":[123],"reason":"permission denied"}}"#).unwrap();
        assert!(RetirementProof::from_receipt(SLOT, "job", &unknown).is_err());
        let empty = decode_receipt(br#"{"schema":"fractal.darwin-custody.v2","status":{"status":"empty"}}"#).unwrap();
        assert!(RetirementProof::from_receipt(SLOT, "job", &empty).is_err());
        let stale = decode_receipt(br#"{"schema":"fractal.darwin-custody.v2","status":{"status":"staleJob","boundJobId":"replacement"}}"#).unwrap();
        assert!(RetirementProof::from_receipt(SLOT, "job", &stale).is_err());
        assert!(decode_receipt(br#"{"schema":"future","status":{"status":"empty"}}"#).is_err());
    }
}
