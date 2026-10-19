#![allow(unused_imports)]
pub use st3_store::store::seat_status_tests_shared_seats::*;
pub use st3_store::store::seat_status_tests_shared_seats::observe;
pub use st3_store::store::seat_status_tests_shared_seats::runtime;
use crate::SeatsAttentionStore;
use crate::store::*;
use crate::store::seat_status::*;


/// `current_harness`, after checking that the login-only fold, which the attention read
    /// uses, reaches the same answer to "does this seat need a login?" for the same claims.
    pub fn checked_harness(store: &Store, subject: &str) -> Result<Option<crate::model::CurrentHarnessView>> {
        let full = store.current_harness(subject)?;
        let fast = store.current_harness_for_login(subject)?;
        assert_eq!(
            fast.as_ref().is_some_and(|harness| harness.state == "needs-login"),
            full.as_ref().is_some_and(|harness| harness.state == "needs-login"),
            "the login-only fold disagrees for {subject}: {fast:?} against {full:?}"
        );
        if let (Some(fast), Some(full)) = (&fast, &full)
            && fast.state == "needs-login"
        {
            assert_eq!(fast.incarnation_id, full.incarnation_id);
            assert_eq!(fast.observed_at_unix_ms, full.observed_at_unix_ms);
        }
        Ok(full)
    }
