//! Owner-writer admission for automatic suspension and wake. Runtime actions remain the
//! durable authority; preflight evidence is usable only at an unchanged writer frontier.
use super::*;

const FEATURE: declared_admission::FleetFeature = declared_admission::FleetFeature {
    keys: &["idle_residency"],
    code: "idle-residency-feature-barrier",
    carries: "automatic idle suspension and wake",
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IdleWakeAdmission {
    pub operation_id: Option<String>,
    pub retry_at_unix_ms: Option<u128>,
}

fn frontier_fields(frontier: &wake_demand::Frontier) -> Value {
    json!({"graph":frontier.graph,"marks":frontier.marks,
        "mirrored_sent":frontier.mirrored_sent,"mirrored_lifecycle":frontier.mirrored_lifecycle,
        "source":frontier.source,"caught_up":frontier.caught_up})
}

fn graph_index(connection: &Connection) -> Result<u64> {
    Ok(connection.query_row("SELECT COALESCE(MAX(store_index),0) FROM claims", [], |row| row.get(0))?)
}

fn operation(connection: &Connection, key: &str) -> Result<Option<String>> {
    Ok(connection.query_row(
        "SELECT canonical_claim_id FROM operations WHERE id=?1 AND state='active'",
        [operation_id_for_key(key)], |row| row.get(0),
    ).optional()?)
}

fn owned(connection: &Connection, origin: &str, subject: &str) -> Result<Option<String>> {
    let desired = desired_row_at(connection, subject, None)?;
    let Some(desired) = desired.filter(|row| row.kind == "agent") else { return Ok(None); };
    let member = desired.member.as_deref().map(serde_json::from_str::<MemberSpec>).transpose()?;
    Ok(member.filter(|member| member.host == origin).map(|_| desired.claim_id))
}

fn frontier_matches(store: &Store, tx: &Transaction<'_>, frontier: &wake_demand::Frontier) -> Result<bool> {
    let source = messages_shard::shard(store).map(|shard| shard.readers.get().query_row(
        "SELECT COALESCE((SELECT MAX(store_index) FROM stream_messages),0),
                COALESCE((SELECT MAX(rowid) FROM stream_lifecycle),0)", [],
        |row| Ok((row.get::<_, u64>(0)?, row.get::<_, u64>(1)?)),
    )).transpose()?;
    let dirty: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM local_wake_demand_pending_v1)", [], |row| row.get(0))?;
    Ok(frontier.caught_up && !dirty && source == frontier.source
        && graph_index(tx)? == frontier.graph
        && message_marks::event_position(tx)? == frontier.marks
        && stream_mirror::sent_through(tx)? == frontier.mirrored_sent
        && stream_mirror::lifecycle_through(tx)? == frontier.mirrored_lifecycle)
}

impl Store {
    /// The same signed fleet barrier used by the actual writes, for diagnostic callers.
    pub fn idle_residency_enabled(&self) -> Result<(), FractalError> {
        declared_admission::validate_fleet(&self.readers.get(), &self.origin, &FEATURE)
    }

    /// Admit at most once per declaration/incarnation/policy/continuous-idle epoch.
    /// A failed request consumes that epoch; only fresh eligibility permits another attempt.
    pub fn admit_idle_suspend(&self, subject: &str, after_ms: Option<u64>, enabled_at: Option<u128>, now: u128) -> Result<Option<String>> {
        let before = graph_index(&self.readers.get())?;
        let residency = crate::residency::evaluate_at(self, subject, None, after_ms)?.not_before(enabled_at, after_ms);
        if residency.refusal_reason.is_some() || residency.due_at_unix_ms.is_none_or(|due| now < due) { return Ok(None); }
        if crate::suspension::current(self, subject)?.is_some_and(|state| state.holds_seat()) { return Ok(None); }
        let demand = self.wake_demand(subject)?;
        if before != demand.frontier.graph { return Ok(None); }
        self.connection.batched(|tx| -> Result<Option<String>> {
            if graph_index(tx)? != demand.frontier.graph { return Ok(None); }
            declared_admission::validate_fleet(tx, &self.origin, &FEATURE).map_err(anyhow::Error::new)?;
            let Some(declaration) = owned(tx, &self.origin, subject)? else { return Ok(None); };
            let runtime = latest_actual_at(tx, subject, None)?;
            let Some(incarnation) = runtime.as_ref().filter(|runtime| runtime["status"] == "running")
                .and_then(|runtime| runtime["incarnation_id"].as_str()) else { return Ok(None); };
            let key = format!("idle-suspend:{subject}:{declaration}:{incarnation}:{after_ms:?}:{enabled_at:?}:{:?}", residency.eligible_since_unix_ms);
            if demand.oldest.is_some() || !demand.frontier.caught_up {
                let refusal_key = format!("idle-refused:{key}");
                if operation(tx, &refusal_key)?.is_none() {
                    let reason = if demand.oldest.is_some() { "wake-pending" } else { "wake-frontier-lag" };
                    append_receipt_claim_tx(tx, &self.origin, subject, "runtime.action.failed", Some(subject),
                        &json!({"evidence":[declaration],"fields":{"action":"suspend","cause":"idle",
                            "operation_status":"refused","operation_id":key,"code":reason,"reason":reason}}),
                        &[], None, &refusal_key)?;
                }
                return Ok(None);
            }
            if !frontier_matches(self, tx, &demand.frontier)? { return Ok(None); }
            if let Some(id) = operation(tx, &key)? { return Ok(Some(id)); }
            let claim = append_receipt_claim_tx(tx, &self.origin, subject, "runtime.action.requested", Some(subject),
                &json!({"evidence":[declaration], "fields": {"action":"suspend", "cause":"idle", "incarnation_id":incarnation,
                    "idle_declaration":declaration, "idle_after_ms":after_ms, "idle_enabled_at_unix_ms":enabled_at,
                    "idle_eligible_since_unix_ms":residency.eligible_since_unix_ms,
                    "idle_frontier":frontier_fields(&demand.frontier)}}), &[], None, &key)?;
            Ok(Some(claim.id))
        })?
    }

    /// Revalidate the admitted launch and policy at the last reversible stop boundary.
    pub fn recheck_idle_suspend(&self, subject: &str, request: &str, after_ms: Option<u64>, enabled_at: Option<u128>) -> Result<bool> {
        let before = graph_index(&self.readers.get())?;
        let residency = crate::residency::evaluate_at(self, subject, None, after_ms)?.not_before(enabled_at, after_ms);
        if residency.refusal_reason.is_some() { return Ok(false); }
        let demand = self.wake_demand(subject)?;
        if demand.oldest.is_some() || before != demand.frontier.graph { return Ok(false); }
        self.connection.batched(|tx| -> Result<bool> {
            if !frontier_matches(self, tx, &demand.frontier)? { return Ok(false); }
            declared_admission::validate_fleet(tx, &self.origin, &FEATURE).map_err(anyhow::Error::new)?;
            let Some(declaration) = owned(tx, &self.origin, subject)? else { return Ok(false); };
            let body: Option<String> = tx.query_row("SELECT body FROM claims WHERE id=?1 AND subject=?2 AND kind='runtime.action.requested'", params![request, subject], |row| row.get(0)).optional()?;
            let Some(body) = body else { return Ok(false); };
            let body: Value = serde_json::from_str(&body)?;
            let fields = &body["fields"];
            let runtime = latest_actual_at(tx, subject, None)?;
            Ok(fields["action"] == "suspend" && fields["cause"] == "idle"
                && fields["idle_declaration"] == declaration
                && fields["idle_after_ms"] == json!(after_ms)
                && fields["idle_enabled_at_unix_ms"] == json!(enabled_at)
                && fields["idle_eligible_since_unix_ms"] == json!(residency.eligible_since_unix_ms)
                && runtime.is_some_and(|runtime| runtime["status"] == "running" && runtime["incarnation_id"] == fields["incarnation_id"]))
        })?
    }

    /// Person demand wins the bounded owner-local wake queue. Failures retain their lineage
    /// and durable acceptance time, so daemon restarts cannot reset retry backoff.
    pub fn admit_idle_wake(&self, subject: &str, concurrency: usize, now: u128) -> Result<IdleWakeAdmission> {
        smallclaims::touched::note_read(|| "kind:runtime.action.requested".to_owned());
        smallclaims::touched::note_read(|| "kind:runtime.action.succeeded".to_owned());
        smallclaims::touched::note_read(|| "kind:runtime.action.failed".to_owned());
        let before = self.wake_demand(subject)?.frontier;
        let mut active = 0usize;
        let mut candidates = Vec::new();
        let mut retry_at = None;
        let seats = self.readers.get().prepare_cached(
            "SELECT DISTINCT subject FROM claims INDEXED BY claims_idle_action_index
             WHERE kind='runtime.action.requested' AND json_extract(body,'$.fields.cause')='idle'"
        )?.query_map([], |row| row.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        for seat in seats {
            let Some((desired, _)) = self.desired_subject_with_writer(&seat)? else { continue; };
            if desired.kind != "agent" || desired.member.as_ref().is_none_or(|member| member.host != self.origin) { continue; }
            let Some(state) = crate::suspension::current(self, &desired.subject)? else { continue; };
            if state.cause() != "idle" { continue; }
            if matches!(state.phase.as_str(), "restoring" | "verifying" | "fencing-source" | "transferring") {
                if desired.subject == subject {
                    return Ok(IdleWakeAdmission { operation_id: Some(state.operation_id), retry_at_unix_ms: None });
                }
                active += 1; continue;
            }
            if state.phase != "suspended" && !(state.action == "resume" && state.phase == "failed") { continue; }
            let demand = self.wake_demand(&desired.subject)?;
            if !demand.frontier.caught_up { return Ok(IdleWakeAdmission::default()); }
            let Some(pending) = demand.oldest else { continue; };
            let lineage = state.suspend_operation_id.as_deref().unwrap_or(&state.operation_id).to_owned();
            let attempts: u64 = self.readers.get().query_row(
                "SELECT COUNT(*) FROM claims WHERE subject=?1 AND kind='runtime.action.requested'
                 AND json_extract(body,'$.fields.idle_wake_lineage')=?2", params![desired.subject, lineage], |row| row.get(0))?;
            if state.action == "resume" && state.code.is_some() {
                if matches!(state.code.as_deref(), Some("transcript-missing" | "native-session-mismatch"
                    | "snapshot-unavailable" | "snapshot-in-place" | "workspace-path-mismatch"
                    | "transcript-occupied" | "native-resume-unavailable")) { continue; }
                let delay = 1_000u128.saturating_mul(1u128 << attempts.saturating_sub(1).min(6));
                let due = state.updated_at_unix_ms.saturating_add(delay);
                if now < due { if desired.subject == subject { retry_at = Some(due); } continue; }
            }
            candidates.push((!pending.person, pending.sent_ms, pending.subject, desired.subject, lineage, attempts));
        }
        candidates.sort();
        let available = concurrency.saturating_sub(active);
        let Some((_, _, _, _, lineage, attempt)) = candidates.into_iter().take(available).find(|candidate| candidate.3 == subject) else {
            return Ok(IdleWakeAdmission { operation_id: None, retry_at_unix_ms: retry_at });
        };
        let demand = self.wake_demand(subject)?;
        if before != demand.frontier { return Ok(IdleWakeAdmission::default()); }
        self.connection.batched(|tx| -> Result<IdleWakeAdmission> {
            if !frontier_matches(self, tx, &demand.frontier)? || owned(tx, &self.origin, subject)?.is_none() { return Ok(IdleWakeAdmission::default()); }
            declared_admission::validate_fleet(tx, &self.origin, &FEATURE).map_err(anyhow::Error::new)?;
            let key = format!("idle-wake:{subject}:{lineage}:{attempt}");
            let id = if let Some(id) = operation(tx, &key)? { id } else {
                let declaration = owned(tx, &self.origin, subject)?.expect("owner checked in this transaction");
                append_receipt_claim_tx(tx, &self.origin, subject, "runtime.action.requested", Some(subject),
                    &json!({"evidence":[declaration,lineage], "fields":{"action":"resume", "cause":"idle", "suspend_operation_id":lineage,
                        "idle_wake_lineage":lineage,"idle_wake_attempt":attempt,
                        "idle_frontier":frontier_fields(&demand.frontier)}}), &[], None, &key)?.id
            };
            Ok(IdleWakeAdmission { operation_id: Some(id), retry_at_unix_ms: None })
        })?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::directive_notes::tests_shared_delivery::{claim, enable};

    const SEAT: &str = "agent/alder.garden";

    fn started(store: &Store, supported: bool) {
        claim(store, "daemon/alder", "daemon.started", None,
            json!({"status":"running","features":{"idle_residency":u8::from(supported)}}));
    }

    fn fixture(path: &Path, root: &Path) -> Arc<Store> {
        let store = Arc::new(Store::open(path, "alder").unwrap());
        enable(&store);
        started(&store, true);
        store.replication_snapshot().unwrap();
        messages_shard::open(&store, root).unwrap();
        let intent = crate::parse_intent(
            "version 2\nagent \"garden\" { workspace \".\"; command \"true\"; idle-suspend \"member\" }",
            "alder").unwrap();
        store.apply(&intent, &BTreeMap::from([(SEAT.into(), Vec::new())]), "fixture").unwrap();
        for (kind, fields) in [
            ("runtime.observed", json!({"status":"running","incarnation_id":"one"})),
            ("harness.session-file", json!({"harness":"omp","session_id":"native-one","incarnation_id":"one","agent":SEAT})),
            ("harness.diagnostic", json!({"code":"omp-channel-ready","driver":"omp","incarnation_id":"one","reason":"confirmed"})),
            ("harness.observed", json!({"driver":"omp","transport":"omp-channel","state":"idle",
                "incarnation_id":"one","background_jobs":0,"quiescent":true,"blocking":[],
                "ask":"none","blocked_on":"none","input_buffer":"unknown"})),
        ] {
            claim(&store, SEAT, kind, Some(SEAT), fields);
        }
        store
    }

    #[test]
    fn signed_latest_advertisement_controls_idle_gate() {
        let store = Store::open_memory("alder").unwrap();
        assert!(store.idle_residency_enabled().is_err());
        enable(&store);
        started(&store, true);
        assert!(store.idle_residency_enabled().is_err(), "unsigned advertisement is not support");
        store.replication_snapshot().unwrap();
        store.idle_residency_enabled().unwrap();
        started(&store, false);
        assert!(store.idle_residency_enabled().is_err(), "never fall back to older support");
        store.replication_snapshot().unwrap();
        assert!(store.idle_residency_enabled().is_err());
    }

    #[test]
    fn two_handles_share_epoch_receipt_and_policy_fence() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("graph.sqlite");
        let first = fixture(&path, root.path());
        let second = Arc::new(Store::open(&path, "alder").unwrap());
        messages_shard::open(&second, root.path()).unwrap();
        let request = first.admit_idle_suspend(SEAT, Some(0), Some(0), u128::MAX).unwrap().unwrap();
        assert_eq!(crate::suspension::current(&first, SEAT).unwrap().unwrap().operation_id, request);
        assert!(second.admit_idle_suspend(SEAT, Some(0), Some(0), u128::MAX).unwrap().is_none());
        assert!(second.recheck_idle_suspend(SEAT, &request, Some(0), Some(0)).unwrap());
        assert!(!second.recheck_idle_suspend(SEAT, &request, Some(1), Some(0)).unwrap());
        assert!(!second.recheck_idle_suspend(SEAT, &request, Some(0), Some(1)).unwrap());
        let count: u64 = second.readers.get().query_row(
            "SELECT COUNT(*) FROM claims WHERE subject=?1 AND kind='runtime.action.requested'",
            [SEAT], |row| row.get(0)).unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn stale_frontier_and_nonowner_cannot_admit() {
        let root = tempfile::tempdir().unwrap();
        let store = fixture(&root.path().join("graph.sqlite"), root.path());
        let frontier = store.wake_demand(SEAT).unwrap().frontier;
        claim(&store, SEAT, "harness.diagnostic", Some(SEAT), json!({"code":"fixture-note","reason":"new evidence"}));
        assert!(!store.connection.batched(|tx| frontier_matches(&store, tx, &frontier)).unwrap().unwrap());
        assert!(owned(&store.readers.get(), "birch", SEAT).unwrap().is_none());
        assert!(store.admit_idle_suspend(SEAT, None, Some(0), u128::MAX).unwrap().is_none());
    }

    #[test]
    fn wake_is_idempotent_and_failed_retry_deadline_survives_reopen() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("graph.sqlite");
        let store = fixture(&path, root.path());
        let suspend = store.admit_idle_suspend(SEAT, Some(0), Some(0), u128::MAX).unwrap().unwrap();
        store.connection.batched(|tx| append_receipt_claim_tx(tx, "alder", SEAT,
            "runtime.action.succeeded", Some(SEAT),
            &json!({"evidence":[suspend],"fields":{"action":"suspend","operation_status":"suspended"}}),
            &[], None, &crate::suspension::suspend_completed_key(&suspend))).unwrap().unwrap();
        claim(&store, "message/wake", "message.sent", Some("person/avery"),
            json!({"from":"person/avery","to":SEAT,"content":"Please continue.","status":"sent","tags":[]}));
        assert!(store.admit_idle_wake(SEAT, 0, u128::MAX).unwrap().operation_id.is_none());
        let resume = store.admit_idle_wake(SEAT, 1, u128::MAX).unwrap().operation_id.unwrap();
        assert_eq!(store.admit_idle_wake(SEAT, 1, u128::MAX).unwrap().operation_id.as_deref(), Some(resume.as_str()));
        let failed = store.connection.batched(|tx| append_receipt_claim_tx(tx, "alder", SEAT,
            "runtime.action.failed", Some(SEAT),
            &json!({"evidence":[resume],"fields":{"action":"resume","operation_status":"failed","code":"fixture-failure","reason":"retry later"}}),
            &[], None, &crate::suspension::resume_failed_key(&resume))).unwrap().unwrap();
        let due = failed.accepted_at_unix_ms + 1_000;
        assert_eq!(store.admit_idle_wake(SEAT, 1, due - 1).unwrap().retry_at_unix_ms, Some(due));
        drop(store);
        let reopened = Arc::new(Store::open(&path, "alder").unwrap());
        messages_shard::open(&reopened, root.path()).unwrap();
        assert_eq!(reopened.admit_idle_wake(SEAT, 1, due - 1).unwrap().retry_at_unix_ms, Some(due));
        let retry = reopened.admit_idle_wake(SEAT, 1, due).unwrap().operation_id.unwrap();
        assert_ne!(retry, resume);
    }
}
