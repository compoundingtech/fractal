//! Owner-local staging freshness, never replicated. The accepting member may learn it only
//! through an answered owner read (the expiry writer's separate rollout).
use anyhow::Result;
use rusqlite::{Connection, OptionalExtension as _, params};

fn installed(connection: &Connection) -> Result<bool> {
    Ok(connection.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='local_staging_attempts')", [], |row| row.get(0))?)
}

/// Forward-only, atomic, once-only backfill. Table existence is the completion marker.
/// Old unresolved attempts may have lost a retry timestamp: delay only those rows from
/// migration time, never a sent time, a resolved receipt, or a silent wait's original age.
pub(super) fn backfill(connection: &Connection, shard: bool, now: u128) -> Result<()> {
    if installed(connection)? { return Ok(()); }
    let transaction = connection.unchecked_transaction()?;
    transaction.execute_batch("CREATE TABLE IF NOT EXISTS local_staging_attempts (
        message TEXT PRIMARY KEY, last_attempt_at INTEGER NOT NULL
    );")?;
    let (attempts, resolved) = if shard {
        ("SELECT subject AS message, recorded_at_unix_ms AS recorded FROM stream_lifecycle WHERE kind='message.staged'",
         "SELECT subject AS message FROM stream_lifecycle WHERE kind IN ('message.delivered','message.read','message.closed')")
    } else {
        ("SELECT message, recorded_at_unix_ms AS recorded FROM local_message_staged
          UNION ALL SELECT subject AS message, CAST(accepted_at_unix_ms AS INTEGER) AS recorded FROM claims WHERE kind='message.staged'",
         "SELECT message FROM local_message_marks
          UNION ALL SELECT subject AS message FROM claims WHERE kind IN ('message.delivered','message.read','message.closed')")
    };
    transaction.execute(&format!("INSERT INTO local_staging_attempts(message,last_attempt_at)
        SELECT message, CASE WHEN message IN ({resolved}) THEN MAX(recorded)
               ELSE MAX(MAX(recorded), ?1) END FROM ({attempts}) GROUP BY message
        ON CONFLICT(message) DO NOTHING"), [now.min(i64::MAX as u128) as i64])?;
    transaction.commit()?;
    Ok(())
}

/// Track an admitted attempt without changing its first receipt or its idempotent outcome.
pub(super) fn record(connection: &Connection, message: &str, now: u128) -> Result<()> {
    connection.execute("INSERT INTO local_staging_attempts(message,last_attempt_at) VALUES (?1,?2)
        ON CONFLICT(message) DO UPDATE SET last_attempt_at=MAX(last_attempt_at,excluded.last_attempt_at)",
        params![message, now.min(i64::MAX as u128) as i64])?;
    Ok(())
}

/// Older owner databases have no freshness table. Absence is not evidence of no attempt.
pub fn last_attempt_at(connection: &Connection, message: &str) -> Result<Option<u128>> {
    if !installed(connection)? { return Ok(None); }
    Ok(connection.query_row("SELECT last_attempt_at FROM local_staging_attempts WHERE message=?1",
        [message], |row| row.get::<_, i64>(0)).optional()?.map(|at| at.max(0) as u128))
}

#[cfg(test)]
mod tests {
    #[test]
    fn production_restage_after_upgrade_preserves_first_receipt() {
        use crate::store::{Store, messages_shard, stream_mirror};
        use crate::model::ClaimInput;
        use serde_json::json;
        use std::{collections::BTreeMap, sync::Arc};
        for storage in ["graph", "marks", "shard"] {
            let dir = tempfile::tempdir().unwrap();
            let store = Arc::new(Store::open(&dir.path().join("claims.sqlite3"), "fixture").unwrap());
            let subject = "message/attempt";
            let sent = ClaimInput { subject: subject.into(), kind: "message.sent".into(),
                actor: Some("agent/sender".into()),
                fields: BTreeMap::from([("status".into(),json!("sent")),("from".into(),json!("agent/sender")),
                    ("to".into(),json!("agent/fixture.recipient")),("content".into(),json!("Attempt"))]),
                evidence: vec![], expected_subject: None, idempotency_key: None };
            let shard = if storage == "shard" {
                store.bind_fleet("attempt-fixture").unwrap();
                store.load_member_key(Some(Arc::new(smallclaims::fleet::MemberKey::generate().unwrap().0))).unwrap();
                store.use_key_directory(&dir.path().join("keys")).unwrap();
                let shard = messages_shard::open(&store, dir.path()).unwrap();
                shard.append_claim(&sent).unwrap();
                stream_mirror::mirror(&store).unwrap();
                Some(shard)
            } else {
                store.append_claim(&sent).unwrap();
                None
            };
            if storage == "marks" {
                let intent = crate::graph::parse_intent("version 2\nagent \"recipient\" { host \"fixture\"; command \"true\"; }", "fixture").unwrap();
                crate::store::card_fold_tests_shared_missions::declare(&store, &intent, "attempt-local-seat");
                assert!(store.refresh_marks_switch(&[]).unwrap().0);
            }
            let staged = ClaimInput { kind: "message.staged".into(), actor: Some("agent/fixture.recipient".into()),
                fields: BTreeMap::from([("status".into(),json!("staged"))]), ..sent };
            let stage = || {
                if shard.is_some() {
                    messages_shard::record_lifecycle(&store, subject, &staged.kind, staged.actor.as_deref(), &staged.fields).unwrap()
                } else { store.append_claim_outcome(&staged).unwrap().1 }
            };
            assert!(stage());
            let first_mark = (storage == "marks").then(|| crate::store::message_marks::staged(&store.readers.get(), subject).unwrap().unwrap().1);
            let db = shard.as_deref().unwrap_or(&store.graph);
            let first = last_attempt_at(&db.readers.get(), subject).unwrap().unwrap();
            // Model an upgraded attempt table, without changing any first-receipt fields.
            db.connection.write().execute("UPDATE local_staging_attempts SET last_attempt_at=0", []).unwrap();
            assert!(!stage(), "a restage remains an idempotent receipt");
            assert!(last_attempt_at(&db.readers.get(), subject).unwrap().unwrap() >= first);
            if let Some(shard) = &shard {
                assert_eq!(shard.readers.get().query_row("SELECT COUNT(*) FROM stream_lifecycle WHERE kind='message.staged'", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
            } else if storage == "marks" {
                assert_eq!(crate::store::message_marks::staged(&store.readers.get(), subject).unwrap().unwrap().1, first_mark.unwrap());
            } else {
                assert_eq!(store.claims_for(subject, Some("message.staged")).unwrap().len(), 1);
            }
        }
    }

    use super::*;

    fn old_store(shard: bool) -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE claims(subject TEXT,kind TEXT,accepted_at_unix_ms TEXT);
            CREATE TABLE local_message_staged(message TEXT,recorded_at_unix_ms INTEGER);
            CREATE TABLE local_message_marks(message TEXT);
            CREATE TABLE stream_lifecycle(subject TEXT,kind TEXT,recorded_at_unix_ms INTEGER);").unwrap();
        for kind in ["message.sent", "message.staged"] {
            db.execute("INSERT INTO claims VALUES ('message/staged',?1,'10')", [kind]).unwrap();
            if shard { db.execute("INSERT INTO stream_lifecycle VALUES ('message/staged',?1,10)", [kind]).unwrap(); }
        }
        db
    }

    #[test]
    fn unresolved_staged_backfill_delays_once_and_restage_advances_only_freshness() {
        for shard in [false, true] {
            let db = old_store(shard);
            backfill(&db, shard, 100).unwrap();
            assert_eq!(last_attempt_at(&db, "message/staged").unwrap(), Some(100));
            backfill(&db, shard, 200).unwrap();
            assert_eq!(last_attempt_at(&db, "message/staged").unwrap(), Some(100));
            record(&db, "message/staged", 300).unwrap();
            record(&db, "message/staged", 250).unwrap();
            assert_eq!(last_attempt_at(&db, "message/staged").unwrap(), Some(300));
            assert_eq!(db.query_row("SELECT accepted_at_unix_ms FROM claims WHERE kind='message.staged'", [], |r| r.get::<_, String>(0)).unwrap(), "10");
        }
    }

    #[test]
    fn never_offered_keeps_original_accepted_time() {
        let db = old_store(false);
        db.execute("INSERT INTO claims VALUES ('message/never','message.sent','7')", []).unwrap();
        assert_eq!(last_attempt_at(&db, "message/never").unwrap(), None);
        backfill(&db, false, 100).unwrap();
        assert_eq!(last_attempt_at(&db, "message/never").unwrap(), None);
        assert_eq!(db.query_row("SELECT accepted_at_unix_ms FROM claims WHERE subject='message/never'", [], |r| r.get::<_, String>(0)).unwrap(), "7");
    }

    #[test]
    fn waits_for_turn_keeps_original_accepted_time() {
        let db = old_store(false);
        // A silent wait has no staged receipt: waits_for_turn requires status == sent.
        db.execute("INSERT INTO claims VALUES ('message/silent','message.sent','8')", []).unwrap();
        backfill(&db, false, 100).unwrap();
        assert_eq!(last_attempt_at(&db, "message/silent").unwrap(), None);
        assert_eq!(db.query_row("SELECT accepted_at_unix_ms FROM claims WHERE subject='message/silent'", [], |r| r.get::<_, String>(0)).unwrap(), "8");
    }

    #[test]
    fn resolved_staged_backfill_keeps_recorded_time() {
        for shard in [false, true] {
            let db = old_store(shard);
            db.execute("INSERT INTO claims VALUES ('message/staged','message.delivered','20')", []).unwrap();
            db.execute("INSERT INTO stream_lifecycle VALUES ('message/staged','message.delivered',20)", []).unwrap();
            backfill(&db, shard, 100).unwrap();
            assert_eq!(last_attempt_at(&db, "message/staged").unwrap(), Some(10));
        }
    }
}
