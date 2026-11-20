//! Resolve profile intent once at the execution host; replay uses persisted membership estimates.
use super::*;
use crate::store::resource_membership::LaneRequest;

impl<R: RuntimeControl> Reconciler<R> {
    pub(super) fn resolve_resource_profile(&self, request: &mut LaneRequest) -> Result<()> {
        let Some(key) = request.profile.as_deref() else {
            anyhow::ensure!(request.profile_estimate.is_none(), "invalid-profile: estimate without a profile key");
            return Ok(());
        };
        fractal_runtime::validate_profile_key(key)?;
        if let Some(stored) = self.store.resource_profile_request(&request.id)? {
            anyhow::ensure!(stored.profile == request.profile && stored.owner == request.owner
                && stored.name == request.name && stored.host == request.host
                && stored.max_members == request.max_members && stored.coverage == request.coverage
                && stored.class == request.class && stored.admission_timeout_ms == request.admission_timeout_ms
                && stored.disk_path == request.disk_path,
                "resource profile request identity reused with different intent");
            if request.profile_estimate.is_some() {
                anyhow::ensure!(*request == stored, "resource profile estimate changed after acceptance");
            } else {
                anyhow::ensure!(request.capacity.is_none() && request.disk.is_none() && request.io.is_none()
                    || stored.profile_estimate.as_ref().is_some_and(|estimate| estimate.seed_revision.is_none())
                        && request.capacity == stored.capacity && request.disk == stored.disk && request.io == stored.io,
                    "profile-override-conflict: profile estimates are execution-host owned");
            }
            *request = stored;
            return Ok(());
        }
        // Only internal, already-persisted shell commands carry a resolved estimate here.
        // Public API and program/DSL decoders reject caller estimates before this seam.
        if request.profile_estimate.is_some() { return Ok(()); }
        let Some(seed) = self.seat_admission.finite_policy.profiles.get(key) else {
            anyhow::ensure!(request.capacity.is_some(), "unknown-profile: no seed for `{key}` on this execution host; an explicit declaration is required until cutover");
            request.profile_estimate = Some(fractal_runtime::ResourceProfileEstimate {
                seed_revision: None, learned_revision: None,
                samples: fractal_runtime::ResourceProfileSamples::default(), memory_high_bytes: None,
                source: None, reason: None,
                learning_coverage: fractal_runtime::ResourceLearningCoverage::ClientOnly,
            });
            return Ok(());
        };
        anyhow::ensure!(request.capacity.is_none() && request.disk.is_none() && request.io.is_none(),
            "profile-override-conflict: known profile `{key}` does not accept caller resource overrides");
        let estimate = seed.estimate()?;
        let disk = seed.disk.map(|bytes| -> Result<fractal_runtime::DiskRequest> {
            let path = request.disk_path.as_deref().context("profile-disk-path: a disk profile requires an execution-host disk path")?;
            anyhow::ensure!(std::path::Path::new(path).is_absolute(), "profile-disk-path: disk path must be absolute");
            let path = std::fs::canonicalize(path).context("profile disk path is unavailable on the execution host")?;
            Ok(fractal_runtime::DiskRequest { bytes, path: path.to_string_lossy().into_owned() })
        }).transpose()?;
        request.capacity = Some(seed.capacity());
        request.disk = disk;
        request.io = seed.io;
        request.profile_estimate = Some(estimate);
        Ok(())
    }
}
