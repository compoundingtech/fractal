//! st3-delivery: the delivery domain of the store, cut out of st3-store at function level. Everything of st3-store is re-exported at the root;
//! the modules below shadow the lower crate's modules of the same name and re-export them, so every old path still resolves.

// Test helpers widened for st3-daemon's tests are unused when only the feature is on.
#![cfg_attr(all(feature = "test-support", not(test)), allow(dead_code))]

#[cfg(any(test, feature = "test-support"))]
#[macro_use]
#[path = "../../../scripts/ci-test-paths.rs"]
#[allow(dead_code, unused_macros)]
mod ci_test_paths;

pub use st3_store::*;

pub mod maintenance;
pub mod store;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub use store::DeliveryStoreCore;
pub use store::agent_messages::AgentMessagesStore;
pub use store::conversation_reads::ConversationReadsStore;
pub use store::custom::CustomStore;
pub use store::delivery::DeliveryStore;
pub use store::directive_notes::DirectiveNotesStore;
pub use store::glasses::GlassesStore;
pub use store::mailbox_changes::MailboxChangesStore;
pub use store::mailbox_lease::MailboxLeaseStore;
pub use store::mailbox_wakes::MailboxWakesStore;
pub use store::projection_divergence::ProjectionDivergenceStore;
