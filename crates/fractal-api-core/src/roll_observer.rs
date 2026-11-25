//! Only the target daemon measures and signs host adoption; callers supply a signed resolution id.
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, ensure};
use serde_json::Value;

use crate::roll_completion::{self, Adoption, Identity, Receipt, Verification};
use crate::store::Store;

pub fn observe(store: &Store, claim_id: &str) -> Result<Value> {
    // A source cut may not overtake an observation timestamp that has not yet
    // committed. Share capture/publication serialization with the native census.
    let source = store.extension::<crate::roll_source::CaptureLock>();
    let _capture = source.0.lock().map_err(|_| anyhow::anyhow!("source capture lock poisoned"))?;
    let ((resolved, adoption, evidence), now) = after_measurement(store, || {
        let resolved = roll_completion::resolved_target(store, claim_id)?;
        ensure!(resolved.host == store.origin(), "resolved target is not this daemon's host");
        // Match the running daemon's health implementation, not an executable on PATH.
        store.index()?;
        let ready = fractal_runtime::systemd::health().unresponsive.is_none();
        let home_profile = home_profile();
        let (adoption, evidence) = measure(
            &resolved.target,
            &resolved.expected_system_path,
            &resolved.expected_home_path,
            Path::new("/run/current-system"),
            home_profile.as_deref(),
            fractal_drivers::version::build_commit(),
            ready,
        );
        Ok((resolved, adoption, evidence))
    })?;
    roll_completion::publish_observation(store, Receipt::HostObserved {
        version: 1,
        roll_id: resolved.roll_id,
        declared_at_ms: resolved.declared_at_ms,
        host: resolved.host,
        observed_at_ms: now,
        target: resolved.target,
        adoption,
        evidence: evidence.map(Box::new),
    }, now)
}

/// Never date an observation from the beginning of a possibly slow measurement.
/// The caller retains source-capture serialization through signed publication.
fn after_measurement<T>(store: &Store, measure: impl FnOnce() -> Result<T>) -> Result<(T, u64)> {
    let measured = measure()?;
    let now = {
        let writer = store.connection.write();
        let accepted: u64 = smallclaims::store::checkpoint_agreement::write_time(&writer, store.origin())?
            .try_into().context("native acceptance clock overflow")?;
        crate::roll_source::now_ms()?.max(accepted)
    };
    Ok((measured, now))
}

fn home_profile() -> Option<PathBuf> {
    let state = std::env::var_os("XDG_STATE_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))?;
    state.is_absolute().then(|| state.join("nix/profiles/home-manager"))
}

fn active_path(path: &Path) -> Result<String> {
    let resolved = path.canonicalize().with_context(|| format!("read active profile {}", path.display()))?;
    ensure!(resolved.starts_with("/nix/store"), "active profile is not an immutable store output");
    Ok(resolved.to_str().context("profile path is not UTF-8")?.to_owned())
}

fn full_revision(revision: &str) -> bool {
    matches!(revision.len(), 40 | 64) && revision.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[allow(clippy::too_many_arguments)]
fn measure(
    target: &Identity,
    expected_system: &str,
    expected_home: &str,
    system_profile: &Path,
    home_profile: Option<&Path>,
    revision: &str,
    ready: bool,
) -> (Adoption, Option<Verification>) {
    if !full_revision(revision) {
        return (Adoption::Unknown, None);
    }
    let Some((system, home)) = active_path(system_profile).ok().zip(
        home_profile.and_then(|path| active_path(path).ok())
    ) else {
        return (Adoption::Unknown, None);
    };
    let proof = Verification {
        system_path: system,
        home_path: home,
        expected_system_path: expected_system.to_owned(),
        expected_home_path: expected_home.to_owned(),
        daemon_revision: revision.to_owned(),
        daemon_ready: ready,
    };
    (classify(target, &proof), Some(proof))
}

fn classify(target: &Identity, proof: &Verification) -> Adoption {
    let exact = proof.system_path == proof.expected_system_path
        && proof.home_path == proof.expected_home_path
        && target.system_path.as_deref().is_none_or(|path| path == proof.expected_system_path)
        && target.home_path.as_deref().is_none_or(|path| path == proof.expected_home_path)
        && proof.daemon_revision == target.daemon_revision && proof.daemon_ready;
    if exact { Adoption::Verified } else { Adoption::NotAdopted }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measurement_crossing_deadline_uses_completion_native_clock() {
        let store = Store::open_memory("host-a").unwrap();
        let root = tempfile::tempdir().unwrap();
        let profile = root.path().join("current-system");
        std::os::unix::fs::symlink(root.path(), &profile).unwrap();
        let source = store.extension::<crate::roll_source::CaptureLock>();
        let _capture = source.0.lock().unwrap();
        let started = crate::roll_source::now_ms().unwrap();
        let deadline = started + 24 * 60 * 60 * 1_000;
        let ((adoption, proof), observed) = after_measurement(&store, || {
            // Exercise a real profile read, not caller-supplied evidence. A
            // non-store target correctly stays unknown independently of timing.
            let result = measure(&target(), "/nix/store/system", "/nix/store/home",
                &profile, Some(&profile), &"b".repeat(40), true);
            // Advance the actual native acceptance clock while measurement is
            // in progress: sampling before this callback would falsely predate
            // the deadline. No sleep or fake clock implementation is involved.
            store.set_write_clock_at(u128::from(deadline + 1))?;
            Ok(result)
        }).unwrap();
        assert_eq!(adoption, Adoption::Unknown);
        assert!(proof.is_none());
        assert!(started <= deadline);
        assert!(observed > deadline, "a measurement finishing late cannot be dated at its start");
    }

    #[test]
    fn every_declared_component_requires_exact_observed_proof() {
        let target = target();
        let proof = Verification {
            system_path: "/nix/store/system".into(), home_path: "/nix/store/home".into(),
            expected_system_path: "/nix/store/system".into(), expected_home_path: "/nix/store/home".into(),
            daemon_revision: target.daemon_revision.clone(), daemon_ready: true,
        };
        assert_eq!(classify(&target, &proof), Adoption::Verified);
        for mismatch in [
            Verification { system_path: "/nix/store/old".into(), ..proof.clone() },
            Verification { home_path: "/nix/store/old".into(), ..proof.clone() },
            Verification { daemon_revision: "c".repeat(40), ..proof.clone() },
            Verification { daemon_ready: false, ..proof.clone() },
        ] {
            assert_eq!(classify(&target, &mismatch), Adoption::NotAdopted);
        }
    }

    fn target() -> Identity {
        Identity { dotfiles_revision: "a".repeat(40),
            daemon_revision: "b".repeat(40), system_path: None, home_path: None }
    }

    #[test]
    fn missing_profiles_are_unknown() {
        let root = tempfile::tempdir().unwrap();
        let missing = root.path().join("missing");
        let (adoption, proof) = measure(&target(), "/nix/store/system", "/nix/store/home",
            &missing, Some(&missing), &"b".repeat(40), true);
        assert_eq!(adoption, Adoption::Unknown);
        assert!(proof.is_none());
    }

    #[test]
    fn local_nonstore_profiles_cannot_become_adoption_proof() {
        let root = tempfile::tempdir().unwrap();
        let system = root.path().join("system");
        let home = root.path().join("home");
        std::fs::create_dir(&system).unwrap();
        std::fs::create_dir(&home).unwrap();
        let (adoption, proof) = measure(&target(), system.to_str().unwrap(), home.to_str().unwrap(),
            &system, Some(&home), &"b".repeat(40), true);
        assert_eq!(adoption, Adoption::Unknown);
        assert!(proof.is_none());
    }

    #[test]
    fn short_or_unknown_compiled_revision_is_never_proof() {
        assert!(full_revision(&"a".repeat(40)));
        assert!(full_revision(&"b".repeat(64)));
        for revision in ["unknown", "abcdef0", "", &"g".repeat(40)] {
            assert!(!full_revision(revision));
        }
    }
}
