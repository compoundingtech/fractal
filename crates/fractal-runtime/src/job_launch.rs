//! Unreachable job capabilities: admission is NOT wired; no startup caller exists.
//! A delegated host/class keeper owns mkdir'd J; no per-job unit or sentinel.
pub mod cgroup;
pub mod keeper;

pub use cgroup::{ExitUsage, sample_exit};
