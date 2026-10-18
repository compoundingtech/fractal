//! Local, rebuildable launch evidence. Source triggers record exact deltas; a native
//! transaction finalizer derives canonical keys after all admission metadata is present.
use super::*;

pub const PAGE: usize = 64;
const SCHEMA: &str = /* sqlite-large-value: allow-for-now lv-h35d4b93324cd owner=st3_store::store::publication_facts TODO=chunk-or-reference-variable-columns */ r#"
CREATE TABLE IF NOT EXISTS local_publication_claims (
    claim_id TEXT PRIMARY KEY,
    subject TEXT NOT NULL,
    origin TEXT NOT NULL,
    kind TEXT NOT NULL,
    incarnation TEXT NOT NULL,
    sort_key BLOB NOT NULL
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS local_publication_claims_lookup
ON local_publication_claims(subject,kind,origin,incarnation,sort_key);
CREATE INDEX IF NOT EXISTS local_publication_runtime_lookup
ON local_publication_claims(subject,kind,origin,sort_key);
CREATE TABLE IF NOT EXISTS local_publication_pending (claim_id TEXT PRIMARY KEY) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS local_publication_batches (batch_id TEXT PRIMARY KEY, cursor INTEGER NOT NULL DEFAULT 0) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS local_publication_starts (
    observation_id INTEGER PRIMARY KEY,
    subject TEXT NOT NULL,
    incarnation TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS local_publication_starts_lookup
ON local_publication_starts(subject,incarnation,observation_id);
CREATE TABLE IF NOT EXISTS local_publication_bootstrap (
    singleton INTEGER PRIMARY KEY CHECK(singleton=1),
    phase INTEGER NOT NULL,
    cursor INTEGER NOT NULL,
    ceiling INTEGER NOT NULL,
    local_subject TEXT NOT NULL DEFAULT '',
    local_kind TEXT NOT NULL DEFAULT '',
    local_id INTEGER NOT NULL DEFAULT 0,
    local_ceiling INTEGER NOT NULL,
    progress_ms TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS local_publication_launch_epoch (
    singleton INTEGER PRIMARY KEY CHECK(singleton=1),
    generation INTEGER NOT NULL
);
INSERT OR IGNORE INTO local_publication_launch_epoch VALUES(1,0);
CREATE TRIGGER IF NOT EXISTS publication_launch_insert AFTER INSERT ON local_publication_claims
WHEN NEW.kind='runtime.action.succeeded' BEGIN
    UPDATE local_publication_launch_epoch SET generation=generation+1;
END;
CREATE TRIGGER IF NOT EXISTS publication_launch_delete AFTER DELETE ON local_publication_claims
WHEN OLD.kind='runtime.action.succeeded' BEGIN
    UPDATE local_publication_launch_epoch SET generation=generation+1;
END;
CREATE TRIGGER IF NOT EXISTS publication_launch_update AFTER UPDATE ON local_publication_claims
WHEN OLD.kind='runtime.action.succeeded' OR NEW.kind='runtime.action.succeeded' BEGIN
    UPDATE local_publication_launch_epoch SET generation=generation+1;
END;
CREATE TRIGGER IF NOT EXISTS publication_local_launch_insert AFTER INSERT ON local_publication_starts BEGIN
    UPDATE local_publication_launch_epoch SET generation=generation+1;
END;
CREATE TRIGGER IF NOT EXISTS publication_local_launch_delete AFTER DELETE ON local_publication_starts BEGIN
    UPDATE local_publication_launch_epoch SET generation=generation+1;
END;
CREATE TRIGGER IF NOT EXISTS publication_local_launch_update AFTER UPDATE ON local_publication_starts BEGIN
    UPDATE local_publication_launch_epoch SET generation=generation+1;
END;
CREATE TRIGGER IF NOT EXISTS publication_launch_definition_delete AFTER DELETE ON claims
WHEN OLD.kind='intent.desired' BEGIN
    UPDATE local_publication_launch_epoch SET generation=generation+1;
END;
CREATE TRIGGER IF NOT EXISTS publication_launch_definition_update
AFTER UPDATE OF subject,kind,body ON claims WHEN OLD.kind='intent.desired' BEGIN
    UPDATE local_publication_launch_epoch SET generation=generation+1;
END;
CREATE TRIGGER IF NOT EXISTS publication_claim_insert AFTER INSERT ON claims
WHEN NEW.kind IN ('runtime.observed','runtime.action.succeeded') BEGIN
    INSERT OR IGNORE INTO local_publication_pending VALUES(NEW.id);
END;
CREATE TRIGGER IF NOT EXISTS publication_claim_delete AFTER DELETE ON claims BEGIN
    DELETE FROM local_publication_claims WHERE claim_id=OLD.id;
    DELETE FROM local_publication_pending WHERE claim_id=OLD.id;
    INSERT INTO local_publication_batches
      SELECT OLD.batch_id,0 WHERE EXISTS(SELECT 1 FROM claims INDEXED BY claims_batch_index WHERE batch_id=OLD.batch_id LIMIT 1)
      ON CONFLICT(batch_id) DO UPDATE SET cursor=0;
END;
CREATE TRIGGER IF NOT EXISTS publication_claim_update
AFTER UPDATE OF subject,kind,origin,body,accepted_at_unix_ms,batch_id,store_index ON claims
WHEN OLD.kind IN ('runtime.observed','runtime.action.succeeded')
  OR NEW.kind IN ('runtime.observed','runtime.action.succeeded') BEGIN
    DELETE FROM local_publication_claims WHERE claim_id=OLD.id;
    INSERT OR IGNORE INTO local_publication_pending VALUES(NEW.id);
END;
CREATE TRIGGER IF NOT EXISTS publication_local_insert AFTER INSERT ON local_observations
WHEN NEW.kind='runtime.action.succeeded' AND json_extract(NEW.body,'$.fields.action')='start' BEGIN
    INSERT OR REPLACE INTO local_publication_starts VALUES
        (NEW.id,NEW.subject,COALESCE(json_extract(NEW.body,'$.fields.incarnation_id'),''));
END;
CREATE TRIGGER IF NOT EXISTS publication_local_delete AFTER DELETE ON local_observations BEGIN
    DELETE FROM local_publication_starts WHERE observation_id=OLD.id;
END;
CREATE TRIGGER IF NOT EXISTS publication_local_update
AFTER UPDATE OF subject,kind,body ON local_observations BEGIN
    DELETE FROM local_publication_starts WHERE observation_id=OLD.id;
    INSERT INTO local_publication_starts
    SELECT NEW.id,NEW.subject,COALESCE(json_extract(NEW.body,'$.fields.incarnation_id'),'')
    WHERE NEW.kind='runtime.action.succeeded' AND json_extract(NEW.body,'$.fields.action')='start';
END;
CREATE TRIGGER IF NOT EXISTS publication_batch_update
AFTER UPDATE OF origin,replica_sequence ON batches BEGIN
    INSERT OR IGNORE INTO local_publication_pending
    SELECT id FROM claims INDEXED BY claims_batch_index WHERE batch_id=NEW.id
      AND kind IN ('runtime.observed','runtime.action.succeeded');
END;
CREATE TRIGGER IF NOT EXISTS publication_record_insert AFTER INSERT ON replica_records BEGIN
    INSERT OR IGNORE INTO local_publication_pending SELECT id FROM claims WHERE id=NEW.claim_id
      AND kind IN ('runtime.observed','runtime.action.succeeded');
END;
CREATE TRIGGER IF NOT EXISTS publication_record_delete AFTER DELETE ON replica_records BEGIN
    INSERT OR IGNORE INTO local_publication_pending SELECT id FROM claims WHERE id=OLD.claim_id
      AND kind IN ('runtime.observed','runtime.action.succeeded');
END;
CREATE TRIGGER IF NOT EXISTS publication_record_update
AFTER UPDATE OF claim_id,position,state ON replica_records BEGIN
    INSERT OR IGNORE INTO local_publication_pending SELECT id FROM claims WHERE id IN (OLD.claim_id,NEW.claim_id)
      AND kind IN ('runtime.observed','runtime.action.succeeded');
END;

"#;

const REPAIR_SCHEMA: &str = /* sqlite-large-value: allow-for-now lv-h275b422fa86b owner=st3_store::store::publication_facts TODO=prepare-body-and-predecessor-values */ r#"
CREATE TRIGGER IF NOT EXISTS publication_repair_insert AFTER INSERT ON projection_digest_repaired_claims BEGIN
    DELETE FROM local_publication_claims WHERE claim_id=NEW.id;
    DELETE FROM local_publication_pending WHERE claim_id=NEW.id;
    UPDATE local_publication_launch_epoch SET generation=generation+1
    WHERE EXISTS(SELECT 1 FROM claims WHERE id=NEW.id AND kind='intent.desired');
END;
CREATE TRIGGER IF NOT EXISTS publication_repair_delete AFTER DELETE ON projection_digest_repaired_claims BEGIN
    INSERT OR IGNORE INTO local_publication_pending SELECT id FROM claims WHERE id=OLD.id
      AND kind IN ('runtime.observed','runtime.action.succeeded');
    UPDATE local_publication_launch_epoch SET generation=generation+1
    WHERE EXISTS(SELECT 1 FROM claims WHERE id=OLD.id AND kind='intent.desired');
END;
"#;

pub fn create_schema(connection: &Connection) -> Result<()> {
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-h5370042848e7 owner=st3_store::store::publication_facts TODO=trace-statement-and-value-representation */ SCHEMA)?;
    let repairs:bool=connection.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='projection_digest_repaired_claims' AND type='table')",[],|row|row.get(0))?;
    if repairs {
        connection.execute_batch(/* sqlite-large-value: allow-for-now lv-hccbc6149f9cb owner=st3_store::store::publication_facts TODO=trace-statement-and-value-representation */ REPAIR_SCHEMA)?;
    }
    Ok(())
}

/// Publish a process-lifetime cache invalidation hint after native commit resolution.
/// This is not authority for an effect. The publication writer reads its own evidence;
/// physical-effect authorization must independently fence the current declaration. Read failure invalidates every cached proof.
pub fn install_cache_observer(graph: &GraphStore, runtime: &Arc<SmalltalkRuntime>) {
    let weak = Arc::downgrade(runtime);
    let observer = graph.connection.observe_commits(move |connection| {
        if let Some(runtime) = weak.upgrade() {
            *runtime
                .publication_cache_epoch
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = cache_epoch(connection).ok();
        }
    });
    *runtime
        .publication_cache_observer
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = Some(observer);
    // Registration precedes the fresh writer cut, so no intervening commit can be missed.
    let connection = graph.connection.write();
    *runtime
        .publication_cache_epoch
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = cache_epoch(&connection).ok();
}

fn cache_epoch(connection: &Connection) -> Result<(i64, bool)> {
    Ok(connection.prepare_cached(
        "SELECT generation,COALESCE((SELECT phase=3 FROM local_publication_bootstrap WHERE singleton=1),0)
            AND NOT EXISTS(SELECT 1 FROM local_publication_pending)
            AND NOT EXISTS(SELECT 1 FROM local_publication_batches)
         FROM local_publication_launch_epoch WHERE singleton=1"
    )?.query_row(/* sqlite-large-value: allow-for-now lv-hde3bcca96a77 owner=st3_store::store::publication_facts TODO=trace-statement-and-value-representation */ [], |row| Ok((row.get(0)?,row.get(1)?)))?)
}

/// Indexed ceilings only. Never reads historical bodies or advances a bootstrap page in open.
pub fn initialize(transaction: &Transaction<'_>) -> Result<()> {
    transaction.execute(
        /* sqlite-large-value: allow-for-now lv-h699e40ec6bf4 owner=st3_store::store::publication_facts TODO=prepare-body-and-predecessor-values */ "INSERT OR IGNORE INTO local_publication_bootstrap(singleton,phase,cursor,ceiling,local_ceiling,progress_ms)
         SELECT 1,CASE WHEN shared=0 AND local=0 THEN 3 ELSE 0 END,0,shared,local,?1 FROM
         (SELECT COALESCE((SELECT MAX(store_index) FROM claims),0) AS shared,
                 COALESCE((SELECT MAX(id) FROM local_observations),0) AS local)",
        [now_ms().to_string()],
    )?;
    Ok(())
}

pub fn sync(transaction: &Transaction<'_>, id: &str) -> Result<()> {
    sync_from_source(transaction, transaction, id)
}

fn sync_from_source(transaction: &Transaction<'_>, source: &Connection, id: &str) -> Result<()> {
    let query = format!(
        "SELECT subject,origin,kind,body FROM claims
         WHERE id=?1 AND kind IN ('runtime.observed','runtime.action.succeeded') AND {UNREPAIRED_CLAIM}"
    );
    let row: Option<(String, String, String, String)> = source
        .prepare_cached(/* sqlite-large-value: allow-for-now lv-hd7e991aefb8c owner=st3_store::store::publication_facts TODO=trace-statement-and-value-representation */ &query)?
        .query_row(/* sqlite-large-value: allow-for-now lv-he8c2bb53ed28 owner=st3_store::store::publication_facts TODO=trace-statement-and-value-representation */ [id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .optional()?;
    let Some((subject, origin, kind, body)) = row else {
        transaction.execute(
            /* sqlite-large-value: allow-for-now lv-hf398015c9644 owner=st3_store::store::publication_facts TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_publication_claims WHERE claim_id=?1",
            [id],
        )?;
        return Ok(());
    };
    let body: Value = serde_json::from_str(&body)?;
    let fields = body.get("fields").unwrap_or(&body);
    if kind == "runtime.action.succeeded" && fields["action"] != "start" {
        transaction.execute(
            /* sqlite-large-value: allow-for-now lv-hefd7f488cb7a owner=st3_store::store::publication_facts TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_publication_claims WHERE claim_id=?1",
            [id],
        )?;
        return Ok(());
    }
    let key = canonical::sortable_key(&canonical::claim_key(source, id)?);
    transaction.execute(
        /* sqlite-large-value: allow-for-now lv-h5bc391c9e173 owner=st3_store::store::publication_facts TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO local_publication_claims VALUES(?1,?2,?3,?4,?5,?6)
         ON CONFLICT(claim_id) DO UPDATE SET subject=excluded.subject,origin=excluded.origin,
             kind=excluded.kind,incarnation=excluded.incarnation,sort_key=excluded.sort_key
         WHERE local_publication_claims.subject<>excluded.subject
            OR local_publication_claims.origin<>excluded.origin
            OR local_publication_claims.kind<>excluded.kind
            OR local_publication_claims.incarnation<>excluded.incarnation
            OR local_publication_claims.sort_key<>excluded.sort_key",
        params![
            id,
            subject,
            origin,
            kind,
            fields["incarnation_id"].as_str().unwrap_or_default(),
            key
        ],
    )?;
    Ok(())
}

/// Bound every writer loan. A larger admission leaves a pending queue and Unknown;
/// the commit-driven background worker finishes it without a history fold or polling.
pub fn flush(transaction: &Transaction<'_>) -> Result<()> {
    let started = std::time::Instant::now();
    // Deleting a legacy batch member changes later fallback canonical positions.
    // Coalesce that invalidation per batch, then repair it in bounded durable pages.
    // Never rescan the remaining batch in each row's delete trigger.
    let batch: Option<(String, i64)> = transaction.prepare_cached(
        "SELECT batch_id,cursor FROM local_publication_batches WHERE batch_id>='' ORDER BY batch_id LIMIT 1"
    )?.query_row(/* sqlite-large-value: allow-for-now lv-hb5b48b19bcd3 owner=st3_store::store::publication_facts TODO=trace-statement-and-value-representation */ [], |row| Ok((row.get(0)?,row.get(1)?))).optional()?;
    if let Some((batch_id, cursor)) = batch {
        let rows = transaction
            .prepare_cached(
                "SELECT store_index,id,kind FROM claims INDEXED BY claims_batch_index
             WHERE batch_id=?1 AND store_index>?2 ORDER BY store_index LIMIT ?3",
            )?
            .query_map(params![batch_id, cursor, PAGE], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut through = cursor;
        let mut processed = 0;
        for (index, id, kind) in &rows {
            if processed > 0 && started.elapsed() >= std::time::Duration::from_millis(10) {
                break;
            }
            if matches!(
                kind.as_str(),
                "runtime.observed" | "runtime.action.succeeded"
            ) {
                sync(transaction, id)?;
            }
            through = *index;
            processed += 1;
        }
        if processed == rows.len() && rows.len() < PAGE {
            transaction.execute(
                /* sqlite-large-value: allow-for-now lv-ha920539459ec owner=st3_store::store::publication_facts TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_publication_batches WHERE batch_id=?1",
                [&batch_id],
            )?;
        } else {
            transaction.execute(
                /* sqlite-large-value: allow-for-now lv-h87ce8e47b695 owner=st3_store::store::publication_facts TODO=bound-or-reference-values-and-reclamation */ "UPDATE local_publication_batches SET cursor=?2 WHERE batch_id=?1",
                params![batch_id, through],
            )?;
        }
        return Ok(());
    }
    let pending = transaction
        .prepare_cached("SELECT claim_id FROM local_publication_pending WHERE claim_id>='' ORDER BY claim_id LIMIT ?1")?
        .query_map([PAGE], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (offset, id) in pending.iter().enumerate() {
        if offset > 0 && started.elapsed() >= std::time::Duration::from_millis(10) {
            break;
        }
        sync(transaction, id)?;
        transaction.execute(
            /* sqlite-large-value: allow-for-now lv-hc771fe02f20f owner=st3_store::store::publication_facts TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_publication_pending WHERE claim_id=?1",
            [id],
        )?;
    }
    Ok(())
}

pub fn ready(connection: &Connection) -> Result<bool> {
    Ok(connection
        .prepare_cached(
            "SELECT COALESCE((SELECT phase=3 FROM local_publication_bootstrap WHERE singleton=1),0)
            AND NOT EXISTS(SELECT 1 FROM local_publication_pending)
            AND NOT EXISTS(SELECT 1 FROM local_publication_batches)",
        )?
        .query_row(/* sqlite-large-value: allow-for-now lv-hfa8f38b908ef owner=st3_store::store::publication_facts TODO=trace-statement-and-value-representation */ [], |row| row.get(0))?)
}

pub fn backfill(transaction: &Transaction<'_>) -> Result<()> {
    let started = std::time::Instant::now();
    let (phase, cursor, ceiling, subject, kind, local_id, local_ceiling): (
        u8,
        u64,
        u64,
        String,
        String,
        i64,
        i64,
    ) = transaction.query_row(
        "SELECT phase,cursor,ceiling,local_subject,local_kind,local_id,local_ceiling
         FROM local_publication_bootstrap WHERE singleton=1",
        [],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
            ))
        },
    )?;
    if phase < 2 {
        let source_kind = if phase == 0 {
            "runtime.observed"
        } else {
            "runtime.action.succeeded"
        };
        let batch = transaction
            .prepare_cached(
                "SELECT store_index,id FROM claims INDEXED BY claims_kind_index
             WHERE kind=?1 AND store_index>?2 AND store_index<=?3 ORDER BY store_index LIMIT ?4",
            )?
            .query_map(params![source_kind, cursor, ceiling, PAGE], |row| {
                Ok((row.get::<_, u64>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut through = cursor;
        let mut processed = 0;
        for (index, id) in &batch {
            if processed > 0 && started.elapsed() >= std::time::Duration::from_millis(10) {
                break;
            }
            sync(transaction, id)?;
            through = *index;
            processed += 1;
        }
        let next = processed == batch.len() && batch.len() < PAGE;
        transaction.execute(/* sqlite-large-value: allow-for-now lv-h723c5ee91890 owner=st3_store::store::publication_facts TODO=bound-or-reference-values-and-reclamation */ "UPDATE local_publication_bootstrap SET phase=?1,cursor=?2,progress_ms=?3 WHERE singleton=1",
            params![phase+u8::from(next),if next {0}else{through},now_ms().to_string()])?;
    } else if phase == 2 {
        // There is no kind-leading local index. Walk its existing covering index in
        // bounded pages; fetch bodies only for action-success rows, never other kinds.
        let batch=transaction.prepare_cached(
            "SELECT subject,kind,id FROM local_observations INDEXED BY local_observations_subject_kind_index
             WHERE (subject,kind,id)>(?1,?2,?3) ORDER BY subject,kind,id LIMIT ?4"
        )?.query_map(params![subject,kind,local_id,PAGE],|row|Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,i64>(2)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut through = (subject, kind, local_id);
        let mut processed = 0;
        for (subject, kind, id) in &batch {
            if processed > 0 && started.elapsed() >= std::time::Duration::from_millis(10) {
                break;
            }
            if kind == "runtime.action.succeeded" && *id <= local_ceiling {
                transaction.execute(
                    /* sqlite-large-value: allow-for-now lv-hd72f59ea913a owner=st3_store::store::publication_facts TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO local_publication_starts
                     SELECT id,subject,COALESCE(json_extract(body,'$.fields.incarnation_id'),'')
                     FROM local_observations WHERE id=?1 AND json_extract(body,'$.fields.action')='start'",[id])?;
            }
            through = (subject.clone(), kind.clone(), *id);
            processed += 1;
        }
        transaction.execute(/* sqlite-large-value: allow-for-now lv-h723c22ad62de owner=st3_store::store::publication_facts TODO=bound-or-reference-values-and-reclamation */ "UPDATE local_publication_bootstrap SET phase=?1,local_subject=?2,local_kind=?3,local_id=?4,progress_ms=?5 WHERE singleton=1",
            params![if processed==batch.len() && batch.len()<PAGE {3}else{2},through.0,through.1,through.2,now_ms().to_string()])?;
    }
    Ok(())
}

pub fn reset(transaction: &Transaction<'_>) -> Result<()> {
    transaction.execute_batch(
        /* sqlite-large-value: allow-for-now lv-hb5f00b6bdc05 owner=st3_store::store::publication_facts TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_publication_claims;
        DELETE FROM local_publication_starts; DELETE FROM local_publication_pending; DELETE FROM local_publication_batches;
        DELETE FROM local_publication_bootstrap;",
    )?;
    initialize(transaction)
}

#[cfg(test)]
mod tests {
    #[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::publication_facts_tests_shared::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::publication_facts_tests_shared::{input};
use super::*;

    #[test]
    #[ignore = "frozen census URI required; writes only a new scratch file; prints aggregates"]
    fn publication_facts_census_source_bootstrap() {
        let uri = std::env::var("APPLY_RESTART_CENSUS_SOURCE")
            .expect("immutable read-only frozen census URI");
        assert!(uri.contains("immutable=1") && uri.contains("mode=ro"));
        let source = Connection::open_with_flags(
            uri,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
        )
        .unwrap();
        source.pragma_update(None, "query_only", true).unwrap();
        let root = tempfile::tempdir().unwrap();
        let mut scratch = Connection::open(root.path().join("facts.db")).unwrap();
        // Match the native writer's WAL/FULL settings; DELETE-journal commits do not
        // measure the supported bootstrap path's writer loans.
        scratch
            .execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;")
            .unwrap();
        scratch.execute_batch("CREATE TABLE claims(id TEXT,subject TEXT,kind TEXT,origin TEXT,body TEXT,accepted_at_unix_ms TEXT,batch_id TEXT,store_index INTEGER);
            CREATE INDEX claims_batch_index ON claims(batch_id,store_index);
            CREATE TABLE batches(id TEXT,origin TEXT,replica_sequence INTEGER);
            CREATE TABLE replica_records(claim_id TEXT,position INTEGER,state TEXT);
            CREATE TABLE local_observations(id INTEGER,subject TEXT,kind TEXT,body TEXT);").unwrap();
        let started = std::time::Instant::now();
        create_schema(&scratch).unwrap();
        println!(
            "publication census source schema_seconds={}",
            started.elapsed().as_secs_f64()
        );
        for kind in ["runtime.observed", "runtime.action.succeeded"] {
            let started = std::time::Instant::now();
            let mut cursor = 0_u64;
            let mut count = 0;
            let mut holds = Vec::new();
            loop {
                let batch = source
                    .prepare_cached(
                        "SELECT store_index,id FROM claims INDEXED BY claims_kind_index
                    WHERE kind=?1 AND store_index>?2 ORDER BY store_index LIMIT ?3",
                    )
                    .unwrap()
                    .query_map(params![kind, cursor, PAGE], |row| {
                        Ok((row.get::<_, u64>(0)?, row.get::<_, String>(1)?))
                    })
                    .unwrap()
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .unwrap();
                if batch.is_empty() {
                    break;
                }
                let page = std::time::Instant::now();
                let tx = scratch.transaction().unwrap();
                for (processed, (index, id)) in batch.iter().enumerate() {
                    if processed > 0 && page.elapsed() >= std::time::Duration::from_millis(10) {
                        break;
                    }
                    sync_from_source(&tx, &source, id).unwrap();
                    cursor = *index;
                    count += 1;
                }
                tx.commit().unwrap();
                holds.push(page.elapsed().as_micros());
            }
            holds.sort();
            println!(
                "publication census source kind={kind} indexed_source_rows={count} elapsed_seconds={} pages={} writer_hold_p95_us={} writer_hold_max_us={}",
                started.elapsed().as_secs_f64(),
                holds.len(),
                holds[holds.len() * 95 / 100],
                holds.last().unwrap()
            );
        }
        let version: String = source
            .query_row("SELECT sqlite_version()", [], |row| row.get(0))
            .unwrap();
        let facts: u64 = scratch
            .query_row("SELECT count(*) FROM local_publication_claims", [], |row| {
                row.get(0)
            })
            .unwrap();
        println!("publication census source SQLite={version} retained_shared_facts={facts}");
    }

    #[test]
    #[ignore = "private census scratch path required; run alone and print aggregates only"]
    fn publication_facts_census_bootstrap() {
        let path = std::env::var_os("APPLY_RESTART_CENSUS_SCRATCH")
            .expect("private matching-row scratch path");
        // Repeated probes must start from source rows, not a previously completed cursor.
        // Copy only the matching-row fixture, retaining its existing source indexes.
        let root = tempfile::tempdir().unwrap();
        let mut connection = Connection::open(root.path().join("facts.db")).unwrap();
        connection
            .execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;")
            .unwrap();
        let uri = format!("file:{}?mode=ro", std::path::Path::new(&path).display());
        connection
            .execute("ATTACH DATABASE ?1 AS census", [uri])
            .unwrap();
        let copy_started = std::time::Instant::now();
        let schemas = connection.prepare("SELECT sql FROM census.sqlite_schema
            WHERE type IN ('table','index') AND sql IS NOT NULL AND tbl_name IN
                ('claims','batches','replica_records','local_observations','projection_digest_repaired_claims')
            ORDER BY type='index'").unwrap().query_map([],|row|row.get::<_,String>(0)).unwrap()
            .collect::<rusqlite::Result<Vec<_>>>().unwrap();
        for sql in schemas {
            connection.execute_batch(&sql).unwrap();
        }
        let tx = connection.transaction().unwrap();
        for table in [
            "claims",
            "batches",
            "replica_records",
            "local_observations",
            "projection_digest_repaired_claims",
        ] {
            tx.execute_batch(&format!(
                "INSERT INTO main.{table} SELECT * FROM census.{table}"
            ))
            .unwrap();
        }
        tx.commit().unwrap();
        connection.execute_batch("DETACH DATABASE census").unwrap();
        println!(
            "publication census matching_fixture_copy_seconds={}",
            copy_started.elapsed().as_secs_f64()
        );
        let source_version: String = connection
            .query_row("SELECT sqlite_version()", [], |row| row.get(0))
            .unwrap();
        let started = std::time::Instant::now();
        create_schema(&connection).unwrap();
        let schema_seconds = started.elapsed().as_secs_f64();
        let tx = connection.transaction().unwrap();
        initialize(&tx).unwrap();
        tx.commit().unwrap();
        let started = std::time::Instant::now();
        let mut pages = 0;
        let mut holds = Vec::new();
        loop {
            let page_started = std::time::Instant::now();
            let tx = connection.transaction().unwrap();
            backfill(&tx).unwrap();
            flush(&tx).unwrap();
            let done = ready(&tx).unwrap();
            tx.commit().unwrap();
            holds.push(page_started.elapsed().as_micros());
            pages += 1;
            if done {
                break;
            }
        }
        let total = started.elapsed().as_secs_f64();
        holds.sort();
        let shared: u64 = connection
            .query_row("SELECT count(*) FROM local_publication_claims", [], |row| {
                row.get(0)
            })
            .unwrap();
        let local: u64 = connection
            .query_row("SELECT count(*) FROM local_publication_starts", [], |row| {
                row.get(0)
            })
            .unwrap();
        println!(
            "publication census SQLite={source_version} schema_seconds={schema_seconds} bootstrap_seconds={total} pages={pages} shared_rows={shared} local_rows={local} writer_hold_p95_us={} writer_hold_max_us={}",
            holds[holds.len() * 95 / 100],
            holds.last().unwrap()
        );
        let bytes:u64=connection.query_row("SELECT sum(pgsize) FROM dbstat WHERE name IN
            ('local_publication_claims','local_publication_claims_lookup','local_publication_runtime_lookup',
             'local_publication_starts','local_publication_starts_lookup','local_publication_pending','local_publication_bootstrap','local_publication_launch_epoch','local_publication_batches')",[],|row|row.get(0)).unwrap();
        println!("publication census derived_bytes={bytes}");
    }

    #[test]
    #[ignore = "private native measurements: run alone with --exact --ignored --test-threads=1 --nocapture"]
    fn publication_facts_native_write_cost() {
        use smallclaims::sqlite::work;
        for after in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join("scratch.db");
            let store = if after {
                None
            } else {
                Some(
                    GraphStore::open(&path, "node", Arc::new(SmalltalkRuntime::default())).unwrap(),
                )
            };
            let revised = if after {
                Some(Store::open(&path, "node").unwrap())
            } else {
                None
            };
            let graph: &GraphStore = store
                .as_ref()
                .unwrap_or_else(|| &revised.as_ref().unwrap().graph);
            if !after {
                let names=graph.readers.get().prepare("SELECT name FROM sqlite_schema WHERE type='trigger' AND name LIKE 'publication_%'")
                    .unwrap().query_map([],|row|row.get::<_,String>(0)).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
                let connection = graph.connection.write();
                for name in names {
                    connection
                        .execute_batch(&format!("DROP TRIGGER {name}"))
                        .unwrap();
                }
            }
            for (kind, local) in [
                ("runtime.observed", false),
                ("runtime.action.succeeded", true),
                ("runtime.action.succeeded", false),
            ] {
                let mut elapsed = Vec::new();
                let mut total = work::SqliteWork::default();
                for i in 0..100 {
                    let input = input(kind, &format!("cost-{kind}-{local}-{i}"));
                    let mut connection = graph.connection.write();
                    let before = work::total();
                    let started = std::time::Instant::now();
                    let tx = connection.transaction().unwrap();
                    if local {
                        insert_local_observation_tx(&tx, "node", &input, now_ms()).unwrap();
                    } else {
                        append_claim_tx(
                            &tx,
                            "node",
                            &input.subject,
                            kind,
                            input.actor.as_deref(),
                            &json!({"fields":input.fields}),
                            &[],
                            None,
                        )
                        .unwrap();
                    }
                    tx.commit().unwrap();
                    drop(connection);
                    elapsed.push(started.elapsed().as_nanos());
                    let cost = work::total() - before;
                    total.vm_steps += cost.vm_steps;
                    total.statements += cost.statements;
                }
                elapsed.sort();
                println!(
                    "publication write after={after} kind={kind} local={local}: per insert vm_steps={} statements={} writer_hold_median_us={} writer_hold_p95_us={} writer_hold_max_us={}",
                    total.vm_steps / 100,
                    total.statements / 100,
                    elapsed[50] / 1000,
                    elapsed[95] / 1000,
                    elapsed[99] / 1000
                );
            }
        }
    }

}
