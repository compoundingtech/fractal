//! Published glasses and arrangements against their full reads: every person's published rows
//! equal `glasses_at`/`arrangements_at` (and a selected arrangement equals `arrangement_at`) at
//! the publication's cut, through edits, deletions, retirement, replication in either order,
//! reopen and forgotten views; a refresh reads again only the people whose rows may have changed.
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::owner_lists::tests_shared_attention_tests::{ADA_LAYOUT, PEOPLE, ROBIN_LAYOUT, arrangement, glass, sync};




