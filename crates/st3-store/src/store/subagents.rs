//! Subagents: helpers a seat's harness runs inside its own session, recorded as claims on the
//! parent seat. A subagent is not a seat or a mission.
//!
//! The seat's driver records `subagent.appeared` when its harness starts one, `subagent.renewed`
//! while it runs and its lease nears its end, and `subagent.ended` when the harness reports its
//! end. A subagent is open from its appearance until its end. The node that recorded an
//! appearance also ends the subagent when its lease runs out, its harness exits or restarts, or
//! its seat is stopped or removed, so a subagent never outlives what started it. On any node, an
//! open subagent whose lease ran out reads as expired.

use super::*;

/// The kinds that make up a subagent's record on its parent seat.
pub const SUBAGENT_KINDS: [&str; 3] = ["subagent.appeared", "subagent.renewed", "subagent.ended"];

/// One open subagent: appeared and not yet ended.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SubagentView {
    /// The parent seat.
    pub agent: String,
    /// The harness's own ID for the subagent, unique within its parent seat.
    pub subagent_id: String,
    pub subagent_type: Option<String>,
    pub description: Option<String>,
    pub driver: String,
    pub session_id: Option<String>,
    pub incarnation_id: String,
    /// The step the parent held when the subagent appeared.
    pub step_run: Option<String>,
    pub started_at_unix_ms: u64,
    pub lease_expires_at_unix_ms: u64,
    /// The `subagent.appeared` claim, and the node that recorded it.
    pub appeared: String,
    pub origin: String,
    #[serde(skip)]
    pub store_index: u64,
}

impl SubagentView {
    pub fn expired_at(&self, now: u64) -> bool {
        self.lease_expires_at_unix_ms <= now
    }
}

pub fn text(fields: &Value, name: &str) -> Option<String> {
    fields
        .get(name)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

/// Which open subagents a read wants.
#[derive(Clone, Copy)]
pub enum Scope<'a> {
    Seat(&'a str),
    /// Recorded on this node from this claim on.
    RecordedOn(&'a str, u64),
    /// Whose lease runs past this time.
    Running(u64),
    All,
}

/// SQL shared by the read and its query-plan/cost regression test.
fn open_subagents_sql(scope: Scope<'_>) -> String {
    let (live, from, filter) = match scope {
        Scope::Seat(_) => ("", "FROM claims", "AND claims.subject=?1"),
        // The sweep's own bound on this node's claims, not an order.
        Scope::RecordedOn(..) => (
            "",
            "FROM claims",
            "AND claims.origin=?1 AND claims.store_index>=?2",
        ),
        // Pin the expiry seeks: SQLite otherwise prefers its generic kind index without
        // ANALYZE and scans historical JSON. Keep the live pairs outermost for the join.
        Scope::Running(_) => (
            "WITH live(subject, subagent_id) AS (
                 SELECT subject, json_extract(body, '$.fields.subagent_id')
                 FROM claims INDEXED BY claims_subagent_appeared_lease_index
                 WHERE kind='subagent.appeared'
                   AND CAST(json_extract(body, '$.fields.lease_expires_at_unix_ms') AS INTEGER)>?1
                 UNION
                 SELECT subject, json_extract(body, '$.fields.subagent_id')
                 FROM claims INDEXED BY claims_subagent_renewed_lease_index
                 WHERE kind='subagent.renewed'
                   AND CAST(json_extract(body, '$.fields.lease_expires_at_unix_ms') AS INTEGER)>?1)",
            "FROM live CROSS JOIN claims INDEXED BY claims_subagent_appeared_index
               ON claims.subject=live.subject
               AND json_extract(claims.body, '$.fields.subagent_id')=live.subagent_id",
            "",
        ),
        Scope::All => ("", "FROM claims", ""),
    };
    canonical_sql(&format!(
        "{live} SELECT claims.subject, claims.id, claims.origin, claims.body, claims.store_index,
             (SELECT MAX(CAST(json_extract(renewed.body, '$.fields.lease_expires_at_unix_ms')
                  AS INTEGER))
              FROM claims renewed INDEXED BY claims_subagent_renewed_max_lease_index
              WHERE renewed.kind='subagent.renewed' AND renewed.subject=claims.subject
                AND json_extract(renewed.body, '$.fields.subagent_id')
                    =json_extract(claims.body, '$.fields.subagent_id'))
         {from}
         WHERE claims.kind='subagent.appeared' {filter}
           AND NOT EXISTS (
               SELECT 1 FROM claims ended INDEXED BY claims_subagent_ended_index
               WHERE ended.kind='subagent.ended' AND ended.subject=claims.subject
                 AND json_extract(ended.body, '$.fields.subagent_id')
                     =json_extract(claims.body, '$.fields.subagent_id'))
         ORDER BY CANONICAL_ASC(claims)"
    ))
}

/// Open subagents in `scope`, oldest first in the canonical claim order.
pub fn open_subagents_at(connection: &Connection, scope: Scope<'_>) -> Result<Vec<SubagentView>> {
    let mut statement = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h593466ebd586 owner=st3_store::store::subagents TODO=trace-statement-and-value-representation */ &open_subagents_sql(scope))?;
    let read = |row: &rusqlite::Row<'_>| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, u64>(4)?,
            row.get::<_, Option<i64>>(5)?,
        ))
    };
    let rows = match scope {
        Scope::Seat(agent) => statement.query_map([agent], read)?.collect::<Vec<_>>(),
        Scope::Running(now) => statement
            .query_map([i64::try_from(now).unwrap_or(i64::MAX)], read)?
            .collect::<Vec<_>>(),
        Scope::RecordedOn(origin, after) => statement
            .query_map(params![origin, after], read)?
            .collect::<Vec<_>>(),
        Scope::All => statement.query_map([], read)?.collect::<Vec<_>>(),
    };
    let mut open = Vec::new();
    for row in rows {
        let (agent, appeared, origin, body, store_index, renewed) = row?;
        let body: Value = serde_json::from_str(&body)?;
        let fields = &body["fields"];
        let Some(subagent_id) = text(fields, "subagent_id") else {
            continue;
        };
        let leased = fields["lease_expires_at_unix_ms"].as_u64().unwrap_or(0);
        open.push(SubagentView {
            agent,
            subagent_id,
            subagent_type: text(fields, "subagent_type"),
            description: text(fields, "description"),
            driver: text(fields, "driver").unwrap_or_default(),
            session_id: text(fields, "session_id"),
            incarnation_id: text(fields, "incarnation_id").unwrap_or_default(),
            step_run: text(fields, "step_run"),
            started_at_unix_ms: fields["started_at_unix_ms"].as_u64().unwrap_or(0),
            lease_expires_at_unix_ms: leased.max(renewed.map_or(0, |at| at.max(0) as u64)),
            appeared,
            origin,
            store_index,
        });
    }
    Ok(open)
}

/// The claim of `kind` that names `subagent_id` on `agent`, if any.
fn subagent_claim_tx(
    transaction: &Transaction<'_>,
    agent: &str,
    kind: &str,
    subagent_id: &str,
) -> Result<Option<ClaimRecord>, St3Error> {
    debug_assert!(SUBAGENT_KINDS.contains(&kind));
    // The kind is written into the statement so SQLite can use that kind's partial index.
    transaction
        .query_row(
            /* sqlite-large-value: allow-for-now lv-he2f4e2be7bbb owner=st3_store::store::subagents TODO=trace-statement-and-value-representation */ &format!(
                "SELECT {CLAIM_COLUMNS} FROM claims JOIN batches ON batches.id=claims.batch_id
                 WHERE claims.kind='{kind}' AND claims.subject=?1
                   AND json_extract(claims.body, '$.fields.subagent_id')=?2
                 ORDER BY {CANONICAL_ORDER} LIMIT 1"
            ),
            params![agent, subagent_id],
            claim_from_row,
        )
        .optional()
        .map_err(internal)
}

/// Keep each subagent's record whole: one appearance, renewals only while it is open, and one end.
/// A repeated appearance or end answers with the claim already recorded. Returns that claim when
/// the input adds nothing.
pub fn check_subagent_claim_tx(
    transaction: &Transaction<'_>,
    input: &ClaimInput,
) -> Result<Option<ClaimRecord>, St3Error> {
    if !SUBAGENT_KINDS.contains(&input.kind.as_str()) {
        return Ok(None);
    }
    let Some(subagent_id) = input
        .fields
        .get("subagent_id")
        .and_then(Value::as_str)
        .filter(|id| !id.trim().is_empty())
    else {
        return Err(St3Error::new(
            "invalid-subagent",
            "a subagent claim needs the harness's subagent ID",
        ));
    };
    let appeared = subagent_claim_tx(
        transaction,
        &input.subject,
        "subagent.appeared",
        subagent_id,
    )?;
    if input.kind == "subagent.appeared" {
        return Ok(appeared);
    }
    if appeared.is_none() {
        return Err(St3Error::new(
            "unknown-subagent",
            format!(
                "subagent `{subagent_id}` never appeared on `{}`",
                input.subject
            ),
        ));
    }
    let ended = subagent_claim_tx(transaction, &input.subject, "subagent.ended", subagent_id)?;
    match (input.kind.as_str(), ended) {
        ("subagent.ended", ended) => Ok(ended),
        (_, Some(_)) => Err(St3Error::new(
            "subagent-ended",
            format!(
                "subagent `{subagent_id}` on `{}` already ended",
                input.subject
            ),
        )),
        (_, None) => Ok(None),
    }
}

/// The newest runtime record of a seat: its status and incarnation.
fn seat_runtime(connection: &Connection, seat: &str) -> Result<Option<(String, Option<String>)>> {
    let mut statement = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h055df751c251 owner=st3_store::store::subagents TODO=trace-statement-and-value-representation */ &canonical_sql(
        "SELECT body FROM claims
         WHERE subject=?1 AND kind='runtime.observed'
         ORDER BY CANONICAL_DESC(claims)",
    ))?;
    let bodies = statement.query_map([seat], |row| row.get::<_, String>(0))?;
    for body in bodies {
        let body: Value = serde_json::from_str(&body?)?;
        let fields = body.get("fields").unwrap_or(&body);
        if let Some(status) = text(fields, "status") {
            return Ok(Some((status, text(fields, "incarnation_id"))));
        }
    }
    Ok(None)
}

/// What the sweep needs to know about a parent seat: its declaration's kind and its newest
/// runtime status and incarnation.
pub struct Seat {
    pub declared: Option<String>,
    pub runtime: Option<(String, Option<String>)>,
}

pub fn seat(connection: &Connection, agent: &str) -> Result<Seat> {
    Ok(Seat {
        declared: connection
            .query_row(
                "SELECT kind FROM desired WHERE subject=?1",
                [agent],
                |row| row.get(0),
            )
            .optional()?,
        runtime: seat_runtime(connection, agent)?,
    })
}

impl Store {
    /// A seat's open subagents, oldest first. One whose lease ran out still shows until the node
    /// that recorded it ends it; `SubagentView::expired_at` tells it apart.
    pub fn open_subagents(&self, agent: &str) -> Result<Vec<SubagentView>> {
        open_subagents_at(&self.readers.get(), Scope::Seat(agent))
    }

}

#[cfg(test)]
mod tests {
    #[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::subagents_tests_shared_seats::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::subagents_tests_shared_seats::{SEAT, appear, claim, ids, runtime};
use super::*;

    #[test]
    fn running_subagent_work_is_flat_when_expired_history_grows_tenfold() {
        let store = Store::open_memory("node").unwrap();
        let mut connection = store.connection.write();
        let tx = connection.transaction().unwrap();
        let append = |kind: &str, id: &str, lease: u64| {
            append_claim_record_tx(
                &tx,
                "node",
                SEAT,
                kind,
                None,
                &json!({"fields":{"subagent_id":id,"lease_expires_at_unix_ms":lease}}),
                &[],
                None,
            )
            .unwrap();
        };
        append("subagent.appeared", "fresh", 30_000);
        append("subagent.appeared", "renewed", 1);
        append("subagent.renewed", "renewed", 40_000);
        // An end wins even while its lease still runs.
        append("subagent.appeared", "ended-live", 50_000);
        append("subagent.ended", "ended-live", 0);
        let sql = open_subagents_sql(Scope::Running(10_000));
        let original = sql
            .replace(" INDEXED BY claims_subagent_appeared_lease_index", "")
            .replace("FROM live CROSS JOIN claims", "FROM live JOIN claims")
            .replace(" INDEXED BY claims_subagent_renewed_lease_index", "")
            .replace(" INDEXED BY claims_subagent_appeared_index", "")
            .replace(" INDEXED BY claims_subagent_renewed_max_lease_index", "")
            .replace(" INDEXED BY claims_subagent_ended_index", "");
        let measure = |query: &str| {
            let mut statement = tx.prepare(query).unwrap();
            let mut timings = Vec::new();
            let mut cost = 0;
            for _ in 0..31 {
                statement.reset_status(rusqlite::StatementStatus::VmStep);
                let started = std::time::Instant::now();
                let rows = statement
                    .query_map([10_000], |row| row.get::<_, String>(0))
                    .unwrap()
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .unwrap();
                timings.push(started.elapsed());
                assert_eq!(rows, [SEAT, SEAT]);
                cost = statement.get_status(rusqlite::StatementStatus::VmStep);
            }
            timings.sort();
            (cost, timings[15])
        };
        let explain = |query: &str| {
            tx.prepare(&format!("EXPLAIN QUERY PLAN {query}"))
                .unwrap()
                .query_map([10_000], |row| row.get::<_, String>(3))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap()
        };
        let mut costs = Vec::new();
        for (start, end) in [(0, 1_000), (1_000, 10_000)] {
            for number in start..end {
                let id = format!("expired-{number}");
                append("subagent.appeared", &id, 1);
                append("subagent.renewed", &id, 2);
                append("subagent.ended", &id, 0);
                // Grow renewal history for a pair that still runs, too.
                append("subagent.renewed", "renewed", 3);
            }
            // Measure the exact old query against the old schema as well.
            tx.execute_batch(
                "DROP INDEX claims_subagent_renewed_max_lease_index;
                 CREATE INDEX claims_subagent_renewed_index
                 ON claims(subject, json_extract(body, '$.fields.subagent_id'))
                 WHERE kind='subagent.renewed';",
            )
            .unwrap();
            let before = measure(&original);
            let before_plan = explain(&original);
            tx.execute_batch(SCHEMA).unwrap();
            let after = measure(&sql);
            let plan = explain(&sql);
            println!("history={end} before={before:?} after={after:?}");
            println!("before plan: {before_plan:#?}\nafter plan: {plan:#?}");
            for index in [
                "claims_subagent_appeared_lease_index (<expr>>?)",
                "claims_subagent_renewed_lease_index (<expr>>?)",
                "claims_subagent_appeared_index (subject=? AND <expr>=?)",
                "claims_subagent_renewed_max_lease_index (subject=? AND <expr>=?)",
                "claims_subagent_ended_index (subject=? AND <expr>=?)",
            ] {
                assert!(plan.iter().any(|line| line.contains(index)), "{plan:?}");
            }
            assert!(!plan.iter().any(|line| line.contains("claims_kind_index")));
            let open = open_subagents_at(&tx, Scope::Running(10_000)).unwrap();
            assert_eq!(
                open.iter()
                    .map(|subagent| (subagent.subagent_id.as_str(), subagent.lease_expires_at_unix_ms))
                    .collect::<Vec<_>>(),
                [("fresh", 30_000), ("renewed", 40_000)]
            );
            costs.push(after.0);
        }
        assert_eq!(costs[0], costs[1], "work follows live leases, not expired history");
        assert!(costs[0] < 500, "fixed two-row overlay must stay bounded: {costs:?}");
        tx.rollback().unwrap();
    }

}
