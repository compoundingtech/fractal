//! Suspended seats: a running seat stops at a quiet boundary and later resumes its own native
//! session on the same host or, for portable drivers, another fleet host.
//!
//! A suspension is a pair of fenced runtime actions on the seat, like a restart. A person or
//! agent asks with `runtime.action.requested {action: suspend | resume}`; the seat's owner records
//! each phase as a `runtime.action.succeeded` or `runtime.action.failed` keyed by the request, with
//! the requester as actor so the phase replicates. The phase is derived from those claims:
//!
//! ```text
//! suspend: quiescing -> snapshotting -> suspended        (failed: the seat keeps running)
//! resume:  restoring -> verifying   -> resumed           (failed: the seat stays suspended)
//! ```
//!
//! The snapshot includes a host-local workspace bundle and, for portable drivers, native files.
//! The native session is the one the driver bound for the suspended incarnation, read
//! from its `harness.session-file` claim. Resume launches the driver with that session named in
//! [`RESUME_ENV`]; the driver relaunches the harness on exactly that session or exits with a typed
//! reason, and the resume completes only when the driver binds the same session again.
//!
//! Core never reads harness state itself. Whether a harness is quiet is the driver's report
//! (`quiescent` and `blocking` on `harness.observed`); core adds only st's own blockers.

use crate::model::ClaimRecord;
use crate::store::Store;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use crate::native_resume::{CONTINUE_ENV, CONTINUE_PATH_ENV, RESUME_ENV};
pub fn suspend_failed_key(request: &str) -> String {
    format!("agent-suspend-failed:{request}")
}
pub fn suspend_snapshot_key(request: &str) -> String {
    format!("agent-suspend-snapshot:{request}")
}
pub fn suspend_completed_key(request: &str) -> String {
    format!("agent-suspend-completed:{request}")
}
pub fn resume_failed_key(request: &str) -> String {
    format!("agent-resume-failed:{request}")
}
pub fn resume_started_key(request: &str) -> String {
    format!("agent-resume-started:{request}")
}
pub fn resume_completed_key(request: &str) -> String {
    format!("agent-resume-completed:{request}")
}

/// Where a seat's latest suspend or resume stands.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct Suspension {
    /// `suspend` or `resume`: the request this phase belongs to.
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_host: Option<String>,
    /// `quiescing`, `snapshotting`, `suspended`, `failed` (a refused suspend), `restoring`,
    /// `verifying` or `resumed`.
    pub phase: String,
    /// The request claim; clients follow one operation by it.
    pub operation_id: String,
    /// Who asked. The owner records each phase as this actor, so the phase replicates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_by: Option<String>,
    /// The suspend request this state descends from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suspend_operation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_session_id: Option<String>,
    /// The incarnation that was suspended, or the one a resume launched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub incarnation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suspended_at_unix_ms: Option<u128>,
    /// Why the last suspend or resume of this seat failed, as a stable code.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// What kept a refused suspend from finding the seat quiet.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocking: Vec<String>,
    pub updated_at_unix_ms: u128,
    /// When the request this phase belongs to was made.
    #[serde(default)]
    pub requested_at_unix_ms: u128,
}

impl Suspension {
    /// The seat has no process of its own and the reconciler does not start one.
    pub fn holds_seat(&self) -> bool {
        matches!(
            self.phase.as_str(),
            "quiescing" | "snapshotting" | "suspended" | "fencing-source" | "transferring" | "restoring" | "verifying"
        )
    }
}

/// Describe a publication against its running launch, or its existing suspension lineage.
/// Manual deferral is available only to owned-set publication; this introduces no policy.
pub fn publication_action(
    store: &Store,
    next: &crate::model::DesiredSubject,
    owned: bool,
    previous: impl FnOnce() -> Result<Option<crate::model::MemberSpec>>,
) -> Result<Option<crate::model::PlannedAction>> {
    if !matches!(next.kind.as_str(), "agent" | "stop") {
        return Ok(None);
    }
    let Some(hold) = current(store, &next.subject)?.filter(Suspension::holds_seat) else {
        return store.publication_action(next);
    };
    let suspension = hold
        .suspend_operation_id
        .as_deref()
        .unwrap_or(&hold.operation_id);
    // A stop does not compare launches, including when the prior declaration is legacy.
    if next.kind == "stop" {
        return Ok(Some(crate::model::PlannedAction {
            subject: next.subject.clone(),
            action: "stop".into(),
            reason: format!("suspension {suspension}: the explicit stop would discard the hold"),
        }));
    }
    let pending = crate::rollout::pending_manual_launch(store, &next.subject)?;
    // Only a held agent publication needs its previous launch. Do not impose full-member
    // deserialization on ordinary previews of legacy declarations without a suspension.
    let previous = if pending.is_none() { previous()? } else { None };
    let previous_member = pending
        .as_ref()
        .map(|(_, member)| member)
        .or(previous.as_ref());
    let same_launch = previous_member
        .zip(next.member.as_ref())
        .is_some_and(|(previous, next)| next.launch_changes(previous).is_empty());
    let manual = owned
        && next.kind == "agent"
        && crate::rollout::manual(next)
        && store
            .rollout(&next.subject)?
            .is_none_or(|operation| operation.phase == "superseded");
    let (action, reason) = if same_launch || manual {
        (
            "hold-kept",
            format!(
                "suspension {suspension}: hold kept; {}",
                if manual && !same_launch {
                    "launch change stays pending until explicit rollout after resume"
                } else {
                    "declared launch unchanged; environment edits take effect only on explicit resume"
                }
            ),
        )
    } else {
        (
            "start-suspended",
            format!(
                "suspension {suspension}: would start a suspended agent; the declared launch changed"
            ),
        )
    };
    Ok(Some(crate::model::PlannedAction {
        subject: next.subject.clone(),
        action: action.into(),
        reason,
    }))
}

pub fn field<'a>(claim: &'a ClaimRecord, name: &str) -> Option<&'a str> {
    claim
        .body
        .get("fields")
        .unwrap_or(&claim.body)
        .get(name)
        .and_then(Value::as_str)
}

pub fn action(claim: &ClaimRecord) -> Option<&str> {
    field(claim, "action")
}

fn evidence(claim: &ClaimRecord, index: usize) -> Option<&str> {
    claim
        .body
        .get("evidence")
        .and_then(|evidence| evidence.get(index))
        .and_then(Value::as_str)
}

/// The seat's requester-authored suspends and resumes, oldest first.
fn requests(store: &Store, subject: &str, at: Option<u64>) -> Result<Vec<ClaimRecord>> {
    let mut requests = store.claims_for(subject, Some("runtime.action.requested"))?;
    requests.retain(|claim| {
        at.is_none_or(|index| claim.store_index <= index)
            && claim.actor.is_some() && matches!(action(claim), Some("suspend" | "resume"))
    });
    Ok(requests)
}

fn operation_at(store: &Store, key: &str, at: Option<u64>) -> Result<Option<ClaimRecord>> {
    Ok(store.operation_claim(key)?.filter(|claim| at.is_none_or(|index| claim.store_index <= index)))
}

fn apply_failure(state: &mut Suspension, claim: &ClaimRecord) {
    state.code = field(claim, "code").map(str::to_owned);
    state.reason = field(claim, "reason").map(str::to_owned);
    state.blocking = claim
        .body
        .pointer("/fields/blocking")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    state.updated_at_unix_ms = claim.accepted_at_unix_ms;
}

/// The phase of the suspend request `request`.
pub fn suspend_state(store: &Store, request: &ClaimRecord, at: Option<u64>) -> Result<Suspension> {
    let mut state = Suspension {
        action: "suspend".into(),
        phase: "quiescing".into(),
        operation_id: request.id.clone(),
        requested_by: request.actor.clone(),
        suspend_operation_id: Some(request.id.clone()),
        incarnation_id: field(request, "incarnation_id").map(str::to_owned),
        updated_at_unix_ms: request.accepted_at_unix_ms,
        requested_at_unix_ms: request.accepted_at_unix_ms,
        ..Suspension::default()
    };
    if let Some(failed) = operation_at(store, &suspend_failed_key(&request.id), at)? {
        state.phase = "failed".into();
        apply_failure(&mut state, &failed);
        return Ok(state);
    }
    let snapshot = operation_at(store, &suspend_snapshot_key(&request.id), at)?;
    if let Some(snapshot) = &snapshot {
        state.phase = "snapshotting".into();
        state.harness = field(snapshot, "harness").map(str::to_owned);
        state.native_session_id = field(snapshot, "native_session_id").map(str::to_owned);
        state.source_host = field(snapshot, "source_host").map(str::to_owned);
        state.updated_at_unix_ms = snapshot.accepted_at_unix_ms;
    }
    if let Some(completed) = operation_at(store, &suspend_completed_key(&request.id), at)? {
        state.phase = "suspended".into();
        state.suspended_at_unix_ms = Some(completed.accepted_at_unix_ms);
        state.updated_at_unix_ms = completed.accepted_at_unix_ms;
    }
    Ok(state)
}

/// Where the seat's latest suspend or resume stands, or `None` when it has none that still
/// applies. A stop or an applied launch change ends it. A pending manual publication still
/// selects the incumbent launch for suspension and resume.
pub fn current(store: &Store, subject: &str) -> Result<Option<Suspension>> {
    current_at(store, subject, None)
}

/// Suspend/resume evidence and launch ancestry at a client snapshot.
pub fn current_at(store: &Store, subject: &str, at: Option<u64>) -> Result<Option<Suspension>> {
    let requests = requests(store, subject, at)?;
    let Some(request) = requests.last() else {
        return Ok(None);
    };
    // Only an agent declaration at this snapshot can be suspended; a stopped seat has none,
    // whatever manual launch is still pending.
    if store.selected_desired_kind_at(subject, at)?.as_deref() != Some("agent") {
        return Ok(None);
    }
    let lineage = store.suspension_lineage_at(subject, at)?;
    let visible = |index: u64| at.is_none_or(|at| index <= at);
    let mut moved = false;
    if action(request) == Some("resume") && field(request, "host").is_some() {
        for token in &lineage {
            if store.claim_by_id(token)?.filter(|claim| visible(claim.store_index)).is_some_and(|selected| {
                selected.body.get("evidence").and_then(Value::as_array)
                    .is_some_and(|items| items.iter().any(|id| id.as_str() == Some(&request.id)))
            }) { moved = true; break; }
        }
    }
    let mut applies = evidence(request, 0)
        .is_some_and(|token| lineage.iter().any(|item| item == token));
    if !applies
        && let Some(token) = evidence(request, 0)
        && let Some((incumbent, _)) = crate::rollout::pending_manual_launch_at(store, subject, at)?
    {
        let incumbent_lineage = store.suspension_lineage_from_at(&incumbent, at)?;
        let request_lineage = store.suspension_lineage_from_at(token, at)?;
        applies = request_lineage.iter().any(|token| incumbent_lineage.contains(token));
    }
    if !moved && !applies {
        return Ok(None);
    }
    if action(request) == Some("suspend") {
        return suspend_state(store, request, at).map(Some);
    }
    // A resume names the suspend it resumes; without that suspension it has nothing to resume.
    let Some(suspend) =
        evidence(request, 1).and_then(|id| requests.iter().find(|claim| claim.id == id))
    else {
        return Ok(None);
    };
    let mut state = suspend_state(store, suspend, at)?;
    if state.phase != "suspended" {
        return Ok(None);
    }
    state.action = "resume".into();
    state.operation_id = request.id.clone();
    state.requested_by = request.actor.clone();
    state.updated_at_unix_ms = request.accepted_at_unix_ms;
    state.requested_at_unix_ms = request.accepted_at_unix_ms;
    state.host = field(request, "host").map(str::to_owned);
    state.source_host = field(request, "source_host").map(str::to_owned).or(state.source_host);
    state.phase = if state.host.is_some() { "fencing-source" } else { "restoring" }.into();
    for (suffix, phase) in [("transfer", "transferring"), ("restored", "restoring")] {
        if let Some(claim) = operation_at(store, &format!("agent-resume-{suffix}:{}", request.id), at)? {
            state.phase = phase.into();
            state.updated_at_unix_ms = claim.accepted_at_unix_ms;
        }
    }
    // Placement is published only after full restoration. A crash before the separate phase
    // receipt must still let the destination continue the strict resume, without another fetch.
    if moved && matches!(state.phase.as_str(), "fencing-source" | "transferring") {
        state.phase = "restoring".into();
    }
    if let Some(failed) = operation_at(store, &resume_failed_key(&request.id), at)? {
        // A resume that fails leaves the seat suspended on the same snapshot, with the reason.
        state.phase = "suspended".into();
        apply_failure(&mut state, &failed);
        return Ok(Some(state));
    }
    if let Some(started) = operation_at(store, &resume_started_key(&request.id), at)? {
        state.phase = "verifying".into();
        state.updated_at_unix_ms = started.accepted_at_unix_ms;
    }
    if let Some(completed) = operation_at(store, &resume_completed_key(&request.id), at)? {
        state.phase = "resumed".into();
        state.incarnation_id = field(&completed, "incarnation_id").map(str::to_owned);
        state.updated_at_unix_ms = completed.accepted_at_unix_ms;
    }
    Ok(Some(state))
}

/// The native session the seat's driver bound for `incarnation`, with its harness.
pub fn bound_session(
    store: &Store,
    subject: &str,
    incarnation: &str,
) -> Result<Option<(String, String)>> {
    Ok(store
        .claims_for(subject, Some("harness.session-file"))?
        .into_iter()
        .rev()
        .find(|claim| field(claim, "incarnation_id") == Some(incarnation))
        .and_then(|claim| {
            Some((
                field(&claim, "harness")?.to_owned(),
                field(&claim, "session_id")?.to_owned(),
            ))
        }))
}

/// Recover the path from the exact binding a suspension captured, without extending its snapshot.
/// A later binding for another incarnation cannot substitute its transcript.
pub fn bound_session_path(
    store: &Store,
    subject: &str,
    incarnation: &str,
    harness: &str,
    session: &str,
) -> Result<Option<String>> {
    let Some(bound) = store
        .claims_for(subject, Some("harness.session-file"))?
        .into_iter()
        .rev()
        .find(|claim| field(claim, "incarnation_id") == Some(incarnation))
    else {
        return Ok(None);
    };
    anyhow::ensure!(
        field(&bound, "harness") == Some(harness)
            && field(&bound, "session_id") == Some(session),
        "the suspended native session binding no longer matches its snapshot"
    );
    let Some(path) = field(&bound, "path") else {
        // Legacy bindings without a path keep the existing ID-based restoration behavior.
        return Ok(None);
    };
    anyhow::ensure!(
        field(&bound, "agent") == Some(subject),
        "the suspended native session path belongs to another agent"
    );
    Ok(Some(path.to_owned()))
}

/// Why `subject`'s running `incarnation` cannot be suspended now; empty when it can.
///
/// The harness's own state comes only from its driver's `quiescent` and `blocking` report. The
/// rest is st's: a claimed step whose lease would lapse, and subagents st still records.
pub fn blockers(store: &Store, subject: &str, incarnation: &str) -> Result<Vec<String>> {
    let mut blocking = Vec::new();
    if let Some(harness) = store.current_harness(subject)?
        && harness.incarnation_id == incarnation
        && matches!(
            harness.state.as_str(),
            "unauthenticated" | "needs-login" | "blocked"
        )
        && harness.blocked_on.as_deref() == Some("human")
    {
        blocking.push("pending-ask".into());
    }
    let harness = store
        .latest_observation(subject, "harness.observed")?
        .filter(|claim| field(claim, "incarnation_id") == Some(incarnation));
    match harness {
        None => blocking.push("harness-unobserved".into()),
        Some(claim) => {
            let fields = claim.body.get("fields").unwrap_or(&claim.body);
            // Older OMP drivers reported idle without observing their native jobs.
            // Their existing `true` is not proof under the upgraded report contract.
            if fields.get("driver").and_then(Value::as_str) == Some("omp")
                && fields.get("background_jobs").is_none()
            {
                blocking.push("background-jobs-unreported".into());
            }
            match fields.get("quiescent").and_then(Value::as_bool) {
                Some(true) => {}
                Some(false) => {
                    let reported = fields
                        .get("blocking")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect::<Vec<_>>();
                    if reported.is_empty() {
                        blocking.push("harness-busy".into());
                    }
                    blocking.extend(reported);
                }
                None => blocking.push("quiescence-unreported".into()),
            }
        }
    }
    if store.work_for_reconcile(subject)?.iter().any(|step| {
        step.claimant.as_deref() == Some(subject)
            && matches!(step.status.as_str(), "claimed" | "working" | "verifying")
    }) {
        blocking.push("claimed-work".into());
    }
    if !store.open_subagents(subject)?.is_empty() {
        blocking.push("subagent-running".into());
    }
    if bound_session(store, subject, incarnation)?.is_none() {
        blocking.push("native-session-unbound".into());
    }
    blocking.sort();
    blocking.dedup();
    Ok(blocking)
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suspension_recovers_only_its_exact_incarnation_transcript_path() {
        let store = Store::open_memory("suspended-path").unwrap();
        let owner = "agent/suspended-path";
        let bind = |incarnation: &str, session: &str, path: &str| {
            store.append_claim(&crate::model::ClaimInput {
                subject: owner.into(), kind: "runtime.observed".into(), actor: Some(owner.into()),
                fields: std::collections::BTreeMap::from([
                    ("status".into(), Value::from("running")),
                    ("incarnation_id".into(), Value::from(incarnation)),
                ]),
                evidence: Vec::new(), expected_subject: None, idempotency_key: None,
            }).unwrap();
            store.append_claim(&crate::model::ClaimInput {
                subject: owner.into(), kind: "harness.session-file".into(), actor: Some(owner.into()),
                fields: std::collections::BTreeMap::from([
                    ("agent".into(), Value::from(owner)), ("harness".into(), Value::from("omp")),
                    ("incarnation_id".into(), Value::from(incarnation)),
                    ("session_id".into(), Value::from(session)), ("path".into(), Value::from(path)),
                ]),
                evidence: Vec::new(), expected_subject: None, idempotency_key: None,
            }).unwrap();
        };
        bind("suspended", "native", "/external/suspended.jsonl");
        bind("replacement", "native", "/external/replacement.jsonl");
        assert_eq!(
            bound_session_path(&store, owner, "suspended", "omp", "native").unwrap(),
            Some("/external/suspended.jsonl".into())
        );
        assert_eq!(bound_session_path(&store, owner, "missing", "omp", "native").unwrap(), None);
        assert!(bound_session_path(&store, owner, "suspended", "pi", "native").is_err());
        bind("suspended", "other", "/external/other.jsonl");
        assert!(bound_session_path(&store, owner, "suspended", "omp", "native").is_err());
    }

    #[test]
    fn legacy_id_only_suspension_has_no_path_to_recover() {
        let store = Store::open_memory("legacy-path").unwrap();
        store.append_claim(&crate::model::ClaimInput {
            subject: "agent/example".into(),
            kind: "runtime.observed".into(),
            actor: Some("agent/example".into()),
            fields: std::collections::BTreeMap::from([
                ("status".into(), Value::from("running")),
                ("incarnation_id".into(), Value::from("captured")),
            ]),
            evidence: Vec::new(), expected_subject: None, idempotency_key: None,
        }).unwrap();
        store.append_claim(&crate::model::ClaimInput {
            subject: "agent/example".into(),
            kind: "harness.session-file".into(),
            actor: Some("agent/example".into()),
            fields: std::collections::BTreeMap::from([
                ("harness".into(), Value::from("claude")),
                ("incarnation_id".into(), Value::from("captured")),
                ("session_id".into(), Value::from("native")),
            ]),
            evidence: Vec::new(), expected_subject: None, idempotency_key: None,
        }).unwrap();
        assert_eq!(bound_session_path(&store, "agent/example", "captured", "claude", "native").unwrap(), None);
        assert!(bound_session_path(&store, "agent/example", "captured", "claude", "other").is_err());
    }

}

#[cfg(test)]
mod background_job_tests {
    use super::*;

    #[test]
    fn legacy_omp_idle_is_unproven_until_the_upgraded_driver_reports_jobs() {
        let store = Store::open_memory("node").unwrap();
        for upgraded in [false, true] {
            let mut fields = std::collections::BTreeMap::from([
                ("driver".into(), Value::from("omp")),
                ("state".into(), Value::from("idle")),
                ("incarnation_id".into(), Value::from("fixture-one")),
                ("quiescent".into(), Value::from(true)),
            ]);
            if upgraded {
                fields.insert("background_jobs".into(), Value::from(0));
            }
            store
                .append_claim(&crate::model::ClaimInput {
                    subject: "agent/example".into(),
                    kind: "harness.observed".into(),
                    actor: Some("agent/example".into()),
                    fields,
                    evidence: Vec::new(),
                    expected_subject: None,
                    idempotency_key: None,
                })
                .unwrap();
            let blocking = blockers(&store, "agent/example", "fixture-one").unwrap();
            assert_eq!(
                blocking.iter().any(|reason| reason == "background-jobs-unreported"),
                !upgraded
            );
        }
    }

}

#[cfg(any(test, feature = "test-support"))]
pub mod shared_tests_seats;
