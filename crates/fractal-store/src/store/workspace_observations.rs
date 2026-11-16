//! Selected seats' workspace observations, without a query per roster card.

use super::*;

const SUBJECT_CHUNK: usize = 256;

impl Store {
    /// The same observation as [`Self::latest_observation`] for `workspace.observed`, keyed by
    /// selected subject. Each chunk reads both the canonical latest claim and the latest local
    /// observation; store-log order decides between them, not the observation's wall clock.
    pub fn latest_workspace_observations(&self, subjects: &[&str]) -> Result<HashMap<String, ClaimRecord>> {
        let mut newest = HashMap::new();
        if subjects.is_empty() {
            return Ok(newest);
        }
        for subject in subjects {
            smallclaims::touched::note_read(|| (*subject).to_owned());
        }
        let connection = self.readers.get();
        let mut statement = connection.prepare_cached(
            /* sqlite-large-value: allow-for-now lv-hae6731a417ce owner=fractal_store::store::workspace_observations TODO=trace-statement-and-value-representation */
            &format!(
                "SELECT {CLAIM_COLUMNS}, 0 FROM json_each(?1) AS seats
                 JOIN claims ON claims.id=(
                     SELECT claims.id FROM claims INDEXED BY claims_subject_kind_accepted_index
                     JOIN batches ON batches.id=claims.batch_id
                     WHERE claims.subject=seats.value AND claims.kind='workspace.observed'
                     ORDER BY {CANONICAL_ORDER_DESC} LIMIT 1)
                 UNION ALL
                 SELECT local.id, local.after_store_index, local.subject, local.kind,
                        local.actor, local.body, local.observed_at_unix_ms, NULL, NULL, NULL, 1
                 FROM json_each(?1) AS seats
                 JOIN local_observations AS local ON local.id=(
                     SELECT id FROM local_observations INDEXED BY local_observations_subject_kind_index
                     WHERE subject=seats.value AND kind='workspace.observed' ORDER BY id DESC LIMIT 1)"
            ),
        )?;
        for chunk in subjects.chunks(SUBJECT_CHUNK) {
            let rows = statement.query_map([serde_json::to_string(chunk)?], |row| {
                if row.get::<_, bool>(10)? {
                    local_observation_from_row(&self.origin, row)
                } else {
                    claim_from_row(row)
                }
            })?;
            for row in rows {
                let observation = row?;
                match newest.entry(observation.subject.clone()) {
                    std::collections::hash_map::Entry::Vacant(entry) => {
                        entry.insert(observation);
                    }
                    std::collections::hash_map::Entry::Occupied(mut entry) => {
                        if claim_log_order(&observation) >= claim_log_order(entry.get()) {
                            entry.insert(observation);
                        }
                    }
                }
            }
        }
        Ok(newest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observation(subject: &str, repository: &str) -> ClaimInput {
        ClaimInput {
            subject: subject.into(),
            kind: "workspace.observed".into(),
            actor: None,
            fields: BTreeMap::from([
                ("host".into(), json!("node")),
                ("workspace".into(), json!("/workspace")),
                ("repository".into(), json!(repository)),
            ]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        }
    }

    fn local(store: &Store, input: &ClaimInput, at: u128) {
        let mut connection = store.connection.write();
        let transaction = connection.transaction().unwrap();
        insert_local_observation_tx(&transaction, store.origin(), input, at).unwrap();
        transaction.commit().unwrap();
    }

    #[test]
    fn workspace_batch_matches_point_reader_for_canonical_and_local_observations() {
        let store = Store::open_memory("node").unwrap();
        store.append_as_older_build_for_test(&observation("agent/canonical", "example/canonical")).unwrap();
        local(&store, &observation("agent/local", "example/local"), 100);
        store.append_as_older_build_for_test(&observation("agent/local-newer", "example/old")).unwrap();
        local(&store, &observation("agent/local-newer", "example/local"), 1);
        local(&store, &observation("agent/canonical-newer", "example/old"), 1_000_000);
        store.append_as_older_build_for_test(&observation("agent/canonical-newer", "example/new")).unwrap();
        store.append_as_older_build_for_test(&observation("agent/unselected", "example/other")).unwrap();
        let subjects = ["agent/canonical", "agent/local", "agent/local-newer", "agent/canonical-newer", "agent/missing"];
        let batch = store.latest_workspace_observations(&subjects).unwrap();
        assert_eq!(batch.len(), 4);
        for subject in subjects {
            let expected = store.latest_observation(subject, "workspace.observed").unwrap();
            assert_eq!(batch.get(subject).map(|row| (&row.id, &row.body)),
                expected.as_ref().map(|row| (&row.id, &row.body)), "{subject}");
        }
        assert_eq!(batch["agent/local-newer"].body["fields"]["repository"], "example/local");
        assert_eq!(batch["agent/canonical-newer"].body["fields"]["repository"], "example/new");
        assert!(store.latest_workspace_observations(&[]).unwrap().is_empty());
    }

    #[test]
    fn workspace_batch_query_growth_is_per_chunk_not_per_subject() {
        let store = Store::open_memory("node").unwrap();
        let subjects = (0..=SUBJECT_CHUNK).map(|i| format!("agent/seat-{i}")).collect::<Vec<_>>();
        for subject in &subjects {
            store.append_as_older_build_for_test(&observation(subject, "example/repo")).unwrap();
        }
        for count in [0, 1, SUBJECT_CHUNK, SUBJECT_CHUNK + 1] {
            let selected = subjects[..count].iter().map(String::as_str).collect::<Vec<_>>();
            let scope = smallclaims::sqlite::work::SqliteWorkScope::start();
            let rows = store.latest_workspace_observations(&selected).unwrap();
            let work = scope.finish();
            assert_eq!(rows.len(), count);
            assert_eq!(work.statements, count.div_ceil(SUBJECT_CHUNK) as u64, "{count}: {work:?}");
        }
    }
}
