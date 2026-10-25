//! fractal-attention: the attention domain of the store, cut out of fractal-seats at function level. Everything of fractal-seats is re-exported at the root;
//! the modules below shadow the lower crate's modules of the same name and re-export them, so every old path still resolves.

// Test helpers widened for fractal-daemon's tests are unused when only the feature is on.
#![cfg_attr(all(feature = "test-support", not(test)), allow(dead_code))]

#[cfg(any(test, feature = "test-support"))]
#[macro_use]
#[path = "../../../scripts/ci-test-paths.rs"]
#[allow(dead_code, unused_macros)]
mod ci_test_paths;

pub use fractal_seats::*;

pub mod render;
pub mod store;

pub use store::attention::AttentionStore;
pub use store::attention_list::AttentionListStore;
pub use store::attention_snapshot::AttentionSnapshotStore;
pub use store::client_summary::ClientSummaryStore;
pub use store::owner_lists::OwnerListsStore;
pub use store::published_list::PublishedListStore;
pub use store::published_views::PublishedViewsAttentionStore;
pub use store::summary_list::SummaryListStore;
