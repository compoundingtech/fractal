//! st3-seats: the seats domain of the store, cut out of st3-store at function level. Everything of st3-store is re-exported at the root;
//! the modules below shadow the lower crate's modules of the same name and re-export them, so every old path still resolves.

// Test helpers widened for st3-daemon's tests are unused when only the feature is on.
#![cfg_attr(all(feature = "test-support", not(test)), allow(dead_code))]

#[cfg(any(test, feature = "test-support"))]
#[macro_use]
#[path = "../../../scripts/ci-test-paths.rs"]
#[allow(dead_code, unused_macros)]
mod ci_test_paths;

pub use st3_attention::*;

pub mod external_sessions;
pub mod references;
pub mod rollout;
pub mod sekrets_daemon;
pub mod store;
pub mod suspension;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub use sekrets_daemon::SekretsDaemonStore;
pub use store::SeatsStoreCore;
pub use store::accounts::AccountsStore;
pub use store::evidence::EvidenceSeatsStore;
pub use store::harness_questions::HarnessQuestionsStore;
pub use store::limits::LimitsStore;
pub use store::retirements::RetirementsStore;
pub use store::revision_seats::RevisionSeatsStore;
pub use store::rollouts::RolloutsStore;
pub use store::roster_admission::RosterAdmissionStore;
pub use store::runtime_starts::RuntimeStartsStore;
pub use store::seat_status::SeatStatusStore;
pub use store::seats::SeatsStore;
pub use store::stop_deferral::StopDeferralStore;
pub use store::subagents::SubagentsStore;
pub use store::trace_principal::TracePrincipalStore;
