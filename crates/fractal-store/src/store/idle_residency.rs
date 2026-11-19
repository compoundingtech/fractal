//! Owner-writer admission for automatic suspension and wake. Runtime actions remain the
//! durable authority; preflight evidence is usable only at an unchanged writer frontier.
use super::*;
use super::residency_evidence::append_residency_claim_tx;
use crate::model::MemberSpec;
pub use fractal_telemetry::residency::UnroutedKind;

const FEATURE: declared_admission::FleetFeature = declared_admission::FleetFeature {
    keys: &["idle_residency"],
    code: "idle-residency-feature-barrier",
    carries: "automatic idle suspension and wake",
};

// Spare-capacity rivals and frontier races recheck; active owners invalidate slot release.
const WAKE_QUEUE_RECHECK_MS: u128 = 1_000;

#[cfg(test)]
thread_local! {
    static AFTER_WAKE_DEMAND: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
    static AFTER_WAKE_RANKING: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
    static BEFORE_WAKE_WRITER: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
}

// Reserve the source writer before the graph writer: separate WAL databases need both
// reservations to exclude a shard acceptance between frontier validation and graph commit.
fn with_source_fence<T>(store: &Store, action: impl FnOnce() -> Result<T>) -> Result<T> {
    let shard = messages_shard::shard(store);
    let mut writer = shard.as_ref().map(|shard| shard.connection.write());
    let _reservation = writer.as_mut().map(|writer|
        writer.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
    ).transpose()?;
    action()
}

fn evidence(tx: &Transaction<'_>, origin: &str, seat: &str, episode: &str, key: &str, kind: &str, mut fields: Value) -> Result<()> {
    let key = format!("residency-evidence:{key}");
    if operation(tx, &key)?.is_some() { return Ok(()); }
    let id = operation_id_for_key(&key);
    let subject = format!("residency/{origin}/{episode}/{}", id.trim_start_matches("op/"));
    fields["host"] = json!(origin);
    fields["seat"] = json!(seat);
    append_residency_claim_tx(tx, origin, &subject, kind, None, &json!({"fields":fields}), &[], None, &key)?;
    Ok(())
}

/// Stable episode shared by wake admission and subsequent runtime evidence.
pub fn wake_evidence_subject(origin: &str, seat: &str, lineage: &str, _attempt: u64) -> String {
    let id = operation_id_for_key(&format!("residency-evidence:idle-wake-demand:{seat}:{lineage}"));
    format!("residency/{origin}/wake/{}", id.trim_start_matches("op/"))
}

fn message_received_at(connection: &Connection, origin: &str, message: &str) -> Result<Option<i64>> {
    Ok(connection.query_row(
        "SELECT CASE WHEN claims.origin=?2 THEN CAST(claims.accepted_at_unix_ms AS INTEGER)
                     ELSE CAST(replica_envelopes.received_at_unix_ms AS INTEGER) END
         FROM claims LEFT JOIN replica_envelopes ON replica_envelopes.batch_id=claims.batch_id
         WHERE claims.subject=?1 AND claims.kind='message.sent'
         ORDER BY CAST(claims.accepted_at_unix_ms AS INTEGER),claims.id LIMIT 1",
        params![message, origin], |row| row.get::<_, Option<i64>>(0),
    ).optional()?.flatten())
}

fn owner_received_at(store: &Store, tx: &Transaction<'_>, message: &str) -> Result<i64> {
    if let Some(received) = message_received_at(tx, &store.origin, message)? { return Ok(received); }
    if let Some(shard) = messages_shard::shard(store)
        && let Some(received) = message_received_at(&shard.readers.get(), &store.origin, message)?
    { return Ok(received); }
    Err(anyhow::anyhow!("wake message has no owner receipt timestamp"))
}

/// Record the end of an idle suspension inside the caller's retire transaction. The receipt
/// is idempotent per (seat, suspend operation), so a replayed retire cannot reopen a sleep.
pub fn end_sleep_tx(
    transaction: &Transaction<'_>,
    origin: &str,
    seat: &str,
    suspension: &crate::suspension::Suspension,
    ended_at_unix_ms: u128,
    end_class: &str,
) -> Result<(), FractalError> {
    if suspension.suspended_at_unix_ms.is_none() { return Ok(()); }
    if declared_admission::validate_fleet(transaction, origin, &FEATURE).is_err() { return Ok(()); }
    let suspension_operation = suspension.suspend_operation_id.as_deref()
        .unwrap_or(&suspension.operation_id);
    let key = format!("runtime-residency:sleep:{suspension_operation}");
    if operation(transaction, &key).map_err(internal)?.is_some() { return Ok(()); }
    let id = operation_id_for_key(suspension_operation);
    let subject = format!("residency/{origin}/sleep/{}", id.trim_start_matches("op/"));
    let threshold: Option<u64> = transaction.query_row(
        "SELECT json_extract(body,'$.fields.idle_after_ms') FROM claims WHERE id=?1",
        [suspension_operation], |row| row.get(0)).optional().map_err(internal)?.flatten();
    let short_sleep = suspension.suspended_at_unix_ms.zip(threshold)
        .is_some_and(|(suspended, threshold)| ended_at_unix_ms.saturating_sub(suspended) < u128::from(threshold)) as i64;
    append_residency_claim_tx(transaction, origin, &subject, "residency.sleep-ended", None,
        &json!({"fields": {"suspended_at": suspension.suspended_at_unix_ms, "ended_at": ended_at_unix_ms,
            "short_sleep": short_sleep, "end_class": end_class, "host": origin, "seat": seat}}),
        &[], None, &key).map_err(internal)?;
    Ok(())
}

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
        && !projection_work_pending_tx(tx).map_err(anyhow::Error::new)?
        && graph_index(tx)? == frontier.graph
        && message_marks::event_position(tx)? == frontier.marks
        && stream_mirror::sent_through(tx)? == frontier.mirrored_sent
        && stream_mirror::lifecycle_through(tx)? == frontier.mirrored_lifecycle)
}

impl Store {
    /// Indexed owner-local suspend/resume episodes, including manual holds for sensors.
    pub fn residency_episode_seats(&self) -> Result<Vec<String>> {
        smallclaims::touched::note_read(|| "kind:runtime.action.requested".to_owned());
        Ok(self.readers.get().prepare_cached(
            "SELECT DISTINCT claims.subject FROM claims INDEXED BY claims_suspension_action_index
             JOIN desired ON desired.subject=claims.subject
             WHERE claims.kind='runtime.action.requested'
               AND json_extract(claims.body,'$.fields.action') IN ('suspend','resume')
               AND desired.kind='agent' AND json_extract(desired.member,'$.host')=?1"
        )?.query_map([&self.origin], |row| row.get(0))?.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Indexed owner-local idle episodes, not a scan of the seat roster.
    pub fn idle_residency_episodes(&self) -> Result<Vec<String>> {
        smallclaims::touched::note_read(|| "kind:runtime.action.requested".to_owned());
        Ok(self.readers.get().prepare_cached(
            "SELECT DISTINCT claims.subject FROM claims INDEXED BY claims_idle_action_index
             JOIN desired ON desired.subject=claims.subject
             WHERE claims.kind='runtime.action.requested' AND json_extract(claims.body,'$.fields.cause')='idle'
               AND desired.kind='agent' AND json_extract(desired.member,'$.host')=?1"
        )?.query_map([&self.origin], |row| row.get(0))?.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Owner-local pending mail ages at one verified wake-demand frontier.
    /// A false completeness bit forbids treating missing timestamps as an empty inbox.
    pub fn idle_residency_mail_summary(&self) -> Result<(bool, Option<i64>, Option<i64>)> {
        smallclaims::touched::note_read(|| "kind:message.sent".to_owned());
        let before = graph_index(&self.readers.get())?;
        let mut asleep = Vec::new();
        for seat in self.idle_residency_episodes()? {
            if crate::suspension::current(self, &seat)?.is_some_and(|state|
                state.cause() == "idle" && state.phase == "suspended")
            {
                asleep.push(seat);
            }
        }
        let frontier = self.wake_demand("")?.frontier;
        let connection = self.readers.get();
        let tx = connection.unchecked_transaction()?;
        let (wake, silent) = tx.query_row(
            "SELECT MIN(CASE WHEN mail.class>0 THEN mail.sent_ms END),
                    MIN(CASE WHEN mail.class=0 THEN mail.sent_ms END)
             FROM json_each(?2) seats JOIN local_wake_demand_v1 mail
               ON mail.recipient=seats.value AND mail.pending=1
             JOIN desired ON desired.subject=mail.recipient
             WHERE desired.kind='agent' AND json_extract(desired.member,'$.host')=?1",
            params![self.origin, serde_json::to_string(&asleep)?], |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        Ok((before == frontier.graph && frontier_matches(self, &tx, &frontier)?, wake, silent))
    }

    /// The same signed fleet barrier used by the actual writes, for diagnostic callers.
    pub fn idle_residency_enabled(&self) -> Result<(), FractalError> {
        declared_admission::validate_fleet(&self.readers.get(), &self.origin, &FEATURE)
    }

    /// Record one owner-local missing-route incident per idle sleep and entry point.
    /// Lag is unknown, not proof that a wake route is absent.
    pub fn record_idle_unrouted(&self, subject: &str, kind: UnroutedKind) -> Result<bool> {
        if self.idle_residency_enabled().is_err() { return Ok(false); }
        if !crate::suspension::current(self, subject)?.is_some_and(|state|
            state.cause() == "idle" && state.phase == "suspended")
        { return Ok(false); }
        let demand = self.wake_demand(subject)?;
        if demand.oldest.is_some() || !demand.frontier.caught_up { return Ok(false); }
        let inserted = with_source_fence(self, || self.connection.batched(|tx| -> Result<bool> {
            if declared_admission::validate_fleet(tx, &self.origin, &FEATURE).is_err()
                || owned(tx, &self.origin, subject)?.is_none()
                || !frontier_matches(self, tx, &demand.frontier)?
            { return Ok(false); }
            let Some(suspension) = crate::suspension::current_at_tx(tx, subject, None)?
                .filter(|state| state.cause() == "idle" && state.phase == "suspended")
            else { return Ok(false); };
            let suspend = suspension.suspend_operation_id.as_deref().unwrap_or(&suspension.operation_id);
            let key = format!("idle-unrouted:{subject}:{suspend}:{}", kind.label());
            if operation(tx, &format!("residency-evidence:{key}"))?.is_some() { return Ok(false); }
            evidence(tx, &self.origin, subject, "unrouted", &key, "residency.unrouted",
                json!({"observed_at":now_ms(),"kind":kind.label()}))?;
            Ok(true)
        }).map_err(anyhow::Error::msg)?)?;
        if inserted {
            static COUNTER: std::sync::LazyLock<opentelemetry::metrics::Counter<u64>> =
                std::sync::LazyLock::new(|| opentelemetry::global::meter("fractal.residency")
                    .u64_counter("st.residency.unrouted").build());
            COUNTER.add(1, &[opentelemetry::KeyValue::new("kind", kind.label())]);
        }
        Ok(inserted)
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
        with_source_fence(self, || self.connection.batched(|tx| -> Result<Option<String>> {
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
                    append_residency_claim_tx(tx, &self.origin, subject, "runtime.action.failed", Some(subject),
                        &json!({"evidence":[declaration],"fields":{"action":"suspend","cause":"idle",
                            "operation_status":"refused","operation_id":key,"code":reason,"reason":reason}}),
                        &[], None, &refusal_key)?;
                    evidence(tx, &self.origin, subject, "eligibility", &refusal_key, "residency.eligibility-outcome",
                        json!({"eligible_since":residency.eligible_since_unix_ms,"decided_at":now,"refused":1,"reason":reason}))?;
                }
                return Ok(None);
            }
            if !frontier_matches(self, tx, &demand.frontier)? { return Ok(None); }
            if let Some(id) = operation(tx, &key)? { return Ok(Some(id)); }
            let claim = append_residency_claim_tx(tx, &self.origin, subject, "runtime.action.requested", Some(subject),
                &json!({"evidence":[declaration], "fields": {"action":"suspend", "cause":"idle", "incarnation_id":incarnation,
                    "idle_declaration":declaration, "idle_after_ms":after_ms, "idle_enabled_at_unix_ms":enabled_at,
                    "idle_eligible_since_unix_ms":residency.eligible_since_unix_ms,
                    "idle_frontier":frontier_fields(&demand.frontier)}}), &[], None, &key)?;
            Ok(Some(claim.id))
        }).map_err(anyhow::Error::msg)?)
    }

    /// Revalidate without crossing the stop boundary (used before workspace capture).
    pub fn recheck_idle_suspend(&self, subject: &str, request: &str, after_ms: Option<u64>, enabled_at: Option<u128>) -> Result<bool> {
        self.fence_idle_suspend(subject, request, after_ms, enabled_at, None)
    }

    /// Commit the stop boundary under the same source and owner reservations as the final
    /// demand check. Demand cannot be accepted in a gap between validation and this receipt.
    pub fn commit_idle_stop_boundary(&self, subject: &str, request: &str, after_ms: Option<u64>,
        enabled_at: Option<u128>, fields: BTreeMap<String, Value>) -> Result<bool>
    {
        self.fence_idle_suspend(subject, request, after_ms, enabled_at, Some(fields))
    }

    fn fence_idle_suspend(&self, subject: &str, request: &str, after_ms: Option<u64>,
        enabled_at: Option<u128>, snapshot: Option<BTreeMap<String, Value>>) -> Result<bool>
    {
        let before = graph_index(&self.readers.get())?;
        let residency = crate::residency::evaluate_at(self, subject, None, after_ms)?.not_before(enabled_at, after_ms);
        if residency.refusal_reason.is_some() { return Ok(false); }
        let demand = self.wake_demand(subject)?;
        if demand.oldest.is_some() || before != demand.frontier.graph { return Ok(false); }
        with_source_fence(self, || self.connection.batched(|tx| -> Result<bool> {
            if !frontier_matches(self, tx, &demand.frontier)? { return Ok(false); }
            declared_admission::validate_fleet(tx, &self.origin, &FEATURE).map_err(anyhow::Error::new)?;
            let Some(declaration) = owned(tx, &self.origin, subject)? else { return Ok(false); };
            let latest: Option<String> = tx.query_row(/* sqlite-large-value: allow-for-now lv-ha0e88fcd8a5e owner=fractal_store::store::idle_residency TODO=trace-statement-and-value-representation */ &canonical_sql(
                "SELECT claims.id FROM claims JOIN batches ON batches.id=claims.batch_id
                 WHERE claims.subject=?1 AND claims.kind='runtime.action.requested' AND claims.actor IS NOT NULL
                 ORDER BY CANONICAL_DESC(claims) LIMIT 1"
            ), [subject], |row| row.get(0)).optional()?;
            if latest.as_deref() != Some(request) { return Ok(false); }
            let body: Option<String> = tx.query_row("SELECT body FROM claims WHERE id=?1 AND subject=?2 AND kind='runtime.action.requested'", params![request, subject], |row| row.get(0)).optional()?;
            let Some(body) = body else { return Ok(false); };
            let body: Value = serde_json::from_str(&body)?;
            let fields = &body["fields"];
            let runtime = latest_actual_at(tx, subject, None)?;
            let valid = fields["action"] == "suspend" && fields["cause"] == "idle"
                && fields["idle_declaration"] == declaration
                && fields["idle_after_ms"] == json!(after_ms)
                && fields["idle_enabled_at_unix_ms"] == json!(enabled_at)
                && fields["idle_eligible_since_unix_ms"] == json!(residency.eligible_since_unix_ms)
                && runtime.is_some_and(|runtime| runtime["status"] == "running" && runtime["incarnation_id"] == fields["incarnation_id"]);
            if !valid { return Ok(false); }
            if let Some(mut fields) = snapshot {
                fields.insert("action".into(), json!("suspend"));
                let key = crate::suspension::suspend_snapshot_key(request);
                if operation(tx, &key)?.is_none() {
                    append_residency_claim_tx(tx, &self.origin, subject, "runtime.action.succeeded", Some(subject),
                        &json!({"fields":fields,"evidence":[request]}), &[], None, &key)?;
                }
            }
            Ok(true)
        }).map_err(anyhow::Error::msg)?)
    }

    /// Person demand wins the bounded owner-local wake queue. Failures retain their lineage
    /// and durable acceptance time, so daemon restarts cannot reset retry backoff.
    pub fn admit_idle_wake(&self, subject: &str, concurrency: usize, now: u128) -> Result<IdleWakeAdmission> {
        smallclaims::touched::note_read(|| "kind:runtime.action.requested".to_owned());
        smallclaims::touched::note_read(|| "kind:runtime.action.succeeded".to_owned());
        smallclaims::touched::note_read(|| "kind:runtime.action.failed".to_owned());
        let initial = self.wake_demand(subject)?;
        if initial.oldest.is_none() { return Ok(IdleWakeAdmission::default()); }
        let mut frontier_raced = !initial.frontier.caught_up;
        #[cfg(test)]
        if let Some(hook) = AFTER_WAKE_DEMAND.with(|hook| hook.borrow_mut().take()) { hook(); }
        let before = initial.frontier;
        let mut own_pending = initial.oldest;
        let mut active = 0usize;
        let mut candidates = Vec::new();
        let mut queued = Vec::new();
        let mut retry_at = None;
        let mut own_eligible = false;
        let seats = self.idle_residency_episodes()?;
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
            let pending = if desired.subject == subject {
                own_pending.take()
            } else {
                let demand = self.wake_demand_untracked(&desired.subject)?;
                if !demand.frontier.caught_up {
                    // Finish the active-slot census before deciding whether a retry may poll.
                    frontier_raced = true;
                    continue;
                }
                demand.oldest
            };
            let Some(pending) = pending else { continue; };
            let lineage = state.suspend_operation_id.as_deref().unwrap_or(&state.operation_id).to_owned();
            let incarnation = state.incarnation_id.clone().ok_or_else(|| anyhow::anyhow!("idle suspension has no incarnation"))?;
            queued.push((desired.subject.clone(), lineage.clone(), pending.clone(), incarnation));
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
            if desired.subject == subject { own_eligible = true; }
            candidates.push((!pending.person, pending.sent_ms, pending.subject, desired.subject, lineage, attempts));
        }
        candidates.sort();
        let available = concurrency.saturating_sub(active);
        let selected = candidates.into_iter().take(available).find(|candidate| candidate.3 == subject);
        if available == 0 {
            retry_at = None;
        } else if own_eligible && selected.is_none() {
            retry_at = Some(now.saturating_add(WAKE_QUEUE_RECHECK_MS));
        }
        let frontier_race_retry = || IdleWakeAdmission {
            operation_id: None,
            retry_at_unix_ms: (available > 0).then(|| {
                let recheck = now.saturating_add(WAKE_QUEUE_RECHECK_MS);
                retry_at.map_or(recheck, |due| due.min(recheck))
            }),
        };
        if frontier_raced { return Ok(frontier_race_retry()); }
        #[cfg(test)]
        if let Some(hook) = AFTER_WAKE_RANKING.with(|hook| hook.borrow_mut().take()) { hook(); }
        let demand = self.wake_demand(subject)?;
        if before != demand.frontier { return Ok(frontier_race_retry()); }
        #[cfg(test)]
        if let Some(hook) = BEFORE_WAKE_WRITER.with(|hook| hook.borrow_mut().take()) { hook(); }
        with_source_fence(self, || self.connection.batched(|tx| -> Result<IdleWakeAdmission> {
            if !frontier_matches(self, tx, &demand.frontier)? { return Ok(frontier_race_retry()); }
            if owned(tx, &self.origin, subject)?.is_none() { return Ok(IdleWakeAdmission::default()); }
            declared_admission::validate_fleet(tx, &self.origin, &FEATURE).map_err(anyhow::Error::new)?;
            for (seat, lineage, pending, incarnation) in &queued {
                let key = format!("idle-wake-demand:{seat}:{lineage}");
                if operation(tx, &format!("residency-evidence:{key}"))?.is_none() {
                    let received_at = owner_received_at(self, tx, &pending.subject)?;
                    evidence(tx, &self.origin, seat, "wake", &key, "residency.wake-demand",
                        json!({"demand_at":received_at,"origin_accepted_at":pending.sent_ms,
                            "source_message":pending.subject,"suspended_incarnation":incarnation}))?;
                }
            }
            let Some((_, _, _, _, lineage, attempt)) = selected else {
                return Ok(IdleWakeAdmission { operation_id: None, retry_at_unix_ms: retry_at });
            };
            let key = format!("idle-wake:{subject}:{lineage}:{attempt}");
            let id = if let Some(id) = operation(tx, &key)? { id } else {
                let declaration = owned(tx, &self.origin, subject)?.expect("owner checked in this transaction");
                append_residency_claim_tx(tx, &self.origin, subject, "runtime.action.requested", Some(subject),
                    &json!({"evidence":[declaration,lineage], "fields":{"action":"resume", "cause":"idle", "suspend_operation_id":lineage,
                        "idle_wake_lineage":lineage,"idle_wake_attempt":attempt,
                        "idle_frontier":frontier_fields(&demand.frontier)}}), &[], None, &key)?.id
            };
            Ok(IdleWakeAdmission { operation_id: Some(id), retry_at_unix_ms: None })
        }).map_err(anyhow::Error::msg)?)
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
    fn unrouted_incidents_are_gated_and_once_per_sleep_kind() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("graph.sqlite");
        let store = fixture(&path, root.path());
        assert!(!store.record_idle_unrouted(SEAT, UnroutedKind::Attach).unwrap());
        let suspend = store.admit_idle_suspend(SEAT, Some(0), Some(0), i64::MAX as u128).unwrap().unwrap();
        store.connection.batched(|tx| append_residency_claim_tx(tx, "alder", SEAT,
            "runtime.action.succeeded", Some(SEAT),
            &json!({"evidence":[suspend],"fields":{"action":"suspend","operation_status":"suspended"}}),
            &[], None, &crate::suspension::suspend_completed_key(&suspend))).unwrap().unwrap();
        started(&store, false);
        store.replication_snapshot().unwrap();
        assert!(!store.record_idle_unrouted(SEAT, UnroutedKind::Attach).unwrap());
        started(&store, true);
        store.replication_snapshot().unwrap();
        assert!(store.record_idle_unrouted(SEAT, UnroutedKind::Attach).unwrap());
        assert!(!store.record_idle_unrouted(SEAT, UnroutedKind::Attach).unwrap());
        assert!(store.record_idle_unrouted(SEAT, UnroutedKind::ExplicitResume).unwrap());
        let second = Arc::new(Store::open(&path, "alder").unwrap());
        messages_shard::open(&second, root.path()).unwrap();
        assert!(!second.record_idle_unrouted(SEAT, UnroutedKind::ExplicitResume).unwrap());
        let count: u64 = store.readers.get().query_row(
            "SELECT COUNT(*) FROM claims WHERE kind='residency.unrouted'", [], |row| row.get(0)).unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn pending_wake_is_not_an_unrouted_incident() {
        let root = tempfile::tempdir().unwrap();
        let store = fixture(&root.path().join("graph.sqlite"), root.path());
        let suspend = store.admit_idle_suspend(SEAT, Some(0), Some(0), i64::MAX as u128).unwrap().unwrap();
        store.connection.batched(|tx| append_residency_claim_tx(tx, "alder", SEAT,
            "runtime.action.succeeded", Some(SEAT),
            &json!({"evidence":[suspend],"fields":{"action":"suspend","operation_status":"suspended"}}),
            &[], None, &crate::suspension::suspend_completed_key(&suspend))).unwrap().unwrap();
        claim(&store, "message/unrouted-test", "message.sent", Some("person/avery"),
            json!({"from":"person/avery","to":SEAT,"content":"Continue.","status":"sent","tags":[]}));
        assert!(!store.record_idle_unrouted(SEAT, UnroutedKind::Attach).unwrap());
        assert!(!store.record_idle_unrouted(SEAT, UnroutedKind::ExplicitResume).unwrap());
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
        let request = first.admit_idle_suspend(SEAT, Some(0), Some(0), i64::MAX as u128).unwrap().unwrap();
        assert_eq!(crate::suspension::current(&first, SEAT).unwrap().unwrap().operation_id, request);
        assert!(second.admit_idle_suspend(SEAT, Some(0), Some(0), i64::MAX as u128).unwrap().is_none());
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
        assert!(store.admit_idle_suspend(SEAT, None, Some(0), i64::MAX as u128).unwrap().is_none());
    }

    #[test]
    fn wake_is_idempotent_and_failed_retry_deadline_survives_reopen() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("graph.sqlite");
        let store = fixture(&path, root.path());
        let suspend = store.admit_idle_suspend(SEAT, Some(0), Some(0), i64::MAX as u128).unwrap().unwrap();
        store.connection.batched(|tx| append_residency_claim_tx(tx, "alder", SEAT,
            "runtime.action.succeeded", Some(SEAT),
            &json!({"evidence":[suspend],"fields":{"action":"suspend","operation_status":"suspended"}}),
            &[], None, &crate::suspension::suspend_completed_key(&suspend))).unwrap().unwrap();
        claim(&store, "message/wake", "message.sent", Some("person/avery"),
            json!({"from":"person/avery","to":SEAT,"content":"Please continue.","status":"sent","tags":[]}));
        assert!(store.admit_idle_wake(SEAT, 0, i64::MAX as u128).unwrap().operation_id.is_none());
        let episode = wake_evidence_subject("alder", SEAT, &suspend, 0);
        let (count, demand_at): (u64, i64) = store.readers.get().query_row(
            "SELECT COUNT(*),json_extract(body,'$.fields.demand_at') FROM claims
             WHERE subject=?1 AND kind='residency.wake-demand'", [&episode],
            |row| Ok((row.get(0)?, row.get(1)?))).unwrap();
        assert_eq!(count, 1, "queue entry records demand before it has capacity");
        let accepted: i64 = store.readers.get().query_row(
            "SELECT CAST(accepted_at_unix_ms AS INTEGER) FROM claims WHERE subject='message/wake' AND kind='message.sent'",
            [], |row| row.get(0)).unwrap();
        assert_eq!(demand_at, accepted, "local demand starts at message acceptance, not admission");
        let resume = store.admit_idle_wake(SEAT, 1, i64::MAX as u128).unwrap().operation_id.unwrap();
        assert_eq!(store.admit_idle_wake(SEAT, 1, i64::MAX as u128).unwrap().operation_id.as_deref(), Some(resume.as_str()));
        let failed = store.connection.batched(|tx| append_residency_claim_tx(tx, "alder", SEAT,
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
        assert_eq!(wake_evidence_subject("alder", SEAT, &suspend, 1), episode);
        let (count, retried_at): (u64, i64) = reopened.readers.get().query_row(
            "SELECT COUNT(*),json_extract(body,'$.fields.demand_at') FROM claims
             WHERE subject=?1 AND kind='residency.wake-demand'", [&episode],
            |row| Ok((row.get(0)?, row.get(1)?))).unwrap();
        assert_eq!((count, retried_at), (1, demand_at), "retry cannot restart the demand clock");
        reopened.connection.batched(|tx| append_residency_claim_tx(tx, "alder", SEAT,
            "runtime.action.failed", Some(SEAT),
            &json!({"evidence":[retry],"fields":{"action":"resume","operation_status":"failed","code":"native-session-mismatch","reason":"wrong session"}}),
            &[], None, &crate::suspension::resume_failed_key(&retry))).unwrap().unwrap();
        assert_eq!(reopened.admit_idle_wake(SEAT, 1, i64::MAX as u128).unwrap(), IdleWakeAdmission::default());
    }

    fn complete_suspend(store: &Store, seat: &str, request: &str) {
        store.connection.batched(|tx| append_residency_claim_tx(tx, "alder", seat,
            "runtime.action.succeeded", Some(seat),
            &json!({"evidence":[request],"fields":{"action":"suspend","operation_status":"suspended"}}),
            &[], None, &crate::suspension::suspend_completed_key(request))).unwrap().unwrap();
    }

    #[test]
    fn person_queue_precedes_agents_and_active_wakes_hold_capacity() {
        let root = tempfile::tempdir().unwrap();
        let store = fixture(&root.path().join("graph.sqlite"), root.path());
        let orchard = "agent/alder.orchard";
        let intent = crate::parse_intent(
            "version 2\nagent \"orchard\" { workspace \".\"; command \"true\"; }", "alder").unwrap();
        store.apply(&intent, &BTreeMap::from([(orchard.into(), Vec::new())]), "orchard").unwrap();
        let declaration = store.selected_desired_token(orchard).unwrap().unwrap();
        let suspend = store.connection.batched(|tx| append_residency_claim_tx(tx, "alder", orchard,
            "runtime.action.requested", Some(orchard),
            &json!({"evidence":[declaration],"fields":{"action":"suspend","cause":"idle","incarnation_id":"two"}}),
            &[], None, "orchard-suspend")).unwrap().unwrap();
        complete_suspend(&store, orchard, &suspend.id);
        let garden = store.admit_idle_suspend(SEAT, Some(0), Some(0), i64::MAX as u128).unwrap().unwrap();
        complete_suspend(&store, SEAT, &garden);
        claim(&store, "message/agent", "message.sent", Some(SEAT),
            json!({"from":SEAT,"to":SEAT,"content":"Agent demand.","status":"sent","tags":[]}));
        claim(&store, "message/person", "message.sent", Some("person/avery"),
            json!({"from":"person/avery","to":orchard,"content":"Person demand.","status":"sent","tags":[]}));
        assert!(store.admit_idle_wake(SEAT, 1, 9_000_000_000_000).unwrap().operation_id.is_none());
        let resume = store.admit_idle_wake(orchard, 1, 9_000_000_000_000).unwrap().operation_id.unwrap();
        let now = 9_000_000_000_000;
        let (waiting, reads) = smallclaims::touched::record(|| store.admit_idle_wake(SEAT, 1, now).unwrap());
        assert_eq!(waiting, IdleWakeAdmission::default(), "full capacity must not arm a polling deadline");
        let incremental = crate::incremental::Incremental::default();
        incremental.observe(&store).unwrap();
        incremental.evaluated("member:garden", reads, waiting.retry_at_unix_ms);
        assert!(!incremental.needs("member:garden", now + 73_000), "slow restores must not poll the queue");
        store.connection.batched(|tx| append_residency_claim_tx(tx, "alder", orchard,
            "runtime.action.succeeded", Some(orchard),
            &json!({"evidence":[resume],"fields":{"action":"resume","operation_status":"resumed","incarnation_id":"three"}}),
            &[], None, &crate::suspension::resume_completed_key(&resume))).unwrap().unwrap();
        incremental.observe(&store).unwrap();
        assert!(incremental.needs("member:garden", now + 73_000), "slot release re-ranks the subscribed waiter");
        assert!(store.admit_idle_wake(SEAT, 1, now + 73_000).unwrap().operation_id.is_some());
    }

    #[test]
    fn queued_wake_rechecks_when_an_unadmitted_leaders_message_closes() {
        let root = tempfile::tempdir().unwrap();
        let store = fixture(&root.path().join("graph.sqlite"), root.path());
        let orchard = "agent/alder.orchard";
        let intent = crate::parse_intent(
            "version 2\nagent \"orchard\" { workspace \".\"; command \"true\"; }", "alder").unwrap();
        store.apply(&intent, &BTreeMap::from([(orchard.into(), Vec::new())]), "orchard").unwrap();
        let declaration = store.selected_desired_token(orchard).unwrap().unwrap();
        let suspend = store.connection.batched(|tx| append_residency_claim_tx(tx, "alder", orchard,
            "runtime.action.requested", Some(orchard),
            &json!({"evidence":[declaration],"fields":{"action":"suspend","cause":"idle","incarnation_id":"two"}}),
            &[], None, "orchard-suspend")).unwrap().unwrap();
        complete_suspend(&store, orchard, &suspend.id);
        let garden = store.admit_idle_suspend(SEAT, Some(0), Some(0), i64::MAX as u128).unwrap().unwrap();
        complete_suspend(&store, SEAT, &garden);
        claim(&store, "message/garden", "message.sent", Some(SEAT),
            json!({"from":SEAT,"to":SEAT,"content":"Agent demand.","status":"sent","tags":[]}));
        claim(&store, "message/orchard", "message.sent", Some("person/avery"),
            json!({"from":"person/avery","to":orchard,"content":"Person demand.","status":"sent","tags":[]}));
        let now = 9_000_000_000_000;
        let (waiting, reads) = smallclaims::touched::record(|| store.admit_idle_wake(SEAT, 1, now).unwrap());
        assert!(waiting.operation_id.is_none(), "person demand on orchard leads the queue");
        let due = waiting.retry_at_unix_ms.unwrap();
        assert_eq!(due, now + WAKE_QUEUE_RECHECK_MS);
        let incremental = crate::incremental::Incremental::default();
        incremental.observe(&store).unwrap();
        incremental.evaluated("member:garden", reads, Some(due));
        // Daemon withdrawal closes sent mail without a recipient turn or runtime action.
        claim(&store, "message/orchard", "message.closed", Some("daemon/runtime"), json!({"status":"closed"}));
        assert_eq!(store.admit_idle_wake(orchard, 1, now + 1).unwrap(), IdleWakeAdmission::default());
        assert_eq!(crate::suspension::current(&store, orchard).unwrap().unwrap().operation_id, suspend.id,
            "the leader disappeared without producing a resume action");
        incremental.observe(&store).unwrap();
        assert!(!incremental.needs("member:garden", due - 1), "the other seat's closure does not dirty garden");
        assert!(incremental.needs("member:garden", due), "the waiting seat's exact deadline restores liveness");
        assert!(store.admit_idle_wake(SEAT, 1, due).unwrap().operation_id.is_some());
    }

    #[test]
    fn wake_frontier_races_retry_with_spare_capacity_and_follow_full_slot_release() {
        for stage in 0..4 {
            for queue in 0..3 {
                let root = tempfile::tempdir().unwrap();
                let store = fixture(&root.path().join("graph.sqlite"), root.path());
                let orchard = "agent/alder.orchard";
                let intent = crate::parse_intent(
                    "version 2\nagent \"orchard\" { workspace \".\"; command \"true\"; }", "alder").unwrap();
                store.apply(&intent, &BTreeMap::from([(orchard.into(), Vec::new())]), "orchard").unwrap();
                let declaration = store.selected_desired_token(orchard).unwrap().unwrap();
                let suspend = store.connection.batched(|tx| append_residency_claim_tx(tx, "alder", orchard,
                    "runtime.action.requested", Some(orchard),
                    &json!({"evidence":[declaration],"fields":{"action":"suspend","cause":"idle","incarnation_id":"two"}}),
                    &[], None, "orchard-suspend")).unwrap().unwrap();
                complete_suspend(&store, orchard, &suspend.id);
                let garden = store.admit_idle_suspend(SEAT, Some(0), Some(0), i64::MAX as u128).unwrap().unwrap();
                complete_suspend(&store, SEAT, &garden);
                claim(&store, "message/garden", "message.sent", Some(SEAT),
                    json!({"from":SEAT,"to":SEAT,"content":"Agent demand.","status":"sent","tags":[]}));
                if queue != 0 {
                    claim(&store, "message/orchard", "message.sent", Some("person/avery"),
                        json!({"from":"person/avery","to":orchard,"content":"Person demand.","status":"sent","tags":[]}));
                }
                let now = 9_000_000_000_000;
                let active_resume = if queue == 2 {
                    Some(store.admit_idle_wake(orchard, 1, now).unwrap().operation_id.unwrap())
                } else { None };
                let incremental = crate::incremental::Incremental::default();
                incremental.observe(&store).unwrap();
                let concurrent = store.clone();
                let inject: Box<dyn FnOnce()> = Box::new(move || {
                    std::thread::spawn(move || {
                        let input = ClaimInput {
                            subject: "message/unrelated".into(), kind: "message.sent".into(),
                            actor: Some("agent/birch".into()),
                            fields: serde_json::from_value(json!({"from":"agent/birch","to":"person/avery",
                                "content":"Unrelated frontier write.","status":"sent","tags":[]})).unwrap(),
                            evidence: Vec::new(), expected_subject: None, idempotency_key: None,
                        };
                        if stage >= 2 {
                            messages_shard::shard(&concurrent).unwrap().append_claim_outcome(&input).unwrap();
                        } else {
                            concurrent.append_claim(&input).unwrap();
                        }
                    }).join().unwrap();
                });
                match stage {
                    0 => AFTER_WAKE_RANKING.with(|hook| *hook.borrow_mut() = Some(inject)),
                    1 => BEFORE_WAKE_WRITER.with(|hook| *hook.borrow_mut() = Some(inject)),
                    2 => AFTER_WAKE_DEMAND.with(|hook| *hook.borrow_mut() = Some(inject)),
                    _ => inject(),
                }
                let (admission, reads) = smallclaims::touched::record(|| store.admit_idle_wake(SEAT, 1, now).unwrap());
                assert!(admission.operation_id.is_none(), "a raced frontier must never admit stale ranking");
                let due = if queue == 2 {
                    assert_eq!(admission.retry_at_unix_ms, None, "a full queue follows slot release, not timers");
                    now + 73_000
                } else {
                    assert_eq!(admission.retry_at_unix_ms, Some(now + WAKE_QUEUE_RECHECK_MS),
                        "both selected seats and rival-blocked seats retain a bounded frontier-race retry");
                    now + WAKE_QUEUE_RECHECK_MS
                };
                incremental.evaluated("member:garden", reads, admission.retry_at_unix_ms);
                if stage >= 2 { stream_mirror::mirror(&store).unwrap(); }
                if queue == 1 {
                    claim(&store, "message/orchard", "message.closed", Some("daemon/runtime"), json!({"status":"closed"}));
                }
                incremental.observe(&store).unwrap();
                assert!(!incremental.needs("member:garden", due - 1),
                    "unrelated writes must not dirty this seat: stage={stage}, queue={queue}");
                if let Some(resume) = active_resume {
                    assert!(!incremental.needs("member:garden", due));
                    store.connection.batched(|tx| append_residency_claim_tx(tx, "alder", orchard,
                        "runtime.action.succeeded", Some(orchard),
                        &json!({"evidence":[resume],"fields":{"action":"resume","operation_status":"resumed","incarnation_id":"three"}}),
                        &[], None, &crate::suspension::resume_completed_key(&resume))).unwrap().unwrap();
                    incremental.observe(&store).unwrap();
                }
                assert!(incremental.needs("member:garden", due));
                assert!(store.admit_idle_wake(SEAT, 1, due).unwrap().operation_id.is_some());
            }
        }
    }

    #[test]
    fn wake_queue_ranking_does_not_subscribe_to_other_seats_mail() {
        let root = tempfile::tempdir().unwrap();
        let store = fixture(&root.path().join("graph.sqlite"), root.path());
        let orchard = "agent/alder.orchard";
        let intent = crate::parse_intent(
            "version 2\nagent \"orchard\" { workspace \".\"; command \"true\"; }", "alder").unwrap();
        store.apply(&intent, &BTreeMap::from([(orchard.into(), Vec::new())]), "orchard").unwrap();
        let declaration = store.selected_desired_token(orchard).unwrap().unwrap();
        let suspend = store.connection.batched(|tx| append_residency_claim_tx(tx, "alder", orchard,
            "runtime.action.requested", Some(orchard),
            &json!({"evidence":[declaration],"fields":{"action":"suspend","cause":"idle","incarnation_id":"two"}}),
            &[], None, "orchard-suspend")).unwrap().unwrap();
        complete_suspend(&store, orchard, &suspend.id);
        let garden = store.admit_idle_suspend(SEAT, Some(0), Some(0), i64::MAX as u128).unwrap().unwrap();
        complete_suspend(&store, SEAT, &garden);
        for (message, seat) in [("message/garden", SEAT), ("message/orchard", orchard)] {
            claim(&store, message, "message.sent", Some("person/avery"),
                json!({"from":"person/avery","to":seat,"content":"Wake.","status":"sent","tags":[]}));
        }
        // Zero capacity forces both items to rank the complete queue without starting a wake.
        let (_, garden_reads) = smallclaims::touched::record(|| store.admit_idle_wake(SEAT, 0, 9_000_000_000_000).unwrap());
        let (_, orchard_reads) = smallclaims::touched::record(|| store.admit_idle_wake(orchard, 0, 9_000_000_000_000).unwrap());
        assert!(garden_reads.contains(&format!("mailbox:{SEAT}")));
        assert!(garden_reads.contains("message/garden"));
        assert!(!garden_reads.contains(&format!("mailbox:{orchard}")));
        assert!(!garden_reads.contains("message/orchard"));
        assert!(orchard_reads.contains(&format!("mailbox:{orchard}")));
        assert!(orchard_reads.contains("message/orchard"));
        assert!(!orchard_reads.contains(&format!("mailbox:{SEAT}")));
        assert!(!orchard_reads.contains("message/garden"));
        let incremental = crate::incremental::Incremental::default();
        incremental.observe(&store).unwrap();
        incremental.evaluated("member:garden", garden_reads, None);
        incremental.evaluated("member:orchard", orchard_reads, None);
        claim(&store, "message/garden-new", "message.sent", Some("person/avery"),
            json!({"from":"person/avery","to":SEAT,"content":"New garden mail.","status":"sent","tags":[]}));
        incremental.observe(&store).unwrap();
        assert!(incremental.needs("member:garden", 0));
        assert!(!incremental.needs("member:orchard", 0), "garden mail must not invalidate orchard's queue ranking");
    }

    #[test]
    fn unavailable_frontier_refuses_once_without_consuming_the_idle_epoch() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("graph.sqlite");
        let store = fixture(&path, root.path());
        let lagging = Store::open(&path, "alder").unwrap();
        for _ in 0..2 {
            assert!(lagging.admit_idle_suspend(SEAT, Some(0), Some(0), 9_000_000_000_000).unwrap().is_none());
        }
        let refusals: u64 = store.readers.get().query_row(
            "SELECT COUNT(*) FROM claims WHERE subject=?1 AND kind='runtime.action.failed'
             AND json_extract(body,'$.fields.code')='wake-frontier-lag'", [SEAT], |row| row.get(0)).unwrap();
        assert_eq!(refusals, 1);
        assert!(store.admit_idle_suspend(SEAT, Some(0), Some(0), 9_000_000_000_000).unwrap().is_some());
    }

    #[test]
    fn retire_records_idempotent_sleep_end_inside_its_own_transaction() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("graph.sqlite");
        let store = fixture(&path, root.path());
        let suspend = store.admit_idle_suspend(SEAT, Some(0), Some(0), 9_000_000_000_000).unwrap().unwrap();
        complete_suspend(&store, SEAT, &suspend);
        let ended: u128 = 9_000_000_300_000;
        store.connection.batched(|tx| {
            let state = crate::suspension::current_at_tx(tx, SEAT, None).unwrap().expect("tx fold sees its own suspension");
            assert_eq!(state.phase, "suspended");
            end_sleep_tx(tx, "alder", SEAT, &state, ended, "retire").unwrap();
            Ok::<_, anyhow::Error>(())
        }).unwrap().unwrap();
        store.connection.batched(|tx| {
            let state = crate::suspension::current_at_tx(tx, SEAT, None).unwrap().unwrap();
            end_sleep_tx(tx, "alder", SEAT, &state, ended, "retire").unwrap();
            Ok::<_, anyhow::Error>(())
        }).unwrap().unwrap();
        let receipts: (u64, String) = store.readers.get().query_row(
            "SELECT COUNT(*), json_extract(body,'$.fields') FROM claims WHERE kind='residency.sleep-ended'",
            [], |row| Ok((row.get(0)?, row.get(1)?))).unwrap();
        assert_eq!(receipts.0, 1, "a replayed retire reuses the sleep episode");
        let fields: Value = serde_json::from_str(&receipts.1).unwrap();
        assert_eq!(fields["end_class"], "retire");
        assert_eq!(fields["short_sleep"], 0, "a zero idle threshold cannot have a shorter sleep");
    }

    #[test]
    fn manual_holds_never_enter_automatic_wake_queue() {
        let root = tempfile::tempdir().unwrap();
        let store = fixture(&root.path().join("graph.sqlite"), root.path());
        let declaration = store.selected_desired_token(SEAT).unwrap().unwrap();
        let manual = store.connection.batched(|tx| append_residency_claim_tx(tx, "alder", SEAT,
            "runtime.action.requested", Some("person/avery"),
            &json!({"evidence":[declaration],"fields":{"action":"suspend","incarnation_id":"one"}}),
            &[], None, "manual-suspend")).unwrap().unwrap();
        complete_suspend(&store, SEAT, &manual.id);
        claim(&store, "message/manual", "message.sent", Some("person/avery"),
            json!({"from":"person/avery","to":SEAT,"content":"Please wait.","status":"sent","tags":[]}));
        assert_eq!(store.admit_idle_wake(SEAT, 1, i64::MAX as u128).unwrap(), IdleWakeAdmission::default());
    }
    #[test]
    fn demand_before_final_boundary_prevents_the_snapshot_receipt() {
        let root = tempfile::tempdir().unwrap();
        let store = fixture(&root.path().join("graph.sqlite"), root.path());
        let request = store.admit_idle_suspend(SEAT, Some(0), Some(0), i64::MAX as u128).unwrap().unwrap();
        assert!(store.recheck_idle_suspend(SEAT, &request, Some(0), Some(0)).unwrap());
        claim(&store, "message/between-validation-and-stop", "message.sent", Some("person/avery"),
            json!({"from":"person/avery","to":SEAT,"content":"Continue","status":"sent","tags":[]}));
        let fields = serde_json::from_value(json!({
            "operation_status":"snapshotting","harness":"omp","native_session_id":"native-one",
            "source_host":"alder","incarnation_id":"one"
        })).unwrap();
        assert!(!store.commit_idle_stop_boundary(SEAT, &request, Some(0), Some(0), fields).unwrap());
        assert!(store.operation_claim(&crate::suspension::suspend_snapshot_key(&request)).unwrap().is_none());
        assert_eq!(crate::suspension::current(&store,SEAT).unwrap().unwrap().phase,"quiescing");
    }
    #[test]
    fn silent_age_covers_only_idle_asleep_seats_at_a_caught_up_frontier() {
        let root = tempfile::tempdir().unwrap();
        let store = fixture(&root.path().join("graph.sqlite"), root.path());
        claim(&store, "message/held-silent", "message.sent", Some("agent/birch"),
            json!({"from":"agent/birch","to":SEAT,"content":"FYI","status":"sent","tags":["st3-silent"]}));
        assert_eq!(store.idle_residency_mail_summary().unwrap(), (true,None,None),
            "a running seat is not part of the asleep backlog");
        let request = store.admit_idle_suspend(SEAT,Some(0),Some(0),i64::MAX as u128).unwrap().unwrap();
        complete_suspend(&store,SEAT,&request);
        let summary = store.idle_residency_mail_summary().unwrap();
        assert!(summary.0 && summary.1.is_none() && summary.2.is_some());
        claim(&store, "message/held-silent", "message.staged", Some(SEAT),json!({"status":"staged"}));
        assert_eq!(store.idle_residency_mail_summary().unwrap(), (true,None,None),
            "a proven cleared backlog supports an age-zero sample");
        let lagging = Store::open(&root.path().join("graph.sqlite"),"alder").unwrap();
        assert!(!lagging.idle_residency_mail_summary().unwrap().0,
            "a reader without its configured source cannot prove a zero backlog");
    }
    #[test]
    fn automatic_boundary_is_visible_to_the_real_reducer_and_replays_after_reopen() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("graph.sqlite");
        let store = fixture(&path, root.path());
        let request = store.admit_idle_suspend(SEAT,Some(0),Some(0),i64::MAX as u128)
            .unwrap().unwrap();
        assert!(store.admit_idle_suspend(SEAT,Some(0),Some(0),i64::MAX as u128)
            .unwrap().is_none(),"the existing hold suppresses a duplicate request");
        let fields: BTreeMap<String,Value> = serde_json::from_value(json!({
            "operation_status":"snapshotting","harness":"omp","native_session_id":"native-one",
            "source_host":"alder","incarnation_id":"one"
        })).unwrap();
        assert!(store.commit_idle_stop_boundary(SEAT,&request,Some(0),Some(0),fields.clone()).unwrap());
        let boundary = store.operation_claim(&crate::suspension::suspend_snapshot_key(&request))
            .unwrap().expect("the ordinary operation reader must see the owner-fenced boundary");
        assert_eq!(crate::suspension::current(&store,SEAT).unwrap().unwrap().phase,"snapshotting");
        assert!(store.commit_idle_stop_boundary(SEAT,&request,Some(0),Some(0),fields).unwrap());
        assert_eq!(store.operation_claim(&crate::suspension::suspend_snapshot_key(&request))
            .unwrap().unwrap().id,boundary.id);
        // The ordinary reconciler completion writer must compose with the fenced boundary.
        store.append_claim(&ClaimInput {
            subject:SEAT.into(),kind:"runtime.action.succeeded".into(),actor:Some(SEAT.into()),
            fields:serde_json::from_value(json!({"action":"suspend","operation_status":"suspended"})).unwrap(),
            evidence:vec![request.clone()],expected_subject:None,
            idempotency_key:Some(crate::suspension::suspend_completed_key(&request)),
        }).unwrap();
        assert_eq!(crate::suspension::current(&store,SEAT).unwrap().unwrap().phase,"suspended");
        let reopened = Store::open(&path,"alder").unwrap();
        assert_eq!(crate::suspension::current(&reopened,SEAT).unwrap().unwrap().phase,"suspended");
        assert_eq!(reopened.operation_claim(&crate::suspension::suspend_snapshot_key(&request))
            .unwrap().unwrap().id,boundary.id);
    }
}
