//! Keyed mission selection over admitted operational projections.
//!
//! This shadow operator covers selection headers and run counts, not public card detail,
//! effective steps or actor authorization. The caller certifies projected-source coverage
//! before flushing; populated-source installation is explicit. Readers never seed or flush.
// Prepared shadow interface; production registration is owned by the source installer.
#![allow(dead_code)]
use super::*;
use smallclaims::ivm::{Definition, LocalChange, Readiness, View, Views, source_cut};

pub const VIEW: &str = "st3.mission-selection.v1";
pub const SOURCE: &str = "st3.mission-selection.changed";
const SCHEMA: &str = /* sqlite-large-value: allow-for-now lv-ha75b45aa6e87 owner=st3_store::store::mission_ivm TODO=chunk-or-reference-variable-columns */ r#"
CREATE TABLE IF NOT EXISTS local_mission_counts (
 mission_id TEXT NOT NULL, status TEXT NOT NULL, count INTEGER NOT NULL CHECK(count>0),
 PRIMARY KEY(mission_id,status)
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS local_mission_pending (mission_id TEXT PRIMARY KEY) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS local_mission_selection (
 mission_id TEXT PRIMARY KEY, updated_ms INTEGER NOT NULL, state TEXT NOT NULL,
 grace_until_ms INTEGER, visible INTEGER NOT NULL CHECK(visible IN (0,1)), body TEXT NOT NULL
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS local_mission_selection_order
 ON local_mission_selection(updated_ms DESC,mission_id ASC);
CREATE INDEX IF NOT EXISTS local_mission_selection_current_order
 ON local_mission_selection(updated_ms DESC,mission_id ASC) WHERE visible=1;
CREATE TABLE IF NOT EXISTS local_mission_clock (singleton INTEGER PRIMARY KEY CHECK(singleton=1), through_ms INTEGER NOT NULL);
INSERT OR IGNORE INTO local_mission_clock VALUES(1,0);
CREATE INDEX IF NOT EXISTS local_mission_selection_due
 ON local_mission_selection(grace_until_ms) WHERE visible=1 AND grace_until_ms IS NOT NULL;
CREATE INDEX IF NOT EXISTS local_mission_selection_deadline
 ON local_mission_selection(grace_until_ms) WHERE grace_until_ms IS NOT NULL AND mission_id NOT LIKE '__st3/%';
CREATE INDEX IF NOT EXISTS mission_ivm_latest_run
 ON mission_runs(mission_id,created_at_unix_ms DESC,id DESC);
CREATE TRIGGER IF NOT EXISTS mission_ivm_run_insert AFTER INSERT ON mission_runs BEGIN
 INSERT INTO local_mission_counts VALUES(NEW.mission_id,NEW.status,1)
 ON CONFLICT(mission_id,status) DO UPDATE SET count=count+1;
 INSERT INTO local_mission_pending VALUES(NEW.mission_id) ON CONFLICT(mission_id) DO NOTHING;
END;
CREATE TRIGGER IF NOT EXISTS mission_ivm_run_delete AFTER DELETE ON mission_runs BEGIN
 DELETE FROM local_mission_counts WHERE mission_id=OLD.mission_id AND status=OLD.status AND count=1;
 UPDATE local_mission_counts SET count=count-1 WHERE mission_id=OLD.mission_id AND status=OLD.status;
 INSERT INTO local_mission_pending VALUES(OLD.mission_id) ON CONFLICT(mission_id) DO NOTHING;
END;
CREATE TRIGGER IF NOT EXISTS mission_ivm_run_count_update
 AFTER UPDATE OF mission_id,status ON mission_runs
 WHEN NEW.mission_id<>OLD.mission_id OR NEW.status<>OLD.status BEGIN
 DELETE FROM local_mission_counts WHERE mission_id=OLD.mission_id AND status=OLD.status AND count=1;
 UPDATE local_mission_counts SET count=count-1 WHERE mission_id=OLD.mission_id AND status=OLD.status;
 INSERT INTO local_mission_counts VALUES(NEW.mission_id,NEW.status,1)
 ON CONFLICT(mission_id,status) DO UPDATE SET count=count+1;
END;
CREATE TRIGGER IF NOT EXISTS mission_ivm_run_update AFTER UPDATE ON mission_runs
 WHEN NEW.id<>OLD.id OR NEW.mission_id<>OLD.mission_id OR NEW.initial_revision<>OLD.initial_revision
 OR NEW.current_generation_id<>OLD.current_generation_id OR NEW.requester<>OLD.requester
 OR NEW.status<>OLD.status OR NEW.phase<>OLD.phase
 OR NEW.created_at_unix_ms<>OLD.created_at_unix_ms OR NEW.updated_at_unix_ms<>OLD.updated_at_unix_ms BEGIN
 INSERT INTO local_mission_pending VALUES(OLD.mission_id) ON CONFLICT(mission_id) DO NOTHING;
 INSERT INTO local_mission_pending VALUES(NEW.mission_id) ON CONFLICT(mission_id) DO NOTHING;
END;
CREATE TRIGGER IF NOT EXISTS mission_ivm_definition_insert AFTER INSERT ON mission_definitions BEGIN
 INSERT INTO local_mission_pending VALUES(NEW.mission_id) ON CONFLICT(mission_id) DO NOTHING;
END;
CREATE TRIGGER IF NOT EXISTS mission_ivm_definition_delete AFTER DELETE ON mission_definitions BEGIN
 INSERT INTO local_mission_pending VALUES(OLD.mission_id) ON CONFLICT(mission_id) DO NOTHING;
END;
CREATE TRIGGER IF NOT EXISTS mission_ivm_definition_update AFTER UPDATE ON mission_definitions
 WHEN NEW.mission_id<>OLD.mission_id OR NEW.state<>OLD.state OR NEW.revision<>OLD.revision OR NEW.claim_id<>OLD.claim_id BEGIN
 INSERT INTO local_mission_pending VALUES(OLD.mission_id) ON CONFLICT(mission_id) DO NOTHING;
 INSERT INTO local_mission_pending VALUES(NEW.mission_id) ON CONFLICT(mission_id) DO NOTHING;
END;
-- Definition publication times are claim inputs. Updates/removals also fence generic IVM.
CREATE TRIGGER IF NOT EXISTS mission_ivm_publication_time AFTER UPDATE OF accepted_at_unix_ms ON claims
 WHEN NEW.accepted_at_unix_ms<>OLD.accepted_at_unix_ms BEGIN
 INSERT INTO local_mission_pending SELECT mission_id FROM mission_definitions WHERE claim_id=NEW.id ON CONFLICT(mission_id) DO NOTHING;
END;
"#;

pub fn definitions() -> Vec<Box<dyn View>> {
    vec![Box::new(MissionSelection)]
}

pub struct MissionSelection;
impl View for MissionSelection {
    fn definition(&self) -> Definition {
        Definition {
            name: VIEW,
            fingerprint: "selection.v1;projected-mission-definitions+run-headers;status-count-deltas;latest-created-text-desc-id-desc;updated-integer-desc-mission-asc;24h-failed-cancelled;captured-clock-current-membership;unicode-public-id-rust-lower;no-card-steps-auth",
            kinds: &[],
            local_kinds: &[SOURCE],
            max_contributions: 1,
        }
    }
    fn create_schema(&self, connection: &Connection) -> Result<()> {
        connection.execute_batch(/* sqlite-large-value: allow-for-now lv-hf16640615599 owner=st3_store::store::mission_ivm TODO=trace-statement-and-value-representation */ SCHEMA)?;
        Ok(())
    }
    fn maintain_local_key(
        &self,
        tx: &Transaction<'_>,
        key: &str,
        change: &LocalChange,
    ) -> Result<bool> {
        maintain(tx, key, change.evaluation_time_unix_ms)
    }
}

fn maintain(tx: &Transaction<'_>, key: &str, at: u128) -> Result<bool> {
    let at = i64::try_from(at).context("mission source clock exceeds SQLite range")?;
    let mission = key
        .strip_prefix("mission/")
        .context("mission selection key")?;
    let definition: Option<(String, String, i64)> = tx
        .query_row(
            "SELECT d.state,d.revision,CAST(COALESCE(c.accepted_at_unix_ms,'0') AS INTEGER)
         FROM mission_definitions d LEFT JOIN claims c ON c.id=d.claim_id WHERE d.mission_id=?1",
            [mission],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let counts = tx
        .prepare_cached(
            "SELECT status,count FROM local_mission_counts WHERE mission_id=?1 ORDER BY status",
        )?
        .query_map([mission], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
        })?
        .collect::<rusqlite::Result<BTreeMap<_, _>>>()?;
    let latest: Option<(String, String, i64, String)> = tx.query_row(
        "SELECT id,status,CAST(updated_at_unix_ms AS INTEGER),
          json_object('id',id,'generation',current_generation_id,'revision',initial_revision,
           'requester',requester,'status',status,'phase',phase,'created',created_at_unix_ms,'updated',updated_at_unix_ms)
         FROM mission_runs WHERE mission_id=?1 ORDER BY created_at_unix_ms DESC,id DESC LIMIT 1",
        [mission], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).optional()?;
    let old: Option<String> = tx
        .query_row(
            "SELECT body FROM local_mission_selection WHERE mission_id=?1",
            [mission],
            |row| row.get(0),
        )
        .optional()?;
    if definition.is_none() && latest.is_none() {
        tx.execute(
            /* sqlite-large-value: allow-for-now lv-h32dfb2a42fcd owner=st3_store::store::mission_ivm TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_mission_selection WHERE mission_id=?1",
            [mission],
        )?;
        return Ok(old.is_some());
    }
    let running = counts.get("running").copied().unwrap_or(0);
    let standing = counts.get("standing").copied().unwrap_or(0);
    let retired = definition.as_ref().is_some_and(|d| d.0 == "retired");
    let state = if running > 0 {
        "running"
    } else if standing > 0 {
        "standing"
    } else if retired {
        "retired"
    } else if let Some(latest) = &latest {
        &latest.1
    } else {
        definition.as_ref().map(|d| d.0.as_str()).unwrap_or("ready")
    };
    let updated = latest
        .as_ref()
        .map(|r| r.2)
        .or_else(|| definition.as_ref().map(|d| d.2))
        .unwrap_or(0);
    let grace = latest
        .as_ref()
        .filter(|r| {
            matches!(r.1.as_str(), "failed" | "cancelled")
                && !retired
                && running == 0
                && standing == 0
        })
        .map(|r| {
            r.2.checked_add(24 * 60 * 60 * 1000)
                .context("mission deadline exceeds SQLite range")
        })
        .transpose()?;
    let external: bool =
        tx.query_row("SELECT ?1 NOT LIKE '__st3/%'", [mission], |row| row.get(0))?;
    let visible = external
        && (!matches!(state, "completed" | "failed" | "cancelled" | "retired")
            || grace.is_some_and(|until| until >= at));
    let latest_body = latest
        .as_ref()
        .map(|r| serde_json::from_str::<Value>(&r.3))
        .transpose()?;
    let body = canonical_json_text(
        &json!({"id":key,"search_id":key.to_lowercase(),"state":state,"updated_ms":updated,
        "definition":definition,"latest":latest_body,"counts":counts,"grace_until_ms":grace,"current":visible}),
    )?;
    if old.as_deref() == Some(&body) {
        return Ok(false);
    }
    tx.execute(
        /* sqlite-large-value: allow-for-now lv-hbf52790b11bb owner=st3_store::store::mission_ivm TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO local_mission_selection VALUES(?1,?2,?3,?4,?5,?6)
        ON CONFLICT(mission_id) DO UPDATE SET updated_ms=excluded.updated_ms,state=excluded.state,
          grace_until_ms=excluded.grace_until_ms,visible=excluded.visible,body=excluded.body",
        params![mission, updated, state, grace, visible, body],
    )?;
    Ok(true)
}

/// Drain at most `limit` affected missions. The owner supplies a source cut only after proving
/// complete projection/dependency coverage; changing the cut here cannot prove that coverage.
/// On a fenced view the queue remains intact for an explicitly initiated recovery installer.
pub fn flush(tx: &Transaction<'_>, views: &Views, at: u128, limit: usize) -> Result<usize> {
    anyhow::ensure!(
        (1..=1024).contains(&limit),
        "mission maintenance page bound"
    );
    let mut cut = source_cut(tx)?.context("mission selection source unavailable")?;
    anyhow::ensure!(
        cut.admitted == cut.projected && cut.admitted == current_index_tx(tx)?,
        "mission selection source pending"
    );
    anyhow::ensure!(
        matches!(views.readiness(tx, VIEW, cut.epoch)?, Readiness::Ready(_)),
        "mission selection unavailable"
    );
    let clock: i64 = tx.query_row(
        "SELECT through_ms FROM local_mission_clock WHERE singleton=1",
        [],
        |row| row.get(0),
    )?;
    anyhow::ensure!(
        i64::try_from(at)? >= clock,
        "mission source clock moved backwards"
    );
    let keys = tx
        .prepare_cached(
            "SELECT mission_id FROM local_mission_pending ORDER BY mission_id LIMIT ?1",
        )?
        .query_map([limit], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for mission in &keys {
        cut.local_generation = cut
            .local_generation
            .checked_add(1)
            .context("local generation exhausted")?;
        let changed = views.local_change(
            tx,
            &LocalChange {
                kind: SOURCE.into(),
                old_keys: BTreeSet::new(),
                new_keys: BTreeSet::from([format!("mission/{mission}")]),
                evaluation_time_unix_ms: at,
            },
            cut,
        )?;
        anyhow::ensure!(
            changed.deferred.is_empty(),
            "mission selection maintenance deferred"
        );
        tx.execute(
            /* sqlite-large-value: allow-for-now lv-hac1eeab81697 owner=st3_store::store::mission_ivm TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_mission_pending WHERE mission_id=?1",
            [mission],
        )?;
    }
    tx.execute(
        /* sqlite-large-value: allow-for-now lv-ha8ce25d71fa1 owner=st3_store::store::mission_ivm TODO=bound-or-reference-values-and-reclamation */ "UPDATE local_mission_clock SET through_ms=?1 WHERE through_ms<?1",
        [i64::try_from(at)?],
    )?;
    Ok(keys.len())
}

/// Explicit captured-time maintenance. Due candidates use a partial deadline index and
/// each page changes at most `limit` public keys. Reads refuse overdue membership.
pub fn clock_page(
    tx: &Transaction<'_>,
    views: &Views,
    at: u128,
    limit: usize,
) -> Result<usize> {
    anyhow::ensure!((1..=1024).contains(&limit), "mission clock page bound");
    ready(tx, views)?;
    let at_ms = i64::try_from(at)?;
    let previous: i64 = tx.query_row(
        "SELECT through_ms FROM local_mission_clock WHERE singleton=1",
        [],
        |row| row.get(0),
    )?;
    anyhow::ensure!(at_ms >= previous, "mission clock moved backwards");
    let keys = tx
        .prepare_cached(
            "SELECT mission_id FROM local_mission_selection
        WHERE visible=1 AND grace_until_ms IS NOT NULL AND grace_until_ms<?1
        ORDER BY grace_until_ms LIMIT ?2",
        )?
        .query_map(params![at_ms, limit], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut cut = source_cut(tx)?.context("mission clock source unavailable")?;
    for id in &keys {
        cut.local_generation = cut
            .local_generation
            .checked_add(1)
            .context("local generation exhausted")?;
        let changes = views.local_change(
            tx,
            &LocalChange {
                kind: SOURCE.into(),
                old_keys: BTreeSet::new(),
                new_keys: BTreeSet::from([format!("mission/{id}")]),
                evaluation_time_unix_ms: at,
            },
            cut,
        )?;
        anyhow::ensure!(
            changes.deferred.is_empty(),
            "mission clock maintenance deferred"
        );
    }
    tx.execute(
        /* sqlite-large-value: allow-for-now lv-he423a47cc437 owner=st3_store::store::mission_ivm TODO=bound-or-reference-values-and-reclamation */ "UPDATE local_mission_clock SET through_ms=?1 WHERE singleton=1 AND through_ms<?1",
        [at_ms],
    )?;
    Ok(keys.len())
}

fn ready_at(connection: &Connection, views: &Views, at: u128) -> Result<()> {
    ready(connection, views)?;
    let at = i64::try_from(at)?;
    let previous: i64 = connection.query_row(
        "SELECT through_ms FROM local_mission_clock WHERE singleton=1",
        [],
        |row| row.get(0),
    )?;
    anyhow::ensure!(
        at >= previous,
        "mission clock snapshot predates maintained membership"
    );
    let due: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM local_mission_selection
        WHERE visible=1 AND grace_until_ms IS NOT NULL AND grace_until_ms<?1)",
        [at],
        |row| row.get(0),
    )?;
    anyhow::ensure!(!due, "mission clock membership pending");
    Ok(())
}

pub fn ready(connection: &Connection, views: &Views) -> Result<()> {
    let cut = source_cut(connection)?.context("mission selection source unavailable")?;
    anyhow::ensure!(
        matches!(
            views.readiness(connection, VIEW, cut.epoch)?,
            Readiness::Ready(_)
        ),
        "mission selection unavailable"
    );
    let dirty: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM local_mission_pending)",
        [],
        |row| row.get(0),
    )?;
    anyhow::ensure!(!dirty, "mission selection has unflushed source changes");
    Ok(())
}

/// Uses the existing timestamp DESC / public ID ASC continuation tuple. Membership is
/// evaluated at the caller's captured time, including the inclusive end of the grace period.
pub fn window(
    connection: &Connection,
    views: &Views,
    history: bool,
    at: u128,
    after: Option<&(u128, String)>,
    text: Option<&str>,
    limit: usize,
) -> Result<Vec<(String, u128)>> {
    anyhow::ensure!((1..=501).contains(&limit), "mission window page bound");
    let at = i64::try_from(at).context("mission snapshot time exceeds SQLite range")?;
    let after_ms = after
        .map(|key| i64::try_from(key.0))
        .transpose()
        .context("mission seek time exceeds SQLite range")?;
    ready_at(connection, views, at as u128)?;
    let membership = if history { "1" } else { "visible=1" };
    // IDs are Unicode-lowercased on maintenance, matching the public list matcher. Only
    // the bounded needle is normalized here; SQLite lower() covers ASCII alone.
    let text = text.map(str::to_lowercase);
    let seek = if after.is_some() {
        "updated_ms<=?3 AND (updated_ms<?3 OR mission_id>?4)"
    } else {
        "?3 IS NULL AND ?4 IS NULL"
    };
    let sql = format!(
        "SELECT mission_id,updated_ms FROM local_mission_selection
        WHERE {membership} AND ?1 IN (0,1) AND ?2>=0 AND ({seek})
          AND (?6 IS NULL OR instr(json_extract(body,'$.search_id'),?6)>0)
        ORDER BY updated_ms DESC,mission_id ASC LIMIT ?5"
    );
    connection
        .prepare_cached(/* sqlite-large-value: allow-for-now lv-he1bc7263fcec owner=st3_store::store::mission_ivm TODO=trace-statement-and-value-representation */ &sql)?
        .query_map(
            params![
                history,
                at,
                after_ms,
                after.map(|key| key.1.trim_start_matches("mission/")),
                limit,
                text
            ],
            |row| {
                Ok((
                    format!("mission/{}", row.get::<_, String>(0)?),
                    row.get::<_, u64>(1)? as u128,
                ))
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

pub fn row(
    connection: &Connection,
    views: &Views,
    at: u128,
    key: &str,
) -> Result<Option<Value>> {
    ready_at(connection, views, at)?;
    connection
        .query_row(
            "SELECT body FROM local_mission_selection WHERE mission_id=?1",
            [key.trim_start_matches("mission/")],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .map(|body| serde_json::from_str(&body))
        .transpose()
        .map_err(Into::into)
}

/// One indexed query for the already selected keys; callers retain their window order.
pub fn rows(
    connection: &Connection,
    views: &Views,
    at: u128,
    keys: &[String],
) -> Result<BTreeMap<String, Value>> {
    anyhow::ensure!(keys.len() <= 501, "mission selected-key batch bound");
    ready_at(connection, views, at)?;
    let ids = keys
        .iter()
        .map(|key| key.trim_start_matches("mission/"))
        .collect::<Vec<_>>();
    let bodies = connection.prepare_cached(
        "SELECT mission_id,body FROM local_mission_selection WHERE mission_id IN (SELECT value FROM json_each(?1))"
    )?.query_map([serde_json::to_string(&ids)?], |row| Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?)))?
      .collect::<rusqlite::Result<Vec<_>>>()?;
    bodies
        .into_iter()
        .map(|(id, body)| Ok((format!("mission/{id}"), serde_json::from_str(&body)?)))
        .collect()
}

pub fn clean(connection: &Connection) -> Result<bool> {
    connection
        .query_row(
            "SELECT NOT EXISTS(SELECT 1 FROM local_mission_pending)",
            [],
            |row| row.get(0),
        )
        .map_err(Into::into)
}

/// Earliest captured-time membership change for transport/reconciler timer ownership.
pub fn next_deadline(
    connection: &Connection,
    views: &Views,
    at: u128,
) -> Result<Option<u128>> {
    ready_at(connection, views, at)?;
    let time: Option<u64> = connection
        .query_row(
            "SELECT grace_until_ms+1 FROM local_mission_selection
      WHERE grace_until_ms IS NOT NULL AND mission_id NOT LIKE '__st3/%' AND grace_until_ms>=?1 ORDER BY grace_until_ms LIMIT 1",
            [i64::try_from(at)?],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    Ok(time.map(u128::from))
}

#[cfg(test)]
mod tests {
    #[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::mission_ivm_tests_shared_seats::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::mission_ivm_tests_shared_seats::start;

    
    

}
