//! Owner-local reboot witnesses. A negative store scan is not a process scan:
//! it is useful only across an actual, recent reboot and under the spawn fence.
use super::*;
use std::time::{SystemTime, UNIX_EPOCH};

mod home_stores;
mod system_stores;

/// A capture older than this before the new kernel boot cannot authorize repair.
pub const REBOOT_WITNESS_MAX_PREBOOT_AGE_MS: u64 = 10 * 60 * 1000;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RebootWitnessScan {
    pub store: String,
    pub records: u64,
    pub bytes: u64,
    pub digest: String,
    pub exclusions: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RebootWitnessManifest {
    pub version: u8,
    pub operation_id: String,
    pub runtime_id: String,
    pub host: String,
    pub owner_uid: u32,
    pub boot_id: String,
    pub captured_at_unix_ms: u64,
    pub home: PathBuf,
    pub state_root: PathBuf,
    pub pty_roots: Vec<PathBuf>,
    pub scans: Vec<RebootWitnessScan>,
}

/// Retain this guard through manifest persistence or the repair writer transaction.
#[derive(Debug)]
pub struct RebootWitnessProof {
    pub evidence: serde_json::Value,
    _spawn_lock: File,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct BootIdentity {
    id: String,
    started_at_unix_ms: u64,
}

fn unavailable() -> anyhow::Error {
    LegacyStartProofUnavailable { stage: "reboot-witness", pid: 0, errno: None }.into()
}

fn boot_identity() -> Result<BootIdentity> {
    #[cfg(not(target_os = "linux"))]
    { Err(unavailable()) }
    #[cfg(target_os = "linux")]
    {
        let id = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")?;
        let id = id.trim().to_owned();
        anyhow::ensure!(id.len() == 36 && id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-'), "invalid boot identity");
        let stat = std::fs::read_to_string("/proc/stat")?;
        let mut times = stat.lines().filter_map(|line| line.strip_prefix("btime "));
        let seconds: u64 = times.next().context("missing boot time")?.trim().parse()?;
        anyhow::ensure!(times.next().is_none(), "ambiguous boot time");
        Ok(BootIdentity { id, started_at_unix_ms: seconds.checked_mul(1000).context("boot time overflow")? })
    }
}

fn validate_manifest(
    manifest: &RebootWitnessManifest,
    host: &str,
    operation: &str,
    runtime_id: &str,
    current: &BootIdentity,
) -> Result<()> {
    anyhow::ensure!(manifest.version == 1 && manifest.host == host
        && manifest.operation_id == operation && manifest.runtime_id == runtime_id,
        "reboot witness binding mismatch");
    anyhow::ensure!(manifest.owner_uid == unsafe { libc::geteuid() }, "reboot witness owner mismatch");
    anyhow::ensure!(manifest.boot_id != current.id && !manifest.boot_id.is_empty(), "reboot witness needs a different boot");
    let age = current.started_at_unix_ms.checked_sub(manifest.captured_at_unix_ms)
        .context("reboot witness was not captured before boot")?;
    anyhow::ensure!(age <= REBOOT_WITNESS_MAX_PREBOOT_AGE_MS, "reboot witness is too old");
    anyhow::ensure!(!manifest.scans.is_empty() && !manifest.pty_roots.is_empty(), "incomplete reboot witness");
    Ok(())
}

fn scan_stores(home: &Path, state_root: &Path, roots: &[PathBuf], operation: &str) -> Result<Vec<RebootWitnessScan>> {
    let mut scans = home_stores::scan_home_stores(home, state_root, roots, operation)?;
    scans.extend(system_stores::scan_system_stores(home, operation)?);
    scans.sort_by(|a, b| a.store.cmp(&b.store));
    anyhow::ensure!(!scans.is_empty(), "empty reboot witness store inventory");
    Ok(scans)
}

impl PtyRuntime {
    /// Capture before the authorized reboot; the caller must persist the manifest
    /// as a daemon-authored owner-local observation before dropping this proof.
    pub fn capture_reboot_witness(
        &self, runtime_id: &str, operation: &str, host: &str,
        home: &Path, state_root: &Path,
    ) -> Result<RebootWitnessProof> {
        (|| {
            anyhow::ensure!(!operation.is_empty() && !host.is_empty(), "invalid reboot witness target");
            let lock = self.try_spawn_lock(runtime_id)?.context("runtime spawn fence busy")?;
            let before = boot_identity()?;
            let mut roots = vec![self.root.clone(), pty_core::registry::session_dir(), home.join(".local/state/pty")];
            roots.sort(); roots.dedup();
            let scans = scan_stores(home, state_root, &roots, operation)?;
            anyhow::ensure!(boot_identity()? == before, "boot changed during witness capture");
            let captured_at_unix_ms = u64::try_from(SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis())?;
            let manifest = RebootWitnessManifest {
                version: 1, operation_id: operation.into(), runtime_id: runtime_id.into(), host: host.into(),
                owner_uid: unsafe { libc::geteuid() }, boot_id: before.id,
                captured_at_unix_ms, home: home.into(), state_root: state_root.into(), pty_roots: roots, scans,
            };
            Ok(RebootWitnessProof { evidence: serde_json::to_value(manifest)?, _spawn_lock: lock })
        })().map_err(|_: anyhow::Error| unavailable())
    }

    /// Freshly rescan at BOTH plan and apply. The retained manifest and its digest
    /// come only from the owner daemon's observation ledger, never request JSON.
    pub fn prove_reboot_witness(
        &self, runtime_id: &str, operation: &str, host: &str,
        manifest: &serde_json::Value, manifest_id: &str, manifest_digest: &str,
        home: &Path, state_root: &Path,
    ) -> Result<RebootWitnessProof> {
        (|| {
            let lock = self.try_spawn_lock(runtime_id)?.context("runtime spawn fence busy")?;
            let manifest: RebootWitnessManifest = serde_json::from_value(manifest.clone())?;
            let boot = boot_identity()?;
            validate_manifest(&manifest, host, operation, runtime_id, &boot)?;
            anyhow::ensure!(manifest.home == home && manifest.state_root == state_root
                && manifest.pty_roots.contains(&self.root), "reboot witness roots changed");
            let scans = scan_stores(home, state_root, &manifest.pty_roots, operation)?;
            anyhow::ensure!(boot_identity()? == boot, "boot changed during witness proof");
            anyhow::ensure!(
                manifest.scans.iter().map(|scan| (&scan.store, &scan.exclusions)).collect::<Vec<_>>()
                    == scans.iter().map(|scan| (&scan.store, &scan.exclusions)).collect::<Vec<_>>(),
                "reboot witness store contract changed"
            );
            let evidence = serde_json::json!({
                "legacy": true, "runtime_id": runtime_id, "operation_id": operation,
                "witness_kind": "reboot", "manifest_id": manifest_id,
                "manifest_digest": manifest_digest, "old_boot_id": manifest.boot_id,
                "current_boot_id": boot.id, "current_btime_unix_ms": boot.started_at_unix_ms,
                "captured_at_unix_ms": manifest.captured_at_unix_ms,
                "max_preboot_age_ms": REBOOT_WITNESS_MAX_PREBOOT_AGE_MS,
                "store_contract": manifest.scans.iter().map(|scan| (&scan.store, &scan.exclusions)).collect::<Vec<_>>(),
                "current_scans": scans,
            });
            Ok(RebootWitnessProof { evidence, _spawn_lock: lock })
        })().map_err(|_: anyhow::Error| unavailable())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn manifest() -> RebootWitnessManifest {
        RebootWitnessManifest { version: 1, operation_id: "operation".into(), runtime_id: "runtime".into(),
            host: "host".into(), owner_uid: unsafe { libc::geteuid() }, boot_id: "old".into(),
            captured_at_unix_ms: 1000, home: "/home/owner".into(), state_root: "/state".into(),
            pty_roots: vec!["/pty".into()], scans: vec![RebootWitnessScan { store: "pty".into(), records: 0,
                bytes: 0, digest: "digest".into(), exclusions: vec![] }] }
    }
    #[test]
    fn reboot_witness_accepts_recent_previous_boot() {
        assert!(validate_manifest(&manifest(), "host", "operation", "runtime", &BootIdentity { id: "new".into(), started_at_unix_ms: 2000 }).is_ok());
    }
    #[test]
    fn reboot_witness_refuses_same_boot_foreign_host_and_old_capture() {
        let boot = BootIdentity { id: "new".into(), started_at_unix_ms: 2000 };
        assert!(validate_manifest(&manifest(), "other", "operation", "runtime", &boot).is_err());
        let mut m = manifest(); m.boot_id = boot.id.clone();
        assert!(validate_manifest(&m, "host", "operation", "runtime", &boot).is_err());
        let mut boot = boot; boot.started_at_unix_ms = 1001 + REBOOT_WITNESS_MAX_PREBOOT_AGE_MS;
        assert!(validate_manifest(&manifest(), "host", "operation", "runtime", &boot).is_err());
    }
}
