//! Declared host-local seeds and immutable estimates; callers do not own learned values.
use crate::{Capacity, IoClass};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum ResourceLearningCoverage {
    #[default]
    ClientOnly,
    Full,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResourceProfileSeed {
    pub cores: u32,
    pub memory: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_high: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disk: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub io: Option<IoClass>,
    #[serde(default)]
    pub learning_coverage: ResourceLearningCoverage,
    #[serde(default)]
    pub recovery_capable: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResourceProfileSamples {
    pub memory: u64,
    pub disk: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResourceProfileEstimate {
    pub seed_revision: Option<String>,
    pub learned_revision: Option<u64>,
    pub samples: ResourceProfileSamples,
    /// Absent for declared-cap seeds; a learned estimate separates high from charged max.
    pub memory_high_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default)]
    pub learning_coverage: ResourceLearningCoverage,
}

pub fn validate_profile_key(key: &str) -> Result<()> {
    ensure!(!key.is_empty() && key.len() <= 128 && key.bytes().all(|byte|
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_' | b'.')),
        "invalid-profile: profile keys must contain 1–128 lowercase ASCII letters, digits, '-', '_' or '.'");
    Ok(())
}

impl ResourceProfileSeed {
    pub fn validate(&self) -> Result<()> {
        ensure!(self.cores > 0, "invalid-profile: profile cores must be positive");
        ensure!(self.memory >= 256 << 20, "invalid-profile: profile memory must be at least 256M");
        ensure!(self.memory_high.is_none_or(|high| high > 0 && high <= self.memory),
            "invalid-profile: memoryHigh must be positive and no larger than memory");
        ensure!(self.source.as_deref() != Some("declared-provisional")
            || self.reason.as_ref().is_some_and(|reason| !reason.trim().is_empty()),
            "invalid-profile: provisional seeds require a reason");
        Ok(())
    }

    pub fn capacity(&self) -> Capacity { Capacity { cores: self.cores, memory: self.memory } }

    pub fn estimate(&self) -> Result<ResourceProfileEstimate> {
        self.validate()?;
        // The domain includes estimate semantics, not timestamps, host labels or other keys.
        let bytes = serde_json::to_vec(&("declared-cap-v1", self))?;
        let digest = Sha256::digest(&bytes);
        Ok(ResourceProfileEstimate {
            seed_revision: Some(format!("sha256:{digest:x}")), learned_revision: None,
            samples: ResourceProfileSamples::default(), memory_high_bytes: self.memory_high,
            source: self.source.clone(), reason: self.reason.clone(),
            learning_coverage: self.learning_coverage,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_revision_is_stable_and_binds_every_declared_dimension() {
        let seed = ResourceProfileSeed { cores: 2, memory: 1 << 30, memory_high: None,
            source: None, reason: None, disk: Some(1000), io: None,
            learning_coverage: ResourceLearningCoverage::ClientOnly, recovery_capable: false };
        let estimate = seed.estimate().unwrap();
        assert_eq!(estimate, seed.clone().estimate().unwrap());
        for changed in [
            ResourceProfileSeed { cores: 3, ..seed.clone() },
            ResourceProfileSeed { memory: 2 << 30, ..seed.clone() },
            ResourceProfileSeed { disk: Some(2000), ..seed.clone() },
            ResourceProfileSeed { io: Some(IoClass::Heavy), ..seed.clone() },
        ] { assert_ne!(estimate.seed_revision, changed.estimate().unwrap().seed_revision); }
        assert_eq!(estimate.learned_revision, None);
        assert_eq!(estimate.samples, ResourceProfileSamples::default());
        assert_eq!(estimate.memory_high_bytes, None);
    }

    #[test]
    fn invalid_keys_and_seeds_refuse_without_defaults() {
        for key in ["", "Land-Check", "../check", "check/quick", "check quick"] {
            assert!(validate_profile_key(key).is_err());
        }
        validate_profile_key("land-check-native").unwrap();
        let seed = ResourceProfileSeed { cores: 0, memory: 1 << 30, memory_high: None,
            source: None, reason: None, disk: None, io: None,
            learning_coverage: ResourceLearningCoverage::ClientOnly, recovery_capable: false };
        assert!(seed.estimate().is_err());
        assert!(ResourceProfileSeed { cores: 1, memory: 1, ..seed.clone() }.estimate().is_err());
        let zero_growth = ResourceProfileSeed { cores: 1, disk: Some(0), ..seed };
        zero_growth.validate().unwrap();
        assert!(zero_growth.estimate().is_ok(), "zero-byte reservations are valid for reclaim operations");
    }
}
