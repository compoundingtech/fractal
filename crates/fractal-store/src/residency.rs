//! Diagnostic idle residency policy. Eligibility is a continuous interval of accepted proof,
//! not an activity timer: mail and duplicate quiet observations never move its opening edge.

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use serde_json::Value;
use smallclaims::store::canonical_sql;

use crate::model::AgentIdleSuspend;
use crate::store::{Store, current_harness_at, desired_row_at, latest_actual_at};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Residency {
    pub policy: String,
    pub eligible_since_unix_ms: Option<u128>,
    pub due_at_unix_ms: Option<u128>,
    pub refusal_reason: Option<String>,
}

impl Residency {
    /// A daemon activation/restart may conservatively shorten, never extend, proven idleness.
    pub fn not_before(mut self, enabled_at: Option<u128>, after_ms: Option<u64>) -> Self {
        if self.refusal_reason.is_none() {
            self.eligible_since_unix_ms = self.eligible_since_unix_ms
                .map(|since| since.max(enabled_at.unwrap_or(since)));
            self.due_at_unix_ms = self.eligible_since_unix_ms.zip(after_ms)
                .map(|(since, after)| since.saturating_add(u128::from(after)));
        }
        self
    }
}

/// Evaluate only evidence visible at `at_index`. This never requests or performs suspension.
/// `after_ms=None` disables the member default, but explicit `never` remains visible.
pub fn evaluate_at(
    store: &Store,
    subject: &str,
    at_index: Option<u64>,
    after_ms: Option<u64>,
) -> Result<Residency> {
    smallclaims::touched::note_read(|| subject.to_owned());
    store.read_snapshot(|index| {
        let connection = store.readers.get();
        let cut = at_index.unwrap_or(index).min(index);
        let policy = policy_at(&connection, subject, cut, after_ms)?;
        let mut result = Residency {
            policy: policy.into(), eligible_since_unix_ms: None,
            due_at_unix_ms: None, refusal_reason: None,
        };
        if policy != "member" {
            result.refusal_reason = Some(if policy == "never" { "policy-never" } else { "policy-off" }.into());
            return Ok(result);
        }
        let runtime = latest_actual_at(&connection, subject, Some(cut))?;
        let incarnation = runtime.as_ref().and_then(|runtime| runtime["incarnation_id"].as_str());
        // Retained claimant ownership is indexed independently of the author: operator and
        // carried claims must remain visible after their current projection has been released.
        let work = if at_index.is_none() { store.work_for_reconcile(subject)? } else { Vec::new() };
        let mut steps = work.iter().filter(|step| step.claimant.as_deref() == Some(subject))
            .map(|step| step.subject.clone()).collect::<std::collections::BTreeSet<_>>();
        let mut claimed = connection.prepare_cached(
            "SELECT DISTINCT subject FROM claims INDEXED BY claims_residency_claimant_index
             WHERE json_extract(body, '$.fields.claimant')=?1
               AND store_index<=?2
               AND kind IN ('work.claimed', 'work.progress', 'work.renewed', 'step-run.carried')",
        )?;
        for step in claimed.query_map(params![subject, cut], |row| row.get::<_, String>(0))? {
            steps.insert(step?);
        }
        let steps = steps.into_iter().collect::<Vec<_>>();
        let live_work = at_index.is_none().then(|| work.iter().any(|step|
            step.claimant.as_deref() == Some(subject)
                && matches!(step.status.as_str(), "claimed" | "working" | "verifying")));
        let as_of = if at_index.is_some() { store.projection_time_at(cut)? } else { smallclaims::store::now_ms() };
        if let Some(reason) = refusal_at(&connection, subject, cut, after_ms, incarnation, WorkAt { steps: &steps, live: live_work, as_of })? {
            result.refusal_reason = Some(reason.into());
            return Ok(result);
        }
        let floor: u64 = connection.query_row(
            "SELECT COALESCE(MIN(store_index),0) FROM claims INDEXED BY claims_subject_kind_index
             WHERE subject=?1 AND kind='runtime.observed' AND store_index<=?2
               AND json_extract(body,'$.fields.incarnation_id')=?3",
            params![subject, cut, incarnation], |row| row.get(0),
        )?;
        let declaration = desired_row_at(&connection, subject, Some(cut))?.map(|row| row.claim_id);
        let steps_json = serde_json::to_string(&steps)?;
        // Only this runtime's seat suffix, selected declaration, and its indexed step subjects.
        // Stream duplicate reports but fold only semantic changes: heartbeat timestamps and
        // transition counters cannot make N repeated quiet reports trigger N harness folds.
        let mut statement = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h6af3a204dd01 owner=fractal_store::residency TODO=trace-statement-and-value-representation */ &canonical_sql(
            "SELECT claims.store_index, claims.accepted_at_unix_ms, claims.kind,
                    CASE WHEN kind='harness.observed' THEN json_array(
                        json_extract(body,'$.fields.state'),json_extract(body,'$.fields.driver'),
                        json_extract(body,'$.fields.transport'),json_extract(body,'$.fields.quiescent'),
                        json_extract(body,'$.fields.blocking'),json_extract(body,'$.fields.background_jobs'),
                        json_extract(body,'$.fields.ask'),json_extract(body,'$.fields.blocked_on'),
                        json_extract(body,'$.fields.input_buffer'),json_extract(body,'$.fields.provider_auth'),
                        json_extract(body,'$.fields.reason'),json_extract(body,'$.fields.incarnation_id'))
                    WHEN kind='harness.diagnostic' THEN json_array(
                        json_extract(body,'$.fields.code'),json_extract(body,'$.fields.status'),
                        json_extract(body,'$.fields.incarnation_id'))
                    ELSE NULL END
             FROM claims WHERE claims.store_index BETWEEN ?3 AND ?2 AND claims.store_index IN (
               SELECT store_index FROM claims INDEXED BY claims_subject_kind_index
                 WHERE subject=?1 AND kind IN ('runtime.observed','harness.observed',
                   'harness.diagnostic','harness.session-file','subagent.appeared','subagent.ended')
                   AND store_index BETWEEN ?3 AND ?2
                   AND (kind='runtime.observed' OR json_extract(body,'$.fields.incarnation_id') IS NULL
                     OR json_extract(body,'$.fields.incarnation_id')=?6)
               UNION ALL SELECT store_index FROM claims INDEXED BY claims_subject_kind_index
                 WHERE subject=(SELECT subject FROM claims WHERE id=?4)
                   AND kind IN ('intent.desired','owned-set.revised')
                   AND store_index BETWEEN ?3 AND ?2
               UNION ALL SELECT claims.store_index FROM json_each(?5) steps
                 JOIN claims INDEXED BY claims_subject_kind_index ON claims.subject=steps.value
                 WHERE kind IN ('step-run.state','step-run.carried','step-run.retried',
                   'work.claimed','work.progress','work.renewed','work.submitted','work.failed','work.released',
                   'work.handoff-returned','work.extended','work.person-asked','work.person-reassigned',
                   'work.person-done','work.person-override','work.person-cancelled')
                   AND claims.store_index BETWEEN ?3 AND ?2)
             ORDER BY CANONICAL_DESC(claims)",
        ))?;
        let events = statement.query_map(params![subject, cut, floor, declaration, steps_json, incarnation], |row| {
            Ok((row.get::<_, u64>(0)?, row.get::<_, String>(1)?,
                row.get::<_, String>(2)?, row.get::<_, Option<String>>(3)?))
        })?;
        let mut expiries = connection.prepare_cached(
            "SELECT CAST(json_extract(body,'$.fields.claim_expires_at_unix_ms') AS TEXT)
             FROM json_each(?1) steps JOIN claims INDEXED BY claims_subject_kind_index
               ON claims.subject=steps.value
             WHERE claims.store_index<=?2
               AND kind IN ('work.claimed','work.renewed','work.progress','work.extended','step-run.carried')
               AND json_extract(body,'$.fields.claim_expires_at_unix_ms') IS NOT NULL",
        )?.query_map(params![steps_json, cut], |row| row.get::<_, String>(0))?
            .map(|row| Ok(row?.parse::<u128>()?)).collect::<Result<Vec<_>>>()?;
        expiries.retain(|expiry| *expiry <= as_of);
        expiries.sort_unstable_by(|left, right| right.cmp(left));
        expiries.dedup();
        let mut expiries = expiries.into_iter().peekable();
        let mut previous = std::collections::BTreeMap::new();
        let mut since = None;
        'history: for event in events {
            let (index, accepted, kind, semantics) = event?;
            let accepted = accepted.parse::<u128>()?;
            // A retained lease supplies a clock boundary without fabricating a claim. Merge it
            // with the seat's accepted transitions and check every other blocker at that cut.
            while let Some(expiry) = expiries.peek().copied().filter(|expiry| *expiry >= accepted) {
                expiries.next();
                if refusal_at(&connection, subject, index, after_ms, incarnation,
                    WorkAt { steps: &steps, live: None, as_of: expiry })?.is_some() {
                    break 'history;
                }
                since = Some(expiry);
                previous.clear();
            }
            let unchanged = semantics.is_some() && previous.get(&kind) == semantics.as_ref();
            if !unchanged {
                if refusal_at(&connection, subject, index, after_ms, incarnation,
                    WorkAt { steps: &steps, live: None, as_of: accepted })?.is_some() {
                    break;
                }
                if let Some(semantics) = semantics {
                    previous.insert(kind, semantics);
                } else {
                    previous.clear();
                }
            }
            since = Some(accepted);
        }
        if since.is_none() {
            result.refusal_reason = Some("eligibility-unproven".into());
        }
        result.eligible_since_unix_ms = since;
        result.due_at_unix_ms = since.zip(after_ms).map(|(since, after)| since.saturating_add(u128::from(after)));
        Ok(result)
    })
}

fn policy_at(connection: &Connection, subject: &str, cut: u64, after: Option<u64>) -> Result<&'static str> {
    let desired = desired_row_at(connection, subject, Some(cut))?;
    let declared = desired.as_ref()
        .map(|row| serde_json::from_str::<Value>(&row.body)).transpose()?;
    Ok(match crate::model::declared_idle_suspend(declared.as_ref()) {
        Some(AgentIdleSuspend::Never) => "never",
        _ if after.is_some() => "member",
        _ => "off",
    })
}

struct WorkAt<'a> {
    steps: &'a [String],
    live: Option<bool>,
    as_of: u128,
}

fn refusal_at(connection: &Connection, subject: &str, cut: u64, after: Option<u64>, expected_incarnation: Option<&str>, work: WorkAt<'_>) -> Result<Option<&'static str>> {
    match policy_at(connection, subject, cut, after)? {
        "never" => return Ok(Some("policy-never")),
        "off" => return Ok(Some("policy-off")),
        _ => {}
    }
    let desired = desired_row_at(connection, subject, Some(cut))?;
    if desired.as_ref().is_none_or(|row| row.kind != "agent") {
        return Ok(Some("runtime-inactive"));
    }
    if let Some(member) = desired.as_ref().and_then(|row| row.member.as_deref()) {
        let member = serde_json::from_str::<crate::model::MemberSpec>(member)?;
        if crate::native_resume::authored_session_selection(&member).is_some() {
            return Ok(Some("suspend-authored-session-selection"));
        }
    }
    let runtime = latest_actual_at(connection, subject, Some(cut))?;
    let Some(runtime) = runtime else { return Ok(Some("runtime-inactive")); };
    if runtime["status"] != "running" {
        return Ok(Some("runtime-inactive"));
    }
    let Some(incarnation) = runtime["incarnation_id"].as_str() else {
        return Ok(Some("runtime-inactive"));
    };
    let Some(harness) = current_harness_at(connection, subject, Some(cut))? else {
        return Ok(Some("harness-unobserved"));
    };
    if expected_incarnation.is_some_and(|expected| expected != incarnation) {
        return Ok(Some("runtime-replaced"));
    }
    if harness.driver.as_deref() != Some("omp") {
        return Ok(Some("driver-unsupported"));
    }
    if harness.incarnation_id != incarnation {
        return Ok(Some("harness-unobserved"));
    }
    if harness.blocked_on.as_deref() == Some("human")
        || harness.ask.as_deref().is_some_and(|ask| !matches!(ask, "" | "none" | "unknown"))
    {
        return Ok(Some("pending-ask"));
    }
    if harness.input_buffer.as_deref() == Some("nonempty") {
        return Ok(Some("unsent-input"));
    }
    if !matches!(harness.state.as_str(), "idle" | "ready") {
        return Ok(Some("harness-busy"));
    }
    let latest: Option<String> = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h9dc8df36b4a2 owner=fractal_store::residency TODO=trace-statement-and-value-representation */ &canonical_sql(
        "SELECT json_object('background_jobs',json_extract(body,'$.fields.background_jobs'),
            'quiescent',json_type(body,'$.fields.quiescent'),
            'blocking',json_extract(body,'$.fields.blocking'),
            'transport',json_extract(body,'$.fields.transport'))
         FROM claims WHERE subject=?1 AND kind='harness.observed'
         AND store_index<=?2 AND json_extract(body,'$.fields.incarnation_id')=?3
         ORDER BY CANONICAL_DESC(claims) LIMIT 1",
    ))?.query_row(/* sqlite-large-value: allow-for-now lv-h71dc350f789b owner=fractal_store::residency TODO=trace-statement-and-value-representation */ params![subject, cut, incarnation], |row| row.get(0)).optional()?;
    let Some(latest) = latest else { return Ok(Some("harness-unobserved")); };
    let report: Value = serde_json::from_str(&latest)?;
    let fields = report.get("fields").unwrap_or(&report);
    match fields.get("background_jobs").and_then(Value::as_u64) {
        None => return Ok(Some("background-jobs-unreported")),
        Some(0) => {}
        Some(_) => return Ok(Some("background-jobs-running")),
    }
    if fields["quiescent"] != "true" || !fields["blocking"].as_array().is_some_and(Vec::is_empty) {
        return Ok(Some("native-not-quiet"));
    }
    let readiness: Option<String> = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h3c04c7174fbc owner=fractal_store::residency TODO=trace-statement-and-value-representation */ &canonical_sql(
        "SELECT json_extract(body,'$.fields.code') FROM claims WHERE subject=?1
         AND store_index<=?2 AND kind='harness.diagnostic'
         AND json_extract(body,'$.fields.incarnation_id')=?3
         AND json_extract(body,'$.fields.code') IN ('omp-channel-ready','omp-channel-unready')
         ORDER BY CANONICAL_DESC(claims) LIMIT 1",
    ))?.query_row(/* sqlite-large-value: allow-for-now lv-hbfaf663e1973 owner=fractal_store::residency TODO=trace-statement-and-value-representation */ params![subject, cut, incarnation], |row| row.get(0)).optional()?;
    if fields["transport"] != "omp-channel" || readiness.as_deref() != Some("omp-channel-ready") {
        return Ok(Some("channel-unready"));
    }
    let binding: Option<(Option<String>, Option<String>)> = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h7b1756b2a4ba owner=fractal_store::residency TODO=trace-statement-and-value-representation */ &canonical_sql(
        "SELECT json_extract(body,'$.fields.harness'),json_extract(body,'$.fields.session_id')
         FROM claims WHERE subject=?1 AND kind='harness.session-file'
         AND store_index<=?2 AND json_extract(body,'$.fields.incarnation_id')=?3
         ORDER BY CANONICAL_DESC(claims) LIMIT 1",
    ))?.query_row(/* sqlite-large-value: allow-for-now lv-h22c40f569516 owner=fractal_store::residency TODO=trace-statement-and-value-representation */ params![subject, cut, incarnation], |row| Ok((row.get(0)?, row.get(1)?))).optional()?;
    if binding.as_ref().is_none_or(|(harness, session)| {
        harness.as_deref() != Some("omp") || session.as_deref().is_none_or(str::is_empty)
    }) {
        return Ok(Some("native-session-unbound"));
    }
    if !crate::store::subagents::open_subagents_at(
        connection, crate::store::subagents::Scope::SeatAt(subject, cut),
    )?.is_empty() {
        return Ok(Some("subagent-running"));
    }
    if match work.live { Some(active) => active, None => claimed_work_at(connection, subject, cut, work.steps, work.as_of)? } {
        return Ok(Some("claimed-work"));
    }
    Ok(None)
}

fn claimed_work_at(connection: &Connection, subject: &str, cut: u64, steps: &[String], as_of: u128) -> Result<bool> {
    let mut history = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h1f92ed1a5e88 owner=fractal_store::residency TODO=trace-statement-and-value-representation */ &canonical_sql(
        "SELECT json_extract(body,'$.fields.status'),
                json_type(body,'$.fields.claimant') IS NOT NULL,
                json_extract(body,'$.fields.claimant'),
                json_type(body,'$.fields.claim_expires_at_unix_ms') IS NOT NULL,
                CAST(json_extract(body,'$.fields.claim_expires_at_unix_ms') AS TEXT)
         FROM claims WHERE subject=?1 AND store_index<=?2
         AND (kind LIKE 'work.%' OR kind LIKE 'step-run.%')
         ORDER BY CANONICAL_DESC(claims)",
    ))?;
    for step in steps {
        smallclaims::touched::note_read(|| step.clone());
        let mut status = None;
        let mut claimant = None;
        let mut expiry = None;
        for row in history.query_map(params![step, cut], |row| Ok((
            row.get::<_, Option<String>>(0)?, row.get::<_, bool>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, bool>(3)?, row.get::<_, Option<String>>(4)?,
        )))? {
            let (next_status, has_claimant, next_claimant, has_expiry, next_expiry) = row?;
            if status.is_none() { status = next_status; }
            if claimant.is_none() && has_claimant { claimant = Some(next_claimant); }
            if expiry.is_none() && has_expiry {
                expiry = Some(next_expiry.map(|value| value.parse::<u128>()).transpose()?);
            }
            if status.is_some() && claimant.is_some() && expiry.is_some() { break; }
        }
        if claimant.flatten().as_deref() == Some(subject)
            && status.as_deref().is_some_and(|status| matches!(status, "claimed" | "working" | "verifying"))
            && expiry.flatten().is_none_or(|expiry| expiry > as_of)
        { return Ok(true); }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ClaimInput, ClaimRecord};
    use serde_json::json;
    use std::collections::BTreeMap;

    const SEAT: &str = "agent/node.garden";
    static WRITE_CLOCK: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1_800_000_000_000);

    fn append(store: &Store, subject: &str, kind: &str, fields: Value) -> ClaimRecord {
        // Keep acceptance order distinct while honoring an explicitly advanced lease clock.
        let now = u64::try_from(smallclaims::store::now_ms()).unwrap();
        let previous = WRITE_CLOCK.fetch_update(
            std::sync::atomic::Ordering::SeqCst,
            std::sync::atomic::Ordering::SeqCst,
            |previous| Some(previous.max(now).saturating_add(1)),
        ).unwrap();
        store.set_write_clock_at(previous.max(now).into()).unwrap();
        store.append_claim(&ClaimInput {
            subject: subject.into(), kind: kind.into(), actor: Some(SEAT.into()),
            fields: serde_json::from_value(fields).unwrap(), evidence: vec![],
            expected_subject: None, idempotency_key: None,
        }).unwrap()
    }

    fn fixture(policy: &str) -> Store {
        let store = Store::open_memory("node").unwrap();
        let intent = crate::parse_intent(&format!(
            "version 2\nagent \"garden\" {{ workspace \".\"; command \"true\"; idle-suspend \"{policy}\" }}"
        ), "node").unwrap();
        store.apply(&intent, &BTreeMap::from([(SEAT.into(), Vec::new())]), "fixture").unwrap();
        append(&store, SEAT, "runtime.observed", json!({"status":"running","incarnation_id":"one"}));
        append(&store, SEAT, "harness.session-file", json!({
            "harness":"omp","session_id":"native-one","incarnation_id":"one","agent":SEAT
        }));
        append(&store, SEAT, "harness.diagnostic", json!({
            "code":"omp-channel-ready","driver":"omp","incarnation_id":"one","reason":"confirmed"
        }));
        quiet(&store, json!({}));
        store
    }

    fn quiet(store: &Store, changes: Value) -> ClaimRecord {
        let mut fields = json!({
            "driver":"omp", "transport":"omp-channel", "state":"idle",
            "incarnation_id":"one", "background_jobs":0,
            "quiescent":true, "blocking":[], "ask":"none", "blocked_on":"none",
            "input_buffer":"unknown",
            // Repeated observations must be distinct claims, not content-addressed replays of
            // the fixture's first quiet report (which would predate newly authored work).
            "reason":format!("fixture-observation-{}", WRITE_CLOCK.load(std::sync::atomic::Ordering::SeqCst))
        });
        fields.as_object_mut().unwrap().extend(changes.as_object().unwrap().clone());
        append(store, SEAT, "harness.observed", fields)
    }

    #[test]
    fn quiet_restatements_and_silent_mail_preserve_the_epoch_and_snapshot() {
        let store = fixture("member");
        let first = evaluate_at(&store, SEAT, None, Some(500)).unwrap();
        assert!(first.eligible_since_unix_ms.is_some(), "a zero-work seat is eligible");
        let cut = store.index().unwrap();
        append(&store, SEAT, "harness.diagnostic", json!({"code":"fixture-note","reason":"silent mail does not prove activity"}));
        append(&store, "message/silent", "message.sent", json!({
            "from":SEAT,"to":SEAT,"content":"For later.","status":"sent","tags":["st3-silent"]
        }));
        for _ in 0..32 {
            quiet(&store, json!({}));
        }
        assert_eq!(evaluate_at(&store, SEAT, None, Some(500)).unwrap(), first);
        quiet(&store, json!({"state":"working","quiescent":false,"blocking":["turn-in-flight"]}));
        assert_eq!(evaluate_at(&store, SEAT, Some(cut), Some(500)).unwrap(), first);
        assert_eq!(evaluate_at(&store, SEAT, None, Some(500)).unwrap().eligible_since_unix_ms, None);
        let opened = quiet(&store, json!({}));
        let next = evaluate_at(&store, SEAT, None, Some(500)).unwrap();
        assert_eq!(next.eligible_since_unix_ms, Some(opened.accepted_at_unix_ms));
        assert_eq!(next.due_at_unix_ms, Some(opened.accepted_at_unix_ms + 500));
    }

    #[test]
    fn unknown_and_native_blockers_clear_until_fresh_proof() {
        let store = fixture("member");
        for (change, reason) in [
            (json!({"transport":"native"}), "channel-unready"),
            (json!({"background_jobs":null}), "background-jobs-unreported"),
            (json!({"background_jobs":1}), "background-jobs-running"),
            (json!({"ask":"permission"}), "pending-ask"),
            (json!({"input_buffer":"nonempty"}), "unsent-input"),
            (json!({"quiescent":null}), "native-not-quiet"),
            (json!({"driver":"claude"}), "driver-unsupported"),
        ] {
            quiet(&store, change);
            let blocked = evaluate_at(&store, SEAT, None, Some(5)).unwrap();
            assert_eq!(blocked.refusal_reason.as_deref(), Some(reason));
            assert_eq!(blocked.eligible_since_unix_ms, None);
            let opened = quiet(&store, json!({}));
            assert_eq!(evaluate_at(&store, SEAT, None, Some(5)).unwrap().eligible_since_unix_ms,
                Some(opened.accepted_at_unix_ms));
        }
    }

    #[test]
    fn work_completion_and_subagent_closure_open_new_intervals() {
        let store = fixture("member");
        append(&store, "step-run/fixture/work", "work.claimed", json!({
            "attempt":1,"status":"claimed","claimant":SEAT,"claim_incarnation":"one"
        }));
        quiet(&store, json!({}));
        assert_eq!(evaluate_at(&store, SEAT, Some(store.index().unwrap()), Some(5)).unwrap().refusal_reason.as_deref(), Some("claimed-work"));
        let completed = append(&store, "step-run/fixture/work", "step-run.state", json!({
            "status":"completed","attempt":1
        }));
        assert_eq!(evaluate_at(&store, SEAT, None, Some(5)).unwrap().eligible_since_unix_ms,
            Some(completed.accepted_at_unix_ms));
        append(&store, SEAT, "subagent.appeared", json!({
            "subagent_id":"helper","driver":"omp","incarnation_id":"one",
            "started_at_unix_ms":1,"lease_expires_at_unix_ms":9999999999999_u64
        }));
        let cut = store.index().unwrap();
        assert_eq!(evaluate_at(&store, SEAT, None, Some(5)).unwrap().refusal_reason.as_deref(), Some("subagent-running"));
        let ended = append(&store, SEAT, "subagent.ended", json!({"subagent_id":"helper","outcome":"completed","reason":"completed"}));
        assert_eq!(evaluate_at(&store, SEAT, None, Some(5)).unwrap().eligible_since_unix_ms,
            Some(ended.accepted_at_unix_ms));
        assert_eq!(evaluate_at(&store, SEAT, Some(cut), Some(5)).unwrap().refusal_reason.as_deref(), Some("subagent-running"));
    }

    #[test]
    fn disabled_opt_out_and_stopped_runtime_are_diagnostics() {
        let store = fixture("member");
        assert_eq!(evaluate_at(&store, SEAT, None, None).unwrap().refusal_reason.as_deref(), Some("policy-off"));
        append(&store, SEAT, "runtime.observed", json!({"status":"stopped","incarnation_id":"one"}));
        assert_eq!(evaluate_at(&store, SEAT, None, Some(5)).unwrap().refusal_reason.as_deref(), Some("runtime-inactive"));
        let never = fixture("never");
        for after in [None, Some(5)] {
            let result = evaluate_at(&never, SEAT, None, after).unwrap();
            assert_eq!(result.policy, "never");
            assert_eq!(result.refusal_reason.as_deref(), Some("policy-never"));
        }
    }

    #[test]
    fn channel_handshake_loss_and_runtime_replacement_fence_the_epoch() {
        let store = fixture("member");
        let first = evaluate_at(&store, SEAT, None, Some(50)).unwrap();
        let cut = store.index().unwrap();
        append(&store, SEAT, "harness.diagnostic", json!({
            "code":"omp-channel-unready","driver":"omp","incarnation_id":"one","reason":"reconnecting"
        }));
        quiet(&store, json!({}));
        assert_eq!(evaluate_at(&store, SEAT, None, Some(50)).unwrap().refusal_reason.as_deref(), Some("channel-unready"));
        assert_eq!(evaluate_at(&store, SEAT, Some(cut), Some(50)).unwrap(), first);
        let restored = append(&store, SEAT, "harness.diagnostic", json!({
            "code":"omp-channel-ready","driver":"omp","incarnation_id":"one","reason":"confirmed"
        }));
        assert_eq!(evaluate_at(&store, SEAT, None, Some(50)).unwrap().eligible_since_unix_ms,
            Some(restored.accepted_at_unix_ms));
        append(&store, SEAT, "runtime.observed", json!({"status":"running","incarnation_id":"two"}));
        append(&store, SEAT, "harness.session-file", json!({
            "harness":"omp","session_id":"native-two","incarnation_id":"two","agent":SEAT
        }));
        quiet(&store, json!({"incarnation_id":"two"}));
        assert_eq!(evaluate_at(&store, SEAT, None, Some(50)).unwrap().refusal_reason.as_deref(), Some("channel-unready"));
        let ready = append(&store, SEAT, "harness.diagnostic", json!({
            "code":"omp-channel-ready","driver":"omp","incarnation_id":"two","reason":"confirmed"
        }));
        assert_eq!(evaluate_at(&store, SEAT, None, Some(50)).unwrap().eligible_since_unix_ms,
            Some(ready.accepted_at_unix_ms));
    }

    #[test]
    fn readiness_does_not_substitute_for_the_durable_native_binding() {
        let store = fixture("member");
        append(&store, SEAT, "runtime.observed", json!({"status":"running","incarnation_id":"unbound"}));
        append(&store, SEAT, "harness.diagnostic", json!({
            "code":"omp-channel-ready","driver":"omp","incarnation_id":"unbound","reason":"confirmed"
        }));
        quiet(&store, json!({"incarnation_id":"unbound"}));
        assert_eq!(evaluate_at(&store, SEAT, None, Some(50)).unwrap().refusal_reason.as_deref(),
            Some("native-session-unbound"));
        let bound = append(&store, SEAT, "harness.session-file", json!({
            "harness":"omp","session_id":"native-bound","incarnation_id":"unbound","agent":SEAT
        }));
        assert_eq!(evaluate_at(&store, SEAT, None, Some(50)).unwrap().eligible_since_unix_ms,
            Some(bound.accepted_at_unix_ms));
    }

    #[test]
    fn activation_and_restart_clamps_never_create_eligibility() {
        let eligible = Residency {
            policy:"member".into(), eligible_since_unix_ms:Some(100),
            due_at_unix_ms:Some(150), refusal_reason:None,
        };
        let enabled = eligible.clone().not_before(Some(200), Some(50));
        assert_eq!(enabled.eligible_since_unix_ms, Some(200));
        assert_eq!(enabled.due_at_unix_ms, Some(250));
        let restarted = enabled.not_before(Some(300), Some(50));
        assert_eq!(restarted.eligible_since_unix_ms, Some(300));
        assert_eq!(restarted.due_at_unix_ms, Some(350));
        assert_eq!(eligible.clone().not_before(Some(50), Some(50)), eligible);
        let blocked = Residency {
            policy:"member".into(), eligible_since_unix_ms:None,
            due_at_unix_ms:None, refusal_reason:Some("claimed-work".into()),
        };
        assert_eq!(blocked.clone().not_before(Some(500), Some(50)), blocked);
    }

    struct Clock;
    impl Drop for Clock {
        fn drop(&mut self) { smallclaims::store::set_thread_clock(None); }
    }

    #[test]
    fn retained_lease_expiry_opens_at_the_exact_clock_boundary() {
        const START: u128 = 1_800_000_000_000;
        let _clock = Clock;
        smallclaims::store::set_thread_clock(Some(START));
        let store = fixture("member");
        smallclaims::store::set_thread_clock(Some(START + 10));
        append(&store, "step-run/lease/work", "work.claimed", json!({
            "status":"claimed","claimant":SEAT,"claim_incarnation":"one",
            "claim_expires_at_unix_ms":START + 100
        }));
        smallclaims::store::set_thread_clock(Some(START + 20));
        quiet(&store, json!({}));
        let before = store.index().unwrap();
        assert_eq!(evaluate_at(&store, SEAT, Some(before), Some(50)).unwrap().refusal_reason.as_deref(),
            Some("claimed-work"));
        smallclaims::store::set_thread_clock(Some(START + 100));
        let expired = evaluate_at(&store, SEAT, None, Some(50)).unwrap();
        assert_eq!(expired.eligible_since_unix_ms, Some(START + 100));
        assert_eq!(expired.due_at_unix_ms, Some(START + 150));
        // The historical claim cut still precedes the clock edge.
        assert_eq!(evaluate_at(&store, SEAT, Some(before), Some(50)).unwrap().refusal_reason.as_deref(),
            Some("claimed-work"));
        quiet(&store, json!({"ask":"permission"}));
        assert_eq!(evaluate_at(&store, SEAT, None, Some(50)).unwrap().eligible_since_unix_ms, None);
        smallclaims::store::set_thread_clock(Some(START + 110));
        let opened = quiet(&store, json!({}));
        assert_eq!(evaluate_at(&store, SEAT, None, Some(50)).unwrap().eligible_since_unix_ms,
            Some(opened.accepted_at_unix_ms));
    }

    #[test]
    fn operator_authored_carried_ownership_remains_visible_at_old_snapshots() {
        let store = fixture("member");
        store.append_claim(&ClaimInput {
            subject:"step-run/carried/work".into(), kind:"step-run.carried".into(),
            actor:Some("person/operator".into()),
            fields:serde_json::from_value(json!({
                "status":"working","claimant":SEAT,"claim_incarnation":"one","attempt":1
            })).unwrap(), evidence:vec![], expected_subject:None, idempotency_key:None,
        }).unwrap();
        let cut = store.index().unwrap();
        append(&store, "step-run/carried/work", "step-run.state", json!({"status":"completed","attempt":1}));
        assert_eq!(evaluate_at(&store, SEAT, Some(cut), Some(50)).unwrap().refusal_reason.as_deref(),
            Some("claimed-work"));
    }

    #[test]
    fn renewed_leases_do_not_open_at_superseded_expiration_times() {
        const START: u128 = 1_800_000_000_000;
        let _clock = Clock;
        smallclaims::store::set_thread_clock(Some(START));
        let store = fixture("member");
        smallclaims::store::set_thread_clock(Some(START + 10));
        append(&store, "step-run/renewed/work", "work.claimed", json!({
            "status":"claimed","claimant":SEAT,"claim_incarnation":"one",
            "claim_expires_at_unix_ms":START + 100
        }));
        smallclaims::store::set_thread_clock(Some(START + 30));
        append(&store, "step-run/renewed/work", "work.renewed", json!({
            "status":"claimed","claimant":SEAT,"claim_incarnation":"one",
            "claim_expires_at_unix_ms":START + 200
        }));
        smallclaims::store::set_thread_clock(Some(START + 40));
        quiet(&store, json!({}));
        smallclaims::store::set_thread_clock(Some(START + 150));
        assert_eq!(evaluate_at(&store, SEAT, None, Some(50)).unwrap().eligible_since_unix_ms, None);
        smallclaims::store::set_thread_clock(Some(START + 200));
        let expired = evaluate_at(&store, SEAT, None, Some(50)).unwrap();
        assert_eq!(expired.eligible_since_unix_ms, Some(START + 200));
        assert_eq!(expired.due_at_unix_ms, Some(START + 250));
    }
}
