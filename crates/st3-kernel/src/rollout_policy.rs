//! The restart policy an apply carries, shared by the wire model and the store.
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub mode: Mode,
    #[serde(default = "default_deadline")]
    pub deadline_ms: u64,
    #[serde(default)]
    pub force_after_deadline: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    WhenIdle,
}
fn default_deadline() -> u64 {
    30 * 60 * 1000
}
impl Policy {
    pub fn when_idle(deadline_ms: u64, force_after_deadline: bool) -> Self {
        Self {
            mode: Mode::WhenIdle,
            deadline_ms,
            force_after_deadline,
        }
    }
    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            (1..=7 * 24 * 60 * 60 * 1000).contains(&self.deadline_ms),
            "rollout deadline must be positive and at most seven days"
        );
        Ok(())
    }
}

/// Unflagged apply waits for a safe point, alerting after thirty minutes without forcing.
pub fn default_restart_policy() -> Option<Policy> {
    Some(Policy::when_idle(1_800_000, false))
}
