//! The part of `store::publication` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use crate::PublicationFactsStore;
use super::*;
use crate::model::MemberSpec;
use crate::publication::{Interruption, interruption};
pub use fractal_attention::store::publication::*;
pub use fractal_attention::store::publication::{Check, CommitChecks, Guard, check, launched_member};

pub trait PublicationStore {
    fn publication_cache_epoch(&self) -> Option<(i64, bool)>;
    fn publication_evidence_epoch(&self, subject: &str) -> Result<(u64, i64, bool)>;
    fn publication_launch(
            &self,
            subject: &str,
            incarnation: &str,
        ) -> Result<Option<MemberSpec>>;
}

impl PublicationStore for Store {

    fn publication_cache_epoch(&self) -> Option<(i64, bool)> {
        *self
            .smalltalk
            .publication_cache_epoch
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn publication_evidence_epoch(&self, subject: &str) -> Result<(u64, i64, bool)> {
        smallclaims::touched::note_read(|| subject.to_owned());
        Ok(self
            .readers
            .get()
            .prepare_cached(
                "SELECT (SELECT COALESCE(MAX(store_index),0) FROM claims WHERE subject=?1),
                    (SELECT COALESCE(MAX(id),0) FROM local_observations WHERE subject=?1),
                    (SELECT phase=3 FROM local_publication_bootstrap WHERE singleton=1)
                        AND NOT EXISTS(SELECT 1 FROM local_publication_pending)",
            )?
            .query_row(/* sqlite-large-value: allow-for-now lv-h2865f1f7e276 owner=fractal_evidence::store::publication TODO=trace-statement-and-value-representation */ [subject], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?)
    }

    fn publication_launch(
            &self,
            subject: &str,
            incarnation: &str,
        ) -> Result<Option<MemberSpec>> {
        launched_member(
            &self.readers.get(),
            subject,
            &self.origin,
            incarnation,
            i64::MAX as u64,
            &self.origin,
        )
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::parse_intent;
    use fractal_attention::store::shared::*;
    use fractal_attention::store::publication_tests_shared::*;
    use fractal_attention::store::publication_tests_shared::{declaration, observe, publish};

    #[test]
    fn publication_check_stopped_origin_cannot_hide_another_origins_running_seat() {
        let local = Store::open_memory("node").unwrap();
        let peer = Store::open_memory("peer").unwrap();
        let original = declaration("true");
        let applied = publish(&peer, &original, "peer-original");
        peer.append_claim(&ClaimInput {
            subject: "agent/example/worker".into(),
            kind: "runtime.action.succeeded".into(),
            actor: Some("agent/example/writer".into()),
            fields: BTreeMap::from([
                ("action".into(), json!("start")),
                ("incarnation_id".into(), json!("peer-inc")),
                (
                    "desired_token".into(),
                    json!(applied.subject_tokens["agent/example/worker"][0]),
                ),
            ]),
            evidence: vec![],
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
        peer.append_claim(&ClaimInput {
            subject: "agent/example/worker".into(),
            kind: "runtime.observed".into(),
            actor: None,
            fields: BTreeMap::from([
                ("status".into(), json!("running")),
                ("incarnation_id".into(), json!("peer-inc")),
            ]),
            evidence: vec![],
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
        local
            .import_replication("peer", &peer.export_replication(0).unwrap())
            .unwrap();
        local
            .append_claim(&ClaimInput {
                subject: "agent/example/worker".into(),
                kind: "runtime.observed".into(),
                actor: None,
                fields: BTreeMap::from([
                    ("status".into(), json!("stopped")),
                    ("incarnation_id".into(), json!("node-inc")),
                ]),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
        while !local.advance_publication_evidence().unwrap() {}
        let proposed = declaration("false");
        let judgment = check(
            &local.readers.get(),
            &proposed.subjects["agent/example/worker"],
            u64::MAX,
            "node",
        )
        .unwrap();
        assert!(matches!(judgment.interruption, Interruption::Restart(_)));
        assert_eq!(judgment.incarnation.as_deref(), Some("peer-inc"));
    }

    #[test]
    #[ignore = "process-wide SQL counters: run alone with --exact --ignored --test-threads=1"]
    fn publication_check_cost_is_independent_of_unrelated_and_old_incarnation_history() {
        use smallclaims::sqlite::work;
        let measure = |count: u64| {
            let store = Store::open_memory("node").unwrap();
            let intent = declaration("true");
            let applied = publish(&store, &intent, "original");
            observe(
                &store,
                "current",
                Some(&applied.subject_tokens["agent/example/worker"][0]),
            );
            let mut connection = store.connection.write();
            let tx = connection.transaction().unwrap();
            tx.execute(
                "INSERT INTO batches(id,origin,replica_sequence,hash,accepted_at_unix_ms)
                VALUES ('noise-batch','node',100000,'synthetic','1')",
                [],
            )
            .unwrap();
            for index in 0..count {
                let (subject, kind, body) = match index % 3 {
                    0 => (
                        "agent/unrelated",
                        "harness.observed",
                        json!({"fields":{"state":"idle"}}),
                    ),
                    1 => (
                        "agent/example/worker",
                        "runtime.observed",
                        json!({"fields":{"status":"running","incarnation_id":"old"}}),
                    ),
                    _ => (
                        "agent/example/worker",
                        "runtime.action.succeeded",
                        json!({"fields":{"action":"start","incarnation_id":"old","desired_token":"missing"}}),
                    ),
                };
                tx.execute("INSERT INTO claims(store_index,id,batch_id,subject,kind,origin,body,predecessors,accepted_at_unix_ms)
                    VALUES (?1,?2,'noise-batch',?3,?4,'node',?5,'[]','1')",
                    params![100000 + index, format!("noise-{index}"), subject, kind, body.to_string()]).unwrap();
            }
            tx.commit().unwrap();
            drop(connection);
            while !store.advance_publication_evidence().unwrap() {}
            let connection = store.readers.get();
            let before = work::total();
            let started = std::time::Instant::now();
            let result = check(
                &connection,
                &intent.subjects["agent/example/worker"],
                u64::MAX,
                "node",
            )
            .unwrap();
            let elapsed = started.elapsed();
            let cost = work::total() - before;
            assert_eq!(result.interruption, Interruption::None);
            assert_eq!(result.incarnation.as_deref(), Some("current"));
            println!("publication check history={count}: {cost:?}, elapsed={elapsed:?}");
            assert!(elapsed < std::time::Duration::from_millis(100));
            cost
        };
        let small = measure(1000);
        let large = measure(10000);
        assert!(
            small.vm_steps > 0 && large.vm_steps <= small.vm_steps + 100,
            "small={small:?}, large={large:?}"
        );
        assert_eq!(large.fullscan_steps, 0);
    }
}
