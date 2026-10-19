//! The part of `store::runtime` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use crate::EvidenceStore;
use super::*;
pub use st3_attention::store::runtime::*;
pub use st3_attention::store::runtime::{AgentResourcesEntry, AgentRosterFaultRetries, Extensions, SmalltalkRuntime, compatibility_digest};

#[cfg(test)]
mod ivm_attachment_tests {
    use super::*;
    use smallclaims::ivm::{Contribution, Definition, Readiness, View, Views, events};
    use smallclaims::store::canonical;
    use std::sync::Barrier;
    use st3_attention::store::runtime_ivm_attachment_tests_shared::*;
    use st3_attention::store::runtime_ivm_attachment_tests_shared::{Shadow, boundary, registry};

    #[test]
    fn ordinary_store_does_not_install_or_attach_shadow_views() {
        let store = Store::open_memory("alder").unwrap();
        assert!(store.ivm_views().is_none());
        assert!(store.ivm_publisher().unwrap().is_none());
        let installed: bool = store
            .readers
            .get()
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='ivm_event_state')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!installed);
    }

    #[test]
    fn concurrent_consumers_share_one_store_publisher_and_registry() {
        let root = tempfile::tempdir().unwrap();
        let views = registry();
        let store = Arc::new(
            Store::open_with_ivm_views(&root.path().join("claims.db"), "alder", views.clone())
                .unwrap(),
        );
        assert!(Arc::ptr_eq(&store.ivm_views().unwrap(), &views));
        assert!(matches!(
            boundary(&store, &views).availability.readiness,
            Readiness::Ready(_)
        ));
        let start = Arc::new(Barrier::new(8));
        let threads = (0..8)
            .map(|_| {
                let (store, start) = (store.clone(), start.clone());
                std::thread::spawn(move || {
                    start.wait();
                    store.ivm_publisher().unwrap().unwrap()
                })
            })
            .collect::<Vec<_>>();
        let publishers = threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect::<Vec<_>>();
        assert!(
            publishers
                .iter()
                .all(|publisher| Arc::ptr_eq(publisher, &publishers[0]))
        );
        assert!(Arc::ptr_eq(
            &store.ivm_publisher().unwrap().unwrap(),
            &publishers[0]
        ));
        let mut socket = publishers[0].subscribe();
        let mut receipt = publishers[1].subscribe();
        store
            .connection
            .batched(events::rotate_identity)
            .unwrap()
            .unwrap();
        let socket_notice = socket.try_recv().unwrap();
        let receipt_notice = receipt.try_recv().unwrap();
        assert_eq!(socket_notice, receipt_notice);
        assert!(matches!(socket_notice, events::Notice::Committed(_)));
    }
}
