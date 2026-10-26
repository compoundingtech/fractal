//! Keyed mission selection over admitted operational projections.
//!
//! This shadow operator covers selection headers and run counts, not public card detail,
//! effective steps or actor authorization. The caller certifies projected-source coverage
//! before flushing; populated-source installation is explicit. Readers never seed or flush.
// Prepared shadow interface; production registration is owned by the source installer.
#![allow(dead_code)]
use super::*;
use smallclaims::ivm::{LocalChange, Readiness, Views, source_cut};

pub const VIEW: &str = "st3.mission-selection.v1";
pub const SOURCE: &str = "st3.mission-selection.changed";
pub const SCHEMA: &str = /* sqlite-large-value: allow-for-now lv-h77c407af0deb owner=fractal_store::store::mission_ivm TODO=chunk-or-reference-variable-columns */ r#"
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
            /* sqlite-large-value: allow-for-now lv-h080ed3f99217 owner=fractal_store::store::mission_ivm TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_mission_pending WHERE mission_id=?1",
            [mission],
        )?;
    }
    tx.execute(
        /* sqlite-large-value: allow-for-now lv-hde04efd8bcf1 owner=fractal_store::store::mission_ivm TODO=bound-or-reference-values-and-reclamation */ "UPDATE local_mission_clock SET through_ms=?1 WHERE through_ms<?1",
        [i64::try_from(at)?],
    )?;
    Ok(keys.len())
}

pub fn ready_at(connection: &Connection, views: &Views, at: u128) -> Result<()> {
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
