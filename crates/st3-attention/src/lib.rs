//! st3-attention: the attention domain of the store, cut out of st3-store at function level. Everything of st3-store is re-exported at the root;
//! the modules below shadow the lower crate's modules of the same name and re-export them, so every old path still resolves.

// Test helpers widened for st3-daemon's tests are unused when only the feature is on.
#![cfg_attr(all(feature = "test-support", not(test)), allow(dead_code))]

#[cfg(any(test, feature = "test-support"))]
#[macro_use]
#[path = "../../../scripts/ci-test-paths.rs"]
#[allow(dead_code, unused_macros)]
mod ci_test_paths;

pub use st3_store::*;

pub mod backup;
pub mod external_sessions;
pub mod render;
pub mod store;

pub use store::AttentionStoreCore;
pub use store::accounts::AccountsAttentionStore;
pub use store::attention::AttentionStore;
pub use store::attention_list::AttentionListStore;
pub use store::attention_snapshot::AttentionSnapshotStore;
pub use store::card_fold::CardFoldStore;
pub use store::checkpoint_rules::CheckpointRulesStore;
pub use store::client_summary::ClientSummaryStore;
pub use store::custom::CustomStore;
pub use store::operations::OperationsStore;
pub use store::owned_sets::OwnedSetsAttentionStore;
pub use store::owner_lists::OwnerListsStore;
pub use store::published_list::PublishedListStore;
pub use store::published_views::PublishedViewsAttentionStore;
pub use store::seat_status::SeatStatusAttentionStore;
pub use store::seats::SeatsAttentionStore;
pub use store::stop_deferral::StopDeferralAttentionStore;
pub use store::summary_list::SummaryListStore;
