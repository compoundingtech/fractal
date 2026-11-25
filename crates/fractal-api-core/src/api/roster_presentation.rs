//! Changed-agent live presentation; clocks invalidate only the card whose evidence expires.
use super::*;
use anyhow::Context as _;

#[derive(Default)]
struct Cache {
    cards: BTreeMap<String, Card>,
    rebuilds: u64,
}

struct Card {
    source: Value,
    harness: Option<crate::model::CurrentHarnessView>,
    clock_state: [bool; 5],
    delivery_revision: Option<u64>,
    subagents: Vec<crate::store::SubagentView>,
    deadline: Option<u128>,
    row: Value,
}

#[derive(Default)]
struct Presentation(std::sync::Mutex<Cache>);

pub(super) fn work(store: &Store) -> Value {
    let state = store.extension::<Presentation>();
    let cache = state.0.lock().expect("roster presentation poisoned");
    json!({"card_rebuilds": cache.rebuilds})
}

pub(super) fn present(
    store: &Store,
    source: &[Value],
    subagents: Vec<crate::store::SubagentView>,
    now: u128,
) -> anyhow::Result<(Vec<Value>, Option<u128>)> {
    let state = store.extension::<Presentation>();
    let mut cache = state.0.lock().expect("roster presentation poisoned");
    let subjects = source.iter().filter_map(|row| row["id"].as_str()).collect::<Vec<_>>();
    let heard = HeardSeats::read(store, &subjects)?;
    let mut by_agent = BTreeMap::<String, Vec<crate::store::SubagentView>>::new();
    for subagent in subagents {
        by_agent.entry(subagent.agent.clone()).or_default().push(subagent);
    }
    let local_host = client_host_id(store.origin());
    let mut rows = Vec::with_capacity(source.len());
    let mut deadline = None;
    for raw in source {
        let id = raw["id"].as_str().context("agent card without id")?;
        let source_changed = cache.cards.get(id).is_none_or(|old| old.source != *raw);
        let harness = if source_changed {
            Some(serde_json::from_value::<Option<crate::model::CurrentHarnessView>>(raw["_status_source"].clone())?)
        } else { None };
        let heard_at = harness.as_ref().or_else(|| cache.cards.get(id).map(|old| &old.harness))
            .and_then(Option::as_ref).map(|harness| heard.heard_at(store, id, harness));
        let state_after_restart = agent_state_after_restart(raw, now);
        let delivery = if raw["host_id"].as_str() == Some(local_host.as_str())
            && (matches!(state_after_restart, Some("running" | "waiting"))
                || matches!(state_after_restart, Some("desired" | "starting"))
                    && live_launch_report(raw, heard_at, now))
            && let Some(driver) = raw["driver"].as_str().filter(|driver| matches!(*driver, "claude" | "codex" | "opencode" | "pi" | "omp")) {
            Some(delivery_presence::assess_current_semantic(id, driver, raw["incarnation_id"].as_str()))
        } else {
            if cache.cards.get(id).is_some_and(|old| old.delivery_revision.is_some()) {
                delivery_presence::forget_current_semantic(id);
            }
            None
        };
        let delivery_revision = delivery.as_ref().map(|delivery| delivery.revision);
        let subagents = by_agent.remove(id).unwrap_or_default();
        let clock_state = clock_state(raw, heard_at, now);
        let mut card_deadline = presentation_deadline(raw, heard_at, &subagents, now);
        if let Some(at) = delivery.as_ref().and_then(|delivery| delivery.next_deadline_ms) {
            earliest(&mut card_deadline, u128::from(at), now);
        }
        let reusable = cache.cards.get(id).is_some_and(|old| old.source == *raw
            && old.clock_state == clock_state && old.delivery_revision == delivery_revision
            && old.subagents == subagents);
        if !reusable {
            let mut row = raw.clone();
            overlay_agent_resources_at(store, std::slice::from_mut(&mut row), &client_timestamp(now), now, Some(subagents.clone()))?;
            // Diagnostic ages are snapshots, not a reason to rebuild a resting card.
            if let Some(delivery) = &delivery
                && row.get("delivery").is_some() {
                let attachment = row["delivery"].get("attachment_push").cloned();
                row["delivery"] = delivery.assessment.to_value();
                if let Some(attachment) = attachment { row["delivery"]["attachment_push"] = attachment; }
            }
            let harness = harness.unwrap_or_else(|| cache.cards.remove(id)
                .and_then(|old| old.harness));
            cache.cards.insert(id.to_owned(), Card {
                source: raw.clone(), harness, clock_state, delivery_revision, subagents,
                deadline: card_deadline, row,
            });
            cache.rebuilds += 1;
        }
        cache.cards.get_mut(id).expect("presented agent").deadline = card_deadline;
        let card = &cache.cards[id];
        if let Some(at) = card.deadline { earliest(&mut deadline, at, now); }
        rows.push(card.row.clone());
    }
    // Release only seats owned by this roster, not other stores' semantic projections.
    cache.cards.retain(|id, card| {
        if subjects.contains(&id.as_str()) { return true; }
        if card.delivery_revision.is_some() {
            delivery_presence::forget_current_semantic(id);
        }
        false
    });
    Ok((rows, deadline))
}

fn earliest(deadline: &mut Option<u128>, at: u128, now: u128) {
    if at > now { *deadline = Some(deadline.map_or(at, |old| old.min(at))); }
}

fn clock_state(raw: &Value, heard_at: Option<u128>, now: u128) -> [bool; 5] {
    let launching = matches!(agent_state_after_restart(raw, now), Some("desired" | "starting"));
    [
        heard_at.is_some_and(|at| now.saturating_sub(at) > HARNESS_OBSERVATION_FRESHNESS.as_millis()),
        launching && heard_at.is_some_and(|at| now.saturating_sub(at) > u128::from(HEARTBEAT_MS)),
        launching && !live_launch_report(raw, heard_at, now)
            && raw["_launch_since_ms"].as_u64().is_some_and(|at| now.saturating_sub(u128::from(at)) > u128::from(LAUNCH_WINDOW_MS)),
        raw["state"] == "restarting" && raw["_restart_until_unix_ms"].as_u64().is_some_and(|at| now >= u128::from(at)),
        raw["model_drift"]["state"] != "unknown" && raw["model_drift"]["observed_at"].as_str().is_some_and(|at| {
            let at = snapshot_time_ms(at);
            at > now || now - at > HARNESS_OBSERVATION_FRESHNESS.as_millis()
        }),
    ]
}

fn live_launch_report(raw: &Value, heard_at: Option<u128>, now: u128) -> bool {
    heard_at.is_some_and(|at| now.saturating_sub(at) <= u128::from(HEARTBEAT_MS))
        && !matches!(raw["harness_state"].as_str(), None | Some("starting" | "ended" | "failed"))
}

fn presentation_deadline(
    raw: &Value, heard_at: Option<u128>, subagents: &[crate::store::SubagentView], now: u128,
) -> Option<u128> {
    let mut deadline = None;
    if let Some(at) = heard_at {
        earliest(&mut deadline, at.saturating_add(HARNESS_OBSERVATION_FRESHNESS.as_millis()).saturating_add(1), now);
        if matches!(agent_state_after_restart(raw, now), Some("desired" | "starting")) {
            earliest(&mut deadline, at.saturating_add(u128::from(HEARTBEAT_MS)).saturating_add(1), now);
        }
    }
    if matches!(agent_state_after_restart(raw, now), Some("desired" | "starting"))
        && !live_launch_report(raw, heard_at, now)
        && let Some(at) = raw["_launch_since_ms"].as_u64() {
        earliest(&mut deadline, u128::from(at).saturating_add(u128::from(LAUNCH_WINDOW_MS)).saturating_add(1), now);
    }
    if raw["state"] == "restarting" && let Some(at) = raw["_restart_until_unix_ms"].as_u64() {
        earliest(&mut deadline, u128::from(at), now);
    }
    if raw["model_drift"]["state"] != "unknown"
        && let Some(at) = raw["model_drift"]["observed_at"].as_str() {
        let observed = snapshot_time_ms(at);
        earliest(&mut deadline, observed, now);
        earliest(&mut deadline, observed.saturating_add(HARNESS_OBSERVATION_FRESHNESS.as_millis()).saturating_add(1), now);
    }
    for subagent in subagents {
        earliest(&mut deadline, u128::from(subagent.lease_expires_at_unix_ms), now);
    }
    deadline
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leaving_eligible_local_roster_forgets_semantic_delivery_rows() {
        let _clock = smallclaims::store::clock_snapshot();
        let root = tempfile::tempdir().unwrap();
        let app = crate::api::tests_shared::state(root.path());
        let host = client_host_id(app.store.origin());
        let now = client_now_ms();
        let unrelated = "agent/roster-retention-unrelated";
        delivery_presence::assess_current_semantic(unrelated, "claude", Some("one"));
        let unrelated_revision =
            delivery_presence::assess_current_semantic(unrelated, "omp", Some("one")).revision;

        for departure in ["retired", "stopped", "desired", "starting", "remote", "custom", "removed"] {
            let subject = format!("agent/roster-retention-{departure}");
            let row = json!({"id":subject, "state":"running", "driver":"omp",
                "incarnation_id":"one", "host_id":host});
            // A noninitial revision lets the public assessment prove that cleanup happened.
            delivery_presence::assess_current_semantic(&subject, "claude", Some("one"));
            present(&app.store, &[row.clone()], vec![], now).unwrap();
            assert!(app.store.extension::<Presentation>().0.lock().unwrap()
                .cards[&subject].delivery_revision.unwrap() > 1);

            let mut departed = row;
            match departure {
                "remote" => departed["host_id"] = json!("other-host"),
                "custom" => departed["driver"] = json!("custom"),
                state => departed["state"] = json!(state),
            }
            let source = if departure == "removed" { vec![] } else { vec![departed] };
            present(&app.store, &source, vec![], now).unwrap();
            assert_eq!(delivery_presence::assess_current_semantic(&subject, "omp", Some("one")).revision,
                1, "{departure} must release semantic bookkeeping");
            delivery_presence::forget_current_semantic(&subject);
            assert_eq!(delivery_presence::assess_current_semantic(unrelated, "omp", Some("one")).revision,
                unrelated_revision, "one roster must not prune unrelated recipients");
        }
        delivery_presence::forget_current_semantic(unrelated);
    }
}
