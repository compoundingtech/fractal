//! Rebuildable timestamp seeks for declared objectives. Original claim bodies remain authoritative.
use super::*;

const SCHEMA: &str = /* sqlite-large-value: allow-for-now lv-h65b5b05b828f owner=fractal_store::store::objective_series TODO=chunk-or-reference-variable-columns */ "
CREATE TABLE IF NOT EXISTS local_objective_times_v1 (
 store_index INTEGER NOT NULL, field TEXT NOT NULL, kind TEXT NOT NULL,
 at BLOB NOT NULL, subject TEXT NOT NULL, PRIMARY KEY(store_index,field)
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS local_objective_times_seek_v1
 ON local_objective_times_v1(kind,field,at,subject);
CREATE TABLE IF NOT EXISTS local_objective_times_pending_v1(store_index INTEGER PRIMARY KEY);
CREATE TRIGGER IF NOT EXISTS local_objective_times_insert_v1 AFTER INSERT ON claims BEGIN
 INSERT OR IGNORE INTO local_objective_times_pending_v1 VALUES(NEW.store_index); END;
CREATE TRIGGER IF NOT EXISTS local_objective_times_delete_v1 AFTER DELETE ON claims BEGIN
 DELETE FROM local_objective_times_v1 WHERE store_index=OLD.store_index;
 DELETE FROM local_objective_times_pending_v1 WHERE store_index=OLD.store_index; END;
CREATE TRIGGER IF NOT EXISTS local_objective_times_update_v1 AFTER UPDATE ON claims BEGIN
 DELETE FROM local_objective_times_v1 WHERE store_index=OLD.store_index;
 INSERT OR IGNORE INTO local_objective_times_pending_v1 VALUES(NEW.store_index); END;";

/// Exactly the evaluator's timestamp grammar, including JSON-encoded strings.
pub fn time_ms(value: &Value) -> Option<u64> {
    match value {
        Value::Number(number) => number.as_u64(),
        Value::String(text) => chrono::DateTime::parse_from_rfc3339(text)
            .ok()
            .and_then(|at| u64::try_from(at.timestamp_millis()).ok())
            .or_else(|| text.parse().ok())
            .or_else(|| serde_json::from_str::<Value>(text).ok().filter(Value::is_string).and_then(|value| time_ms(&value))),
        _ => None,
    }
}

pub(super) fn open(tx: &Transaction<'_>) -> Result<()> {
    tx.execute_batch(/* sqlite-large-value: allow-for-now lv-h346f4a67bec1 owner=fractal_store::store::objective_series TODO=trace-statement-and-value-representation */ SCHEMA)?;
    let initialized: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM meta WHERE key='local_objective_times_v1')", [], |row| row.get(0))?;
    if !initialized {
        // Once at startup, not on a series request. The transaction also installs capture.
        tx.execute(/* sqlite-large-value: allow-for-now lv-hc4f3532cf370 owner=fractal_store::store::objective_series TODO=prepare-body-and-predecessor-values */ "INSERT OR IGNORE INTO local_objective_times_pending_v1 SELECT store_index FROM claims", [])?;
        flush(tx)?;
        tx.execute(/* sqlite-large-value: allow-for-now lv-h747854030285 owner=fractal_store::store::objective_series TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta(key,value) VALUES('local_objective_times_v1','ready')", [])?;
    }
    flush(tx)
}

pub(super) fn flush(tx: &Transaction<'_>) -> Result<()> {
    loop {
        let indexes = tx.prepare_cached("SELECT store_index FROM local_objective_times_pending_v1 ORDER BY store_index LIMIT 1000")?
            .query_map([], |row| row.get::<_, u64>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        if indexes.is_empty() { return Ok(()); }
        for index in indexes {
            let claim = tx.query_row("SELECT id,store_index,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms FROM claims WHERE store_index=?1", [index], claim_from_row).optional()?;
            tx.execute(/* sqlite-large-value: allow-for-now lv-h9982846b0d0e owner=fractal_store::store::objective_series TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_objective_times_v1 WHERE store_index=?1", [index])?;
            if let Some(claim) = claim {
                let accepted = u64::try_from(claim.accepted_at_unix_ms).unwrap_or(u64::MAX);
                insert(tx, &claim, "accepted", accepted)?;
                if let Some(fields) = claim.body.get("fields").and_then(Value::as_object) {
                    for (field, value) in fields {
                        if let Some(at) = time_ms(value) { insert(tx, &claim, &format!("field:{field}"), at)?; }
                    }
                }
            }
            tx.execute(/* sqlite-large-value: allow-for-now lv-h9f1ce24f4131 owner=fractal_store::store::objective_series TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_objective_times_pending_v1 WHERE store_index=?1", [index])?;
        }
    }
}

fn insert(tx: &Transaction<'_>, claim: &ClaimRecord, field: &str, at: u64) -> Result<()> {
    tx.execute(/* sqlite-large-value: allow-for-now lv-hee8c9060116b owner=fractal_store::store::objective_series TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO local_objective_times_v1(store_index,field,kind,at,subject) VALUES(?1,?2,?3,?4,?5)",
        params![claim.store_index, field, claim.kind, at.to_be_bytes().as_slice(), claim.subject])?;
    Ok(())
}

const CANDIDATES: &str = "SELECT DISTINCT subject FROM local_objective_times_v1 INDEXED BY local_objective_times_seek_v1 WHERE kind=?1 AND field=?2 AND at>=?3 AND at<?4 AND subject>=?5 AND subject<?6";
const CONTEXT: &str = "SELECT id,store_index,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms FROM claims INDEXED BY claims_subject_kind_index WHERE subject=?1 AND kind=?2 ORDER BY store_index";
const RAW_CANDIDATES: &str = "SELECT store_index FROM local_objective_times_v1 INDEXED BY local_objective_times_seek_v1 WHERE kind=?1 AND field=?2 AND at>=?3 AND at<?4 AND (?5 IS NULL OR substr(subject,1,length(?5))=?5)";
const CLAIM_AT_INDEX: &str = "SELECT id,store_index,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms FROM claims WHERE store_index=?1";

impl Store {
    /// Seek timestamp candidates and fetch each physical claim once, without loading other
    /// claims of its subject. The caller resolves timestamp precedence from the original body.
    pub fn series_claims(&self, kinds: &[String], prefix: Option<&str>, from: u64, to: u64) -> Result<Vec<ClaimRecord>> {
        let db = self.readers.get();
        let tx = db.unchecked_transaction()?;
        let mut indexes = BTreeSet::new();
        let kinds: BTreeSet<&str> = kinds.iter().map(String::as_str).collect();
        for kind in kinds {
            for field in ["field:at", "field:occurred_at_unix_ms", "field:occurred_at", "accepted"] {
                let mut statement = tx.prepare_cached(/* sqlite-large-value: allow-for-now lv-h3783e61dee21 owner=fractal_store::store::objective_series TODO=trace-statement-and-value-representation */ RAW_CANDIDATES)?;
                for row in statement.query_map(params![kind, field, from.to_be_bytes().as_slice(), to.to_be_bytes().as_slice(), prefix], |row| row.get::<_, u64>(0))? {
                    indexes.insert(row?);
                }
            }
        }
        let mut statement = tx.prepare_cached(/* sqlite-large-value: allow-for-now lv-ha6520970df6e owner=fractal_store::store::objective_series TODO=trace-statement-and-value-representation */ CLAIM_AT_INDEX)?;
        indexes.into_iter().map(|index| statement.query_row(/* sqlite-large-value: allow-for-now lv-h09a0333577a9 owner=fractal_store::store::objective_series TODO=trace-statement-and-value-representation */ [index], claim_from_row).map_err(Into::into)).collect()
    }

    /// Seek event timestamps, then load only matching subjects' relevant kinds. Full subject
    /// context is essential: starts, retries, cancellations and expect changes can lie outside
    /// the requested range. The evaluator, not this index, selects the resulting event/sample.
    pub fn objective_series_claims(&self, events: &[(&str, Option<&str>)], kinds: &[&str], prefix: &str, from: u64, to: u64) -> Result<Vec<ClaimRecord>> {
        let upper = prefix_upper_bound(prefix).ok_or_else(|| anyhow::anyhow!("invalid objective subject prefix"))?;
        let db = self.readers.get();
        // A single read transaction keeps candidates and their context at the same snapshot.
        let tx = db.unchecked_transaction()?;
        let mut subjects = BTreeSet::new();
        for (kind, field) in events {
            let field = field.map_or_else(|| "accepted".to_owned(), |field| format!("field:{field}"));
            subjects.extend(tx.prepare_cached(/* sqlite-large-value: allow-for-now lv-he1f5fdf7608d owner=fractal_store::store::objective_series TODO=trace-statement-and-value-representation */ CANDIDATES)?.query_map(params![kind, field, from.to_be_bytes().as_slice(), to.to_be_bytes().as_slice(), prefix, upper], |row| row.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?);
        }
        let mut claims = Vec::new();
        for subject in subjects {
            for kind in kinds {
                claims.extend(tx.prepare_cached(/* sqlite-large-value: allow-for-now lv-hb026bfd6774d owner=fractal_store::store::objective_series TODO=trace-statement-and-value-representation */ CONTEXT)?.query_map(params![subject, kind], claim_from_row)?.collect::<rusqlite::Result<Vec<_>>>()?);
            }
        }
        Ok(claims)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_and_context_queries_seek_bounded_indexes() {
        let store = Store::open_memory("node").unwrap();
        let db = store.readers.get();
        for (sql, expected) in [
            (CANDIDATES, "USING COVERING INDEX local_objective_times_seek_v1 (kind=? AND field=? AND at>? AND at<?)"),
            (RAW_CANDIDATES, "USING COVERING INDEX local_objective_times_seek_v1 (kind=? AND field=? AND at>? AND at<?)"),
            (CONTEXT, "USING INDEX claims_subject_kind_index (subject=? AND kind=?)"),
            (CLAIM_AT_INDEX, "USING INTEGER PRIMARY KEY (rowid=?)"),
        ] {
            let mut statement = db.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
            let nulls = vec![None::<String>; statement.parameter_count()];
            let plan = statement.query_map(rusqlite::params_from_iter(nulls), |row| row.get::<_, String>(3)).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap().join("; ");
            assert!(plan.contains(expected), "{plan}");
            assert!(!plan.contains("SCAN claims"), "{plan}");
        }
    }

    fn append(store: &Store, fields: Value) -> ClaimRecord {
        store.append_claim(&ClaimInput {
            subject: "custom/acme/one".into(), kind: "custom.acme.event".into(), actor: None,
            fields: fields.as_object().unwrap().clone().into_iter().collect(), evidence: vec![],
            expected_subject: None, idempotency_key: None,
        }).unwrap()
    }

    #[test]
    fn startup_backfills_old_claims_once_without_rewriting_timestamp_fields() {
        let store = Store::open_memory("node").unwrap();
        let fields = json!({"at":1000,"numeric":1000,"text":"1000","rfc":"1970-01-01T00:00:01Z","encoded":"\"1970-01-01T00:00:01Z\"","bad":"yesterday","max":u64::MAX});
        let claim = append(&store, fields);
        {
            let mut db = store.connection.write();
            let tx = db.transaction().unwrap();
            tx.execute_batch("DROP TRIGGER local_objective_times_insert_v1;
                DROP TRIGGER local_objective_times_delete_v1;
                DROP TRIGGER local_objective_times_update_v1;
                DROP TABLE local_objective_times_v1;
                DROP TABLE local_objective_times_pending_v1;
                DELETE FROM meta WHERE key='local_objective_times_v1';").unwrap();
            open(&tx).unwrap();
            open(&tx).unwrap();
            let count: u64 = tx.query_row("SELECT COUNT(*) FROM local_objective_times_v1 WHERE store_index=?1", [claim.store_index], |row| row.get(0)).unwrap();
            assert_eq!(count, 7, "acceptance plus six readable timestamp fields");
            let body: String = tx.query_row("SELECT body FROM claims WHERE store_index=?1", [claim.store_index], |row| row.get(0)).unwrap();
            assert_eq!(serde_json::from_str::<Value>(&body).unwrap(), claim.body);
            tx.commit().unwrap();
        }
        for field in ["numeric", "text", "rfc", "encoded"] {
            let claims = store.objective_series_claims(&[("custom.acme.event", Some(field))], &["custom.acme.event"], "custom/acme/", 1000, 1001).unwrap();
            assert_eq!(claims.len(), 1, "{field}");
            assert_eq!(claims[0].body, claim.body);
        }
        let raw = store.series_claims(&["custom.acme.event".into()], Some("custom/acme/"), 1000, 1001).unwrap();
        assert_eq!(raw.len(), 1);
        assert_eq!(raw[0].body, claim.body);
        assert!(claim.accepted_at_unix_ms > 1001);
        let db = store.readers.get();
        let max: Vec<u8> = db.query_row("SELECT at FROM local_objective_times_v1 WHERE store_index=?1 AND field='field:max'", [claim.store_index], |row| row.get(0)).unwrap();
        assert_eq!(max, u64::MAX.to_be_bytes());
    }

    #[test]
    fn pending_capture_tracks_replication_and_corrections() {
        let source = Store::open_memory("source").unwrap();
        let target = Store::open_memory("target").unwrap();
        let claim = append(&source, json!({"at":1000}));
        super::super::glasses_tests_shared_delivery::sync(&source, &target);
        let read = |store: &Store, from, to| store.objective_series_claims(&[("custom.acme.event", Some("at"))], &["custom.acme.event"], "custom/acme/", from, to).unwrap();
        assert_eq!(read(&source, 1000, 2000).len(), 1);
        assert_eq!(read(&target, 1000, 2000).len(), 1);
        let mut db = target.connection.write();
        let tx = db.transaction().unwrap();
        tx.execute("UPDATE claims SET body=json_set(body,'$.fields.at',3000) WHERE id=?1", [&claim.id]).unwrap();
        flush(&tx).unwrap();
        tx.commit().unwrap();
        drop(db);
        assert!(read(&target, 1000, 2000).is_empty());
        assert_eq!(read(&target, 3000, 4000).len(), 1);
    }
}
