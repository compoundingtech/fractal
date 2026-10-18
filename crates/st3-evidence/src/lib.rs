//! st3-evidence: the evidence domain of the store, cut out of st3-store at function level. Everything of st3-store is re-exported at the root;
//! the modules below shadow the lower crate's modules of the same name and re-export them, so every old path still resolves.

// Test helpers widened for st3-daemon's tests are unused when only the feature is on.
#![cfg_attr(all(feature = "test-support", not(test)), allow(dead_code))]

#[cfg(any(test, feature = "test-support"))]
#[macro_use]
#[path = "../../../scripts/ci-test-paths.rs"]
#[allow(dead_code, unused_macros)]
mod ci_test_paths;

pub use st3_seats::*;

pub mod store;

pub use store::evidence::EvidenceStore;
pub use store::owned_sets::OwnedSetsStore;
pub use store::publication::PublicationStore;
pub use store::publication_facts::PublicationFactsStore;
pub use store::published_views::PublishedViewsStore;
