//! The part of `store::coordination` that moved up from fractal-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use crate::DeliveryStore;
use super::*;
pub use fractal_delivery::store::coordination::*;
pub use fractal_delivery::store::coordination::{backfill, cover_synced_sends, create_schema, initialize, sync};

pub trait CoordinationStore {
    /// One bounded bootstrap job on the existing FIFO writer. The daemon awaits this job
    /// and pauses between jobs, so several per-claim projections cannot multiply its budget.
    fn advance_coordination_counts(&self) -> Result<bool>;
    fn coordination_backfill_status(&self) -> Result<(u64, u64, bool, u64)>;
    /// One covering range read; counts distinct message subjects, including already read mail.
    /// silent is the held subset of agent-to-agent sends. Person-directed sends are separate.
    fn coordination_counts(&self, since: u64, until: u64) -> Result<Value>;
}

impl CoordinationStore for Store {

    fn advance_coordination_counts(&self) -> Result<bool> {
        let mut connection = self.connection.write_background();
        let transaction = connection.transaction()?;
        let complete = backfill(&transaction)?;
        transaction.commit()?;
        Ok(complete)
    }

    fn coordination_backfill_status(&self) -> Result<(u64, u64, bool, u64)> {
        Ok(self.readers.get().query_row(
            "SELECT cursor,ceiling,complete,progress_ms FROM local_coordination_backfill WHERE singleton=1",
            [], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)))?)
    }

    fn coordination_counts(&self, since: u64, until: u64) -> Result<Value> {
        let connection = self.readers.get();
        let (agents, silent, people, complete, cursor, ceiling, progress): (u64, u64, u64, bool, u64, u64, u64) = connection.query_row(
            "SELECT COALESCE(SUM(agent_to_agent),0),COALESCE(SUM(silent),0),COALESCE(SUM(to_person),0),
                COALESCE((SELECT complete FROM local_coordination_backfill WHERE singleton=1),0),
                COALESCE((SELECT cursor FROM local_coordination_backfill WHERE singleton=1),0),
                COALESCE((SELECT ceiling FROM local_coordination_backfill WHERE singleton=1),0),
                COALESCE((SELECT progress_ms FROM local_coordination_backfill WHERE singleton=1),0)
             FROM coordination_sends INDEXED BY coordination_sends_time WHERE sent_ms>=?1 AND sent_ms<?2",
            params![i64::try_from(since).unwrap_or(i64::MAX), i64::try_from(until).unwrap_or(i64::MAX)], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?)),
        )?;
        let merged = merged_pull_requests(&connection, since, until)?;
        // Messages per landed change: unknown while the counts are incomplete or nothing merged.
        let per_merged = (complete && merged > 0).then(|| agents as f64 / merged as f64);
        Ok(
            json!({"since_ms":since,"until_ms":until,"agent_to_agent":agents,"silent":silent,"to_person":people,"merged_pull_requests":merged,"agent_to_agent_per_merged_pull_request":per_merged,"complete":complete,"cursor":cursor,"ceiling":ceiling,"progress_ms":progress}),
        )
    }
}

/// Pull requests this daemon has observed merging in the period, from the latest observation of
/// each, so a pull request counts once. One that merged unobserved is not counted.
fn merged_pull_requests(connection: &rusqlite::Connection, since: u64, until: u64) -> Result<u64> {
    Ok(connection.query_row(
        "SELECT COUNT(*) FROM (SELECT CAST(round((julianday(json_extract(facts,'$.merged_at'))-2440587.5)*86400000) AS INTEGER) AS merged_ms
             FROM resource_observations INDEXED BY resource_observations_kind WHERE kind='vcs.pull-request')
         WHERE merged_ms>=?1 AND merged_ms<?2",
        params![i64::try_from(since).unwrap_or(i64::MAX), i64::try_from(until).unwrap_or(i64::MAX)],
        |row| row.get(0),
    )?)
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previous_fyi_preview_reopens_forward_and_preserves_unoffered_silent_mail() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("graph.db");
        let store = Store::open(&path, "node").unwrap();
        let send = |subject: &str, tag: &str| ClaimInput {
            subject: subject.into(),
            kind: "message.sent".into(),
            actor: Some("agent/example/writer".into()),
            fields: BTreeMap::from([
                ("status".into(), json!("sent")),
                ("from".into(), json!("agent/example/writer")),
                ("to".into(), json!("agent/example/reader")),
                ("content".into(), json!("held")),
                ("tags".into(), json!([tag])),
            ]),
            evidence: vec![],
            expected_subject: None,
            idempotency_key: None,
        };
        store
            .append_claim(&send("message/preview", crate::silent::LEGACY_FYI_TAG))
            .unwrap();
        store
            .connection
            .batched(|tx| {
                tx.execute_batch("ALTER TABLE coordination_sends RENAME COLUMN silent TO fyi;")?;
                Ok::<_, anyhow::Error>(())
            })
            .unwrap()
            .unwrap();
        drop(store);
        let reopened = Store::open(&path, "node").unwrap();
        let old = reopened.message("message/preview").unwrap().unwrap();
        assert!(crate::silent::is_held(&old));
        assert_eq!(old.tags, vec![crate::silent::SILENT_TAG]);
        let mut unoffered = vec![old];
        crate::silent::release(&mut unoffered);
        assert!(unoffered.is_empty());
        assert_eq!(
            reopened
                .claims_for("message/preview", Some("message.sent"))
                .unwrap()[0]
                .body["fields"]["tags"],
            json!([crate::silent::LEGACY_FYI_TAG])
        );
        reopened
            .append_claim(&send("message/new", crate::silent::SILENT_TAG))
            .unwrap();
        let counts = reopened.coordination_counts(0, u64::MAX / 2).unwrap();
        assert_eq!(counts["silent"], 2);
        assert_eq!(counts["agent_to_agent"], 2);
        assert_eq!(counts["complete"], true);
        assert_eq!(reopened.held_mail_before(u128::MAX).unwrap()[0].1, 2);
        let plan = reopened.readers.get().prepare("EXPLAIN QUERY PLAN SELECT SUM(silent) FROM coordination_sends WHERE sent_ms>=0 AND sent_ms<=10000000000000").unwrap().query_map([], |row| row.get::<_,String>(3)).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
        assert!(
            plan.iter()
                .any(|row| row.contains("COVERING INDEX coordination_sends_time")),
            "{plan:?}"
        );
    }

    #[test]
    fn counts_agent_messages_per_pull_request_merged_in_the_period() {
        let store = Store::open_memory("node").unwrap();
        let claim = |subject: &str, kind: &str, fields: Value| ClaimInput {
            subject: subject.into(),
            kind: kind.into(),
            actor: None,
            fields: serde_json::from_value(fields).unwrap(),
            evidence: vec![],
            expected_subject: None,
            idempotency_key: None,
        };
        let pull = |number: u32, merged_at: Value| {
            claim(
                &format!("resource/github/acme/app/pull-request/{number}"),
                "resource.observed",
                json!({"kind": "vcs.pull-request", "facts": {"number": number, "merged_at": merged_at}}),
            )
        };
        let since = 1_791_244_800_000; // 2026-10-06T00:00:00Z
        let until = since + 86_400_000;
        store.append_claim(&pull(1, json!(null))).unwrap();
        // Merged in the period, observed twice: one pull request.
        store.append_claim(&pull(1, json!("2026-10-06T00:00:00Z"))).unwrap();
        store.append_claim(&pull(2, json!("2026-10-06T00:00:00Z"))).unwrap();
        store.append_claim(&pull(3, json!("2026-10-06T23:59:59.999Z"))).unwrap();
        store.append_claim(&pull(4, json!("2026-10-07T00:00:00Z"))).unwrap();
        store.append_claim(&pull(5, json!("2026-10-05T23:59:59Z"))).unwrap();
        store.append_claim(&pull(6, json!(null))).unwrap();
        let counts = store.coordination_counts(since, until).unwrap();
        assert_eq!(counts["merged_pull_requests"], 3);
        assert_eq!(counts["agent_to_agent_per_merged_pull_request"], 0.0);
        let nothing_merged = store.coordination_counts(since - 86_400_000, since - 1_000).unwrap();
        assert_eq!(nothing_merged["merged_pull_requests"], 0);
        assert_eq!(nothing_merged["agent_to_agent_per_merged_pull_request"], Value::Null);

        let now = chrono::Utc::now();
        for n in 0..6 {
            store
                .append_claim(&ClaimInput {
                    actor: Some("agent/example/writer".into()),
                    ..claim(
                        &format!("message/per-pull-{n}"),
                        "message.sent",
                        json!({"status": "sent", "from": "agent/example/writer", "to": "agent/example/reader", "content": "merged"}),
                    )
                })
                .unwrap();
        }
        store.append_claim(&pull(8, json!(now.to_rfc3339()))).unwrap();
        store.append_claim(&pull(9, json!(now.to_rfc3339()))).unwrap();
        let now = now.timestamp_millis() as u64;
        let counts = store.coordination_counts(now - 60_000, now + 60_000).unwrap();
        assert_eq!(counts["complete"], true);
        assert_eq!(counts["agent_to_agent"], 6);
        assert_eq!(counts["merged_pull_requests"], 2);
        assert_eq!(counts["agent_to_agent_per_merged_pull_request"], 3.0);
    }

    #[test]
    fn malformed_historical_body_keeps_bootstrap_incomplete_without_blocking_open() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("graph.db");
        let store = Store::open(&path, "node").unwrap();
        let claim = store
            .append_claim(&ClaimInput {
                subject: "message/corrupt-history".into(),
                kind: "message.sent".into(),
                actor: Some("agent/example/writer".into()),
                fields: BTreeMap::from([
                    ("status".into(), json!("sent")),
                    ("from".into(), json!("agent/example/writer")),
                    ("to".into(), json!("agent/example/reader")),
                    ("content".into(), json!("history")),
                ]),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
        // SQLite accepts this JSON; serde's recursion guard rejects it.
        let body = format!(
            "{},\"deep\":{}0{}}}",
            serde_json::to_string(&claim.body)
                .unwrap()
                .strip_suffix('}')
                .unwrap(),
            "[".repeat(140),
            "]".repeat(140)
        );
        store
            .connection
            .batched(|tx| {
                tx.execute(
                    "UPDATE claims SET body=?1 WHERE id=?2",
                    params![body, claim.id],
                )?;
                // The old allowance projection is already populated. The source edit
                // above dirties it through an UPDATE trigger; remove that fixture-only
                // rebuild request so startup reaches the new historical bootstrap.
                tx.execute("DELETE FROM local_agent_message_pending", [])?;
                tx.execute("DELETE FROM coordination_sends", [])?;
                tx.execute(
                    "UPDATE local_coordination_backfill SET cursor=0,ceiling=?1,complete=0",
                    [current_index(tx)?],
                )?;
                Ok::<_, anyhow::Error>(())
            })
            .unwrap()
            .unwrap();
        drop(store);
        let reopened = Store::open(&path, "node")
            .expect("a failed count bootstrap must not block daemon startup");
        assert_eq!(
            reopened.coordination_counts(0, u64::MAX / 2).unwrap()["complete"],
            false
        );
        assert_eq!(reopened.coordination_backfill_status().unwrap().0, 0);
        assert!(reopened.advance_coordination_counts().is_err());
    }

    #[test]
    fn reconnect_hydrates_eight_held_subjects_and_doctor_reads_at_most_128() {
        let store = Store::open_memory("node").unwrap();
        for n in 0..140 {
            store
                .append_claim(&ClaimInput {
                    subject: format!("message/held-{n}"),
                    kind: "message.sent".into(),
                    actor: Some("agent/example/writer".into()),
                    fields: BTreeMap::from([
                        ("status".into(), json!("sent")),
                        ("from".into(), json!("agent/example/writer")),
                        ("to".into(), json!("agent/example/reader")),
                        ("content".into(), json!("held")),
                        ("tags".into(), json!([crate::silent::SILENT_TAG])),
                    ]),
                    evidence: vec![],
                    expected_subject: None,
                    idempotency_key: None,
                })
                .unwrap();
        }
        let delivery = store
            .messages_for_delivery_through("agent/example/reader", store.index().unwrap())
            .unwrap();
        assert_eq!(delivery.len(), crate::silent::BATCH_LIMIT);
        assert_eq!(delivery[0].subject, "message/held-132");
        assert!(
            delivery[0]
                .tags
                .contains(&format!("{}132", crate::silent::REMAINING_PREFIX))
        );
        assert_eq!(
            store
                .messages(Some("agent/example/reader"), false)
                .unwrap()
                .len(),
            140
        );
        assert_eq!(store.held_mail_before(u128::MAX).unwrap()[0].1, 128);
        assert_eq!(store.held_mail_count_before(u128::MAX).unwrap(), 140);
        store
            .append_claim(&ClaimInput {
                subject: delivery[0].subject.clone(),
                kind: "message.staged".into(),
                actor: Some("agent/example/reader".into()),
                fields: BTreeMap::from([("status".into(), json!("staged"))]),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
        assert_eq!(store.held_mail_count_before(u128::MAX).unwrap(), 139);
        // Simulate an old binary's missing cache and a previously complete marker.
        store
            .connection
            .batched(|tx| {
                tx.execute("DELETE FROM coordination_sends", [])?;
                tx.execute(
                    "UPDATE local_coordination_backfill SET cursor=0,ceiling=0,complete=1",
                    [],
                )?;
                initialize(tx)?;
                Ok::<(), anyhow::Error>(())
            })
            .unwrap()
            .unwrap();
        assert_eq!(
            store.coordination_counts(0, u64::MAX / 2).unwrap()["complete"],
            false
        );
        while !store.advance_coordination_counts().unwrap() {}
        assert_eq!(
            store.coordination_counts(0, u64::MAX / 2).unwrap()["agent_to_agent"],
            140
        );
        assert_eq!(store.held_mail_count_before(u128::MAX).unwrap(), 139);
    }

    #[test]
    fn historical_backfill_is_bounded_and_reads_leave_partial_state_untouched() {
        let store = Store::open_memory("node").unwrap();
        for n in 0..19 {
            store
                .append_claim(&ClaimInput {
                    subject: format!("message/history-{n}"),
                    kind: "message.sent".into(),
                    actor: Some("agent/example/writer".into()),
                    fields: BTreeMap::from([
                        ("status".into(), json!("sent")),
                        ("from".into(), json!("agent/example/writer")),
                        ("to".into(), json!("agent/example/reader")),
                        ("tags".into(), json!([crate::silent::SILENT_TAG])),
                    ]),
                    evidence: vec![],
                    expected_subject: None,
                    idempotency_key: None,
                })
                .unwrap();
        }
        let digest = graph_digest(&store.readers.get()).unwrap();
        store
            .connection
            .batched(|tx| {
                tx.execute("DELETE FROM coordination_sends", [])?;
                tx.execute(
                    "UPDATE local_coordination_backfill SET cursor=0,ceiling=?1,complete=0",
                    [current_index(tx)?],
                )?;
                // Simulate opening an older store before the allowance cache existed.
                // Its existing rebuild must not populate all new count metadata at once.
                tx.execute("DELETE FROM meta WHERE key='agent_message_days_v1'", [])?;
                agent_messages::open(tx)
            })
            .unwrap()
            .unwrap();
        let partial = store.coordination_counts(0, u64::MAX / 2).unwrap();
        assert!(partial["agent_to_agent"].as_u64().unwrap() <= 8);
        assert_eq!(partial["complete"], false);
        assert_eq!(store.coordination_counts(0, u64::MAX / 2).unwrap(), partial);
        // Many projections can share one outer transaction. None may spend another
        // historical budget: only the single daemon bootstrap job advances the cursor.
        store
            .connection
            .batched(|tx| {
                for _ in 0..16 {
                    agent_messages::flush(tx)?;
                }
                Ok::<_, anyhow::Error>(())
            })
            .unwrap()
            .unwrap();
        assert_eq!(store.coordination_counts(0, u64::MAX / 2).unwrap(), partial);
        let mut complete = partial;
        for _ in 0..20 {
            if complete["complete"] == true {
                break;
            }
            let previous = complete["agent_to_agent"].as_u64().unwrap();
            store.advance_coordination_counts().unwrap();
            complete = store.coordination_counts(0, u64::MAX / 2).unwrap();
            let next = complete["agent_to_agent"].as_u64().unwrap();
            assert!(next >= previous && next <= previous + 8);
        }
        assert_eq!(complete["agent_to_agent"], 19);
        assert_eq!(complete["silent"], 19);
        assert_eq!(complete["complete"], true);
        assert_eq!(graph_digest(&store.readers.get()).unwrap(), digest);
    }

    #[test]
    fn distinct_counts_survive_receipts_and_cover_the_time_range_without_read_writes() {
        let store = Store::open_memory("node").unwrap();
        for (subject, to, tags) in [
            ("message/ordinary", "agent/example/reader", vec![]),
            (
                "message/probe",
                "agent/example/delivery-probe/reader",
                vec![crate::silent::SILENT_TAG],
            ),
            (
                "message/soak",
                "agent/example/reader",
                vec!["soak", crate::silent::SILENT_TAG],
            ),
            (
                "message/silent",
                "agent/example/reader",
                vec![crate::silent::SILENT_TAG],
            ),
            (
                "message/person",
                "person/example",
                vec![crate::silent::SILENT_TAG],
            ),
            (
                "message/handoff",
                "agent/example/reader",
                vec![
                    crate::silent::SILENT_TAG,
                    "st3-work-handoff:step-run/example/task",
                ],
            ),
        ] {
            store
                .append_claim(&ClaimInput {
                    subject: subject.into(),
                    kind: "message.sent".into(),
                    actor: Some("agent/example/writer".into()),
                    fields: BTreeMap::from([
                        ("status".into(), json!("sent")),
                        ("from".into(), json!("agent/example/writer")),
                        ("to".into(), json!(to)),
                        ("content".into(), json!("metadata count fixture")),
                        ("tags".into(), json!(tags)),
                    ]),
                    evidence: vec![],
                    expected_subject: None,
                    idempotency_key: Some(subject.into()),
                })
                .unwrap();
        }
        for status in ["staged", "delivered", "read", "read", "closed"] {
            store
                .append_claim(&ClaimInput {
                    subject: "message/silent".into(),
                    kind: format!("message.{status}"),
                    actor: Some("agent/example/reader".into()),
                    fields: BTreeMap::from([("status".into(), json!(status))]),
                    evidence: vec![],
                    expected_subject: None,
                    idempotency_key: None,
                })
                .unwrap();
        }
        let before = store.index().unwrap();
        let counts = store.coordination_counts(0, u64::MAX / 2).unwrap();
        assert_eq!(counts["agent_to_agent"], 3);
        assert_eq!(counts["silent"], 1);
        assert_eq!(counts["to_person"], 1);
        assert_eq!(counts["complete"], true);
        assert_eq!(
            store.coordination_counts(0, 1).unwrap()["agent_to_agent"],
            0
        );
        assert_eq!(store.index().unwrap(), before);
        let plan:String=store.readers.get().query_row(
            "EXPLAIN QUERY PLAN SELECT SUM(silent) FROM coordination_sends INDEXED BY coordination_sends_time WHERE sent_ms>=1 AND sent_ms<2", [], |row| row.get(3)).unwrap();
        assert!(plan.contains("COVERING INDEX"), "{plan}");
    }

    /// A restart used to recheck every send since the previous start, one bounded writer page
    /// per eight, while every seat reconnected. Sends this binary synced need no recheck.
    #[test]
    fn a_reopen_rechecks_only_sends_this_binary_did_not_sync() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("graph.db");
        let send = |store: &Store, subject: &str| {
            store
                .append_claim(&ClaimInput {
                    subject: subject.into(),
                    kind: "message.sent".into(),
                    actor: Some("agent/example/writer".into()),
                    fields: BTreeMap::from([
                        ("status".into(), json!("sent")),
                        ("from".into(), json!("agent/example/writer")),
                        ("to".into(), json!("agent/example/reader")),
                        ("content".into(), json!("ceiling fixture")),
                    ]),
                    evidence: vec![],
                    expected_subject: None,
                    idempotency_key: None,
                })
                .unwrap()
                .store_index
        };
        let latest = {
            let store = Store::open(&path, "node").unwrap();
            for n in 0..20 {
                send(&store, &format!("message/before-restart-{n}"));
            }
            send(&store, "message/last-before-restart")
        };
        let store = Store::open(&path, "node").unwrap();
        let (_, ceiling, complete, _) = store.coordination_backfill_status().unwrap();
        assert!(complete, "nothing to recheck after a restart of the same binary");
        assert_eq!(ceiling, latest);

        // An older binary writes a send without maintaining the counts or the ceiling.
        let older = send(&store, "message/from-an-older-binary");
        store
            .connection
            .batched(|tx| {
                tx.execute(
                    "DELETE FROM coordination_sends WHERE subject='message/from-an-older-binary'",
                    [],
                )?;
                tx.execute(
                    "UPDATE local_coordination_backfill SET ceiling=?1",
                    [latest],
                )?;
                Ok::<_, anyhow::Error>(())
            })
            .unwrap()
            .unwrap();
        drop(store);
        let store = Store::open(&path, "node").unwrap();
        while !store.advance_coordination_counts().unwrap() {}
        let (_, ceiling, complete, _) = store.coordination_backfill_status().unwrap();
        assert!(complete);
        assert_eq!(ceiling, older);
        assert_eq!(
            store.coordination_counts(0, u64::MAX / 2).unwrap()["agent_to_agent"],
            22
        );
    }
}
