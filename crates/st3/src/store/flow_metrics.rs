//! Fleet-wide flow measurements over the sparse current-work projection.
use super::*;
#[cfg(test)]
#[path = "flow_metrics_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "attention_metrics_tests.rs"]
mod attention_tests;

pub(crate) const WORK_STATES: [&str; 4] = ["ready", "claimed", "waiting_person", "blocked"];

#[derive(Clone, Debug, Default)]
pub(crate) struct FlowSnapshot {
    pub counts: [u64; 4],
    pub oldest_seconds: [f64; 4],
    pub with_work: u64,
    pub idle_with_work: u64,
    pub mission_wait: super::mission_metrics::MissionWaitMetrics,
}

// One statement; four latest-entry index seeks per distinct holder.
// MATERIALIZED prevents the two consumers from repeating the open-step traversal.
const SNAPSHOT_SQL: &str = r#"
WITH work AS MATERIALIZED (
 SELECT s.status, s.assignee, s.lease_owner, s.lease_incarnation,
        CAST(s.lease_expires_at_unix_ms AS INTEGER) AS expires,
        CAST(s.created_at_unix_ms AS INTEGER) AS updated,
        COALESCE((SELECT CAST(accepted_at_unix_ms AS INTEGER) FROM claims
          WHERE subject=s.subject AND kind='work.claimed'
          ORDER BY length(accepted_at_unix_ms) DESC, accepted_at_unix_ms DESC LIMIT 1),
          CAST(s.created_at_unix_ms AS INTEGER)) AS claimed_at,
        CASE WHEN s.blocked_reason GLOB 'step baseline `*' THEN 1
             WHEN s.blocked_reason GLOB 'waiting for `mission-run/*' THEN 2
             ELSE 0 END AS reason_class,
        CAST(s.activated_at_unix_ms AS INTEGER) AS activated,
        MAX(CAST(s.created_at_unix_ms AS INTEGER),
          COALESCE((SELECT CAST(accepted_at_unix_ms AS INTEGER) FROM claims
            WHERE subject=s.subject AND kind='step-run.state'
            ORDER BY length(accepted_at_unix_ms) DESC, accepted_at_unix_ms DESC LIMIT 1),0),
          COALESCE((SELECT CAST(accepted_at_unix_ms AS INTEGER) FROM claims
            WHERE subject=s.subject AND kind='step-run.retried'
            ORDER BY length(accepted_at_unix_ms) DESC, accepted_at_unix_ms DESC LIMIT 1),0),
          COALESCE((SELECT CAST(accepted_at_unix_ms AS INTEGER) FROM claims
            WHERE subject=s.subject AND kind='step-run.carried'
            ORDER BY length(accepted_at_unix_ms) DESC, accepted_at_unix_ms DESC LIMIT 1),0)
        ) AS state_since,
        json_extract(c.state,'$.pause_started') AS pause_started,
        json_extract(c.state,'$.paused_until') AS paused_until,
        c.subject IS NOT NULL AS clock_known, s.agentless
 FROM step_runs s INDEXED BY step_runs_open_index
 JOIN mission_runs r ON r.id=s.run_id
 JOIN run_generations g ON g.id=s.generation_id
 JOIN mission_runs root ON root.id=r.root_run_id
 LEFT JOIN local_step_clocks c ON c.subject=s.subject AND c.attempt=s.attempt
   AND NOT EXISTS(SELECT 1 FROM local_step_clock_pending WHERE subject=s.subject)
   AND NOT EXISTS(SELECT 1 FROM local_step_clock_dirty WHERE subject=s.subject)
 WHERE s.status NOT IN ('completed','failed','cancelled')
   AND s.generation_id=r.current_generation_id
   AND r.status NOT IN ('completed','failed','cancelled') AND r.phase!='terminal'
   AND root.status NOT IN ('completed','failed','cancelled') AND root.phase!='terminal'
   AND g.status NOT IN ('completed','failed','cancelled','superseded')
), holders AS MATERIALIZED (
 SELECT agent, incarnation, since FROM (
  SELECT lease_owner AS agent, lease_incarnation AS incarnation, claimed_at AS since,
         ROW_NUMBER() OVER (
          PARTITION BY lease_owner ORDER BY claimed_at DESC, lease_incarnation DESC
         ) AS position
  FROM work WHERE lease_owner LIKE 'agent/%'
    AND status IN ('claimed','working','verifying','blocked')
    AND (expires IS NULL OR expires>?1)
 ) WHERE position=1
)
SELECT 0, status, assignee, lease_owner, expires, updated, NULL,
       reason_class, activated, state_since, pause_started, paused_until, clock_known, agentless
FROM work
UNION ALL
SELECT 1, agent, NULL, NULL, NULL, since,
 MAX(
 COALESCE((SELECT CAST(accepted_at_unix_ms AS INTEGER) FROM claims INDEXED BY claims_actor_activity_time_index
  WHERE actor=holders.agent AND kind IN ('work.progress','work.submitted')
  ORDER BY length(accepted_at_unix_ms) DESC, accepted_at_unix_ms DESC LIMIT 1),0),
 COALESCE((SELECT CAST(accepted_at_unix_ms AS INTEGER) FROM claims INDEXED BY claims_message_from_activity_time_index
  WHERE kind='message.sent' AND json_extract(body,'$.fields.from')=holders.agent
  ORDER BY length(accepted_at_unix_ms) DESC, accepted_at_unix_ms DESC LIMIT 1),0),
 COALESCE((SELECT CAST(accepted_at_unix_ms AS INTEGER) FROM claims INDEXED BY claims_message_to_activity_time_index
  WHERE kind='message.sent' AND json_extract(body,'$.fields.to')=holders.agent
  ORDER BY length(accepted_at_unix_ms) DESC, accepted_at_unix_ms DESC LIMIT 1),0),
 COALESCE((SELECT CAST(accepted_at_unix_ms AS INTEGER) FROM claims INDEXED BY claims_timeline_activity_time_index
  WHERE subject=holders.agent AND kind='harness.timeline'
   AND json_extract(body,'$.fields.incarnation_id')=holders.incarnation
   AND json_extract(body,'$.fields.entry_type') IN ('message','content','tool_call','tool_result')
  ORDER BY length(accepted_at_unix_ms) DESC, accepted_at_unix_ms DESC LIMIT 1),0)),
 NULL, NULL, NULL, NULL, NULL, NULL, NULL
FROM holders
"#;

pub(crate) const ATTENTION_KINDS: [&str; 2] = ["ask", "update"];

#[derive(Default)]
pub(crate) struct AttentionMetrics {
    pub counts: [u64; 2],
    pub oldest_seconds: [f64; 2],
}

impl Store {
    pub(crate) fn flow_snapshot(&self, now: u128) -> Result<FlowSnapshot> {
        let connection = self.readers.get();
        flow_snapshot_tx(&connection, i64::try_from(now).unwrap_or(i64::MAX))
    }

    /// Read the same publication attention windows serve; never fall back to a fold.
    pub(crate) fn attention_metrics(&self, person: &str, now: u128) -> Option<AttentionMetrics> {
        self.request_attention_list_refresh();
        if !self.published_view_serving("attention") { return None; }
        let publication = self.published_attention_list()?;
        let mut result = AttentionMetrics::default();
        for row in publication.rows.iter().filter(|row| row["person_id"] == person && row["state"] == "open") {
            let bucket = usize::from(row.get("update").is_some());
            let requested = chrono::DateTime::parse_from_rfc3339(row["requested_at"].as_str()?).ok()?;
            let requested = u128::try_from(requested.timestamp_millis()).ok()?;
            result.counts[bucket] += 1;
            result.oldest_seconds[bucket] = result.oldest_seconds[bucket]
                .max(now.saturating_sub(requested) as f64 / 1000.0);
        }
        Some(result)
    }
}

fn flow_snapshot_tx(connection: &Connection, now: i64) -> Result<FlowSnapshot> {
    let mut result = FlowSnapshot::default();
    let mut holders = BTreeMap::<String, i64>::new();
    let mut statement = connection.prepare_cached(SNAPSHOT_SQL)?;
    let mut rows = statement.query([now])?;
    while let Some(row) = rows.next()? {
        let kind: i64 = row.get(0)?;
        let state: String = row.get(1)?;
        let updated: i64 = row.get(5)?;
        if kind == 1 {
            let activity: i64 = row.get(6)?;
            let last = updated.max(activity);
            holders.entry(state).and_modify(|time| *time = (*time).max(last)).or_insert(last);
            continue;
        }
        let assignee: Option<String> = row.get(2)?;
        let expires: Option<i64> = row.get(4)?;
        let state = if matches!(state.as_str(), "claimed" | "working" | "verifying" | "blocked")
            && expires.is_some_and(|expiry| expiry <= now) { "ready" } else { &state };
        let bucket = match state {
            "waiting-person" => 2,
            "ready" | "pending" if assignee.as_deref().is_some_and(|a| a.starts_with("person/")) => 2,
            "ready" => 0,
            "claimed" | "working" | "verifying" => 1,
            _ => 3,
        };
        result.counts[bucket] += 1;
        result.oldest_seconds[bucket] = result.oldest_seconds[bucket]
            .max(now.saturating_sub(updated).max(0) as f64 / 1000.0);
        let evidence = super::mission_metrics::MissionWaitEvidence {
            state,
            person: assignee.as_deref().is_some_and(|a| a.starts_with("person/")),
            reason: row.get(7)?,
            activated: row.get(8)?,
            state_since: row.get(9)?,
            pause_started: row.get(10)?,
            paused_until: row.get(11)?,
            clock_known: row.get(12)?,
            agentless: row.get(13)?,
        };
        evidence.observe(&mut result.mission_wait, now);
    }
    result.with_work = holders.len() as u64;
    result.idle_with_work = holders.values().filter(|last| now.saturating_sub(**last) > 600_000).count() as u64;
    Ok(result)
}
