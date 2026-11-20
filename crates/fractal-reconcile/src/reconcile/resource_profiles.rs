//! Resolve profile intent once at the execution host; replay uses persisted membership estimates.
use super::*;
use crate::store::resource_membership::LaneRequest;

impl<R: RuntimeControl> Reconciler<R> {
    pub(super) fn resolve_resource_profile(&self, request: &mut LaneRequest) -> Result<()> {
        let Some(key) = request.profile.as_deref() else {
            anyhow::bail!("raw-resource-declaration: Roll B requires an execution-host profile key");
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
        let seed = self.seat_admission.finite_policy.profiles.get(key)
            .with_context(|| format!("unknown-profile: no execution-host seed for `{key}`"))?;
        anyhow::ensure!(request.capacity.is_none() && request.disk.is_none() && request.io.is_none(),
            "profile-override-conflict: profile `{key}` does not accept caller resource overrides");
        let mut estimate = seed.estimate()?;
        let revision = estimate.seed_revision.as_deref().context("profile seed revision is missing")?;
        let learned = self.store.resource_profile_estimate(
            &request.host, key, revision,
            crate::store::resource_profile_learner::SeedLimits {
                memory_high_bytes: seed.memory_high.unwrap_or(seed.memory),
                memory_max_bytes: seed.memory, disk_bytes: seed.disk.unwrap_or(0),
            },
            self.seat_admission.finite_policy.profile_readiness,
        )?;
        estimate.learned_revision = learned.revision;
        estimate.samples = fractal_runtime::ResourceProfileSamples {
            memory: learned.memory_samples, disk: learned.disk_samples,
        };
        if learned.memory_learned {
            estimate.memory_high_bytes = Some(learned.memory_high_bytes);
        }
        let disk = seed.disk.map(|_| learned.disk_bytes).map(|bytes| -> Result<fractal_runtime::DiskRequest> {
            let path = request.disk_path.as_deref().context("profile-disk-path: a disk profile requires an execution-host disk path")?;
            anyhow::ensure!(std::path::Path::new(path).is_absolute(), "profile-disk-path: disk path must be absolute");
            let path = std::fs::canonicalize(path).context("profile disk path is unavailable on the execution host")?;
            Ok(fractal_runtime::DiskRequest { bytes, path: path.to_string_lossy().into_owned() })
        }).transpose()?;
        request.capacity = Some(fractal_runtime::Capacity { cores: seed.cores, memory: learned.memory_max_bytes });
        request.disk = disk;
        request.io = seed.io;
        request.profile_estimate = Some(estimate);
        Ok(())
    }
}
