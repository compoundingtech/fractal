use super::*;
use crate::store::{checkpoint_agreement::SealTerms, runtime::Plain};
use std::sync::Arc;

const NOW: u128 = 10 * DAY_MS;

fn store() -> Store {
    Store::open_memory("status-node", Arc::new(Plain)).unwrap()
}

#[test]
fn cold_status_does_not_walk_or_invent_terms() {
    let store = store();
    SEALED_IDENTITY_WALKS.with(|walks| walks.set(0));
    let pending = store.checkpoint_status(NOW, &[]).unwrap().pending.unwrap();
    assert_eq!(pending.terms, None);
    assert_eq!(pending.local_digest_state, "unavailable");
    assert!(pending.sealed.is_empty());
    assert!(pending.disagreeing.is_empty());
    SEALED_IDENTITY_WALKS.with(|walks| assert_eq!(walks.get(), 0));
}

#[test]
fn warm_status_reuses_the_maintenance_digest_without_walking() {
    let store = store();
    store.register_checkpoint_capture(newest_due_cut(NOW), None).unwrap();
    let sealed = store.checkpoint_sealed_identities(newest_due_cut(NOW), None).unwrap();
    SEALED_IDENTITY_WALKS.with(|walks| walks.set(0));
    let pending = store.checkpoint_status(NOW, &[]).unwrap().pending.unwrap();
    assert_eq!(pending.local_digest_state, "ready");
    let terms = pending.terms.unwrap();
    assert_eq!(terms.sealed_digest, sealed.digest);
    assert_eq!(terms.rules_digest, "plain");
    assert_eq!(terms.cut_unix_ms, newest_due_cut(NOW));
    SEALED_IDENTITY_WALKS.with(|walks| assert_eq!(walks.get(), 0));
}

#[test]
fn cold_status_preserves_our_published_terms_without_claiming_a_ready_digest() {
    let store = store();
    let cut = newest_due_cut(NOW);
    let sealed = store.checkpoint_sealed_identities(cut, None).unwrap();
    let terms = SealTerms {
        cut_unix_ms: cut,
        participants: BTreeSet::from(["status-node".to_owned()]),
        sealed_digest: sealed.digest.clone(),
        rules_digest: "published-rules".into(),
    };
    store.publish_seal(&checkpoint_name(cut), &terms, &sealed, None).unwrap();
    *store.sealed_identity_memo.lock().unwrap() = None;
    SEALED_IDENTITY_WALKS.with(|walks| walks.set(0));
    let pending = store.checkpoint_status(NOW, &[]).unwrap().pending.unwrap();
    assert_eq!(pending.terms, Some(terms));
    assert_eq!(pending.local_digest_state, "unavailable");
    assert!(pending.sealed.contains("status-node"));
    SEALED_IDENTITY_WALKS.with(|walks| assert_eq!(walks.get(), 0));
}
