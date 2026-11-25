//! Store-local, cancellation-safe revision subscriptions; no graph wake or inventory RPC.
use std::sync::{Arc, OnceLock};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tokio::sync::watch;

use crate::{sqlite::CommitObserver, store::Store};

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RevisionKind {
    Authority,
    Membership,
}

/// An epoch prevents a restarted daemon (or another store) from matching an old cursor.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct StoreRevision {
    pub epoch: uuid::Uuid,
    pub generation: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authority_root: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RevisionRequest {
    pub kind: RevisionKind,
    pub after: Option<StoreRevision>,
}

pub(crate) struct StoreRevisions {
    authority: watch::Sender<StoreRevision>,
    membership: watch::Sender<StoreRevision>,
    observers: OnceLock<Vec<CommitObserver>>,
}

impl StoreRevisions {
    fn new() -> Self {
        let channel = || watch::channel(StoreRevision {
            epoch: uuid::Uuid::now_v7(), generation: 0, authority_root: None,
        }).0;
        Self { authority: channel(), membership: channel(), observers: OnceLock::new() }
    }

    fn bump(sender: &watch::Sender<StoreRevision>, label: &str) {
        sender.send_modify(|revision| revision.generation += 1);
        crate::performance::record_request(label, None, std::time::Duration::ZERO);
    }
}

impl Store {
    pub fn subscribe_replication_revision(&self, kind: RevisionKind) -> watch::Receiver<StoreRevision> {
        let revisions = self.sync_revisions.get_or_init(|| Arc::new(StoreRevisions::new()));
        revisions.observers.get_or_init(|| {
            // Register before taking the initial cursor. Commits before registration are
            // included in the initial synchronization; commits after it advance the cursor.
            let initial_generation = self.readers.get().query_row(
                "SELECT COALESCE((SELECT value FROM meta WHERE key='replication_inventory_generation'),'0')",
                [], |row| row.get::<_, String>(0),
            ).ok();
            let inventory_generation = std::sync::Mutex::new(initial_generation);
            let membership = Arc::downgrade(revisions);
            let metadata = Arc::downgrade(revisions);
            let authority = Arc::downgrade(revisions);
            vec![
                self.observe_tables(&[
                    // A local append is a batch/claim until the next snapshot seals it.
                    "batches", "claims",
                    "replica_envelopes", "replica_envelope_signatures", "checkpoint_envelopes",
                    "released_envelopes", "retired_envelopes", "checkpoints",
                ], move |_| {
                    if let Some(revisions) = authority.upgrade() {
                        StoreRevisions::bump(&revisions.authority, "replication/authority-revision");
                    }
                }),
                self.observe_tables(&["fleet_generation"], move |_| {
                    if let Some(revisions) = membership.upgrade() {
                        StoreRevisions::bump(&revisions.membership, "replication/membership-revision");
                    }
                }),
                self.observe_tables(&["meta"], move |connection| {
                    // The metadata table also carries local progress. Only inventory
                    // invalidation changes replicated authority.
                    let generation: rusqlite::Result<String> = connection.query_row(
                        "SELECT COALESCE((SELECT value FROM meta WHERE key='replication_inventory_generation'),'0')",
                        [], |row| row.get(0),
                    );
                    let mut previous = inventory_generation.lock().expect("revision lock poisoned");
                    if let Ok(generation) = generation {
                        let changed = previous.as_ref() != Some(&generation);
                        *previous = Some(generation);
                        if changed && let Some(revisions) = metadata.upgrade() {
                            StoreRevisions::bump(&revisions.authority, "replication/authority-revision");
                        }
                    } else if let Some(revisions) = metadata.upgrade() {
                        // Coverage loss must invalidate rather than silently miss a change.
                        StoreRevisions::bump(&revisions.authority, "replication/authority-revision");
                    }
                }),
            ]
        });
        match kind {
            RevisionKind::Authority => revisions.authority.subscribe(),
            RevisionKind::Membership => revisions.membership.subscribe(),
        }
    }

    pub async fn wait_replication_revision(&self, request: RevisionRequest) -> Result<StoreRevision> {
        let mut receiver = self.subscribe_replication_revision(request.kind);
        crate::performance::record_request(match request.kind {
            RevisionKind::Authority => "replication/authority-wait",
            RevisionKind::Membership => "replication/membership-wait",
        }, None, std::time::Duration::ZERO);
        loop {
            let current = receiver.borrow_and_update().clone();
            if !request.after.as_ref().is_some_and(|after| {
                after.epoch == current.epoch && after.generation == current.generation
            }) {
                return Ok(current);
            }
            receiver.changed().await?;
        }
    }

    /// Read a coherent revision/root cut without holding a reader across an await.
    /// Call from a blocking store task: an initial snapshot may need to fold the inventory.
    pub fn replication_revision_snapshot(&self, kind: RevisionKind) -> Result<StoreRevision> {
        let receiver = self.subscribe_replication_revision(kind);
        loop {
            let mut revision = receiver.borrow().clone();
            if matches!(kind, RevisionKind::Authority) {
                let started = std::time::Instant::now();
                let snapshot = self.replication_snapshot();
                crate::performance::record_request(
                    "replication/authority-root-read", None, started.elapsed(),
                );
                revision.authority_root = Some(snapshot?.authority_digest.clone());
            }
            let current = receiver.borrow();
            if revision.epoch == current.epoch && revision.generation == current.generation {
                return Ok(revision);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ store::runtime::Plain};
    use fractal_wire::{ClaimInput};
    use std::collections::BTreeMap;

    fn store() -> Store {
        Store::open_memory("revision-fixture", Arc::new(Plain)).unwrap()
    }

    fn append(store: &Store) {
        store.append_claim(&ClaimInput {
            subject: format!("note/revision/{}", store.index().unwrap()), kind: "example.note".into(), actor: None,
            fields: BTreeMap::new(), evidence: Vec::new(), expected_subject: None,
            idempotency_key: None,
        }).unwrap();
    }

    #[tokio::test]
    async fn local_mirrors_and_other_stores_do_not_advance_authority_or_membership() {
        let graph = store();
        let shard = store();
        let authority = graph.subscribe_replication_revision(RevisionKind::Authority);
        let membership = graph.subscribe_replication_revision(RevisionKind::Membership);
        let graph_cut = authority.borrow().clone();
        let membership_cut = membership.borrow().clone();
        let shard_cut = shard.subscribe_replication_revision(RevisionKind::Authority).borrow().clone();
        for _ in 0..100 {
            graph.connection.write().execute(
                "INSERT INTO meta(key,value) VALUES('revision_local_progress','1')
                 ON CONFLICT(key) DO UPDATE SET value=CAST(value AS INTEGER)+1", [],
            ).unwrap();
        }
        append(&shard);
        assert_eq!(*authority.borrow(), graph_cut);
        assert_eq!(*membership.borrow(), membership_cut);
        assert!(!authority.has_changed().unwrap());
        assert!(!membership.has_changed().unwrap());
        let shard_now = shard.wait_replication_revision(RevisionRequest {
            kind: RevisionKind::Authority, after: Some(shard_cut),
        }).await.unwrap();
        assert_ne!(shard_now.epoch, graph_cut.epoch);
        assert!(shard_now.generation > 0);
    }

    #[tokio::test]
    async fn commit_before_wait_and_commit_during_wait_are_not_lost() {
        let store = store();
        let before = store.wait_replication_revision(RevisionRequest {
            kind: RevisionKind::Authority, after: None,
        }).await.unwrap();
        append(&store);
        let after = store.wait_replication_revision(RevisionRequest {
            kind: RevisionKind::Authority, after: Some(before.clone()),
        }).await.unwrap();
        assert!(after.generation > before.generation);
        let wait = store.wait_replication_revision(RevisionRequest {
            kind: RevisionKind::Authority, after: Some(after.clone()),
        });
        tokio::pin!(wait);
        assert!(futures_util::poll!(&mut wait).is_pending());
        append(&store);
        let next = wait.await.unwrap();
        assert!(next.generation > after.generation);
        // Reusing a cursor after cancellation must still return the unconsumed change.
        assert_eq!(store.wait_replication_revision(RevisionRequest {
            kind: RevisionKind::Authority, after: Some(after),
        }).await.unwrap(), next);
    }

    #[tokio::test]
    async fn independent_consumers_and_restart_epochs_recheck_without_a_timer() {
        let store = store();
        let authority = store.wait_replication_revision(RevisionRequest {
            kind: RevisionKind::Authority, after: None,
        }).await.unwrap();
        let membership = store.wait_replication_revision(RevisionRequest {
            kind: RevisionKind::Membership, after: None,
        }).await.unwrap();
        let first = store.wait_replication_revision(RevisionRequest {
            kind: RevisionKind::Membership, after: Some(membership.clone()),
        });
        let second = store.wait_replication_revision(RevisionRequest {
            kind: RevisionKind::Membership, after: Some(membership.clone()),
        });
        tokio::pin!(first, second);
        assert!(futures_util::poll!(&mut first).is_pending());
        assert!(futures_util::poll!(&mut second).is_pending());
        store.connection.write().execute(
            "UPDATE fleet_generation SET value=value+1 WHERE id=1", [],
        ).unwrap();
        let next = first.await.unwrap();
        assert_eq!(next.generation, membership.generation + 1);
        assert_eq!(second.await.unwrap(), next);
        assert_eq!(*store.subscribe_replication_revision(RevisionKind::Authority).borrow(), authority);
        let reopened = self::store();
        let fresh = reopened.wait_replication_revision(RevisionRequest {
            kind: RevisionKind::Authority, after: Some(authority),
        }).await.unwrap();
        assert_eq!(fresh.generation, 0);
    }

    #[tokio::test]
    async fn rolled_back_authority_writes_do_not_publish_a_revision() {
        let store = store();
        let authority = store.subscribe_replication_revision(RevisionKind::Authority);
        let before = authority.borrow().clone();
        {
            let connection = store.connection.write();
            connection.execute_batch(
                "BEGIN; INSERT INTO meta(key,value) VALUES('replication_inventory_generation','99')
                 ON CONFLICT(key) DO UPDATE SET value='99'; ROLLBACK;",
            ).unwrap();
        }
        assert_eq!(*authority.borrow(), before);
        assert!(!authority.has_changed().unwrap());
    }

    #[tokio::test]
    async fn coherent_root_cursor_can_be_cancelled_and_resumed_without_busy_wakes() {
        let store = store();
        append(&store);
        let snapshot = store.replication_revision_snapshot(RevisionKind::Authority).unwrap();
        assert_eq!(snapshot.authority_root.as_deref(),
            Some(store.replication_snapshot().unwrap().authority_digest.as_str()));
        {
            let cancelled = store.wait_replication_revision(RevisionRequest {
                kind: RevisionKind::Authority, after: Some(snapshot.clone()),
            });
            tokio::pin!(cancelled);
            // The root carried back from the daemon does not make the same cursor dirty.
            assert!(futures_util::poll!(&mut cancelled).is_pending());
        }
        append(&store);
        let resumed = store.wait_replication_revision(RevisionRequest {
            kind: RevisionKind::Authority, after: Some(snapshot.clone()),
        }).await.unwrap();
        assert_eq!(resumed.epoch, snapshot.epoch);
        assert!(resumed.generation > snapshot.generation);
    }

    #[tokio::test]
    async fn unsealed_local_append_completes_pending_authority_wait_before_any_export() {
        let store = store();
        let before = store.replication_revision_snapshot(RevisionKind::Authority).unwrap();
        let wait = store.wait_replication_revision(RevisionRequest {
            kind: RevisionKind::Authority, after: Some(before.clone()),
        });
        tokio::pin!(wait);
        assert!(futures_util::poll!(&mut wait).is_pending());
        append(&store);
        let next = match futures_util::poll!(&mut wait) {
            std::task::Poll::Ready(Ok(next)) => next,
            other => panic!("unsealed source append did not wake authority: {other:?}"),
        };
        assert!(next.generation > before.generation);
        let after = store.replication_revision_snapshot(RevisionKind::Authority).unwrap();
        assert_ne!(after.authority_root, before.authority_root);
    }
}
