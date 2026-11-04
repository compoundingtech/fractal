//! Native harness drivers, delivery and state for Fractal.
//!
//! Catalog declaration parsing and compatibility readers support their current callers.
//! Shared helpers keep their established module paths.

#[cfg(test)]
#[macro_use]
#[path = "../../../scripts/ci-test-paths.rs"]
mod ci_test_paths;

/// A temporary directory whose path is already canonical. macOS puts temporary directories
/// below the `/var` and `/tmp` symlinks, and its file watcher reports the physical path, so a
/// test that compares paths needs the spelling the code under test sees.
#[cfg(test)]
pub(crate) fn canonical_tempdir() -> tempfile::TempDir {
    let parent = std::fs::canonicalize(std::env::temp_dir()).expect("canonical temporary directory");
    tempfile::Builder::new()
        .tempdir_in(parent)
        .expect("temporary directory")
}

pub mod account;
pub mod access;
pub mod catalog;
mod catalog_generation;
pub mod catalog_lock;
pub mod catalog_transaction;
pub mod claude_channel;
pub mod claude_denials;
pub mod claude_mcp;
pub mod claude_prompts;
pub mod claude_process;
pub mod claude_session;
mod claude_usage;
pub mod codex_app_server;
pub mod context;
pub mod contracts;
pub mod delivery_ledger;
pub mod ding;
mod directory;
pub mod driver;
pub mod driver_diagnostic;
pub mod driver_paths;
pub mod eval_spec;
pub mod expand;
/// Private on purpose: every consumer is a sibling module in this crate, and the one site that
/// deliberately keeps its own `flock` is documented in the module itself.
mod flock;
/// Atomic filesystem publication for shared driver state.
#[doc(hidden)]
pub mod fsatomic;
pub mod harness_admission;
pub mod harness_context;
pub mod harness_events;
pub mod harness_inventory;
pub mod blocking_screen;
pub mod harness_state;
pub mod harness_timeline;
pub mod harness_version;
pub mod hooks;
pub mod host;
pub mod host_lock;
pub mod identity;
pub mod materialize;
pub mod message;
pub mod metrics;
pub mod migrations;
/// The stdio framing shared by the native channels; crate-internal.
mod native_channel;
pub mod omp_session;
pub mod omp_questions;
pub mod opencode_session;
pub mod pi_channel;
/// The launch body shared by the pi-family wrappers; crate-internal, reached through
/// `pi_session::run_native` and `omp_session::run_native`.
mod pi_family_session;
pub mod pi_session;
pub mod pretrust;
pub mod provider_session;
pub mod push_mailbox;
pub mod reconcile;
pub mod reexec;

pub mod residency;
pub mod run;

pub mod session_control;
pub mod status;
pub mod subagents;
pub mod supervisor_chain;
pub mod telemetry;
pub mod validate;
pub mod version;
pub mod work_claim_failure;
mod watch;

// The declaration model and catalog walk live in `agent-spec`. Keep their established
// driver-library paths for the shared implementation and its callers.
pub use agent_spec::kdl_version;
pub use agent_spec::{discovery, spec};

pub use agent_spec::discovery::{Discovered, SpecError, discover, discover_file, discover_strict};
pub use agent_spec::spec::{
    AgentDesiredState, AgentSpec, ClaudeDriver, CodexDriver, DeliveryTransport, Driver, JobType,
    OmpDriver, OpenCodeDriver, PiDriver, ResidencyPolicy, Resource, Restart, RestartMode,
    SessionDriver, Task, TaskKind, TaskLifecycle, parse_duration,
};
pub use catalog_lock::CatalogLock;
pub use expand::{expand_env, expand_vars};
pub use host_lock::HostLock;
pub use reconcile::{
    Launch, PtyPresentation, ReconcilePlan, Session, TaskLaunch, TaskTarget, Teardown, reconcile,
};
pub use run::detect_host;
