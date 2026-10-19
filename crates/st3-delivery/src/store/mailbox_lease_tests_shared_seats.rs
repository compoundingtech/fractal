#![allow(unused_imports)]
pub use st3_store::store::mailbox_lease_tests_shared_seats::*;
use crate::store::*;
use crate::mailbox::{Authority, Fence};
use crate::model::{LaunchSpec, MemberKind, MemberSpec};
use crate::store::mailbox_lease::*;
use crate::store::mailbox_lease_shared_seats::*;


/// Call `promote_current_mailbox_ownership` on another thread, so a test can hold the writer
    /// on this one. A call that needs the writer cannot finish while the test holds it.
    pub fn promote_elsewhere(
        store: &Arc<Store>,
        fence: &Fence,
        candidate: &Authority,
    ) -> (
        std::sync::mpsc::Receiver<Result<(), String>>,
        std::thread::JoinHandle<()>,
    ) {
        let (sender, receiver) = std::sync::mpsc::channel();
        let (store, fence, candidate) = (Arc::clone(store), fence.clone(), candidate.clone());
        let worker = std::thread::spawn(move || {
            let result = store
                .promote_current_mailbox_ownership(&fence, &candidate)
                .map_err(|error| format!("{error:?}"));
            let _ = sender.send(result);
        });
        (receiver, worker)
    }
