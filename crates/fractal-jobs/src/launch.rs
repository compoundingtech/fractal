//! Inactive launch protocol. An intent is evidence to fence, never permission to execute.
use crate::{Coverage, Covered, ExitOutcome, FailureReason};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::num::NonZeroU64;

/// Allocated before side effects and retained in the durable job history.
/// Unit 8 must fence an uncertain intent as unknown-after-host-loss, never rerun it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ExecutionGeneration(pub NonZeroU64);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LaunchIntent {
    pub generation: ExecutionGeneration,
    /// Generation-specific receipt location, not a reusable per-job file.
    pub outcome_file: String,
}

pub fn cgroup_name(job_id: &str, intent: &LaunchIntent) -> String {
    format!("job-{:x}-{}", Sha256::digest(job_id.as_bytes()), intent.generation.0)
}

pub fn keeper_name(host: &str) -> String {
    format!("fractal-keeper-heavy-{:x}.scope", Sha256::digest(host.as_bytes()))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "kebab-case")]
pub enum LaunchRefusal {
    MissingHeavySlice,
    InvalidOutcomePath,
    CgroupCollision,
    KeeperUnavailable,
    InvalidCgroupPath,
    CgroupNotDedicated,
    Preparation,
    /// Index into memory.high, memory.max, memory.swap.max, pids.max.
    CapReadback { index: usize },
}

impl std::fmt::Display for LaunchRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for LaunchRefusal {}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "kebab-case")]
pub enum CountersUnavailable {
    CgroupVanished,
    ReadFailed,
}

/// All four unit-12 observations must positively prove the configured limits.
pub fn require_caps(matches: &[Covered<bool>; 4]) -> Result<(), LaunchRefusal> {
    for (index, observation) in matches.iter().enumerate() {
        if observation.coverage != Coverage::Measured || observation.value != Some(true) {
            return Err(LaunchRefusal::CapReadback { index });
        }
    }
    Ok(())
}

/// B6's flat JSON object: holderOutcome is authoritative, never the holder exit code.
/// Other receipt fields (mode, label, PID, cgroup, launched, invocationId) are decoded
/// and checked by the runtime before these outcome facts may be consumed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "holderOutcome", rename_all = "kebab-case")]
pub enum HolderOutcome {
    Refused { reason: String },
    Withdrawn,
    MaxHold,
    ChildExit {
        #[serde(rename = "childExitCode")]
        child_exit_code: i32,
    },
    /// Written by B6 reconciliation when the holder disappeared.
    HolderLost,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LaunchOutcome {
    Refused { reason: String },
    Withdrawn,
    Exited(ExitOutcome),
    Failed(FailureReason),
}

impl From<HolderOutcome> for LaunchOutcome {
    fn from(outcome: HolderOutcome) -> Self {
        match outcome {
            HolderOutcome::Refused { reason } => Self::Refused { reason },
            HolderOutcome::Withdrawn => Self::Withdrawn,
            HolderOutcome::MaxHold => Self::Failed(FailureReason::MaxHold),
            HolderOutcome::HolderLost => Self::Failed(FailureReason::HolderLost),
            HolderOutcome::ChildExit { child_exit_code } => Self::Exited(ExitOutcome::Code(child_exit_code)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_names_are_stable_and_generation_specific() {
        let intent = LaunchIntent {
            generation: ExecutionGeneration(NonZeroU64::new(1).unwrap()),
            outcome_file: "/fixture/outcome".into(),
        };
        let mut next = intent.clone();
        next.generation = ExecutionGeneration(NonZeroU64::new(2).unwrap());
        assert_eq!(cgroup_name("job/1", &intent), cgroup_name("job/1", &intent));
        assert_ne!(cgroup_name("job/1", &intent), cgroup_name("job/1", &next));
        assert_ne!(cgroup_name("job/1", &intent), cgroup_name("job/2", &intent));
        assert_ne!(keeper_name("host-1"), keeper_name("host-2"));
    }

    #[test]
    fn every_b6_outcome_maps_without_an_exit_status() {
        for (json, expected) in [
            (r#"{"holderOutcome":"refused","reason":"caps"}"#, LaunchOutcome::Refused { reason: "caps".into() }),
            (r#"{"holderOutcome":"withdrawn"}"#, LaunchOutcome::Withdrawn),
            (r#"{"holderOutcome":"max-hold"}"#, LaunchOutcome::Failed(FailureReason::MaxHold)),
            (r#"{"holderOutcome":"holder-lost"}"#, LaunchOutcome::Failed(FailureReason::HolderLost)),
            (r#"{"holderOutcome":"child-exit","childExitCode":75}"#, LaunchOutcome::Exited(ExitOutcome::Code(75))),
        ] {
            let outcome: HolderOutcome = serde_json::from_str(json).unwrap();
            assert_eq!(LaunchOutcome::from(outcome), expected);
        }
        assert!(serde_json::from_str::<HolderOutcome>(r#"{"holderOutcome":"child-exit"}"#).is_err());
    }

    #[test]
    fn every_cap_requires_measured_true() {
        let good = Covered { value: Some(true), coverage: Coverage::Measured };
        assert_eq!(require_caps(&std::array::from_fn(|_| good.clone())), Ok(()));
        for index in 0..4 {
            for bad in [
                Covered { value: Some(false), coverage: Coverage::Measured },
                Covered { value: None, coverage: Coverage::Measured },
                Covered { value: Some(true), coverage: Coverage::Partial { reason: "partial".into() } },
                Covered { value: Some(true), coverage: Coverage::Unknown { reason: "missing".into() } },
            ] {
                let mut matches = std::array::from_fn(|_| good.clone());
                matches[index] = bad;
                assert_eq!(require_caps(&matches), Err(LaunchRefusal::CapReadback { index }));
            }
        }
    }
}
