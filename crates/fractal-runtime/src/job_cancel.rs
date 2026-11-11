//! INACTIVE whole-tree cancellation capabilities. No daemon or endpoint caller.
//! The activation adapter must authenticate the host binding and audit and hold
//! the SAME per-job generation serializer as launch/release/recovery until the
//! sampled stop result is durably recorded. These APIs grant no authority alone.
use fractal_jobs::{cancel::StopUncertain, recovery::RecoveryUsage};
use std::time::Duration;
pub mod system;
mod sequence;
pub use sequence::{StopAuthentication, StopJournal, StopRequest, StopResult};

pub trait StopEffects {
    /// SIGTERM holder; configured grace; cgroup.kill; configured bounded empty
    /// confirmation; usage sample; rmdir J. Success certifies the entire sequence,
    /// not just holder exit. A lost pre-release gate is an uncertain stop.
    fn stop(&mut self, grace: Duration, escalation: Duration) -> Result<RecoveryUsage, StopUncertain>;
}
