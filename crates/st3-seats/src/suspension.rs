//! The part of `suspension` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use crate::SeatsStoreCore;
use crate::model::ClaimRecord;
use crate::store::Store;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
pub use crate::native_resume::{CONTINUE_ENV, CONTINUE_PATH_ENV, RESUME_ENV};
pub use st3_store::suspension::*;
pub use st3_store::suspension::{Suspension, blockers, bound_session, current, current_at, publication_action, resume_completed_key, resume_failed_key, resume_started_key, suspend_completed_key, suspend_failed_key, suspend_snapshot_key};

/// Launcher-owned declaration identity forwarded into the durable native binding.
pub const LAUNCH_DESIRED_TOKEN_ENV: &str = "ST3_LAUNCH_DESIRED_TOKEN";

/// The diagnostic code a driver records when it cannot relaunch the named native session.
pub const RESUME_UNAVAILABLE_CODE: &str = "native-resume-unavailable";

/// The diagnostic code a driver records when it starts a new session instead of continuing.
pub const CONTINUE_UNAVAILABLE_CODE: &str = "native-continue-unavailable";

/// How long a resumed driver may take to bind its native session before the resume fails.
pub const VERIFY_TIMEOUT_MS: u128 = 180_000;

/// Original completed suspension, not the requester of a subsequently failed resume.
pub fn completed_suspend_at(store: &Store, request_id: &str, at: u64) -> Result<Option<Suspension>> {
    let Some(request) = store.claim_by_id(request_id)?.filter(|claim| claim.store_index <= at && action(claim) == Some("suspend")) else {
        return Ok(None);
    };
    Ok(Some(suspend_state(store, &request, Some(at))?).filter(|state| state.phase == "suspended"))
}

/// Normalized authored selector scope supplied by the owner to its driver.
pub const SELECTOR_SCOPE_ENV: &str = "ST3_NATIVE_SELECTOR_SCOPE";

/// New refusals have a typed identity, so a stale legacy authored refusal cannot swallow
/// a later permanent failure for the same session.
pub fn typed_continue_unavailable_key(subject: &str, session: &str, reason: &str, scope: Option<&str>) -> String {
    format!("{CONTINUE_UNAVAILABLE_CODE}:v2:{subject}:{session}:{reason}:{}", scope.unwrap_or("permanent"))
}

/// The key of the diagnostic a driver records once when it cannot continue `session`.
pub fn continue_unavailable_key(subject: &str, session: &str) -> String {
    format!("{CONTINUE_UNAVAILABLE_CODE}:{subject}:{session}")
}

/// Whether a durable driver binding proves a launch under the current import pair.
/// Launch provenance survives observation trimming and replication; unrelated declaration
/// edits do not reset bootstrap, and a late pre-repair binding cannot consume it.
pub fn omp_import_bootstrap_bound(
    store: &Store,
    subject: &str,
    session: &str,
    path: &str,
) -> Result<bool> {
    let Some(selected) = store.selected_desired_token(subject)? else {
        return Ok(false);
    };
    let declarations = store.claims_for(subject, Some("intent.desired"))?;
    let Some(position) = declarations.iter().position(|claim| claim.id == selected) else {
        anyhow::bail!("the selected import declaration {selected} is missing");
    };
    let carries_pair = |claim: &ClaimRecord| {
        let environment = &claim.body["member"]["environment"];
        environment[RESUME_ENV].as_str() == Some(session)
            && environment[crate::rollout::RESUME_PATH_ENV].as_str() == Some(path)
    };
    let mut introduced = &declarations[position];
    for prior in declarations[..position].iter().rev() {
        if !carries_pair(prior) {
            break;
        }
        introduced = prior;
    }
    for bound in store
        .claims_for(subject, Some("harness.session-file"))?
        .iter()
        .rev()
    {
        if field(bound, "harness") != Some("omp")
            || !field(bound, "session_id").is_some_and(|id| !id.is_empty())
        {
            continue;
        }
        if !field(bound, "incarnation_id").is_some_and(|id| !id.is_empty()) {
            continue;
        }
        let Some(token) = field(bound, "desired_token").filter(|token| !token.is_empty()) else {
            continue;
        };
        let Some(declaration) = declarations.iter().find(|claim| claim.id == token) else {
            continue;
        };
        if carries_pair(declaration)
            && (token == introduced.id || store.claim_is_after(token, &introduced.id)?)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The native session a relaunch of `subject` on `harness` continues, with its path: the last
/// one the seat's driver bound for that harness. A seat relaunched for a fresh context since,
/// or whose driver could not continue that session before, starts a new one.
pub fn continue_session(
    store: &Store,
    subject: &str,
    harness: &str,
    account: Option<&str>,
    selector_scope: Option<&str>,
) -> Result<Option<(String, Option<String>)>> {
    let Some(bound) = store
        .claims_for(subject, Some("harness.session-file"))?
        .into_iter()
        .rev()
        .find(|claim| field(claim, "harness") == Some(harness))
    else {
        return Ok(None);
    };
    // Account directories also hold native sessions. Never carry a transcript between logins;
    // only a relaunch on the account that bound it may continue it.
    if field(&bound, "account_ref") != account {
        return Ok(None);
    }
    let Some(session) = field(&bound, "session_id").map(str::to_owned) else {
        return Ok(None);
    };
    let mut fresh_since = false;
    for claim in store.claims_for(subject, Some("runtime.action.requested"))? {
        if field(&claim, "action") == Some("fresh-context")
            && store.claim_is_after(&claim.id, &bound.id)?
        {
            fresh_since = true;
            break;
        }
    }
    let legacy_refusal = store.operation_claim(&continue_unavailable_key(subject, &session))?;
    let mut refused = false;
    for claim in store.claims_for(subject, Some("harness.diagnostic"))? {
        let legacy = legacy_refusal.as_ref().is_some_and(|legacy| legacy.id == claim.id);
        let typed = field(&claim, "session_id") == Some(session.as_str())
            && field(&claim, "code") == Some(CONTINUE_UNAVAILABLE_CODE);
        let in_scope = legacy || typed;
        let authored = field(&claim, "status") == Some("authored-session-selection");
        let applies = in_scope && if authored {
            selector_scope.is_some_and(|scope| field(&claim, "selector_scope").is_none_or(|recorded| recorded == scope))
        } else {
            true
        };
        // Only scoped selector refusals can be superseded by a newer binding.
        // A permanent refusal still applies to the same session after a rebind.
        if applies && (!authored || store.claim_is_after(&claim.id, &bound.id)?) {
            refused = true;
            break;
        }
    }
    if fresh_since || refused {
        return Ok(None);
    }
    Ok(Some((session, field(&bound, "path").map(str::to_owned))))
}

/// The driver's half of quiescence: whether a harness in this state is at a clean boundary, and
/// the typed reasons it is not. `state`, `blocked_on`, `ask` and `input_buffer` are the values a
/// driver already publishes on `harness.observed`.
pub fn harness_quiescence(
    state: &str,
    blocked_on: &str,
    ask: &str,
    input_buffer: &str,
) -> (bool, Vec<&'static str>) {
    let mut blocking = Vec::new();
    match state {
        "idle" | "ready" => {}
        "working" => blocking.push("turn-in-flight"),
        "starting" => blocking.push("starting"),
        _ => blocking.push("harness-indeterminate"),
    }
    // An axis the harness cannot see (`unknown`) is not a blocker; a person's ask is.
    if blocked_on == "human" || !matches!(ask, "" | "none" | "unknown") {
        blocking.push("pending-ask");
    }
    if input_buffer == "nonempty" {
        blocking.push("unsent-input");
    }
    (blocking.is_empty(), blocking)
}

/// Add the driver's quiescence report to a `harness.observed` claim's fields, read from the
/// state the same claim already carries. Every driver publishes through this.
pub fn annotate_quiescence(fields: &mut std::collections::BTreeMap<String, Value>) {
    if let Ok(operation) = std::env::var(crate::rollout::OPERATION_ENV) {
        fields.insert("rollout_operation".into(), Value::String(operation));
    }
    let text = |name: &str| {
        fields
            .get(name)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let (_, mut blocking) = harness_quiescence(
        &text("state"),
        &text("blocked_on"),
        &text("ask"),
        &text("input_buffer"),
    );
    if fields.get("driver").and_then(Value::as_str) == Some("omp") {
        match fields.get("background_jobs").and_then(Value::as_u64) {
            Some(0) => {}
            Some(_) => blocking.push("background-jobs-running"),
            None => blocking.push("background-jobs-unreported"),
        }
    }
    fields.insert("quiescent".into(), Value::Bool(blocking.is_empty()));
    fields.insert(
        "blocking".into(),
        Value::Array(blocking.into_iter().map(Value::from).collect()),
    );
}

#[cfg(test)]
mod background_job_tests {
    use super::*;

    #[test]
    fn omp_idle_requires_a_known_empty_native_job_snapshot() {
        for (jobs, reason) in [(Value::from(1), Some("background-jobs-running")), (Value::Null, Some("background-jobs-unreported")), (Value::from(0), None)] {
            let mut fields = std::collections::BTreeMap::from([
                ("driver".into(), Value::from("omp")), ("state".into(), Value::from("idle")), ("background_jobs".into(), jobs),
            ]);
            annotate_quiescence(&mut fields);
            assert_eq!(fields["quiescent"], reason.is_none());
            assert_eq!(fields["blocking"], reason.map_or_else(|| serde_json::json!([]), |reason| serde_json::json!([reason])));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use st3_store::suspension::shared_tests_seats::*;
    use st3_store::suspension::shared_tests_seats::{continuation_claim};

    #[test]
    fn continuation_refusal_scope_repairs_legacy_and_keeps_later_failure_permanent() {
        for legacy in [false, true] {
            let store = Store::open_memory("node").unwrap();
            continuation_claim(&store, "harness.session-file", json!({"harness":"omp","session_id":"native","account_ref":"one"}), None);
            let key = if legacy { continue_unavailable_key("agent/example", "native") } else {
                typed_continue_unavailable_key("agent/example", "native", "authored-session-selection", Some("selector"))
            };
            let mut fields = json!({"code":CONTINUE_UNAVAILABLE_CODE,"status":"authored-session-selection","session_id":"native"});
            if !legacy { fields["selector_scope"] = json!("selector"); }
            continuation_claim(&store, "harness.diagnostic", fields, Some(key));
            assert!(continue_session(&store, "agent/example", "omp", Some("one"), Some("selector")).unwrap().is_none());
            assert_eq!(continue_session(&store, "agent/example", "omp", Some("one"), None).unwrap().unwrap().0, "native");
            assert!(continue_session(&store, "agent/example", "omp", Some("two"), None).unwrap().is_none());
            continuation_claim(&store, "harness.session-file", json!({"harness":"omp","session_id":"native","account_ref":"one"}), None);
            assert_eq!(continue_session(&store, "agent/example", "omp", Some("one"), Some("selector")).unwrap().unwrap().0, "native");
            let permanent = typed_continue_unavailable_key("agent/example", "native", "transcript-missing", None);
            continuation_claim(&store, "harness.diagnostic", json!({"code":CONTINUE_UNAVAILABLE_CODE,"status":"transcript-missing","session_id":"native"}), Some(permanent.clone()));
            assert!(store.operation_claim(&permanent).unwrap().is_some());
            assert!(continue_session(&store, "agent/example", "omp", Some("one"), None).unwrap().is_none());
            continuation_claim(&store, "harness.session-file", json!({"harness":"omp","session_id":"native","account_ref":"one"}), None);
            assert!(continue_session(&store, "agent/example", "omp", Some("one"), None).unwrap().is_none());
            assert!(continue_session(&store, "agent/example", "omp", Some("one"), Some("selector")).unwrap().is_none());
        }
    }

    #[test]
    fn continuation_permanent_refusals_survive_rebinding_only_the_refused_session() {
        for legacy in [false, true] {
            let store = Store::open_memory("node").unwrap();
            let key = if legacy {
                continue_unavailable_key("agent/example", "native")
            } else {
                typed_continue_unavailable_key("agent/example", "native", "transcript-missing", None)
            };
            continuation_claim(&store, "harness.diagnostic", json!({
                "code":CONTINUE_UNAVAILABLE_CODE,"status":"transcript-missing","session_id":"native"
            }), Some(key));
            continuation_claim(&store, "harness.session-file", json!({"harness":"omp","session_id":"native"}), None);
            assert!(continue_session(&store, "agent/example", "omp", None, None).unwrap().is_none());
            continuation_claim(&store, "harness.session-file", json!({"harness":"omp","session_id":"other"}), None);
            assert_eq!(continue_session(&store, "agent/example", "omp", None, None).unwrap().unwrap().0, "other");
        }
    }

    #[test]
    fn continuation_fresh_context_wins_over_stale_authored_refusal() {
        let store = Store::open_memory("node").unwrap();
        continuation_claim(&store, "harness.session-file", json!({"harness":"omp","session_id":"native"}), None);
        continuation_claim(&store, "harness.diagnostic", json!({"status":"authored-session-selection"}), Some(continue_unavailable_key("agent/example", "native")));
        continuation_claim(&store, "runtime.action.requested", json!({"action":"fresh-context"}), None);
        assert!(continue_session(&store, "agent/example", "omp", None, None).unwrap().is_none());
    }

    #[test]
    fn continuation_concurrent_permanent_receipts_are_one_durable_refusal() {
        let store = std::sync::Arc::new(Store::open_memory("node").unwrap());
        continuation_claim(&store, "harness.session-file", json!({"harness":"omp","session_id":"native"}), None);
        std::thread::scope(|threads| {
            for _ in 0..8 {
                let store = store.clone();
                threads.spawn(move || continuation_claim(&store, "harness.diagnostic",
                    json!({"code":CONTINUE_UNAVAILABLE_CODE,"status":"transcript-missing","session_id":"native"}),
                    Some(typed_continue_unavailable_key("agent/example", "native", "transcript-missing", None))));
            }
        });
        assert_eq!(store.claims_for("agent/example", Some("harness.diagnostic")).unwrap().len(), 1);
        assert!(continue_session(&store, "agent/example", "omp", None, None).unwrap().is_none());
    }

    #[test]
    fn only_an_idle_harness_with_nothing_pending_is_quiescent() {
        assert_eq!(
            harness_quiescence("idle", "none", "none", "empty"),
            (true, vec![])
        );
        assert_eq!(harness_quiescence("ready", "", "", ""), (true, vec![]));
        assert_eq!(
            harness_quiescence("working", "none", "none", "empty"),
            (false, vec!["turn-in-flight"])
        );
        assert_eq!(
            harness_quiescence("idle", "human", "question", "nonempty"),
            (false, vec!["pending-ask", "unsent-input"])
        );
        assert_eq!(
            harness_quiescence("indeterminate", "none", "none", "empty"),
            (false, vec!["harness-indeterminate"])
        );
    }
}
