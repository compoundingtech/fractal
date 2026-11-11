//! Unreachable job launch path: admission is NOT wired; no scheduler calls it.
//! A shared delegated keeper owns mkdir'd J; no per-job unit or sentinel exists.
//! Unlike ExecRuntime::spawn, this never replaces another generation.
pub mod cgroup;
mod driver;
pub mod gated;
pub mod keeper;
mod keeper_systemd;
mod sequence;

pub use cgroup::{ExitUsage, sample_exit};
pub use driver::{CgroupJobLaunch, JobLaunch, JobLaunchEffects, JobLaunchResult, LaunchJournal};
pub use keeper_systemd::SystemdKeeper;
pub use sequence::UncertainRelease;
