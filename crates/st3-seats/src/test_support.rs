#![allow(unused_imports)]
pub use st3_missions::test_support::*;
pub use st3_secrets::test_support::*;
use crate::RosterAdmissionStore;
use std::sync::OnceLock;


/// Drive real roster admission with isolated spans, without exposing a daemon control.
pub async fn admit_fixture_roster(store: &crate::store::Store, class: &'static str) -> impl Send {
    store.admit_agent_resources(class).await
}

/// Try the same labeled admission used by production multi-gate readers.
pub fn try_admit_fixture_roster(
    store: &crate::store::Store,
    class: &'static str,
) -> Option<impl Send> {
    store.try_admit_agent_resources(class)
}

/// Force a synchronous holder handoff at the real waiter's enqueue boundary.
pub async fn admit_fixture_roster_after_handoff(
    store: &crate::store::Store,
    class: &'static str,
    before_enqueue: impl FnOnce(),
) -> impl Send {
    store.admit_agent_resources_before_enqueue(class, before_enqueue).await
}
