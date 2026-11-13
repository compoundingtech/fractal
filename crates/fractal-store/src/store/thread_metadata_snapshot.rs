//! Causal metadata projection over the existing durable claim graph.
use super::*;

struct MetadataClaim {
    claim: ClaimRecord,
    eligible: bool,
    conflict: bool,
}

pub(super) struct MetadataSnapshot {
    // Canonical order is used only to break ties between causally concurrent edits.
    claims: Vec<MetadataClaim>,
}

impl MetadataSnapshot {
    pub(super) fn read(connection: &Connection, subject: &str) -> Result<Self, FractalError> {
        let sql = canonical_sql(&format!(
            "SELECT {CLAIM_COLUMNS}, operations.state,
                    (operations.state='conflict' OR operations.canonical_claim_id=claims.id)
                    AND {UNREPAIRED_CLAIM}
             FROM claims INDEXED BY claims_subject_kind_accepted_index
             LEFT JOIN operations ON operations.id=json_extract(claims.body,'$._operation.id')
             WHERE claims.subject=?1 AND claims.kind IN ('thread.created','thread.title-set','thread.state-set')
             ORDER BY CANONICAL_DESC(claims)"
        ));
        let mut statement = connection.prepare_cached(
            /* sqlite-large-value: allow-for-now lv-h2488f1c93c2b owner=fractal_store::store::thread_metadata_snapshot TODO=trace-statement-and-value-representation */ &sql
        ).map_err(internal)?;
        let claims = statement.query_map([subject], |row| {
            let state: Option<String> = row.get(10)?;
            Ok(MetadataClaim {
                claim: claim_from_row(row)?,
                eligible: row.get::<_, Option<bool>>(11)?.unwrap_or(false),
                conflict: state.as_deref() == Some("conflict"),
            })
        }).map_err(internal)?.collect::<rusqlite::Result<Vec<_>>>().map_err(internal)?;
        let graph = claims.iter().map(|entry| (entry.claim.id.as_str(), &entry.claim)).collect::<BTreeMap<_, _>>();
        let mut pending = claims.iter().filter(|entry| entry.eligible)
            .flat_map(|entry| entry.claim.predecessors.iter().map(String::as_str)).collect::<Vec<_>>();
        let mut visited = BTreeSet::new();
        while let Some(parent) = pending.pop() {
            if !visited.insert(parent) { continue; }
            if let Some(claim) = graph.get(parent) {
                // Ineligible intermediates still carry immutable causal links. Every edge
                // walked by heads() must be complete before exposing a versioned snapshot.
                pending.extend(claim.predecessors.iter().map(String::as_str));
            } else if !connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM claims WHERE id=?1)", [parent],
                |row| row.get::<_, bool>(0),
            ).map_err(internal)? {
                return Err(unavailable());
            }
        }
        Ok(Self { claims })
    }

    fn heads(&self, kind: Option<&str>) -> Vec<&MetadataClaim> {
        let candidates = self.claims.iter().filter(|entry| entry.eligible
            && kind.is_none_or(|kind| entry.claim.kind == kind)).collect::<Vec<_>>();
        // Traverse through other metadata kinds and repaired/noncanonical intermediates:
        // removing their effect must not erase immutable causal links between valid edits.
        let graph = self.claims.iter().map(|entry| (entry.claim.id.as_str(), &entry.claim))
            .collect::<BTreeMap<_, _>>();
        let mut pending = candidates.iter().flat_map(|entry| entry.claim.predecessors.iter().map(String::as_str))
            .collect::<Vec<_>>();
        let mut ancestors = BTreeSet::new();
        while let Some(id) = pending.pop() {
            if !ancestors.insert(id) { continue; }
            if let Some(claim) = graph.get(id) {
                pending.extend(claim.predecessors.iter().map(String::as_str));
            }
        }
        candidates.into_iter().filter(|entry| !ancestors.contains(entry.claim.id.as_str())).collect()
    }

    pub(super) fn selected(&self, kind: &str) -> Result<Option<ClaimRecord>, FractalError> {
        let Some(selected) = self.heads(Some(kind)).into_iter().next() else { return Ok(None) };
        if selected.conflict { return Err(conflict()); }
        Ok(Some(selected.claim.clone()))
    }

    pub(super) fn predecessors(&self) -> Result<Vec<String>, FractalError> {
        let heads = self.heads(None);
        if heads.iter().any(|entry| entry.conflict) { return Err(conflict()); }
        let mut ids = heads.into_iter().map(|entry| entry.claim.id.clone()).collect::<Vec<_>>();
        ids.sort();
        Ok(ids)
    }

    pub(super) fn commit_snapshot(&self) -> Result<ClaimSubjectSnapshot, FractalError> {
        let predecessors = self.predecessors()?;
        let revision = if predecessors.is_empty() { None } else {
            let creation = self.selected("thread.created")?.map(|claim| claim.id);
            let title = self.selected("thread.title-set")?.map(|claim| claim.id);
            let state = self.selected("thread.state-set")?.map(|claim| claim.id);
            // Bind the effective selections as well as the causal frontier. A repair can
            // exclude an ancestor title while leaving the frontier unchanged.
            Some(format!("thread-revision/{}", canonical_hash(&(
                "fractal.thread-metadata-snapshot.v1", &predecessors, creation, title, state,
            )).map_err(internal)?))
        };
        Ok(ClaimSubjectSnapshot { revision, predecessors })
    }

    pub(super) fn revision(&self) -> Result<Option<String>, FractalError> {
        Ok(self.commit_snapshot()?.revision)
    }
}

fn conflict() -> FractalError {
    FractalError::new("idempotency-conflict", "the thread metadata operation has conflicting requests")
}

#[cfg(test)]
mod tests {
    use super::*;
    const PERSON: &str = "person/owner";
    const THREAD: &str = "thread/snapshot";

    fn fixture() -> Store {
        let store = Store::open_memory("metadata-snapshot").unwrap();
        apply(&store, PERSON, THREAD, "create", Edit::Create {
            agent: "agent/reviewer", title: "Original",
        }).unwrap();
        store
    }

    fn rename(store: &Store) -> ClaimRecord {
        let current = summary(store, PERSON, THREAD).unwrap();
        apply(store, PERSON, THREAD, "rename", Edit::Rename {
            title: "Updated", title_revision: &current.title_revision, revision: &current.revision,
        }).unwrap().0
    }

    fn state(store: &Store, key: &str, closed: bool) -> ClaimRecord {
        let current = summary(store, PERSON, THREAD).unwrap();
        let edit = if closed { Edit::Close { revision: &current.revision } }
            else { Edit::Reopen { revision: &current.revision } };
        apply(store, PERSON, THREAD, key, edit).unwrap().0
    }

    #[test]
    fn excluding_an_ancestor_title_changes_the_snapshot_and_fences_old_writers() {
        let store = fixture();
        let title = rename(&store);
        let closed = state(&store, "close", true);
        let before = summary(&store, PERSON, THREAD).unwrap();
        let frontier = MetadataSnapshot::read(&store.readers.get(), THREAD).unwrap().predecessors().unwrap();
        assert_eq!(frontier, [closed.id]);
        // This is the same retained-source exclusion maintained by record repairs.
        store.connection.batched(|tx| -> anyhow::Result<()> {
            tx.execute("INSERT INTO projection_digest_repaired_claims(id) VALUES(?1)", [&title.id])?;
            Ok(())
        }).unwrap().unwrap();
        let after = summary(&store, PERSON, THREAD).unwrap();
        assert_eq!(after.title, "Original");
        assert_eq!(after.state, "closed");
        assert_ne!(after.revision, before.revision);
        assert_eq!(MetadataSnapshot::read(&store.readers.get(), THREAD).unwrap().predecessors().unwrap(), frontier);
        assert_eq!(apply(&store, PERSON, THREAD, "stale-reopen", Edit::Reopen {
            revision: &before.revision,
        }).unwrap_err().code, "stale-subject");
        assert!(summary(&store, "person/other", THREAD).is_err());
        state(&store, "fresh-reopen", false);
        assert_eq!(summary(&store, PERSON, THREAD).unwrap().state, "open");
    }

    #[test]
    fn a_missing_parent_through_an_excluded_intermediate_never_exposes_a_partial_snapshot() {
        let store = fixture();
        let title = rename(&store);
        let intermediate = state(&store, "close", true);
        let current = state(&store, "reopen", false);
        assert_eq!(current.predecessors, [intermediate.id.clone()]);
        assert_eq!(intermediate.predecessors, [title.id.clone()]);
        store.connection.batched(|tx| -> anyhow::Result<()> {
            tx.execute("INSERT INTO projection_digest_repaired_claims(id) VALUES(?1)", [&intermediate.id])?;
            Ok(())
        }).unwrap().unwrap();
        let complete = summary(&store, PERSON, THREAD).unwrap();
        // Model retained/out-of-order graph state below normal admission: C -> X -> T,
        // with X excluded and T not present yet. No live writer can manufacture this path.
        store.connection.batched(|tx| -> anyhow::Result<()> {
            // Withhold the claim's derived operation row too, preserving the real FK
            // invariant while modeling this out-of-order retained graph state.
            tx.execute("DELETE FROM operations WHERE canonical_claim_id=?1", [&title.id])?;
            tx.execute("DELETE FROM claims WHERE id=?1", [&title.id])?;
            assert!(MetadataSnapshot::read(tx, THREAD).is_err());
            Ok(())
        }).unwrap().unwrap();
        assert!(summary(&store, PERSON, THREAD).is_err());
        assert!(summary(&store, "person/other", THREAD).is_err());
        store.connection.batched(|tx| -> anyhow::Result<()> {
            tx.execute("INSERT INTO claims(id,store_index,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms)
                        VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                params![title.id, title.store_index, title.batch_id, title.subject, title.kind,
                    title.origin, title.actor, serde_json::to_string(&title.body)?,
                    serde_json::to_string(&title.predecessors)?, title.accepted_at_unix_ms.to_string()])?;
            register_operation_tx(tx, &title)?;
            Ok(())
        }).unwrap().unwrap();
        let restored = summary(&store, PERSON, THREAD).unwrap();
        assert_eq!(restored.title, "Updated");
        assert_eq!(restored.state, "open");
        assert_eq!(restored.revision, complete.revision);
    }
}
