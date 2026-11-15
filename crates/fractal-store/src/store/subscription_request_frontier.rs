//! Rebuildable all-status relevant-change frontier. Finishing a request never removes its
//! fence: only a canonically newer snapshot can replace it. Reconciliation reads one indexed
//! row per subscription/resource, not the subscription's lifetime delivery history.
use super::*;
use smallclaims::store::canonical;

const RELEVANT: &str = "kind='subscription.mission-requested' AND json_extract(body,'$.fields.admission')='relevant-changes'";

pub fn open(tx: &Transaction<'_>) -> Result<()> {
    tx.execute_batch("
CREATE TABLE IF NOT EXISTS local_subscription_request_frontier (
 subject TEXT NOT NULL, resource TEXT NOT NULL, mission TEXT NOT NULL, scope TEXT NOT NULL,
 claim_id TEXT NOT NULL, head_key BLOB NOT NULL,
 PRIMARY KEY(subject,resource)
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS subscription_request_stable_frontier
 ON local_subscription_request_frontier(mission,scope,resource,head_key DESC);
CREATE TABLE IF NOT EXISTS local_subscription_request_pending (claim_id TEXT PRIMARY KEY) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS local_subscription_request_dirty (
 subject TEXT NOT NULL, resource TEXT NOT NULL, PRIMARY KEY(subject,resource)
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS claims_subscription_relevant_resource
 ON claims(subject,json_extract(body,'$.fields.resource'))
 WHERE kind='subscription.mission-requested' AND json_extract(body,'$.fields.admission')='relevant-changes';
CREATE INDEX IF NOT EXISTS claims_subscription_legacy_requests ON claims(subject)
 WHERE kind='subscription.mission-requested' AND json_extract(body,'$.fields.admission') IS NOT 'relevant-changes';
CREATE INDEX IF NOT EXISTS claims_subscription_request_closer
 ON claims(subject,json_extract(body,'$.fields.request'))
 WHERE kind IN ('subscription.mission-started','subscription.mission-failed','subscription.mission-request-cancelled');
CREATE TRIGGER IF NOT EXISTS subscription_request_frontier_insert AFTER INSERT ON claims
 WHEN NEW.kind='subscription.mission-requested' AND json_extract(NEW.body,'$.fields.admission')='relevant-changes' BEGIN
 INSERT OR IGNORE INTO local_subscription_request_pending VALUES(NEW.id);
 END;
")?;
    // Ordinary inserts take the constant-size pending path. Changes to ordering metadata or
    // removal are exceptional: replay only the affected resource from authoritative claims.
    for (name, event, selection) in [
        ("claim_update", "AFTER UPDATE OF id,subject,kind,body,batch_id,accepted_at_unix_ms,store_index ON claims", "SELECT OLD.subject,json_extract(OLD.body,'$.fields.resource') WHERE OLD.kind='subscription.mission-requested' AND json_extract(OLD.body,'$.fields.admission')='relevant-changes' UNION SELECT NEW.subject,json_extract(NEW.body,'$.fields.resource') WHERE NEW.kind='subscription.mission-requested' AND json_extract(NEW.body,'$.fields.admission')='relevant-changes'"),
        ("claim_delete", "AFTER DELETE ON claims", "SELECT OLD.subject,json_extract(OLD.body,'$.fields.resource') WHERE OLD.kind='subscription.mission-requested' AND json_extract(OLD.body,'$.fields.admission')='relevant-changes'"),
        ("position_insert", "AFTER INSERT ON claim_positions", "SELECT subject,json_extract(body,'$.fields.resource') FROM claims WHERE id=NEW.claim_id"),
        ("position_update", "AFTER UPDATE ON claim_positions", "SELECT subject,json_extract(body,'$.fields.resource') FROM claims WHERE id IN(OLD.claim_id,NEW.claim_id)"),
        ("position_delete", "AFTER DELETE ON claim_positions", "SELECT subject,json_extract(body,'$.fields.resource') FROM claims WHERE id=OLD.claim_id"),
        ("batch_update", "AFTER UPDATE OF origin,replica_sequence ON batches", "SELECT subject,json_extract(body,'$.fields.resource') FROM claims WHERE batch_id=NEW.id"),
        ("legacy_insert", "AFTER INSERT ON claims", "SELECT subject,json_extract(body,'$.fields.resource') FROM claims WHERE batch_id=NEW.batch_id AND store_index>NEW.store_index AND NOT EXISTS(SELECT 1 FROM claim_positions WHERE claim_id=claims.id)"),
        ("legacy_update", "AFTER UPDATE OF batch_id,store_index ON claims", "SELECT subject,json_extract(body,'$.fields.resource') FROM claims WHERE batch_id IN(OLD.batch_id,NEW.batch_id) AND NOT EXISTS(SELECT 1 FROM claim_positions WHERE claim_id=claims.id)"),
        ("legacy_delete", "AFTER DELETE ON claims", "SELECT subject,json_extract(body,'$.fields.resource') FROM claims WHERE batch_id=OLD.batch_id AND store_index>OLD.store_index AND NOT EXISTS(SELECT 1 FROM claim_positions WHERE claim_id=claims.id)"),
    ] {
        // Metadata triggers select existing relevant claims; claim triggers guard OLD and NEW.
        let filter = if name.starts_with("claim_") { String::new() } else { format!(" AND {RELEVANT}") };
        tx.execute_batch(&format!(
            "CREATE TRIGGER IF NOT EXISTS subscription_request_frontier_{name} {event} BEGIN
             INSERT OR IGNORE INTO local_subscription_request_dirty {selection}{filter}; END;"
        ))?;
    }
    let initialized: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM meta WHERE key='subscription_request_frontier_v1')", [], |row| row.get(0),
    )?;
    if initialized { flush(tx) } else { rebuild(tx) }
}

pub fn rebuild(tx: &Transaction<'_>) -> Result<()> {
    tx.execute_batch("DELETE FROM local_subscription_request_frontier;
        DELETE FROM local_subscription_request_dirty;
        DELETE FROM local_subscription_request_pending;")?;
    tx.execute(&format!("INSERT INTO local_subscription_request_pending SELECT id FROM claims WHERE {RELEVANT}"), [])?;
    flush(tx)?;
    tx.execute("INSERT OR REPLACE INTO meta(key,value) VALUES('subscription_request_frontier_v1','1')", [])?;
    Ok(())
}

pub fn flush(tx: &Transaction<'_>) -> Result<()> {
    let dirty = tx.prepare("SELECT subject,resource FROM local_subscription_request_dirty")?
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (subject, resource) in dirty {
        tx.execute("DELETE FROM local_subscription_request_frontier WHERE subject=?1 AND resource=?2", params![subject,resource])?;
        tx.execute(&format!("INSERT OR IGNORE INTO local_subscription_request_pending
            SELECT id FROM claims WHERE subject=?1 AND json_extract(body,'$.fields.resource')=?2 AND {RELEVANT}"), params![subject,resource])?;
    }
    let pending = tx.prepare("SELECT claims.id,claims.subject,json_extract(claims.body,'$.fields.resource'),
            json_extract(claims.body,'$.fields.mission')
        FROM local_subscription_request_pending pending JOIN claims ON claims.id=pending.claim_id
        WHERE claims.kind='subscription.mission-requested' AND json_extract(claims.body,'$.fields.admission')='relevant-changes'")?
        .query_map([], |row| Ok((row.get::<_, String>(0)?,row.get::<_, String>(1)?,row.get::<_, String>(2)?,row.get::<_, String>(3)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (id, subject, resource, mission) in pending {
        let key = canonical::sortable_key(&canonical::claim_key(tx, &id)?);
        let scope = subject.rsplit('/').next().unwrap_or(&subject);
        tx.execute("INSERT INTO local_subscription_request_frontier(subject,resource,mission,scope,claim_id,head_key)
            VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(subject,resource) DO UPDATE SET
            mission=excluded.mission,scope=excluded.scope,
            claim_id=excluded.claim_id,head_key=excluded.head_key WHERE excluded.head_key>head_key",
            params![subject,resource,mission,scope,id,key])?;
    }
    tx.execute_batch("DELETE FROM local_subscription_request_dirty; DELETE FROM local_subscription_request_pending;")?;
    Ok(())
}
