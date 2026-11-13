//! Fractal kernel: the leaf modules of the Fractal library (model, wire types, CLI output, configuration-free
//! helpers) that name no store, api or reconcile code. fractal-daemon re-exports each module at its old path.

pub mod cli_output;
pub mod accounts;
/// The values every declaration kind's KDL shares: canonical nodes and durations.
pub mod declaration_kdl;
/// An `objective` declaration: an indicator, its targets and the response to a miss.
pub mod objective;
pub mod archive;
pub mod boot;
pub mod config;
pub mod conversation_search;
pub mod disk;
/// Offline signed gate observations in immutable git notes refs.
pub mod evidence;
pub mod native_views;
pub mod conversation_header;
/// Pairing grants a device the client gateway over Fabric; revoking it removes the grant.
pub mod fabric_gateway;
pub mod fleet;
/// Built-in gate kinds: what gates shelled out for most, answered by st itself.
pub mod gate_kinds;
/// What an `st` command run inside an exec gate reports about itself.
pub mod gate_report;
pub mod harness_events;
/// The lifecycle hook set st3 publishes beneath its own state directory.
pub mod hooks;
pub mod instance;
pub mod lane;
pub mod memory;
pub mod read_cache;
pub mod model;
pub mod pricing;
/// What a program step's journal shows: each inner step's latest status, timing and failure.
pub mod program_journal;
pub mod projection;
pub mod rules;
pub mod seat_queue;
pub mod step_execution;
pub use step_execution::{StepExecution, StepExecutionUsage};
/// The st agent skill bundled in this binary and installed for each harness.
pub mod skill;
pub mod publication;
pub mod terminal_binding;
/// What a mission run waits on, and the fleet's blocked runs by cause.
pub mod waiting_on;
pub mod github_admission;
pub mod github_webhook;
