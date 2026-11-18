//! Incremental projection of custody-committed admission facts, not request-time samples.
use std::collections::BTreeMap;
use std::sync::{Mutex, atomic::Ordering};
use serde::Deserialize;
use serde_json::Value;
use crate::store::Store;
use crate::lifecycle_objectives::{LifecycleSample, LifecycleTerminal, ratio_row};
use crate::store::resource_admission_objectives::{Coverage, KIND};

const DEADLINE_MS: u64 = 2 * 60 * 60 * 1000;
#[derive(Deserialize)]
struct Evidence {
    host: String,
    class: String,
    coverage: String,
    queued_at: u64,
    admitted_at: Option<u64>,
    retired_at: Option<u64>,
    state: String,
    refusal: Option<String>,
    finite: bool,
    admission_timeout_ms: Option<u64>,
    clock_uncertain: bool,
}
#[derive(Deserialize)]
struct GateEvidence {
    id: String, host: String, class: String, onset_ms: u64, at_ms: u64,
    outcome: String, reason: String,
}
#[derive(Default)]
struct GateSample {
    onset: u64, accepted: bool, grant: Option<u64>, failure: Option<u64>, excluded: Option<u64>,
}
impl GateSample {
    fn sample(&self) -> LifecycleSample {
        let terminal = if let Some(at) = self.grant { LifecycleTerminal::Accepted(at) }
        else if let Some(at) = self.failure { LifecycleTerminal::Failed(at) }
        else if let Some(at) = self.excluded {
            if at <= self.onset.saturating_add(DEADLINE_MS) { LifecycleTerminal::Excluded }
            else { LifecycleTerminal::Failed(at) }
        } else { LifecycleTerminal::Open };
        LifecycleSample { onset_ms: self.onset, terminal }
    }
}
#[derive(Default)]
struct Projection {
    cursor: u64, members: BTreeMap<String, Evidence>, unreadable: bool,
    gates: BTreeMap<(String, String, String), GateSample>,
}
#[derive(Default)]
struct Cache(Mutex<Projection>);

#[derive(Default)]
struct Started(std::sync::atomic::AtomicBool);

fn wall_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
}

fn outcome_at(onset: u64, terminal: LifecycleTerminal) -> Option<u64> {
    let deadline = onset.saturating_add(DEADLINE_MS);
    match terminal {
        LifecycleTerminal::Accepted(at) | LifecycleTerminal::Failed(at) => Some(at.min(deadline)),
        LifecycleTerminal::Open => Some(deadline),
        LifecycleTerminal::Excluded => None,
    }
}

fn next_boundary(projection: &Projection, now: u64) -> Option<u64> {
    let native = projection.members.values().filter(|evidence| evidence.finite).filter_map(|evidence| {
        let terminal = if let Some(at) = evidence.admitted_at { LifecycleTerminal::Accepted(at) }
        else if evidence.state == "refused" {
            let at = if evidence.refusal.as_deref() == Some("admission-expired") {
                evidence.queued_at.saturating_add(evidence.admission_timeout_ms.unwrap_or(DEADLINE_MS))
            } else { evidence.queued_at };
            LifecycleTerminal::Failed(at)
        } else if let Some(at) = evidence.retired_at { LifecycleTerminal::Failed(at) }
        else { LifecycleTerminal::Open };
        outcome_at(evidence.queued_at, terminal)
    });
    let gate = projection.gates.values().filter(|sample| sample.accepted)
        .filter_map(|sample| outcome_at(sample.onset, sample.sample().terminal));
    native.chain(gate).flat_map(|at| [0, 5 * 60_000, 30 * 60_000, 3_600_000, 6 * 3_600_000, 30 * 86_400_000]
        .into_iter().map(move |width| at.saturating_add(width).saturating_add(1)))
        .filter(|boundary| *boundary > now).min()
}

/// Claim notifications catch up immutable source facts. The sole timer is the next
/// lifecycle/accounting boundary: no membership scans, source polling or timer-produced facts.
pub(crate) fn start(state: &crate::api::AppState) {
    if state.store.fractal.extensions.get::<Started>().0.swap(true, Ordering::AcqRel) { return; }
    let state = state.clone();
    let mut changed = state.event_notify.subscribe();
    tokio::spawn(async move {
        loop {
            changed.borrow_and_update();
            let store = state.store.clone();
            let state_dir = state.state_dir.clone();
            let captured = tokio::task::spawn_blocking(move || {
                let cache = store.fractal.extensions.get::<Cache>();
                let mut projection = cache.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                catch_up(&store, &mut projection)?;
                let boundary = next_boundary(&projection, wall_now());
                drop(projection);
                // Evaluate the complete native row set so existing declared owner bindings
                // dispatch even when nobody has an SLO stream or request open. A partial
                // row set would incorrectly withdraw unrelated native owner bindings.
                let rows = crate::api::operations::current_slo_rows(&store, &state_dir)?;
                crate::declared_objectives::check_and_report(&store, &rows);
                Ok::<_, anyhow::Error>(boundary)
            }).await;
            let boundary = match captured {
                Ok(Ok(boundary)) => boundary,
                error => {
                    tracing::warn!(?error, "admission objective source catchup unavailable");
                    if changed.changed().await.is_err() { return; }
                    continue;
                }
            };
            tokio::select! {
                result = changed.changed() => { if result.is_err() { return; } }
                () = async {
                    match boundary {
                        Some(at) => tokio::time::sleep(std::time::Duration::from_millis(at.saturating_sub(wall_now()).max(1))).await,
                        None => std::future::pending::<()>().await,
                    }
                } => { crate::api::signal_local_change(&state); }
            }
        }
    });
}

fn catch_up(store: &Store, projection: &mut Projection) -> anyhow::Result<()> {
    loop {
        let page = store.claims_by_kinds_after(&[KIND, "gate.admission.observed"], None, projection.cursor, 1000)?;
        for claim in &page.claims {
            projection.cursor = projection.cursor.max(claim.store_index);
            if claim.kind == "gate.admission.observed" {
                match claim.body.get("fields").cloned().and_then(|value| serde_json::from_value::<GateEvidence>(value).ok()) {
                    Some(event) if event.host == claim.origin => {
                        let sample = projection.gates.entry((event.host, event.class, event.id)).or_default();
                        sample.onset = event.onset_ms;
                        match event.outcome.as_str() {
                            "accepted" => sample.accepted = true,
                            "granted" => sample.grant = Some(event.at_ms),
                            "failed" => sample.failure = Some(event.at_ms),
                            "excluded" if event.reason == "superseded" => sample.excluded = Some(event.at_ms),
                            _ => projection.unreadable = true,
                        }
                    }
                    _ => projection.unreadable = true,
                }
                continue;
            }
            match claim.body.get("fields").cloned().and_then(|value| serde_json::from_value::<Evidence>(value).ok()) {
                Some(mut evidence) if evidence.host == claim.origin => {
                    // Later execution/retirement clock uncertainty cannot revoke a proven
                    // admission clock. Replaying the first grant deterministically restores it.
                    if let Some(previous) = projection.members.get(&claim.subject)
                        && previous.admitted_at.is_some() && previous.admitted_at == evidence.admitted_at
                        && previous.queued_at == evidence.queued_at {
                        evidence.clock_uncertain = previous.clock_uncertain;
                    }
                    projection.members.insert(claim.subject.clone(), evidence);
                }
                _ => projection.unreadable = true,
            }
        }
        if page.claims.len() < 1000 { break; }
    }
    Ok(())
}

pub(crate) fn rows(store: &Store, now: u64) -> anyhow::Result<Vec<Value>> {
    let cache = store.fractal.extensions.get::<Cache>();
    let mut projection = cache.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    catch_up(store, &mut projection)?;
    let mut groups = BTreeMap::<(String, String), (Vec<LifecycleSample>, bool, BTreeMap<String, u64>)>::new();
    // Expected local classes remain visible as unknown before authority acquisition, including zero work.
    for class in ["landing", "check", "agent"] {
        groups.insert((store.origin().to_owned(), class.into()), Default::default());
    }
    for evidence in projection.members.values() {
        let cutoff = now.saturating_sub(30 * 24 * 60 * 60 * 1000);
        let terminal_at = evidence.admitted_at.or(evidence.retired_at).or_else(||
            (evidence.state == "refused").then_some(evidence.queued_at.saturating_add(
                evidence.admission_timeout_ms.unwrap_or(0))));
        if terminal_at.is_some_and(|at| at < cutoff)
            && evidence.queued_at.saturating_add(DEADLINE_MS) < cutoff { continue; }
        let (samples, uncertain, coverages) = groups.entry((evidence.host.clone(), evidence.class.clone())).or_default();
        *coverages.entry(evidence.coverage.clone()).or_default() += 1;
        *uncertain |= evidence.clock_uncertain || evidence.queued_at > now
            || evidence.admitted_at.is_some_and(|at| at < evidence.queued_at || at > now)
            || evidence.retired_at.is_some_and(|at| at < evidence.queued_at || at > now)
            || evidence.queued_at.checked_add(DEADLINE_MS).is_none();
        let terminal = if !evidence.finite {
            // Not an eligible finite request; diagnostic exclusion, not a successful grant.
            LifecycleTerminal::Excluded
        } else if let Some(at) = evidence.admitted_at {
            LifecycleTerminal::Accepted(at)
        } else if evidence.state == "refused" {
            match evidence.refusal.as_deref() {
                Some("admission-expired") => match evidence.admission_timeout_ms.and_then(|timeout| evidence.queued_at.checked_add(timeout)) {
                    Some(at) => LifecycleTerminal::Failed(at),
                    None => { *uncertain = true; LifecycleTerminal::Open }
                },
                Some("clock-uncertain") => { *uncertain = true; LifecycleTerminal::Open },
                _ => LifecycleTerminal::Failed(evidence.queued_at),
            }
        } else if let Some(at) = evidence.retired_at {
            // No typed supersession proof exists in the withdrawal API. Never call it success
            // or silently exclude cancellation; even an early withdrawal is failed admission.
            LifecycleTerminal::Failed(at)
        } else { LifecycleTerminal::Open };
        samples.push(LifecycleSample { onset_ms: evidence.queued_at, terminal });
    }
    for ((host, class, _), sample) in &projection.gates {
        if !sample.accepted { continue; }
        let (samples, unknown, coverages) = groups.entry((host.clone(), class.clone())).or_default();
        samples.push(sample.sample());
        *unknown = true; // A spool is not a complete-adoption certificate.
        *coverages.entry("gate-logical-unadmitted".into()).or_default() += 1;
    }
    let active = store.fractal.extensions.get::<Coverage>().0.load(Ordering::Acquire);
    Ok(groups.into_iter().map(|((host, class), (samples, uncertain, coverages))| {
        let known = active && host == store.origin() && !projection.unreadable && !uncertain;
        let id = format!("build/admission/{host}/{class}");
        let mut row = ratio_row(&id, "Accepted finite requests granted capacity within two hours", now,
            DEADLINE_MS, 0.10, &samples, known);
        row["coverage_modes"] = serde_json::json!(coverages);
        row["source"] = serde_json::json!({"kind": KIND, "host": host, "local_authority": known,
            "cut": projection.cursor, "remote_completeness": false});
        row
    }).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::resource_membership::{Admission, Fit, LaneRequest};
    use serde_json::json;

    fn request(id: &str) -> LaneRequest {
        serde_json::from_value(json!({
            "id": id, "name": "build", "host": "example-host", "maxMembers": 1,
            "capacity": {"cores": 1, "memory": 100}, "coverage": "logical",
            "admissionTimeoutMs": null, "class": "check",
            "owner": {"run": format!("mission-run/{id}"), "step": "compile", "attempt": 1, "invocation": null}
        })).unwrap()
    }
    fn fit(blocked: bool) -> Fit {
        Fit { room: Some(fractal_runtime::Capacity { cores: 4, memory: 1000 }),
            held_bytes: if blocked { 1000 } else { 0 }, ..Fit::default() }
    }
    fn check(store: &Store, now: u64) -> Value {
        rows(store, now).unwrap().into_iter().find(|row| row["id"] == "build/admission/example-host/check").unwrap()
    }

    #[test]
    fn queued_crossing_retry_and_late_grant_are_one_failure() {
        let store = Store::open_memory("example-host").unwrap();
        store.start_resource_authority("example-host", "epoch-one").unwrap();
        let request = request("request-one");
        assert!(matches!(store.request_resource_membership(&request, fit(true), 1000).unwrap(), Admission::Queued { .. }));
        assert_eq!(check(&store, 1000 + DEADLINE_MS + 1)["evidence"]["bad"].as_f64(), Some(1.0));
        assert!(matches!(store.request_resource_membership(&request, fit(true), 2000).unwrap(), Admission::Queued { .. }));
        assert!(matches!(store.request_resource_membership(&request, fit(false), 1000 + DEADLINE_MS + 2).unwrap(), Admission::Admitted(_)));
        let row = check(&store, 1000 + DEADLINE_MS + 3);
        assert_eq!(row["evidence"]["bad"].as_f64(), Some(1.0));
        assert_eq!(row["evidence"]["total"].as_f64(), Some(1.0));
    }

    #[test]
    fn impossible_and_withdrawn_requests_are_not_successful_stops() {
        let store = Store::open_memory("example-host").unwrap();
        store.start_resource_authority("example-host", "epoch-one").unwrap();
        let mut impossible = request("impossible");
        impossible.name = "impossible-lane".into();
        impossible.capacity.as_mut().unwrap().cores = 100;
        assert!(matches!(store.request_resource_membership(&impossible, fit(false), 1000).unwrap(), Admission::Refused { .. }));
        let withdrawn = request("withdrawn");
        store.request_resource_membership(&withdrawn, fit(true), 1000).unwrap();
        store.withdraw_resource_membership(&withdrawn.id, &withdrawn.owner, 2000).unwrap();
        let row = check(&store, 3000);
        assert_eq!(row["evidence"]["bad"].as_f64(), Some(2.0));
        assert_eq!(row["evidence"]["total"].as_f64(), Some(2.0));
    }

    #[test]
    fn reopening_replays_open_waiter_without_certifying_an_absent_authority() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("graph.db");
        {
            let store = Store::open(&path, "example-host").unwrap();
            store.start_resource_authority("example-host", "epoch-one").unwrap();
            store.request_resource_membership(&request("survivor"), fit(true), 1000).unwrap();
        }
        let store = Store::open(&path, "example-host").unwrap();
        let row = check(&store, 1000 + DEADLINE_MS + 1);
        assert_eq!(row["status"], "no-data");
        assert_eq!(row["evidence"]["bad"].as_f64(), Some(1.0));
        assert!(row["remaining_percent"].is_null());
        store.start_resource_authority("example-host", "epoch-two").unwrap();
        assert_eq!(check(&store, 1000 + DEADLINE_MS + 1)["evidence"]["total"].as_f64(), Some(1.0));
    }

    #[test]
    fn late_supersession_does_not_exclude_a_crossed_deadline() {
        let sample = GateSample { onset: 1000, excluded: Some(1001 + DEADLINE_MS), ..Default::default() }.sample();
        assert!(matches!(sample.terminal, LifecycleTerminal::Failed(_)));
    }

    #[test]
    fn backwards_queue_clock_is_unknown_not_a_good_admission() {
        let store = Store::open_memory("example-host").unwrap();
        store.start_resource_authority("example-host", "epoch").unwrap();
        let request = request("clock");
        store.request_resource_membership(&request, fit(true), 1000).unwrap();
        store.request_resource_membership(&request, fit(false), 999).unwrap();
        let row = check(&store, 3000);
        assert_eq!(row["status"], "no-data");
        assert!(row["remaining_percent"].is_null());
    }
}

#[cfg(test)]
mod inherited_tests {
    use super::*;
    use crate::store::resource_membership::{Admission, Fit, LaneRequest};

    #[test]
    fn inherited_child_context_does_not_add_an_admission() {
        let store = Store::open_memory("example-host").unwrap();
        store.start_resource_authority("example-host", "epoch").unwrap();
        let request: LaneRequest = serde_json::from_value(serde_json::json!({
            "id": "parent", "name": "checks", "host": "example-host", "maxMembers": 1,
            "capacity": {"cores": 1, "memory": 100}, "coverage": "logical",
            "admissionTimeoutMs": null, "class": "check",
            "owner": {"run": "mission-run/parent", "step": "compile", "attempt": 1, "invocation": null}
        })).unwrap();
        let fit = || Fit { room: Some(fractal_runtime::Capacity { cores: 4, memory: 1000 }), ..Fit::default() };
        let Admission::Admitted(grant) = store.request_resource_membership(&request, fit(), 1000).unwrap() else { panic!("not admitted") };
        store.attach_resource_child(&grant, &request.owner, "mission-run/child").unwrap();
        store.request_resource_membership(&request, fit(), 2000).unwrap();
        let row = rows(&store, 3000).unwrap().into_iter().find(|row| row["id"] == "build/admission/example-host/check").unwrap();
        assert_eq!(row["evidence"]["total"].as_f64(), Some(1.0));
        assert_eq!(row["evidence"]["bad"].as_f64(), Some(0.0));
    }
}

#[cfg(test)]
mod deadline_tests {
    use super::*;

    fn queued() -> Projection {
        let mut projection = Projection::default();
        projection.members.insert("request".into(), Evidence {
            host: "example-host".into(), class: "check".into(), coverage: "logical".into(),
            queued_at: 1000, admitted_at: None, retired_at: None, state: "queued".into(),
            refusal: None, finite: true, admission_timeout_ms: None, clock_uncertain: false,
        });
        projection
    }

    #[test]
    fn an_idle_waiter_schedules_strict_deadline_without_any_read_or_completion() {
        let projection = queued();
        assert_eq!(next_boundary(&projection, 2000), Some(1000 + DEADLINE_MS + 1));
        assert_eq!(next_boundary(&projection, 1000 + DEADLINE_MS), Some(1000 + DEADLINE_MS + 1));
        assert_eq!(next_boundary(&projection, 1000 + DEADLINE_MS + 1), Some(1000 + DEADLINE_MS + 5 * 60_000 + 1));
    }

    #[test]
    fn late_grant_keeps_original_budget_expiry_and_empty_source_has_no_timer() {
        let mut projection = queued();
        projection.members.get_mut("request").unwrap().admitted_at = Some(1000 + DEADLINE_MS + 50);
        assert_eq!(next_boundary(&projection, 1000 + DEADLINE_MS + 100),
            Some(1000 + DEADLINE_MS + 5 * 60_000 + 1));
        assert_eq!(next_boundary(&Projection::default(), 1000), None);
        assert_eq!(next_boundary(&projection, 1000 + DEADLINE_MS + 30 * 86_400_000 + 1), None);
    }
}
