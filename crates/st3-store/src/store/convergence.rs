//! Randomized convergence, to git's bar: clones that fetched from each other hold the same
//! commits, and nodes that synced with each other hold the same envelopes and project the same
//! graph. Isolated nodes write, exchange in any order, restart, crash between receipt and
//! admission or between admission and projection, lose their links for a while, carry history
//! an older build wrote under older retention rules, and suffer an older build's faults: claims
//! it could not admit, claims it lost, and graphs it projected by another rule. Each run ends
//! with every link up and every node restarted on this build. After exchanges, every node must
//! hold the same envelopes. A run without faults must also project the same graph on every node
//! without ever replaying its graph from nothing. Sync never repairs a faulted node's graph from a
//! peer: projections are each node's own functions of its claims.
//!
//! `ST3_CONVERGENCE_RUNS` (default 8) and `ST3_CONVERGENCE_STEPS` (default 80) size the test;
//! `ST3_CONVERGENCE_SEED` repeats one run. A failing run names its seed.

#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::convergence_shared_attention_tests::{Delivery, FLEET, Node, PERSON, Rng, Simulation, env_number, mission_source};



