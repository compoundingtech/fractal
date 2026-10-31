//! This build's identity, compiled in here and nowhere else (see build.rs). Only a final binary depends on this crate: it
//! hands the stamps to `fractal_drivers::version::set_build_stamps` (and Fractal TUI's copy) first thing in `main`. A library that baked
//! the revision itself would be rebuilt, with everything above it, by every commit, even one that changes no Rust.

/// A release stamp from the archive builder, or the NixStamp the flake bakes from `self`; authoritative when present.
pub const CLI_BUILD_STAMP: Option<&str> = option_env!("CLI_BUILD_STAMP");

/// The LocalStamp build.rs derives from Git for a plain `cargo build`.
pub const ST_BUILD_STAMP_LOCAL: Option<&str> = option_env!("ST_BUILD_STAMP_LOCAL");

/// Source revision of the complete public parser and lowering contract (st2's `agentSpecRevision`).
///
/// Hermetic builds inject the full source revision. Native builds use the full clean Git revision
/// or an explicit `local-dirty.*`/`local.unknown` identity, so unlike builds cannot compare equal.
pub const AGENT_SPEC_REVISION: &str = env!("AGENT_SPEC_REVISION");
