//! Source-stamped schedule ownership lifetimes. Unstamped historical terminal owners retain
//! their schedules until an operator migrates them; receiving old history never enables a stop.
use super::*;
use std::borrow::Cow;

const EPOCH_KEY: &str = "schedule_owner_guard_epoch_v1";
const EPOCH_PREFIX: &str = "schedule-owner-lifetime-v1/";
const EPOCH_FIELD: &str = "schedule_owner_guard_epoch";

#[derive(Clone, Debug)]
pub struct ScheduleTerminalOwner {
    pub owner: String,
    pub code: &'static str,
    pub legacy: bool,
}

fn terminal(connection: &Connection, owner: &str, run: bool) -> Result<bool> {
    if run {
        Ok(connection.query_row(
            "SELECT phase='terminal' OR status IN ('completed','failed','cancelled') FROM mission_runs WHERE id=?1",
            [owner.strip_prefix("mission-run/").unwrap_or(owner)], |row| row.get(0),
        ).optional()?.unwrap_or(false))
    } else {
        Ok(connection.query_row(
            "SELECT status IN ('completed','failed','cancelled','skipped') FROM step_runs WHERE subject=?1",
            [owner], |row| row.get(0),
        ).optional()?.unwrap_or(false))
    }
}

fn terminal_epoch(connection: &Connection, owner: &str, kind: &str) -> Result<Option<String>> {
    Ok(connection.query_row(
        "SELECT json_extract(body,'$.schedule_owner_guard_epoch') FROM claims
         WHERE subject=?1 AND (kind=?2 OR (?2='step-run.state'
             AND (kind LIKE 'work.%' OR kind IN ('step-run.carried','step-run.retried'))))
           AND (json_extract(body,'$.fields.status') IN ('completed','failed','cancelled','skipped')
                OR (?2='mission-run.state' AND json_extract(body,'$.fields.phase')='terminal'))
         ORDER BY store_index DESC LIMIT 1",
        params![owner, kind], |row| row.get::<_, Option<String>>(0),
    ).optional()?.flatten())
}

impl Store {
    /// Enable once at daemon startup, not when a schedule is first evaluated. The epoch is
    /// local writer metadata; terminal claims carry it across replication and database reopen.
    pub fn enable_schedule_owner_guard(&self) -> Result<String> {
        let connection = self.connection.write();
        connection.execute(
            "INSERT OR IGNORE INTO meta(key,value) VALUES(?1,?2 || lower(hex(randomblob(16))))",
            params![EPOCH_KEY, EPOCH_PREFIX],
        )?;
        Ok(connection.query_row("SELECT value FROM meta WHERE key=?1", [EPOCH_KEY], |row| row.get(0))?)
    }

    pub fn schedule_terminal_owner(
        &self, owner_run: Option<&str>, owner_step: Option<&str>,
    ) -> Result<Option<ScheduleTerminalOwner>> {
        let connection = self.readers.get();
        for (owner, run, kind, code) in [
            (owner_run, true, "mission-run.state", "schedule-owner-run-terminal"),
            (owner_step, false, "step-run.state", "schedule-owner-step-terminal"),
        ] {
            let Some(owner) = owner else { continue };
            smallclaims::touched::note_read(|| owner.to_owned());
            if !terminal(&connection, owner, run)? { continue; }
            let epoch = terminal_epoch(&connection, owner, kind)?;
            return Ok(Some(ScheduleTerminalOwner {
                owner: owner.into(), code,
                legacy: !epoch.as_deref().is_some_and(|epoch| epoch.starts_with(EPOCH_PREFIX)),
            }));
        }
        Ok(None)
    }
}

/// Only the local authoritative append path stamps transitions. Replication preserves the
/// source body; it must never stamp a historical transition on arrival at an upgraded host.
/// Reasserting an already-terminal outcome preserves its previous epoch (including absence),
/// so an ordinary duplicate write cannot turn a legacy duty into an automatic stop.
pub(super) fn stamp_terminal_claim<'a>(
    connection: &Connection, subject: &str, kind: &str, body: &'a Value,
) -> Result<Cow<'a, Value>> {
    let run = match kind {
        "mission-run.state" => true,
        "step-run.state" | "step-run.carried" => false,
        _ if kind.starts_with("work.") && subject.starts_with("step-run/") => false,
        _ => return Ok(Cow::Borrowed(body)),
    };
    let status = body.pointer("/fields/status").and_then(Value::as_str);
    if !matches!(status, Some("completed" | "failed" | "cancelled" | "skipped"))
        && !(run && body.pointer("/fields/phase").and_then(Value::as_str) == Some("terminal"))
    {
        return Ok(Cow::Borrowed(body));
    }
    // Some mutation APIs update projections before appending their state claim. Determine
    // the previous lifecycle from source history, never from the already-updated projection.
    let previous: Option<(bool, Option<String>)> = connection.query_row(
        "SELECT COALESCE(json_extract(body,'$.fields.status') IN ('completed','failed','cancelled','skipped')
                   OR (?3 AND json_extract(body,'$.fields.phase')='terminal'),0),
                json_extract(body,'$.schedule_owner_guard_epoch')
         FROM claims WHERE subject=?1 AND (kind=?2 OR (?2='step-run.state'
             AND (kind LIKE 'work.%' OR kind IN ('step-run.carried','step-run.retried'))))
           AND (json_type(body,'$.fields.status')='text'
                OR (?3 AND json_type(body,'$.fields.phase')='text'))
         ORDER BY store_index DESC LIMIT 1",
        params![subject, if run { "mission-run.state" } else { "step-run.state" }, run],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ).optional()?;
    let epoch = if let Some((true, epoch)) = previous {
        epoch
    } else {
        connection.query_row("SELECT value FROM meta WHERE key=?1", [EPOCH_KEY], |row| row.get(0))
            .optional()?
    };
    let Some(epoch) = epoch else { return Ok(Cow::Borrowed(body)) };
    let mut stamped = body.clone();
    stamped[EPOCH_FIELD] = Value::String(epoch);
    Ok(Cow::Owned(stamped))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_work_and_carried_step_sources_receive_the_same_durable_epoch() {
        let store = Store::open_memory("node").unwrap();
        let epoch = store.enable_schedule_owner_guard().unwrap();
        let connection = store.readers.get();
        for kind in ["step-run.state", "step-run.carried", "work.failed"] {
            let body = json!({"fields": {"status": "failed"}});
            let stamped = stamp_terminal_claim(&connection, "step-run/example/hold", kind, &body).unwrap();
            assert_eq!(stamped[EPOCH_FIELD], epoch, "{kind}");
        }
        let body = json!({"fields": {"status": "working"}});
        assert!(matches!(stamp_terminal_claim(
            &connection, "step-run/example/hold", "work.progress", &body,
        ).unwrap(), Cow::Borrowed(_)));
    }
}
