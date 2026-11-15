//! Daily informational notices for asks resolved by their deadline.
//! The source identity, not local daemon state, fences retries and owner handovers.
use super::*;
use crate::ask_digest_clock::{ask_digest_policy, ask_digest_window};
use crate::external_sessions::timestamp;
use fractal_schema::notice;

const DAY_MS: u128 = 24 * 60 * 60_000;
const OVERRIDE_WINDOW_MS: u128 = 7 * DAY_MS;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AskDigestPass {
    pub changed: bool,
    pub next_deadline: Option<u128>,
}

impl Store {
    /// Only the bound notice owner's daemon writes. No binding means no notice and no timer;
    /// the declaration/membership change itself wakes reconciliation.
    pub fn reconcile_ask_digests_at(&self, now: u128) -> Result<AskDigestPass> {
        self.connection.batched(|tx| -> Result<AskDigestPass> {
            let Ok((owner, _)) = notice_authority::owner(tx) else {
                return Ok(AskDigestPass::default());
            };
            if owner.member != self.origin {
                return Ok(AskDigestPass::default());
            }
            let (at, timezone) = ask_digest_policy(tx)?;
            let window = ask_digest_window(now, &timezone, &at)?;
            let mut pass = AskDigestPass { changed: false, next_deadline: Some(window.next_deadline) };
            if !window.due { return Ok(pass); }
            // Ask episodes carry the recipient even after their person step has completed.
            let people = tx.prepare_cached(
                "SELECT DISTINCT json_extract(body,'$.fields.person') FROM claims
                 WHERE kind='work.person-asked' AND json_extract(body,'$.fields.person') LIKE 'person/%'
                 ORDER BY 1")?.query_map([], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            for person in people {
                let suffix = format!("digest/{person}/{}", window.date);
                let source_id = format!("{}{suffix}", notice::SOURCE_PREFIX);
                // Deliberately check immutable history, not a current-owner projection. Replaying
                // after restart or handover never creates a second source/snapshot for the day.
                if tx.query_row("SELECT EXISTS(SELECT 1 FROM claims WHERE subject=?1 AND kind='custom.attention.notice-created')",
                    [&source_id], |row| row.get::<_, bool>(0))? { continue; }
                let rows = resolved_rows(tx, &person, window.window_start, window.window_end, now)?;
                if rows.is_empty() { continue; }
                notice_authority::ensure_digest_registrations_tx(tx, &self.origin)?;
                let source = append_notice(tx, &self.origin, &source_id, "custom.attention.notice-created", json!({
                    "schema_version":notice::PAYLOAD_VERSION,"dedupe_key":suffix,"resource":"resource/attention",
                    "recipient":person,"intent":"fyi","urgency":"info","requested":false,
                    "summary":format!("{} ask{} auto-resolved", rows.len(), if rows.len() == 1 { "" } else { "s" }),
                    "links":[],"source":{"host":format!("host/{}",self.origin),"digest":rows},
                    "created_at":timestamp(window.due_at)
                }), &[])?;
                let snapshot_id = format!("{}{suffix}", notice::SNAPSHOT_PREFIX);
                append_notice(tx, &self.origin, &snapshot_id, "custom.attention.snapshot-created", json!({
                    "source_id":source_id,"creation_claim":source.id,"producer":notice::RUNTIME_ACTOR,
                    "recipient":person,"runtime":format!("host/{}",self.origin),
                    "policy":{"disposition":"deliver"},"delivery":[]
                }), &[source.id])?;
                pass.changed = true;
            }
            Ok(pass)
        }).map_err(anyhow::Error::msg)?
    }
}

fn append_notice(tx: &Transaction<'_>, origin: &str, subject: &str, kind: &str,
    fields: Value, predecessors: &[String]) -> Result<ClaimRecord> {
    let input = ClaimInput {
        subject: subject.into(), kind: kind.into(), actor: Some(notice::RUNTIME_ACTOR.into()),
        fields: serde_json::from_value(fields)?, evidence: Vec::new(),
        expected_subject: None, idempotency_key: None,
    };
    let fields = custom::prepare(tx, origin, &input)?.context("notice registration is missing")?;
    append_claim_tx(tx, origin, subject, kind, Some(notice::RUNTIME_ACTOR),
        &json!({"fields":fields}), predecessors, None)
}

/// A local-calendar day runs between consecutive configured digest times, including the
/// 23/25-hour DST days. Already overridden defaults are excluded, rather than advertised
/// as still awaiting the person's answer. All members' replicated deadline answers qualify.
fn resolved_rows(connection: &Connection, person: &str, start: u128, end: u128, now: u128) -> Result<Vec<Value>> {
    let mut query = connection.prepare_cached(
        "SELECT ask.id,ask.subject,ask.actor,ask.body,applied.accepted_at_unix_ms,applied.body
         FROM claims ask JOIN claims applied ON applied.subject=ask.subject
           AND applied.kind='work.person-done' AND applied.actor='daemon/runtime'
           AND applied.origin=ask.origin AND json_extract(applied.body,'$.fields.episode')=ask.id
           AND json_extract(applied.body,'$.fields.resolved_by')='deadline'
         WHERE ask.kind='work.person-asked' AND json_extract(ask.body,'$.fields.person')=?1
           AND CAST(applied.accepted_at_unix_ms AS INTEGER)>=?2
           AND CAST(applied.accepted_at_unix_ms AS INTEGER)<?3
           AND NOT EXISTS(SELECT 1 FROM claims answer WHERE answer.subject=ask.subject
             AND json_extract(answer.body,'$.fields.episode')=ask.id
             AND answer.actor=?1 AND (answer.kind='work.person-override'
               OR answer.kind='work.person-done' OR answer.kind='work.person-cancelled'))
         ORDER BY CAST(applied.accepted_at_unix_ms AS INTEGER),ask.id")?;
    let records = query.query_map(params![person, i64::try_from(start)?, i64::try_from(end)?], |row| {
        Ok((row.get::<_,String>(0)?, row.get::<_,String>(1)?, row.get::<_,Option<String>>(2)?,
            row.get::<_,String>(3)?,row.get::<_,String>(4)?,row.get::<_,String>(5)?))
    })?.collect::<rusqlite::Result<Vec<_>>>()?;
    let mut rows = Vec::with_capacity(records.len());
    let mut episodes = BTreeSet::new();
    for (episode, target, actor, ask, resolved, applied) in records {
        if !episodes.insert(episode.clone()) { continue; }
        let ask: Value = serde_json::from_str(&ask)?;
        let applied: Value = serde_json::from_str(&applied)?;
        let resolved: u128 = resolved.parse()?;
        let mut row = json!({"title":ask["fields"]["title"],"requester_id":actor.unwrap_or_default(),
            "resolved_at":timestamp(resolved),"target_id":target,"episode":episode});
        if let Some(answer) = applied["fields"].get("answer").filter(|answer| !answer.is_null()) {
            row["default_answer"] = json!({"id":answer["id"],"label":answer["label"]});
            if now <= resolved.saturating_add(OVERRIDE_WINDOW_MS) {
                row["action_parameters"] = json!({"work.override":{"target_id":target,"episode":episode}});
            }
        }
        rows.push(row);
    }
    Ok(rows)
}

#[cfg(test)]
#[path = "ask_digest_tests.rs"]
mod tests;
