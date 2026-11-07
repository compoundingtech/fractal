//! Host-owned population, folded once off the scrape path from a consistent store cut.
use super::{Store, client_now_ms};
use crate::SubagentsStore;
use opentelemetry::{KeyValue, metrics::Meter};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

const EMPTY_RETENTION: Duration = Duration::from_secs(600);
const REFRESH_PAUSE: Duration = Duration::from_secs(30);
type Population = BTreeMap<Bucket, u64>;
type Attention = super::population_attention::Counts;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Bucket {
    driver: String,
    kind: &'static str,
    repo: String,
    state: &'static str,
    reason: &'static str,
    has_work: bool,
}
impl Bucket {
    fn attributes(&self) -> [KeyValue; 6] {
        [KeyValue::new("st.agent.driver", self.driver.clone()),
         KeyValue::new("st.agent.kind", self.kind),
         KeyValue::new("st.agent.repo", self.repo.clone()),
         KeyValue::new("st.agent.state", self.state),
         KeyValue::new("st.agent.reason", self.reason),
         KeyValue::new("st.agent.has_work", self.has_work)]
    }
}

#[derive(Default)]
pub struct Summary {
    buckets: BTreeMap<Bucket, (u64, Option<Instant>, [KeyValue; 6])>,
    attention: Attention,
    last_success: Option<u64>,
}
impl Summary {
    fn publish(&mut self, occupied: Population, attention: Attention, now: Instant, epoch: u64) -> anyhow::Result<()> {
        let retained = self.buckets.iter().filter(|(bucket, (_, empty_since, _))| {
            !occupied.contains_key(*bucket)
                && empty_since.is_none_or(|since| now.saturating_duration_since(since) < EMPTY_RETENTION)
        }).count();
        anyhow::ensure!(occupied.len() + retained <= 256, "population exceeds the 256 live/retained bucket allocation");
        self.buckets.retain(|bucket, (count, empty_since, _)| {
            if occupied.contains_key(bucket) { return true; }
            *count = 0;
            let since = *empty_since.get_or_insert(now);
            now.saturating_duration_since(since) < EMPTY_RETENTION
        });
        for (bucket, count) in occupied {
            let attributes = bucket.attributes();
            self.buckets.insert(bucket, (count, None, attributes));
        }
        self.attention = attention;
        self.last_success = Some(epoch);
        Ok(())
    }
}

fn driver(value: Option<&str>) -> &str {
    match value { Some(value @ ("omp" | "claude" | "codex" | "opencode" | "pi")) => value, _ => "unattributed" }
}

/// State-specific reasons are selected here, never copied from an open string vocabulary.
fn classify(card: &Value) -> (&'static str, &'static str) {
    if let Some(fault) = card["fault"].as_str() {
        return ("faulted", match fault {
            "crash_loop" | "crash-loop" => "crash_loop",
            "not_started" | "not-started" => "not_started",
            _ => "unattributed",
        });
    }
    if card["reachability"] == "stale" || card["reachability"] == "unreachable" {
        return ("unreachable", "stale");
    }
    if card["blocked_on"] == "human" {
        return ("asking", match card["ask"].as_str() {
            Some("question") => "question", Some("permission") => "permission",
            Some("review") => "review", _ => "unattributed",
        });
    }
    // Driver sentinels "none" and "unknown" do not establish a wait. A blocked
    // activity interval does; otherwise positive work/idle evidence remains decisive.
    if card["harness_state"] == "blocked" {
        return ("waiting", "unattributed");
    }
    match card["harness_state"].as_str() {
        Some("working") => ("working", "unattributed"),
        Some("idle" | "ready") if card["observation"] == "current" => ("idle", "none"),
        Some("starting") => ("starting", "none"),
        _ => ("unknown", "unattributed"),
    }
}

fn repository(card: &Value, repositories: &[String]) -> String {
    let observed = card.get("observed_repository");
    let candidate = match observed {
        Some(Value::String(value)) => Some(value.as_str()),
        // A current observation without a repository contradicts declaration-only attribution.
        Some(_) => None,
        None => card["checkout"]["repository"].as_str(),
    };
    candidate.filter(|value| repositories.iter().any(|allowed| allowed == value))
        .unwrap_or("unattributed").to_owned()
}

fn live(card: &Value) -> bool {
    let status = card["runtime_state"].as_str();
    if matches!(status, Some("stopped" | "exited" | "absent" | "failed"))
        || card["state"] == "retired" || !card["end_reason"].is_null() {
        return false;
    }
    matches!(status, Some("running" | "ready" | "working" | "idle" | "starting"))
        || (card["incarnation_id"].is_string()
            && !matches!(card["harness_state"].as_str(), None | Some("ended" | "failed")))
}

fn fold(store: &Store, repositories: &[String]) -> anyhow::Result<(Population, Attention)> {
    store.read_snapshot(|index| {
        let members = super::mailbox_metrics::managed_agents_owned_by_local_host(store)?
            .into_iter().collect::<BTreeSet<_>>();
        let mut cards = super::client_agent_resources_selected(store, false, index, Some((&members, &[])))?;
        super::overlay_agent_resources(store, &mut cards, &super::client_timestamp(client_now_ms()))?;
        let mut buckets = BTreeMap::new();
        let mut parents = BTreeMap::new();
        for card in cards.iter().filter(|card| card["id"].as_str().is_some_and(|id| members.contains(id))) {
            let id = card["id"].as_str().expect("selected member");
            let repo = repository(card, repositories);
            parents.insert(id.to_owned(), repo.clone());
            if !live(card) { continue; }
            let (state, reason) = classify(card);
            // Provenance is deliberately unattributed: owner_run_id is not a semantic kind.
            let active_work = card["active_work_count"].as_u64()
                .ok_or_else(|| anyhow::anyhow!("population work count is missing for a live member"))?;
            let queued_work = card["queued_work_count"].as_u64()
                .ok_or_else(|| anyhow::anyhow!("population queued work count is missing for a live member"))?;
            let bucket = Bucket { driver: driver(card["driver"].as_str()).to_owned(), kind: "unattributed",
                repo, state, reason, has_work: active_work > 0 || queued_work > 0 };
            *buckets.entry(bucket).or_default() += 1;
        }
        let mut children = BTreeSet::new();
        for child in store.running_subagents(client_now_ms() as u64)? {
            if !members.contains(&child.agent)
                || !children.insert((child.agent.clone(), child.session_id.clone(), child.subagent_id)) { continue; }
            let bucket = Bucket { driver: driver(Some(&child.driver)).to_owned(), kind: "subagent",
                repo: parents.get(&child.agent).cloned().unwrap_or_else(|| "unattributed".to_owned()),
                state: "unknown", reason: "unattributed", has_work: false };
            *buckets.entry(bucket).or_default() += 1;
        }
        let attention = super::population_attention::summarize(store, client_now_ms())?;
        Ok((buckets, attention))
    })
}

/// A complete initial snapshot, also used by isolated SDK exporter proofs.
pub fn summarize(store: &Store, repositories: &[String]) -> anyhow::Result<Summary> {
    let (buckets, attention) = fold(store, repositories)?;
    let mut summary = Summary::default();
    summary.publish(buckets, attention, Instant::now(), (client_now_ms() / 1000) as u64)?;
    Ok(summary)
}

pub fn register_metrics(meter: &Meter, summary: Arc<RwLock<Summary>>) {
    let population = Arc::clone(&summary);
    meter.u64_observable_gauge("st.agents").with_unit("{agent}")
        .with_callback(move |observer| {
            let snapshot = population.read().unwrap_or_else(std::sync::PoisonError::into_inner);
            for (count, _, attributes) in snapshot.buckets.values() { observer.observe(*count, attributes); }
        }).build();
    let attention = Arc::clone(&summary);
    meter.u64_observable_gauge("st.attention.open").with_unit("{item}")
        .with_callback(move |observer| {
            let snapshot = attention.read().unwrap_or_else(std::sync::PoisonError::into_inner);
            if snapshot.last_success.is_some() {
                for (kind, count) in snapshot.attention { observer.observe(count, &[KeyValue::new("st.attention.kind", kind)]); }
            }
        }).build();
    meter.u64_observable_gauge("st.agents.refresh.last_success").with_unit("s")
        .with_callback(move |observer| {
            let snapshot = summary.read().unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(epoch) = snapshot.last_success { observer.observe(epoch, &[]); }
        }).build();
}

pub fn start(store: &Arc<Store>, repositories: Vec<String>) {
    if !crate::otel::metrics_enabled() { return; }
    let store = Arc::downgrade(store);
    let repositories: Arc<[String]> = repositories.into();
    tokio::spawn(async move {
        let summary = Arc::new(RwLock::new(Summary::default()));
        let mut registered = false;
        loop {
            let Some(store) = store.upgrade() else { return; };
            let started = Instant::now();
            let registry = Arc::clone(&repositories);
            match tokio::task::spawn_blocking(move || {
                crate::task_cpu::profiled_blocking("population_metrics/refresh", || fold(&store, &registry))
            }).await {
                Ok(Ok((buckets, attention))) => {
                    let result = summary.write().unwrap_or_else(std::sync::PoisonError::into_inner)
                        .publish(buckets, attention, Instant::now(), (client_now_ms() / 1000) as u64);
                    if let Err(error) = result {
                        eprintln!("st3: WARN population metrics refresh failed: {error:#}");
                    } else if !registered {
                        register_metrics(&opentelemetry::global::meter("st3"), Arc::clone(&summary));
                        registered = true;
                    }
                }
                Ok(Err(error)) => eprintln!("st3: WARN population metrics refresh failed: {error:#}"),
                Err(error) => eprintln!("st3: WARN population metrics refresh task failed: {error}"),
            }
            tokio::time::sleep(REFRESH_PAUSE.max(started.elapsed())).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn bucket(state: &'static str, reason: &'static str) -> Bucket {
        Bucket { driver: "unattributed".into(), kind: "unattributed", repo: "unattributed".into(),
            state, reason, has_work: false }
    }
    fn attention() -> [(&'static str, u64); 10] {
        [("person-step", 0), ("human-gate", 0), ("launch-approval", 0), ("unread-message", 0),
         ("agent-request", 0), ("revision-approval", 0), ("harness-login", 0),
         ("harness-prompt", 0), ("fault", 0), ("unattributed", 0)]
    }

    #[test]
    fn population_metrics_precedence_and_missing_attribution() {
        let mut card = json!({"fault":"unknown", "reachability":"stale", "blocked_on":"human",
            "ask":"permission", "harness_state":"working", "observation":"current"});
        assert_eq!(classify(&card), ("faulted", "unattributed"));
        card["fault"] = Value::Null;
        assert_eq!(classify(&card), ("unreachable", "stale"));
        card["reachability"] = json!("reachable");
        assert_eq!(classify(&card), ("asking", "permission"));
        card["ask"] = json!("new-subtype");
        assert_eq!(classify(&card), ("asking", "unattributed"));
        card["blocked_on"] = json!("unknown");
        card["harness_state"] = json!("blocked");
        assert_eq!(classify(&card), ("waiting", "unattributed"));
        card["blocked_on"] = json!("none");
        card["harness_state"] = json!("working");
        assert_eq!(classify(&card), ("working", "unattributed"));
        card["harness_state"] = json!("idle");
        assert_eq!(classify(&card), ("idle", "none"));
        card["observation"] = json!("stale");
        assert_eq!(classify(&card), ("unknown", "unattributed"));
        card["harness_state"] = json!("starting");
        assert_eq!(classify(&card), ("starting", "none"));
    }

    #[test]
    fn population_metrics_never_emit_invalid_state_reason_pairs() {
        let legal = BTreeSet::from([("faulted", "crash_loop"), ("faulted", "not_started"),
            ("faulted", "unattributed"), ("unreachable", "stale"), ("asking", "question"),
            ("asking", "permission"), ("asking", "review"), ("asking", "unattributed"),
            ("waiting", "unattributed"), ("working", "unattributed"), ("idle", "none"),
            ("starting", "none"), ("unknown", "unattributed")]);
        for fault in [Value::Null, json!("crash_loop"), json!("not_started"), json!("other")] {
            for reachability in ["reachable", "stale", "indeterminate"] {
                for blocked in [Value::Null, json!("human"), json!("external")] {
                    for ask in ["question", "permission", "review", "other"] {
                        for state in ["working", "ready", "idle", "starting", "other"] {
                            let card = json!({"fault":fault, "reachability":reachability, "blocked_on":blocked,
                                "ask":ask, "harness_state":state, "observation":"current"});
                            assert!(legal.contains(&classify(&card)));
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn population_metrics_retention_expires_without_restarting_and_reoccupancy_cancels() {
        let now = Instant::now();
        let key = bucket("idle", "none");
        let mut summary = Summary::default();
        summary.publish(BTreeMap::from([(key.clone(), 2)]), attention(), now, 10).unwrap();
        summary.publish(BTreeMap::new(), attention(), now + Duration::from_secs(1), 11).unwrap();
        summary.publish(BTreeMap::new(), attention(), now + Duration::from_secs(600), 12).unwrap();
        assert_eq!(summary.buckets[&key].0, 0);
        summary.publish(BTreeMap::new(), attention(), now + Duration::from_secs(601), 13).unwrap();
        assert!(summary.buckets.is_empty());
        summary.publish(BTreeMap::from([(key.clone(), 1)]), attention(), now + Duration::from_secs(602), 14).unwrap();
        summary.publish(BTreeMap::new(), attention(), now + Duration::from_secs(603), 15).unwrap();
        summary.publish(BTreeMap::from([(key.clone(), 3)]), attention(), now + Duration::from_secs(604), 16).unwrap();
        assert_eq!(summary.buckets[&key].0, 3);
        assert!(summary.buckets[&key].1.is_none());
    }

    #[test]
    fn population_metrics_conserve_totals_and_emit_attention_zeros() {
        let mut summary = Summary::default();
        let occupied = BTreeMap::from([(bucket("working", "unattributed"), 17), (bucket("unknown", "unattributed"), 30)]);
        summary.publish(occupied, attention(), Instant::now(), 123).unwrap();
        assert_eq!(summary.buckets.values().map(|(count, _, _)| count).sum::<u64>(), 47);
        assert_eq!(summary.attention.len(), 10);
        assert!(summary.attention.iter().all(|(_, count)| *count == 0));
        assert_eq!(summary.last_success, Some(123));
    }

    #[test]
    fn population_metrics_membership_requires_live_evidence_not_declaration_or_suspension() {
        assert!(!live(&json!({"state":"desired"})));
        let mut card = json!({"incarnation_id":"one", "harness_state":"working", "state":"running",
            "runtime_state":"running", "end_reason":null, "suspension":{"phase":"quiescing"}});
        assert!(live(&card));
        card["runtime_state"] = json!("stopped");
        assert!(!live(&card));
    }

    #[test]
    fn population_metrics_repository_registry_prefers_observation_without_guessing() {
        let registry = vec!["example/repo".to_owned()];
        let mut card = json!({"checkout":{"repository":"example/repo"}});
        assert_eq!(repository(&card, &registry), "example/repo");
        card["observed_repository"] = json!("example/other");
        assert_eq!(repository(&card, &registry), "unattributed");
        card["observed_repository"] = Value::Null;
        assert_eq!(repository(&card, &registry), "unattributed");
        assert_eq!(repository(&card, &[]), "unattributed");
    }
}

#[cfg(test)]
mod membership_tests {
    use super::*;
    use crate::SeatsStoreCore as _;
    use serde_json::json;

    #[test]
    fn population_metrics_unions_children_of_declared_members_even_without_live_parent() {
        let store = Store::open_memory("example").unwrap();
        let intent = crate::graph::parse_intent(
            "version 2\nagent \"example.worker\" { workspace \"/tmp\"; harness \"claude\" {} }\n",
            "example",
        ).unwrap();
        store.apply_internal(&intent, "population-child-union").unwrap();
        let append = |subject: &str, kind: &str, fields: Value| {
            store.append_claim(&crate::model::ClaimInput {
                subject: subject.into(), kind: kind.into(), actor: None,
                fields: serde_json::from_value(fields).unwrap(), evidence: vec![],
                expected_subject: None, idempotency_key: None,
            }).unwrap();
        };
        let now = client_now_ms() as u64;
        for (subject, child, lease) in [
            ("agent/example.worker", "live", now + 60_000),
            ("agent/example.worker", "expired", now - 1),
            ("agent/example.unmanaged", "foreign", now + 60_000),
        ] {
            append(subject, "subagent.appeared", json!({"subagent_id":child, "driver":"omp",
                "incarnation_id":"one", "session_id":"native", "lease_expires_at_unix_ms":lease}));
        }
        let count = || fold(&store, &[]).unwrap().0.values().sum::<u64>();
        assert_eq!(count(), 1, "a declared but never started parent is not a live agent");
        append("agent/example.worker", "runtime.observed", json!({"status":"running", "incarnation_id":"one", "runtime_id":"wrapper"}));
        append("agent/example.worker", "harness.observed", json!({"state":"working", "driver":"claude", "incarnation_id":"one"}));
        assert_eq!(count(), 2, "parent plus live leased child, without inventing a parent wait");
        let buckets = fold(&store, &[]).unwrap().0;
        assert!(buckets.keys().any(|bucket| bucket.state == "working" && bucket.reason == "unattributed"));
        append("agent/example.worker", "runtime.observed", json!({"status":"stopped", "incarnation_id":"one", "runtime_id":"wrapper"}));
        assert_eq!(count(), 1, "parent end alone does not discard an unended leased child");
    }

    #[test]
    fn population_metrics_query_inventory_for_47_members_and_50_open_items() {
        use crate::PersonWorkStore as _;
        let store = Store::open_memory("example").unwrap();
        let mut source = String::from("version 2\n");
        for index in 0..47 {
            source.push_str(&format!("agent \"example.worker-{index}\" {{ workspace \"/tmp\"; harness \"claude\" {{}} }}\n"));
        }
        let intent = crate::graph::parse_intent(&source, "example").unwrap();
        store.apply_internal(&intent, "population-query-inventory").unwrap();
        for index in 0..50 {
            let request = serde_json::from_value(json!({
                "person":"person/reviewer", "actor":"agent/example.worker-0",
                "title":format!("Question {index}"), "reason":"Inventory fixture",
                "idempotency_key":format!("question-{index}")
            })).unwrap();
            store.ask_person(&request).unwrap();
        }
        let before = crate::store::STATEMENTS_RUN.with(std::cell::Cell::get);
        let (_, attention) = fold(&store, &[]).unwrap();
        let statements = crate::store::STATEMENTS_RUN.with(std::cell::Cell::get) - before;
        assert_eq!(attention.iter().map(|(_, count)| count).sum::<u64>(), 50);
        eprintln!("population refresh query inventory: 47 declared members, 50 open agent requests, {statements} SQL statements");
    }

    #[test]
    fn population_metrics_real_driver_blocking_fields_survive_store_to_summary() {
        let store = Store::open_memory("example").unwrap();
        let intent = crate::graph::parse_intent(
            "version 2\nagent \"example.worker\" { workspace \"/tmp\"; harness \"claude\" {} }\n",
            "example",
        ).unwrap();
        store.apply_internal(&intent, "population-blocking-fields").unwrap();
        let append = |kind: &str, fields: Value| {
            store.append_claim(&crate::model::ClaimInput {
                subject: "agent/example.worker".into(), kind: kind.into(), actor: None,
                fields: serde_json::from_value(fields).unwrap(), evidence: vec![],
                expected_subject: None, idempotency_key: None,
            }).unwrap();
        };
        append("runtime.observed", json!({"status":"running", "incarnation_id":"one", "runtime_id":"wrapper"}));
        for (activity, blocking, ask, expected) in [
            ("working", "none", "none", ("working", "unattributed")),
            ("idle", "none", "none", ("idle", "none")),
            ("working", "unknown", "none", ("working", "unattributed")),
            ("idle", "unknown", "none", ("idle", "none")),
            ("indeterminate", "unknown", "none", ("unknown", "unattributed")),
            ("blocked", "unknown", "none", ("waiting", "unattributed")),
            ("working", "human", "question", ("asking", "question")),
        ] {
            append("harness.observed", json!({"state":activity, "driver":"claude",
                "incarnation_id":"one", "blocked_on":blocking, "ask":ask}));
            let summary = summarize(&store, &[]).unwrap();
            assert_eq!(summary.buckets.len(), 1, "{activity}/{blocking}");
            let (bucket, (count, _, _)) = summary.buckets.first_key_value().unwrap();
            assert_eq!((bucket.state, bucket.reason), expected, "{activity}/{blocking}");
            assert_eq!(*count, 1);
        }
    }
}
