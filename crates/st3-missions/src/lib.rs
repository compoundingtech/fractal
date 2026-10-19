//! st3-missions: the missions domain of the store, cut out of st3-store at function level. Everything of st3-store is re-exported at the root;
//! the modules below shadow the lower crate's modules of the same name and re-export them, so every old path still resolves.

// Test helpers widened for st3-daemon's tests are unused when only the feature is on.
#![cfg_attr(all(feature = "test-support", not(test)), allow(dead_code))]

#[cfg(any(test, feature = "test-support"))]
#[macro_use]
#[path = "../../../scripts/ci-test-paths.rs"]
#[allow(dead_code, unused_macros)]
mod ci_test_paths;

pub use st3_store::*;

pub mod graph;
pub mod references;
pub mod store;

pub use store::MissionsStoreCore;
#[cfg(any(test, feature = "test-support"))]
pub use store::operations::OperationsFixtureStore;
pub use store::adhoc_work::AdhocWorkStore;
pub use store::arrangements::ArrangementsStore;
pub use store::coordination::CoordinationStore;
pub use store::declarations::DeclarationsStore;
pub use store::delegation::DelegationStore;
pub use store::lanes::LanesStore;
pub use store::mission_eligibility::MissionEligibilityStore;
pub use store::mission_list::MissionListStore;
pub use store::missions::MissionsStore;
pub use store::person_work::PersonWorkStore;
pub use store::work_list::WorkListStore;
