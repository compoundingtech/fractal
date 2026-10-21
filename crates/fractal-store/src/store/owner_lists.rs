//! The glasses and arrangements a daemon publishes for collection windows: every person's rows
//! of each, kept off the request path by the attention list's refresher and served under the
//! cut they were read at. A refresh reads again only the people whose glass claims or
//! arrangement rows changed since the newest publication; a projection change or a forgotten
//! view reads everyone again. Everything here is volatile cache state, cleared by
//! `forget_views`.

use super::*;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

/// Every person's rows of one view at one cut. A person with no rows has no entry.
pub struct OwnerListPublication {
    pub cut: u64,
    pub projection: Option<(String, u64)>,
    pub published_at_unix_ms: u128,
    pub owners: Arc<HashMap<String, Arc<Vec<Value>>>>,
}

#[derive(Default)]
pub struct OwnerList {
    pub published: Mutex<Option<Arc<OwnerListPublication>>>,
    /// Counts `forget`s. A refresh that began before one may not publish what it read.
    pub forgotten: AtomicU64,
    /// People read again, in total, and full reads of everyone.
    pub people_read: AtomicU64,
    pub full_reads: AtomicU64,
}

impl OwnerList {
    pub fn forget(&self) {
        let mut published = self.published.lock().unwrap_or_else(PoisonError::into_inner);
        self.forgotten.fetch_add(1, AtomicOrdering::AcqRel);
        *published = None;
    }
}

#[cfg(test)]
#[path = "owner_lists/tests.rs"]
mod tests;

#[cfg(any(test, feature = "test-support"))]
pub mod tests_shared_attention_tests;
