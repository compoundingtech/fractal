//! st3 store: the claims store, its domains (missions, mailbox, github, seats' rollout and suspension,
//! sekrets) and the modules they need. st3-daemon re-exports each module at its old path.

// Test helpers widened for st3-daemon's tests are unused when only the feature is on.
#![cfg_attr(all(feature = "test-support", not(test)), allow(dead_code))]

#[cfg(any(test, feature = "test-support"))]
#[macro_use]
#[path = "../../../scripts/ci-test-paths.rs"]
mod ci_test_paths;

pub use fractal_kernel::*;
pub use fractal_telemetry::*;
pub use fractal_secrets::*;
pub use fractal_seats_base::*;
pub use fractal_delivery_base::*;
pub use fractal_missions_base::*;
pub use fractal_github_base::*;
pub use smallclaims::{performance, profile, task_cpu};

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;


pub mod agent_launch;
pub mod backup;
pub mod external_sessions;
pub mod recorder_receipts;
pub mod render;
pub mod client_values;
pub mod graph;
pub mod maintenance;
pub mod owner_indexes;
pub mod references;
pub mod restart;
pub mod rollout;
pub mod sekrets_daemon;
pub mod store;
pub mod suspension;

pub use graph::{parse_intent, validate_mission_runtimes};
pub use model::{NormalizedIntent, St3Error};
