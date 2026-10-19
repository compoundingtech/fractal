//! The part of `store::mission_ivm` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`

use super::*;
use smallclaims::ivm::{Definition, LocalChange, Readiness, View, Views, source_cut};
pub use fractal_delivery::store::mission_ivm::*;
pub use fractal_delivery::store::mission_ivm::SCHEMA;
pub use fractal_delivery::store::mission_ivm::{SOURCE, VIEW, flush, ready, row, rows};

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
        connection.execute_batch(/* sqlite-large-value: allow-for-now lv-h242812352607 owner=fractal_missions::store::mission_ivm TODO=trace-statement-and-value-representation */ SCHEMA)?;
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
            /* sqlite-large-value: allow-for-now lv-he2a7550701ab owner=fractal_missions::store::mission_ivm TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_mission_selection WHERE mission_id=?1",
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
        /* sqlite-large-value: allow-for-now lv-h8bfd7b8e1f8b owner=fractal_missions::store::mission_ivm TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO local_mission_selection VALUES(?1,?2,?3,?4,?5,?6)
        ON CONFLICT(mission_id) DO UPDATE SET updated_ms=excluded.updated_ms,state=excluded.state,
          grace_until_ms=excluded.grace_until_ms,visible=excluded.visible,body=excluded.body",
        params![mission, updated, state, grace, visible, body],
    )?;
    Ok(true)
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
        /* sqlite-large-value: allow-for-now lv-h7fb3c232ab25 owner=fractal_missions::store::mission_ivm TODO=bound-or-reference-values-and-reclamation */ "UPDATE local_mission_clock SET through_ms=?1 WHERE singleton=1 AND through_ms<?1",
        [at_ms],
    )?;
    Ok(keys.len())
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
        .prepare_cached(/* sqlite-large-value: allow-for-now lv-h497417b7ca37 owner=fractal_missions::store::mission_ivm TODO=trace-statement-and-value-representation */ &sql)?
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
