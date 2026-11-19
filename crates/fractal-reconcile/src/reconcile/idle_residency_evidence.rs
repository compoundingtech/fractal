//! Runtime residency evidence. Action receipts are authoritative; no seat polling or exporter I/O.
//! Every evidence identity and event time survives retries and daemon restarts.
use std::collections::BTreeMap;

use anyhow::{Context, Result};
use fractal_telemetry::{residency::{Instruments, Summary, SuspendResult, WakeOutcome, WakeSource}, residency_host_sample::{self as sensor, HeldSilent, Memory, MemoryDelta}};
use serde_json::{Value, json};

use crate::{model::{ClaimInput, ClaimRecord}, store::Store};

const OUTCOME_MS: u128 = 10 * 60 * 1000;
const SAMPLE_MS: u128 = 5 * 60 * 1000;

#[derive(Default)]
pub(super) struct RuntimeEvidence {
    instruments: Option<Instruments>,
    before: BTreeMap<String, Memory>,
    reclaimed: Option<MemoryDelta>,
}

fn fields(claim: &ClaimRecord) -> &Value { &claim.body["fields"] }
fn cites(claim: &ClaimRecord, id: &str) -> bool {
    claim.body["evidence"].as_array().is_some_and(|ids| ids.iter().any(|value| value.as_str() == Some(id)))
        || fields(claim)["operation_id"].as_str() == Some(id)
}
fn episode(host: &str, class: &str, key: &str) -> String {
    let id = smallclaims::store::operation_id_for_key(key);
    format!("residency/{host}/{class}/{}", id.trim_start_matches("op/"))
}
fn common(host: &str, seat: &str, values: Value) -> BTreeMap<String, Value> {
    let mut fields: BTreeMap<String, Value> = serde_json::from_value(values).expect("residency fields are an object");
    fields.insert("host".into(), host.into());
    fields.insert("seat".into(), seat.into());
    fields
}

/// Unlike record_once's newest-value deduplication, this is a permanent operation receipt.
fn receipt(store: &Store, subject: &str, kind: &str, fields: BTreeMap<String, Value>, evidence: Vec<String>, key: String) -> Result<bool> {
    match store.append_residency_evidence(&ClaimInput {
        subject: subject.into(), kind: kind.into(), actor: None, fields, evidence,
        expected_subject: None, idempotency_key: Some(key),
    }) {
        Ok(inserted) => Ok(inserted),
        Err(error) if error.code == "claim-checkpointed" => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn outcome(now: u128, resumed: u128, first_turn: Option<u128>) -> Option<bool> {
    let due = resumed.saturating_add(OUTCOME_MS);
    (now >= due).then(|| first_turn.is_some_and(|turn| turn >= resumed && turn <= due))
}

fn source(store: &Store, demand: &ClaimRecord) -> Result<Option<WakeSource>> {
    let Some(message) = fields(demand)["source_message"].as_str() else { return Ok(None); };
    let Some(message) = store.message(message)? else { return Ok(None); };
    Ok(if message.from.starts_with("github/") || message.tags.iter().any(|tag| tag == "github" || tag.starts_with("github:")) {
        Some(WakeSource::Github)
    } else if message.from.starts_with("person/") || message.from.starts_with("external/") {
        Some(WakeSource::Person)
    } else if message.from.starts_with("agent/") {
        Some(WakeSource::Agent)
    } else if message.from.starts_with("daemon/") {
        Some(WakeSource::Daemon)
    } else { None })
}

impl RuntimeEvidence {
    fn activate(&mut self) -> &Instruments {
        self.instruments.get_or_insert_with(|| Instruments::new(&opentelemetry::global::meter("fractal.residency")))
    }

    /// Call once at the last reversible stop boundary, before stopping the process.
    /// The runtime supplies the root's verified PID and birth token. The sensor
    /// walks every thread's children and refuses incomplete or changing trees.
    pub(super) fn before_suspend(&mut self, request: &str, root: Option<(u32, u64)>) {
        self.before.entry(request.into()).or_insert_with(|| sensor::memory_tree(root));
    }

    /// Invoke after phase receipts and from the seat's normal reactive item. Replays are safe.
    pub(super) fn evaluate_subject(&mut self, store: &Store, seat: &str, now: u128) -> Result<()> {
        let host = store.origin();
        let requests = store.claims_for(seat, Some("runtime.action.requested"))?;
        let succeeded = store.claims_for(seat, Some("runtime.action.succeeded"))?;
        let failed = store.claims_for(seat, Some("runtime.action.failed"))?;
        for request in &requests {
            let request_fields = fields(request);
            if request_fields["cause"] != "idle" { continue; }
            if request_fields["action"] == "suspend" {
                let success = succeeded.iter().find(|claim| cites(claim, &request.id) && fields(claim)["operation_status"] == "suspended");
                let failure = failed.iter().find(|claim| cites(claim, &request.id));
                let Some(terminal) = success.or(failure) else { continue; };
                let code = fields(terminal)["code"].as_str().unwrap_or("other");
                let cancelled = failure.is_some() && matches!(code, "idle-cancelled" | "wake-pending" | "eligibility-changed" | "channel-unready");
                let result = if success.is_some() { SuspendResult::Suspended } else if cancelled { SuspendResult::Cancelled } else { SuspendResult::Refused };
                let reason = if success.is_some() { "suspended" } else if cancelled { "cancelled" } else { code };
                let key = format!("runtime-residency:eligibility:{}", request.id);
                let subject = episode(host, "eligibility", &key);
                let since = request_fields["idle_eligible_since_unix_ms"].as_u64().map(u128::from).unwrap_or(request.accepted_at_unix_ms);
                if receipt(store, &subject, "residency.eligibility-outcome", common(host, seat, json!({
                    "eligible_since":since,"decided_at":terminal.accepted_at_unix_ms,
                    "refused":u8::from(success.is_none()),"reason":reason,
                })), vec![request.id.clone(), terminal.id.clone()], key)? {
                    self.activate().suspend(result);
                    if failure.is_some() { self.activate().refusal(code); }
                }
                if let Some(before) = self.before.remove(&request.id) {
                    // A suspended success is the completed whole-seat stop, not
                    // a missing post-stop /proc entry treated as a zero sample.
                    if success.is_some() { self.reclaimed = Some(MemoryDelta::after_completed_stop(before, sensor::memory(&[]))); }
                }
                if let Some(completed) = success {
                    self.sleep_end(store, seat, request, completed, &requests, &succeeded)?;
                }
                continue;
            }
            if request_fields["action"] != "resume" { continue; }
            let Some(lineage) = request_fields["idle_wake_lineage"].as_str() else { continue; };
            let wake = crate::store::idle_residency::wake_evidence_subject(host, seat, lineage, 0);
            let Some(demand) = store.latest_claim(&wake, Some("residency.wake-demand"))? else { continue; };
            let failure = failed.iter().find(|claim| cites(claim, &request.id));
            let resumed = succeeded.iter().find(|claim| cites(claim, &request.id) && fields(claim)["operation_status"] == "resumed");
            let outcome_subject = episode(host, "outcome", &request.id);
            if let Some(failure) = failure {
                let code = fields(failure)["code"].as_str().unwrap_or("other");
                if receipt(store, &outcome_subject, "residency.outcome", common(host, seat, json!({
                    "observed_at":failure.accepted_at_unix_ms,"resume_failed":1,"useless":1,"reason":code,
                })), vec![request.id.clone(), failure.id.clone(), demand.id.clone()], format!("runtime-residency:outcome:{}", request.id))? {
                    self.activate().resume_failure(code);
                    if let Some(source) = source(store, &demand)? { self.activate().wake(source, WakeOutcome::NoTurn); }
                }
                continue;
            }
            let Some(resumed) = resumed else { continue; };
            let Some(native) = fields(resumed)["native_session_id"].as_str() else { continue; };
            let Some(incarnation) = fields(resumed)["incarnation_id"].as_str() else { continue; };
            receipt(store, &wake, "residency.resumed", common(host, seat, json!({
                "resumed_at":resumed.accepted_at_unix_ms,"native_session":native,"incarnation":incarnation,
            })), vec![demand.id.clone(), resumed.id.clone()], format!("runtime-residency:resumed:{}", request.id))?;
            let first = store.latest_claim(&wake, Some("residency.first-turn"))?;
            let first_at = if let Some(first) = first {
                fields(&first)["first_turn_at"].as_u64().map(u128::from)
            } else {
                self.first_turn(store, seat, &wake, resumed, native, incarnation)?
            };
            if let Some(turn) = outcome(now, resumed.accepted_at_unix_ms, first_at) {
                let due = resumed.accepted_at_unix_ms.saturating_add(OUTCOME_MS);
                if receipt(store, &outcome_subject, "residency.outcome", common(host, seat, json!({
                    "observed_at":due,"resume_failed":0,"useless":u8::from(!turn),"reason":if turn {"turn"} else {"no-turn"},
                })), vec![request.id.clone(), resumed.id.clone(), demand.id.clone()], format!("runtime-residency:outcome:{}", request.id))?
                    && let Some(source) = source(store, &demand)?
                {
                    self.activate().wake(source, if turn { WakeOutcome::Turn } else { WakeOutcome::NoTurn });
                }
            } else {
                smallclaims::touched::note_due(resumed.accepted_at_unix_ms.saturating_add(OUTCOME_MS));
            }
        }
        Ok(())
    }

    fn first_turn(&mut self, store: &Store, seat: &str, wake: &str, resumed: &ClaimRecord, native: &str, incarnation: &str) -> Result<Option<u128>> {
        let host = store.origin();
        // A native working observation identifies a turn, unlike PTY output, delivery, or process liveness.
        // Bind it to both the verified resume incarnation and the exact durable native session.
        let bound = store.claims_for(seat, Some("harness.session-file"))?.iter().any(|claim|
            fields(claim)["incarnation_id"] == incarnation && fields(claim)["session_id"] == native);
        if !bound { return Ok(None); }
        let reports = store.observations_for(seat, "harness.observed")?;
        let first = reports.iter().filter(|claim| {
            fields(claim)["incarnation_id"] == incarnation && fields(claim)["state"] == "active"
                && claim.accepted_at_unix_ms >= resumed.accepted_at_unix_ms
        }).min_by_key(|claim| (claim.accepted_at_unix_ms, &claim.id));
        let Some(first) = first else { return Ok(None); };
        let at = first.accepted_at_unix_ms;
        // The native observation's immutable ID is the turn evidence identity. No generated counter,
        // inferred message delivery, or mutable latest runtime identity participates in correlation.
        receipt(store, wake, "residency.first-turn", common(host, seat, json!({
            "first_turn_at":at,"turn_id":first.id,
        })), vec![resumed.id.clone(), first.id.clone()], format!("runtime-residency:first-turn:{}", resumed.id))?;
        Ok(Some(at))
    }

    fn sleep_end(&mut self, store: &Store, seat: &str, request: &ClaimRecord, suspended: &ClaimRecord, requests: &[ClaimRecord], succeeded: &[ClaimRecord]) -> Result<()> {
        let host = store.origin();
        let subject = episode(host, "sleep", &request.id);
        if store.latest_claim(&subject, Some("residency.sleep-ended"))?.is_some() {
            return Ok(());
        }
        // Failed resume attempts do not end a sleep. A successful manual resume, stop, or retire
        // does; the first end remains the same across later requests and daemon restarts.
        let end = requests.iter().filter_map(|candidate| {
            if candidate.accepted_at_unix_ms < suspended.accepted_at_unix_ms { return None; }
            let action = fields(candidate)["action"].as_str()?;
            if !matches!(action, "resume" | "stop" | "retire") { return None; }
            let completed = succeeded.iter().find(|claim| cites(claim, &candidate.id)
                && (action != "resume" || fields(claim)["operation_status"] == "resumed"))?;
            let class = if action == "resume" { if fields(candidate)["cause"] == "idle" { "wake" } else { "manual" } } else { action };
            Some((completed.accepted_at_unix_ms, class, completed))
        }).min_by_key(|(at, _, claim)| (*at, &claim.id));
        let Some((ended, class, completed)) = end else { return Ok(()); };
        let suspended_at = fields(suspended)["suspended_at_unix_ms"].as_u64().map(u128::from).unwrap_or(suspended.accepted_at_unix_ms);
        let short = class == "wake" && fields(request)["idle_after_ms"].as_u64()
            .is_some_and(|ttl| ended.saturating_sub(suspended_at) < u128::from(ttl));
        let key = format!("runtime-residency:sleep:{}", request.id);
        if receipt(store, &subject, "residency.sleep-ended", common(host, seat, json!({
            "suspended_at":suspended_at,"ended_at":ended,"short_sleep":u8::from(short),"end_class":class,
        })), vec![request.id.clone(), suspended.id.clone(), completed.id.clone()], key)? && short {
            self.activate().short_sleep();
        }
        Ok(())
    }

    /// Stop and retire end an idle sleep without a runtime action receipt of their own.
    /// Replays and reopenings keep the first end; the metric fires only on the durable insert.
    pub(super) fn end_sleep(&mut self, store: &Store, seat: &str,
        suspension: &crate::suspension::Suspension, ended: u128, end_class: &str,
    ) -> Result<()> {
        // An idle request may have failed or been cancelled before the process stopped.
        // Such an attempt has no sleep episode for a later explicit stop to end.
        let Some(suspended_at) = suspension.suspended_at_unix_ms else { return Ok(()); };
        let host = store.origin();
        let requests = store.claims_for(seat, Some("runtime.action.requested"))?;
        let Some(request) = requests.iter().rev().find(|claim|
            claim.id == suspension.suspend_operation_id.as_deref().unwrap_or(&suspension.operation_id))
        else { return Ok(()); };
        let subject = episode(host, "sleep", &request.id);
        if store.latest_claim(&subject, Some("residency.sleep-ended"))?.is_some() {
            return Ok(());
        }
        let short = end_class == "wake" && fields(request)["idle_after_ms"].as_u64()
            .is_some_and(|ttl| ended.saturating_sub(suspended_at) < u128::from(ttl));
        let key = format!("runtime-residency:sleep:{}", request.id);
        if receipt(store, &subject, "residency.sleep-ended", common(host, seat, json!({
            "suspended_at":suspended_at,"ended_at":ended,"short_sleep":u8::from(short),"end_class":end_class,
        })), vec![request.id.clone()], key)? && short {
            self.activate().short_sleep();
        }
        Ok(())
    }

    /// Host reactive item: recover only indexed action episodes, not the agent roster. The
    /// exporter callbacks only read the Summary published here. Disabled hosts perform no sensor I/O.
    pub(super) fn evaluate_host(&mut self, store: &Store, enabled: bool, now: u128) -> Result<()> {
        for kind in ["runtime.action.requested", "runtime.action.succeeded", "runtime.action.failed", "harness.observed", "harness.session-file"] {
            smallclaims::touched::note_read(|| format!("kind:{kind}"));
        }
        let host = store.origin();
        let seats = store.residency_episode_seats()?;
        let mut idle = 0;
        let mut manual = 0;
        for seat in seats {
            self.evaluate_subject(store, &seat, now)?;
            if let Some(state) = crate::suspension::current(store, &seat)?
                && (state.phase == "suspended" || (state.action == "resume" && state.phase == "failed"))
            {
                if state.cause() == "idle" { idle += 1; } else { manual += 1; }
            }
        }
        if !enabled {
            if let Some(instruments) = &self.instruments { instruments.publish(Summary::default()); }
            return Ok(());
        }
        self.activate();
        let (caught_up, oldest_wake, oldest_silent) = store.idle_residency_mail_summary()?;
        let now_i64 = i64::try_from(now).context("residency sample clock exceeds signed milliseconds")?;
        let age = |at: Option<i64>| at.filter(|at| caught_up && *at <= now_i64).map(|at| (now_i64 - at) as f64 / 1000.);
        let delta = self.reclaimed.unwrap_or_else(|| MemoryDelta::between(Memory::default(), Memory::default()));
        self.activate().publish(Summary {
            idle_suspended: Some(idle), manual_suspended: Some(manual), oldest_wake_seconds: age(oldest_wake),
            oldest_silent_seconds: if caught_up {
                HeldSilent::CaughtUp { oldest_accepted_ms: oldest_silent }.age_seconds(now_i64)
            } else { None }, host_available_delta_bytes: delta.host_available_delta_bytes,
            private_reclaimed_bytes: delta.private_reclaimed_bytes,
        });
        let subject = format!("residency/{host}/host/{host}");
        let last = store.latest_claim(&subject, Some("residency.host-sample"))?;
        let due = last.as_ref().and_then(|claim| fields(claim)["observed_at"].as_u64()).map(|at| u128::from(at).saturating_add(SAMPLE_MS));
        if due.is_none_or(|due| now >= due) {
            // The scheduled slot, not the evaluation time, identifies the sample: a retried or
            // late reactive pass repeats the same subject, event time and receipt key.
            let at = due.filter(|due| now >= *due).unwrap_or(now);
            let held = if caught_up { HeldSilent::CaughtUp { oldest_accepted_ms: oldest_silent } } else { HeldSilent::Unavailable };
            let sample = sensor::sample(true, now_i64, held, idle, &[]).expect("enabled sensor");
            let mut values = BTreeMap::from([
                ("host".into(), Value::from(host)), ("observed_at".into(), json!(at)),
                ("asleep_seats".into(), json!(sample.asleep_seats)), ("memory_coverage".into(), json!(delta.coverage().label())),
            ]);
            if let Some(age) = sample.held_silent_age_seconds { values.insert("held_silent_age_seconds".into(), json!(age)); }
            if let Some(bytes) = delta.host_available_delta_bytes { values.insert("host_available_delta_bytes".into(), json!(bytes)); }
            if let Some(bytes) = delta.private_reclaimed_bytes { values.insert("private_reclaimed_bytes".into(), json!(bytes)); }
            receipt(store, &subject, "residency.host-sample", values, Vec::new(), format!("runtime-residency:host:{host}:{at}"))?;
            smallclaims::touched::note_due(at.saturating_add(SAMPLE_MS));
        } else if let Some(due) = due { smallclaims::touched::note_due(due); }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcome_uses_the_fixed_resume_deadline_and_rejects_late_turns() {
        assert_eq!(outcome(OUTCOME_MS - 1, 0, None), None);
        assert_eq!(outcome(OUTCOME_MS, 0, None), Some(false));
        assert_eq!(outcome(OUTCOME_MS, 0, Some(OUTCOME_MS)), Some(true));
        assert_eq!(outcome(OUTCOME_MS + 5, 0, Some(OUTCOME_MS + 1)), Some(false));
        assert_eq!(outcome(OUTCOME_MS + 10, 10, Some(9)), Some(false));
    }

    fn signed(path: &std::path::Path) -> Store {
        let store = Store::open(path, "amber").unwrap();
        let key = std::sync::Arc::new(crate::fleet::MemberKey::generate().unwrap().0);
        const FLEET: &str = "7c9e6679-7425-40de-944b-e07fc1f90ae7";
        store.bind_fleet(FLEET).unwrap();
        store.pin_fleet_anchor(key.public()).unwrap();
        store.set_member_key(Some(key.clone())).unwrap();
        store.admit_fleet_anchor(FLEET, key.public(), "listening").unwrap();
        store.append_claim(&ClaimInput {
            subject: "daemon/amber".into(), kind: "daemon.started".into(), actor: None,
            fields: BTreeMap::from([("status".into(), json!("running")), ("features".into(), json!({"idle_residency":1}))]),
            evidence: Vec::new(), expected_subject: None, idempotency_key: None,
        }).unwrap();
        store.replication_snapshot().unwrap();
        store
    }

    #[test]
    fn receipts_deduplicate_after_reopen_without_overwriting_event_time() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("graph.db");
        let subject = episode("amber", "outcome", "resume-one");
        let input = || common("amber", "agent/amber.worker", json!({"observed_at":100,"resume_failed":0,"useless":1,"reason":"no-turn"}));
        let store = signed(&path);
        assert!(receipt(&store, &subject, "residency.outcome", input(), vec![], "one-outcome".into()).unwrap());
        assert!(!receipt(&store, &subject, "residency.outcome", input(), vec![], "one-outcome".into()).unwrap());
        drop(store);
        let store = Store::open(&path, "amber").unwrap();
        assert!(!receipt(&store, &subject, "residency.outcome", input(), vec![], "one-outcome".into()).unwrap());
        let claims = store.claims_for(&subject, Some("residency.outcome")).unwrap();
        assert_eq!(claims.len(), 1);
        assert_eq!(fields(&claims[0])["observed_at"], 100);
    }

    fn action_claim(store: &Store, kind: &str, values: Value, evidence: Vec<String>) -> ClaimRecord {
        store.append_claim(&ClaimInput {
            // Actor-less runtime actions are local observations, not durable action receipts.
            subject: "agent/amber.worker".into(), kind: kind.into(), actor: Some("person/test".into()),
            fields: serde_json::from_value(values).unwrap(), evidence,
            expected_subject: None, idempotency_key: None,
        }).unwrap()
    }

    #[test]
    fn explicit_sleep_end_keeps_its_first_time_after_restart_for_stop_and_retire() {
        for end_class in ["stop", "retire"] {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join("graph.db");
            let store = signed(&path);
            let request = action_claim(&store, "runtime.action.requested",
                json!({"action":"suspend","cause":"idle","idle_after_ms":100}), vec![]);
            let suspended = action_claim(&store, "runtime.action.succeeded",
                json!({"action":"suspend","operation_status":"suspended"}), vec![request.id.clone()]);
            let hold = crate::suspension::Suspension {
                action: "suspend".into(), cause: Some("idle".into()), phase: "suspended".into(),
                operation_id: request.id.clone(), suspended_at_unix_ms: Some(suspended.accepted_at_unix_ms),
                ..Default::default()
            };
            let ended = suspended.accepted_at_unix_ms + 10;
            store.idle_residency_enabled().unwrap();
            RuntimeEvidence::default().end_sleep(&store, "agent/amber.worker", &hold, ended, end_class).unwrap();
            assert_eq!(store.claims_for(&episode("amber", "sleep", &request.id),
                Some("residency.sleep-ended")).unwrap().len(), 1);
            drop(store);
            let store = Store::open(&path, "amber").unwrap();
            RuntimeEvidence::default().end_sleep(&store, "agent/amber.worker", &hold, ended + 1000, end_class).unwrap();
            let claims = store.claims_for(&episode("amber", "sleep", &request.id), Some("residency.sleep-ended")).unwrap();
            assert_eq!(claims.len(), 1);
            assert_eq!(fields(&claims[0])["ended_at"], json!(ended));
            assert_eq!(fields(&claims[0])["end_class"], end_class);
        }
    }

    #[test]
    fn cancelled_or_failed_idle_attempt_followed_by_explicit_stop_has_no_sleep_end() {
        for code in ["idle-cancelled", "driver-unsupported"] {
            let root = tempfile::tempdir().unwrap();
            let store = signed(&root.path().join("graph.db"));
            let request = action_claim(&store, "runtime.action.requested",
                json!({"action":"suspend","cause":"idle"}), vec![]);
            action_claim(&store, "runtime.action.failed",
                json!({"action":"suspend","code":code,"reason":"suspension did not complete"}), vec![request.id.clone()]);
            let hold = crate::suspension::Suspension {
                action: "suspend".into(), cause: Some("idle".into()), phase: "failed".into(),
                operation_id: request.id.clone(), code: Some(code.into()), ..Default::default()
            };
            RuntimeEvidence::default().end_sleep(&store, "agent/amber.worker", &hold,
                request.accepted_at_unix_ms + 1000, "stop").unwrap();
            RuntimeEvidence::default().evaluate_subject(&store, "agent/amber.worker",
                request.accepted_at_unix_ms + 2000).unwrap();
            assert!(store.claims_for(&episode("amber", "sleep", &request.id),
                Some("residency.sleep-ended")).unwrap().is_empty());
        }
    }

    #[test]
    fn manual_resume_and_retire_replay_preserve_the_original_sleep_end() {
        for (action, end_class) in [("resume", "manual"), ("retire", "retire")] {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join("graph.db");
            let store = signed(&path);
            let request = action_claim(&store, "runtime.action.requested",
                json!({"action":"suspend","cause":"idle","idle_after_ms":100}), vec![]);
            let suspended = action_claim(&store, "runtime.action.succeeded",
                json!({"action":"suspend","operation_status":"suspended"}), vec![request.id.clone()]);
            let end_request = action_claim(&store, "runtime.action.requested",
                json!({"action":action}), vec![]);
            let completed = action_claim(&store, "runtime.action.succeeded",
                json!({"action":action,"operation_status":if action == "resume" {"resumed"} else {"stopped"}}),
                vec![end_request.id.clone()]);
            store.idle_residency_enabled().unwrap();
            RuntimeEvidence::default().evaluate_subject(&store, "agent/amber.worker",
                completed.accepted_at_unix_ms).unwrap();
            assert_eq!(store.claims_for(&episode("amber", "sleep", &request.id),
                Some("residency.sleep-ended")).unwrap().len(), 1);
            drop(store);
            let store = Store::open(&path, "amber").unwrap();
            let requests = store.claims_for("agent/amber.worker", Some("runtime.action.requested")).unwrap();
            let succeeded = store.claims_for("agent/amber.worker", Some("runtime.action.succeeded")).unwrap();
            RuntimeEvidence::default().sleep_end(&store, "agent/amber.worker",
                &request, &suspended, &requests, &succeeded).unwrap();
            let hold = crate::suspension::Suspension {
                action: "resume".into(), cause: Some("idle".into()), phase: "resumed".into(),
                operation_id: end_request.id.clone(), suspend_operation_id: Some(request.id.clone()),
                suspended_at_unix_ms: Some(suspended.accepted_at_unix_ms), ..Default::default()
            };
            RuntimeEvidence::default().end_sleep(&store, "agent/amber.worker", &hold,
                completed.accepted_at_unix_ms + 1000, "stop").unwrap();
            let claims = store.claims_for(&episode("amber", "sleep", &request.id), Some("residency.sleep-ended")).unwrap();
            assert_eq!(claims.len(), 1);
            assert_eq!(fields(&claims[0])["ended_at"], json!(completed.accepted_at_unix_ms));
            assert_eq!(fields(&claims[0])["end_class"], end_class);
            assert_eq!(fields(&claims[0])["short_sleep"], 0);
        }
    }

    #[test]
    fn receipts_fail_closed_without_the_signed_fleet_barrier() {
        let store = Store::open_memory("amber").unwrap();
        assert!(store.idle_residency_enabled().is_err());
        let subject = episode("amber", "sleep", "request-one");
        let input = common("amber", "agent/amber.worker", json!({"suspended_at":1,"ended_at":2,"short_sleep":0,"end_class":"stop"}));
        assert!(!receipt(&store, &subject, "residency.sleep-ended", input, Vec::new(), "gated".into()).unwrap());
        assert!(store.claims_for(&subject, Some("residency.sleep-ended")).unwrap().is_empty());
    }
}
