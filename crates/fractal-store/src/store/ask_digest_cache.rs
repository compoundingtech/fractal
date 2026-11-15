//! Local, rebuildable digest indexes. Claims remain the replicated authority; local arrival
//! order discovers late replicas without confusing their acceptance time with ingestion time.
use super::*;
use fractal_schema::notice;

pub(super) const PAGE: usize = 128;
// A fresh cursor replays immutable claims in bounded pages, including sources skipped by v1.
const CURSOR: &str = "ask_digest_claim_cursor_v2";
const SCHEMA: &str = /* sqlite-large-value: allow-for-now lv-h2edbd01f1dab owner=fractal_store::store::ask_digest_cache TODO=chunk-or-reference-variable-columns */ "
CREATE TABLE IF NOT EXISTS local_ask_digest_pending(
 resolution_id TEXT PRIMARY KEY, episode TEXT NOT NULL, target TEXT NOT NULL,
 person TEXT, resolved_at INTEGER NOT NULL, staged_source TEXT, staged_owner TEXT);
CREATE INDEX IF NOT EXISTS local_ask_digest_due ON local_ask_digest_pending(person,resolved_at,resolution_id) WHERE staged_source IS NULL;
CREATE INDEX IF NOT EXISTS local_ask_digest_episode ON local_ask_digest_pending(episode);
CREATE INDEX IF NOT EXISTS local_ask_digest_staged_source ON local_ask_digest_pending(staged_source,resolution_id) WHERE staged_source IS NOT NULL;
CREATE INDEX IF NOT EXISTS local_ask_digest_staged_owner ON local_ask_digest_pending(staged_owner,resolution_id) WHERE staged_owner IS NOT NULL;
CREATE TABLE IF NOT EXISTS local_ask_digest_emitted(resolution_id TEXT PRIMARY KEY);
CREATE TABLE IF NOT EXISTS local_ask_digest_sources(
 source_id TEXT PRIMARY KEY, person TEXT NOT NULL, date TEXT NOT NULL, covered_through INTEGER NOT NULL);
CREATE INDEX IF NOT EXISTS local_ask_digest_coverage ON local_ask_digest_sources(person,covered_through DESC);
CREATE TABLE IF NOT EXISTS local_ask_digest_source_history(
 claim_id TEXT PRIMARY KEY, source TEXT NOT NULL, cursor TEXT, complete INTEGER NOT NULL DEFAULT 0);
CREATE INDEX IF NOT EXISTS local_ask_digest_history_work ON local_ask_digest_source_history(complete,claim_id);
CREATE TABLE IF NOT EXISTS local_ask_digest_draft(
 person TEXT PRIMARY KEY, source_id TEXT NOT NULL, date TEXT NOT NULL,
 covered_through INTEGER NOT NULL, owner TEXT NOT NULL, payload TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS local_ask_digest_draft_owner ON local_ask_digest_draft(owner,person);
CREATE TABLE IF NOT EXISTS local_ask_digest_deferred(
 claim_id TEXT PRIMARY KEY, dependency TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS local_ask_digest_dependency ON local_ask_digest_deferred(dependency,claim_id);
CREATE TABLE IF NOT EXISTS local_ask_digest_retry(
 dependency TEXT PRIMARY KEY, cursor TEXT NOT NULL DEFAULT '');
CREATE TRIGGER IF NOT EXISTS ask_digest_retry_membership AFTER UPDATE OF value ON fleet_generation
BEGIN
 INSERT INTO local_ask_digest_retry(dependency) SELECT 'fleet-membership'
 WHERE EXISTS(SELECT 1 FROM local_ask_digest_deferred WHERE dependency='fleet-membership')
 ON CONFLICT(dependency) DO UPDATE SET cursor='';
END;
";

pub(super) fn create_schema(connection: &Connection) -> Result<()> {
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-h822dc1235889 owner=fractal_store::store::ask_digest_cache TODO=trace-statement-and-value-representation */ SCHEMA)?;
    Ok(())
}

/// Discover one merged claim page and retry one dependency page. Sleeping dependencies do
/// no work until an arrival enqueues their key; unresolved history never causes idle polling.
pub(super) fn ingest(tx: &Transaction<'_>) -> Result<bool> {
    let after: u64 = tx.query_row("SELECT value FROM meta WHERE key=?1", [CURSOR], |r| r.get::<_,String>(0))?
        .parse()?;
    let mut ids = Vec::with_capacity((PAGE + 1) * 4);
    for kind in ["work.person-asked", "work.person-done", "custom.attention.notice-created", "intent.desired"] {
        ids.extend(tx.prepare_cached("SELECT store_index,id FROM claims INDEXED BY claims_kind_index
            WHERE kind=?1 AND store_index>?2 ORDER BY store_index LIMIT ?3")?
            .query_map(params![kind, after, PAGE + 1], |r| Ok((r.get::<_,u64>(0)?,r.get::<_,String>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?);
    }
    ids.sort_unstable();
    let more = ids.len() > PAGE;
    ids.truncate(PAGE);
    let mut pending_changed = false;
    for (_, id) in &ids {
        let claim = claim_by_id_tx(tx, id)?.context("digest discovery claim disappeared")?;
        pending_changed |= matches!(claim.kind.as_str(), "work.person-asked" | "work.person-done");
        match claim.kind.as_str() {
            "work.person-done" if claim.actor.as_deref() == Some(notice::RUNTIME_ACTOR)
                && claim.body["fields"]["resolved_by"] == "deadline" => {
                let Some(episode) = claim.body["fields"]["episode"].as_str() else { continue };
                let ask = claim_by_id_tx(tx, episode)?;
                let person = ask.as_ref().filter(|ask| ask.subject == claim.subject && ask.origin == claim.origin
                    && ask.kind == "work.person-asked").and_then(|ask| ask.body["fields"]["person"].as_str());
                tx.execute(/* sqlite-large-value: allow-for-now lv-hb2f8a8e73148 owner=fractal_store::store::ask_digest_cache TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO local_ask_digest_pending(resolution_id,episode,target,person,resolved_at)
                    SELECT ?1,?2,?3,?4,?5 WHERE NOT EXISTS(SELECT 1 FROM local_ask_digest_emitted WHERE resolution_id=?1)",
                    params![claim.id,episode,claim.subject,person,i64::try_from(claim.accepted_at_unix_ms)?])?;
            }
            "work.person-asked" => {
                tx.execute(/* sqlite-large-value: allow-for-now lv-h5b47dbcca06d owner=fractal_store::store::ask_digest_cache TODO=prepare-body-and-predecessor-values */ "UPDATE local_ask_digest_pending SET person=?1 WHERE episode=?2 AND target=?3
                    AND EXISTS(SELECT 1 FROM claims applied WHERE applied.id=resolution_id AND applied.origin=?4)",
                    params![claim.body["fields"]["person"].as_str(),claim.id,claim.subject,claim.origin])?;
            }
            "custom.attention.notice-created" if claim.subject.starts_with(notice::DIGEST_SOURCE_PREFIX) => {
                discover_source(tx, &claim)?;
            }
            "intent.desired" if claim.subject == crate::notice_owner::SUBJECT => {
                for dependency in [claim.id.as_str(), crate::notice_owner::SUBJECT] {
                    tx.execute(/* sqlite-large-value: allow-for-now lv-h49be7c29c97f owner=fractal_store::store::ask_digest_cache TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO local_ask_digest_retry(dependency)
                        SELECT ?1 WHERE EXISTS(SELECT 1 FROM local_ask_digest_deferred WHERE dependency=?1)
                        ON CONFLICT(dependency) DO UPDATE SET cursor=''", [dependency])?;
                }
            }
            _ => {}
        }
    }
    if pending_changed {
        tx.execute(/* sqlite-large-value: allow-for-now lv-h1010a890e203 owner=fractal_store::store::ask_digest_cache TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta(key,value) VALUES('ask_digest_pending_generation_v1','1')
            ON CONFLICT(key) DO UPDATE SET value=CAST(value AS INTEGER)+1", [])?;
    }
    if let Some((last, _)) = ids.last() {
        tx.execute(/* sqlite-large-value: allow-for-now lv-h53a7c3351614 owner=fractal_store::store::ask_digest_cache TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO meta(key,value) VALUES(?1,?2)", params![CURSOR,last.to_string()])?;
    }
    let retrying = retry_dependency_page(tx)?;
    Ok(more || retrying)
}

/// Schema creation never scans history. The cursor's absence is the sole initialization marker.
pub(super) fn initialize(tx: &Transaction<'_>) -> Result<()> {
    tx.execute(/* sqlite-large-value: allow-for-now lv-h005b96d7003a owner=fractal_store::store::ask_digest_cache TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO meta(key,value) VALUES(?1,'0')", [CURSOR])?;
    Ok(())
}

/// Index reported IDs a bounded backing page at a time before issuing new digests. A late
/// replica is then either already reported in authenticated history, or remains pending.
pub(super) fn index_source_page(tx: &Transaction<'_>) -> Result<bool> {
    let source: Option<(String,String,Option<String>)> = tx.query_row(
        "SELECT claim_id,source,cursor FROM local_ask_digest_source_history WHERE complete=0 ORDER BY claim_id LIMIT 1", [],
        |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
    let Some((id, source, cursor)) = source else { return Ok(false) };
    let source: Value = serde_json::from_str(&source)?;
    let page = ask_digest_payload::read_page(tx, &source, cursor.as_deref())?;
    for row in page.rows {
        let resolution = row["resolution_id"].as_str().context("digest row has no resolution identity")?;
        tx.execute(/* sqlite-large-value: allow-for-now lv-hd329a4434419 owner=fractal_store::store::ask_digest_cache TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO local_ask_digest_emitted VALUES(?1)", [resolution])?;
        tx.execute(/* sqlite-large-value: allow-for-now lv-ha9788b830cab owner=fractal_store::store::ask_digest_cache TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_ask_digest_pending WHERE resolution_id=?1", [resolution])?;
    }
    tx.execute(/* sqlite-large-value: allow-for-now lv-habed36083304 owner=fractal_store::store::ask_digest_cache TODO=bound-or-reference-values-and-reclamation */ "UPDATE local_ask_digest_source_history SET cursor=?2,complete=?3 WHERE claim_id=?1",
        params![id,page.next,i64::from(page.next.is_none())])?;
    Ok(true)
}

/// Only unknown dependencies enter the sleeping index. Invalid/nonowner claims never fence a
/// date or retain retry work; authenticated sources fence immediately, before payload paging.
fn discover_source(tx: &Transaction<'_>, claim: &ClaimRecord) -> Result<()> {
    use notice_authority::DigestSourceAuthority;
    tx.execute(/* sqlite-large-value: allow-for-now lv-hd076ea3b2e83 owner=fractal_store::store::ask_digest_cache TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_ask_digest_deferred WHERE claim_id=?1", [&claim.id])?;
    match notice_authority::digest_source_authority(tx, claim)? {
        DigestSourceAuthority::Rejected => {}
        DigestSourceAuthority::Deferred(dependency) => {
            tx.execute(/* sqlite-large-value: allow-for-now lv-h6e3e0e3da649 owner=fractal_store::store::ask_digest_cache TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO local_ask_digest_deferred(claim_id,dependency) VALUES(?1,?2)",
                params![claim.id,dependency])?;
        }
        DigestSourceAuthority::Authenticated => {
            let fields = custom::fields(claim);
            let source = &fields["source"];
            let covered = source.get("covered_through").or_else(|| fields.get("created_at"))
                .and_then(Value::as_str).context("digest history has no coverage boundary")?;
            let covered = chrono::DateTime::parse_from_rfc3339(covered)?.timestamp_millis();
            let person = fields["recipient"].as_str().context("digest history has no recipient")?;
            let date = claim.subject.rsplit('/').next().context("digest history has no date")?;
            tx.execute(/* sqlite-large-value: allow-for-now lv-hd24779ff7fd2 owner=fractal_store::store::ask_digest_cache TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO local_ask_digest_sources(source_id,person,date,covered_through)
                VALUES(?1,?2,?3,?4) ON CONFLICT(source_id) DO UPDATE
                SET covered_through=MAX(covered_through,excluded.covered_through)",
                params![claim.subject,person,date,covered])?;
            // All authenticated creations contribute reported IDs, even when a reader chooses
            // a single canonical notice for this date after disconnected owners converge.
            tx.execute(/* sqlite-large-value: allow-for-now lv-h50cadbd1bc45 owner=fractal_store::store::ask_digest_cache TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO local_ask_digest_source_history(claim_id,source) VALUES(?1,?2)",
                params![claim.id,serde_json::to_string(source)?])?;
        }
    }
    Ok(())
}

/// An arrival enqueues one dependency, not one task per dependent source. Fanout is traversed
/// using the dependency index and a durable keyset cursor, never a lifetime history scan.
fn retry_dependency_page(tx: &Transaction<'_>) -> Result<bool> {
    let work: Option<(String,String)> = tx.query_row(
        "SELECT dependency,cursor FROM local_ask_digest_retry ORDER BY dependency LIMIT 1", [],
        |r| Ok((r.get(0)?,r.get(1)?))).optional()?;
    let Some((dependency, cursor)) = work else { return Ok(false) };
    let mut ids = tx.prepare_cached(
        "SELECT claim_id FROM local_ask_digest_deferred WHERE dependency=?1 AND claim_id>?2
         ORDER BY claim_id LIMIT ?3")?
        .query_map(params![dependency,cursor,PAGE + 1], |r| r.get::<_,String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let more = ids.len() > PAGE;
    ids.truncate(PAGE);
    for id in &ids {
        if let Some(claim) = claim_by_id_tx(tx, id)? {
            discover_source(tx, &claim)?;
        } else {
            tx.execute(/* sqlite-large-value: allow-for-now lv-h7dc6789df5f7 owner=fractal_store::store::ask_digest_cache TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_ask_digest_deferred WHERE claim_id=?1", [id])?;
        }
    }
    if more {
        tx.execute(/* sqlite-large-value: allow-for-now lv-he623221fa3c8 owner=fractal_store::store::ask_digest_cache TODO=bound-or-reference-values-and-reclamation */ "UPDATE local_ask_digest_retry SET cursor=?2 WHERE dependency=?1",
            params![dependency,ids.last().context("retry page is empty")?])?;
    } else {
        tx.execute(/* sqlite-large-value: allow-for-now lv-h9f90d74f7c4f owner=fractal_store::store::ask_digest_cache TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_ask_digest_retry WHERE dependency=?1", [&dependency])?;
    }
    Ok(true)
}

#[cfg(test)]
#[path = "ask_digest_cache_tests.rs"]
mod tests;
