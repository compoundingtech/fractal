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
        let mut statement = connection.prepare_cached(&sql).map_err(internal)?;
        let claims = statement.query_map([subject], |row| {
            let state: Option<String> = row.get(10)?;
            Ok(MetadataClaim {
                claim: claim_from_row(row)?,
                eligible: row.get::<_, Option<bool>>(11)?.unwrap_or(false),
                conflict: state.as_deref() == Some("conflict"),
            })
        }).map_err(internal)?.collect::<rusqlite::Result<Vec<_>>>().map_err(internal)?;
        let known = claims.iter().map(|entry| entry.claim.id.as_str()).collect::<BTreeSet<_>>();
        for parent in claims.iter().filter(|entry| entry.eligible)
            .flat_map(|entry| entry.claim.predecessors.iter())
        {
            if !known.contains(parent.as_str())
                && claim_by_id_tx(connection, parent).map_err(internal)?.is_none()
            {
                // Do not expose a partial causal snapshot whose fields could change when
                // a predecessor arrives without changing the frontier claim itself.
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

    pub(super) fn revision(&self) -> Result<Option<String>, FractalError> {
        let heads = self.predecessors()?;
        Ok(match heads.as_slice() {
            [] => None,
            [head] => Some(head.clone()),
            _ => Some(format!("thread-revision/{}", canonical_hash(&(
                "fractal.thread-metadata-frontier.v1", heads,
            )).map_err(internal)?)),
        })
    }
}

fn conflict() -> FractalError {
    FractalError::new("idempotency-conflict", "the thread metadata operation has conflicting requests")
}
