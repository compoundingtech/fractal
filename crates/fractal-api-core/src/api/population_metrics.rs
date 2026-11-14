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
        // The operational allocation is a deployment gate, not admission control:
        // rejecting a complete cut would freeze both counts and empty-bucket aging.
        // The SDK's explicit stream limit reports overflow independently.
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

    fn retire(&mut self, now: Instant) {
        self.buckets.retain(|_, (_, empty_since, _)| {
            empty_since.is_none_or(|since| now.saturating_duration_since(since) < EMPTY_RETENTION)
        });
    }

    fn retention_deadline(&self) -> Option<Instant> {
        self.buckets.values().filter_map(|(_, since, _)| since.map(|since| since + EMPTY_RETENTION)).min()
    }

}

fn driver(value: Option<&str>) -> &str {
    match value { Some(value @ ("omp" | "claude" | "codex" | "opencode" | "pi")) => value, _ => "unattributed" }
}

/// State-specific reasons are selected here, never copied from an open string vocabulary.
fn classify(card: &Value) -> (&'static str, &'static str) {
    // Host-loss is a last-live presence diagnosis, not an authoritative end or
    // a current runtime fault. Keep that incarnation visible as unreachable.
    if card["end_reason"]["kind"] == "host-lost" && live(card) {
        return ("unreachable", "stale");
    }
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
    if matches!(status, Some("stopped" | "exited" | "absent" | "vanished" | "failed"))
        || matches!(card["harness_state"].as_str(), Some("ended" | "failed"))
        || card["state"] == "retired"
        || (!card["end_reason"].is_null() && card["end_reason"]["kind"] != "host-lost") {
        return false;
    }
    matches!(status, Some("running" | "ready" | "working" | "idle" | "starting"))
        || (card["incarnation_id"].is_string()
            && !matches!(card["harness_state"].as_str(), None | Some("ended" | "failed")))
}

fn fold(store: &Store, repositories: &[String]) -> anyhow::Result<(Population, Attention)> {
    fold_with_deadline(store, repositories).map(|(population, attention, _)| (population, attention))
}

fn fold_with_deadline(store: &Store, repositories: &[String]) -> anyhow::Result<(Population, Attention, Option<u128>)> {
    store.read_snapshot(|index| {
        let members = super::mailbox_metrics::managed_agents_owned_by_local_host(store)?
            .into_iter().collect::<BTreeSet<_>>();
        let now = client_now_ms();
        let mut cards = super::client_agent_resources_selected(store, false, index, Some((&members, &[])))?;
        let mut deadline = classification_deadline(store, &cards, now)?;
        // Consume the internal runtime input on this metrics-only copy before ordinary
        // presentation strips it. Neither runtime_state nor its transport enters client-v0.
        for card in &mut cards {
            if let Some(runtime) = card.as_object_mut().expect("agent resource").remove("_runtime_state") {
                card["runtime_state"] = runtime;
            }
        }
        super::overlay_agent_resources_at(store, &mut cards, &super::client_timestamp(now), now, None)?;
        if !repositories.is_empty() {
            super::overlay_population_repositories(store, &mut cards)?;
        }
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
        for child in store.running_subagents(now as u64)? {
            if !members.contains(&child.agent) { continue; }
            deadline = deadline.into_iter().chain(Some(u128::from(child.lease_expires_at_unix_ms))).min();
            if !children.insert((child.agent.clone(), child.session_id.clone(), child.subagent_id)) { continue; }
            let bucket = Bucket { driver: driver(Some(&child.driver)).to_owned(), kind: "subagent",
                repo: parents.get(&child.agent).cloned().unwrap_or_else(|| "unattributed".to_owned()),
                state: "unknown", reason: "unattributed", has_work: false };
            *buckets.entry(bucket).or_default() += 1;
        }
        let (attention, attention_deadline) = super::population_attention::summarize_with_deadline(store, now)?;
        Ok((buckets, attention, deadline.into_iter().chain(attention_deadline).min()))
    })
}

/// Use the same heard-at and strict age predicates as the shared card overlay.
/// Working turns never lapse merely because they stop producing claims.
fn classification_deadline(store: &Store, cards: &[Value], now: u128) -> anyhow::Result<Option<u128>> {
    let subjects = cards.iter().filter_map(|card| card["id"].as_str()).collect::<Vec<_>>();
    let heard = super::HeardSeats::read(store, &subjects)?;
    let mut deadline = None;
    for card in cards {
        let mut candidates = [None; 3];
        if let Some(source) = card.get("_status_source").filter(|source| !source.is_null()) {
            let harness: crate::model::CurrentHarnessView = serde_json::from_value(source.clone())?;
            let heard_at = heard.heard_at(store, card["id"].as_str().unwrap_or_default(), &harness);
            if matches!(harness.state.as_str(), "idle" | "ready") {
                candidates[0] = Some(heard_at.saturating_add(fractal_drivers::harness_state::HARNESS_OBSERVATION_FRESHNESS.as_millis() + 1));
            }
            if matches!(card["state"].as_str(), Some("desired" | "starting")) {
                candidates[1] = Some(heard_at.saturating_add(u128::from(super::HEARTBEAT_MS) + 1));
            }
        }
        if matches!(card["state"].as_str(), Some("desired" | "starting"))
            && let Some(since) = card["_launch_since_ms"].as_u64()
        {
            candidates[2] = Some(u128::from(since).saturating_add(u128::from(super::LAUNCH_WINDOW_MS) + 1));
        }
        deadline = deadline.into_iter().chain(candidates.into_iter().flatten().filter(|at| *at > now)).min();
    }
    Ok(deadline)
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

/// An uninitialized deployment extension is deliberately off, including in embedded callers.
pub(super) fn enabled(store: &Store) -> bool {
    store.extension::<crate::config::AgentPopulationEnabled>().0.get().copied().unwrap_or(false)
}

pub fn start(store: &Arc<Store>, repositories: Vec<String>) {
    if !enabled(store) || !crate::otel::metrics_enabled() { return; }
    let _worker = start_with_meter(store, repositories, || opentelemetry::global::meter("st3"));
}

#[cfg(test)]
#[derive(Default)]
struct RefreshAttempts {
    started: std::sync::atomic::AtomicUsize,
    wake: tokio::sync::Notify,
}

fn start_with_meter(
    store: &Arc<Store>,
    repositories: Vec<String>,
    meter: impl FnOnce() -> Meter,
) -> Option<tokio::task::JoinHandle<()>> {
    if !enabled(store) { return None; }
    let meter = meter();
    let wake = Arc::new(tokio::sync::Notify::new());
    let commits = Arc::clone(&wake);
    // The same writer notification used by published-list refreshers: never read
    // from the commit callback, and subscribe before the initial snapshot.
    let observer = store.observe_commits(move |_| commits.notify_one());
    #[cfg(test)]
    let attempts = store.extension::<RefreshAttempts>();
    let store = Arc::downgrade(store);
    let repositories: Arc<[String]> = repositories.into();
    Some(tokio::spawn(async move {
        let _observer = observer;
        let summary = Arc::new(RwLock::new(Summary::default()));
        let mut registered = false;
        // Only a complete fold can discover a new classification deadline.
        let mut next = None;
        loop {
            let Some(store) = store.upgrade() else { return; };
            let started = Instant::now();
            let registry = Arc::clone(&repositories);
            #[cfg(test)]
            attempts.started.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            match tokio::task::spawn_blocking(move || {
                crate::task_cpu::profiled_blocking("population_metrics/refresh", || fold_with_deadline(&store, &registry))
            }).await {
                Ok(Ok((buckets, attention, deadline))) => {
                    next = deadline.map(|at| tokio::time::Instant::now()
                        + Duration::from_millis(at.saturating_sub(client_now_ms()).min(u128::from(u64::MAX)) as u64));
                    let result = summary.write().unwrap_or_else(std::sync::PoisonError::into_inner)
                        .publish(buckets, attention, Instant::now(), (client_now_ms() / 1000) as u64);
                    if let Err(error) = result {
                        eprintln!("st3: WARN population metrics refresh failed: {error:#}");
                    } else if !registered {
                        register_metrics(&meter, Arc::clone(&summary));
                        registered = true;
                    }
                }
                Ok(Err(error)) => {
                    eprintln!("st3: WARN population metrics refresh failed: {error:#}");
                    // Keep an unconsumed deadline from the last complete cut, but
                    // never retry an unchanged defective fold on a polling timer.
                    next = next.filter(|at| *at > tokio::time::Instant::now());
                }
                Err(error) => {
                    eprintln!("st3: WARN population metrics refresh task failed: {error}");
                    next = next.filter(|at| *at > tokio::time::Instant::now());
                }
            }
            #[cfg(test)]
            attempts.wake.notify_one();
            wait_for_refresh(&wake, &summary, next, REFRESH_PAUSE.max(started.elapsed())).await;
        }
    }))
}

async fn sleep_until(deadline: Option<tokio::time::Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending::<()>().await,
    }
}

/// Retention is independent of successful folds and of the refresh rest floor.
/// A Notify permit coalesces all commits while folding/resting into one refresh.
async fn wait_for_refresh(
    wake: &tokio::sync::Notify,
    summary: &RwLock<Summary>,
    deadline: Option<tokio::time::Instant>,
    rest: Duration,
) {
    let rested = tokio::time::Instant::now() + rest;
    let mut triggered = false;
    loop {
        let retention = summary.read().unwrap_or_else(std::sync::PoisonError::into_inner)
            .retention_deadline().map(tokio::time::Instant::from_std);
        tokio::select! {
            () = wake.notified(), if !triggered => triggered = true,
            () = sleep_until(deadline), if !triggered => triggered = true,
            () = tokio::time::sleep_until(rested), if triggered => {
                // Commits arriving during the rest are included by the next cut,
                // not a second refresh after it.
                let _ = tokio::time::timeout(Duration::ZERO, wake.notified()).await;
                return;
            },
            () = sleep_until(retention) => {
                summary.write().unwrap_or_else(std::sync::PoisonError::into_inner)
                    .retire(tokio::time::Instant::now().into_std());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SeatsStoreCore as _;
    use serde_json::json;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn population_metrics_deployment_switch_controls_real_worker_and_registration() {
        use opentelemetry::metrics::MeterProvider as _;
        use opentelemetry_sdk::metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider};

        let exporter = InMemoryMetricExporter::default();
        let provider = SdkMeterProvider::builder()
            .with_reader(PeriodicReader::builder(exporter.clone()).build()).build();
        let store = Arc::new(Store::open_memory("example").unwrap());
        let before = crate::store::STATEMENTS_RUN.with(std::cell::Cell::get);
        assert!(!enabled(&store));
        assert!(start_with_meter(&store, Vec::new(), || panic!("disabled startup must not obtain a meter")).is_none());
        assert_eq!(crate::store::STATEMENTS_RUN.with(std::cell::Cell::get), before);
        provider.force_flush().unwrap();
        assert!(exporter.get_finished_metrics().unwrap().iter()
            .flat_map(|resource| resource.scope_metrics()).flat_map(|scope| scope.metrics()).next().is_none());

        let disabled = Arc::new(Store::open_memory("example").unwrap());
        disabled.extension::<crate::config::AgentPopulationEnabled>().0.set(false).unwrap();
        let before = crate::store::STATEMENTS_RUN.with(std::cell::Cell::get);
        assert!(start_with_meter(&disabled, Vec::new(), || panic!("explicit false must not obtain a meter")).is_none());
        assert_eq!(crate::store::STATEMENTS_RUN.with(std::cell::Cell::get), before);

        let intent = crate::graph::parse_intent(
            "version 2\nagent \"example.worker\" { workspace \"/tmp\"; harness \"claude\" {} }\n",
            "example",
        ).unwrap();
        store.apply_internal(&intent, "population-activation").unwrap();
        for (kind, fields) in [
            ("runtime.observed", json!({"status":"running", "incarnation_id":"one", "runtime_id":"wrapper"})),
            ("harness.observed", json!({"state":"working", "driver":"claude", "incarnation_id":"one"})),
        ] {
            store.append_claim(&crate::model::ClaimInput {
                subject: "agent/example.worker".into(), kind: kind.into(), actor: None,
                fields: serde_json::from_value(fields).unwrap(), evidence: vec![],
                expected_subject: None, idempotency_key: None,
            }).unwrap();
        }
        store.extension::<crate::config::AgentPopulationEnabled>().0.set(true).unwrap();
        let worker = start_with_meter(&store, Vec::new(), || provider.meter("st3")).unwrap();
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                provider.force_flush().unwrap();
                let finished = exporter.get_finished_metrics().unwrap();
                let names: BTreeSet<_> = finished.iter().flat_map(|resource| resource.scope_metrics())
                    .flat_map(|scope| scope.metrics()).map(|metric| metric.name()).collect();
                if ["st.agents", "st.attention.open", "st.agents.refresh.last_success"]
                    .into_iter().all(|name| names.contains(name)) { break; }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }).await.expect("enabled startup must fold and register all population instruments");
        worker.abort();
        let _ = worker.await;
        provider.shutdown().unwrap();
    }

    fn bucket(state: &'static str, reason: &'static str) -> Bucket {
        Bucket { driver: "unattributed".into(), kind: "unattributed", repo: "unattributed".into(),
            state, reason, has_work: false }
    }
    fn attention() -> [(&'static str, u64); 11] {
        [("person-step", 0), ("human-gate", 0), ("launch-approval", 0), ("unread-message", 0),
         ("agent-request", 0), ("revision-approval", 0), ("harness-login", 0),
         ("harness-prompt", 0), ("fault", 0), ("model-drift", 0), ("unattributed", 0)]
    }

    #[tokio::test(start_paused = true)]
    async fn population_metrics_idle_without_deadline_runs_zero_refreshes_per_minute() {
        let wake = Arc::new(tokio::sync::Notify::new());
        let waiting = {
            let wake = Arc::clone(&wake);
            tokio::spawn(async move {
                wait_for_refresh(&wake, &RwLock::new(Summary::default()), None, REFRESH_PAUSE).await;
            })
        };
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(60)).await;
        assert!(!waiting.is_finished(), "no commit/deadline means zero refreshes in a minute");
        wake.notify_one();
        waiting.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn population_metrics_failed_fold_waits_for_store_commit_without_retry_timer() {
        use opentelemetry::metrics::MeterProvider as _;
        use opentelemetry_sdk::metrics::SdkMeterProvider;
        use std::sync::atomic::Ordering;

        let provider = SdkMeterProvider::default();
        let store = Arc::new(Store::open_memory("example").unwrap());
        store.extension::<crate::config::AgentPopulationEnabled>().0.set(true).unwrap();
        // Break the real membership query without replacing the fold or worker.
        store.connection.write().execute_batch("ALTER TABLE desired RENAME TO unavailable_desired").unwrap();
        assert!(fold_with_deadline(&store, &[]).is_err());
        let attempts = store.extension::<RefreshAttempts>();
        let worker = start_with_meter(&store, Vec::new(), || provider.meter("st3")).unwrap();
        attempts.wake.notified().await;
        assert_eq!(attempts.started.load(Ordering::SeqCst), 1);

        tokio::time::advance(Duration::from_secs(120)).await;
        tokio::task::yield_now().await;
        assert_eq!(attempts.started.load(Ordering::SeqCst), 1,
            "a failed fold with no commit or known deadline must remain asleep");

        store.connection.write().execute_batch("ALTER TABLE unavailable_desired RENAME TO desired").unwrap();
        store.append_claim(&crate::model::ClaimInput {
            subject: "agent/example.worker".into(), kind: "runtime.observed".into(), actor: None,
            fields: serde_json::from_value(json!({"status":"running","incarnation_id":"one"})).unwrap(),
            evidence: vec![], expected_subject: None, idempotency_key: None,
        }).unwrap();
        attempts.wake.notified().await;
        assert_eq!(attempts.started.load(Ordering::SeqCst), 2, "one commit triggers one new fold");
        tokio::time::advance(Duration::from_secs(120)).await;
        tokio::task::yield_now().await;
        assert_eq!(attempts.started.load(Ordering::SeqCst), 2);
        worker.abort();
        let _ = worker.await;
        provider.shutdown().unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn population_metrics_store_commits_coalesce_until_minimum_rest() {
        let store = Store::open_memory("example").unwrap();
        let wake = Arc::new(tokio::sync::Notify::new());
        let commits = Arc::clone(&wake);
        let _observer = store.observe_commits(move |_| commits.notify_one());
        let waiting = {
            let wake = Arc::clone(&wake);
            tokio::spawn(async move {
                wait_for_refresh(&wake, &RwLock::new(Summary::default()), None, REFRESH_PAUSE).await;
            })
        };
        tokio::task::yield_now().await;
        for index in 0..3 {
            store.append_claim(&crate::model::ClaimInput {
                subject: format!("agent/example.worker-{index}"), kind: "runtime.observed".into(),
                actor: None, fields: serde_json::from_value(json!({"status":"running","incarnation_id":"one"})).unwrap(),
                evidence: vec![], expected_subject: None, idempotency_key: None,
            }).unwrap();
        }
        tokio::task::yield_now().await;
        tokio::time::advance(REFRESH_PAUSE - Duration::from_millis(1)).await;
        assert!(!waiting.is_finished());
        tokio::time::advance(Duration::from_millis(1)).await;
        waiting.await.unwrap();
        assert!(tokio::time::timeout(Duration::ZERO, wake.notified()).await.is_err(),
            "the burst must leave no extra refresh permit");
    }

    #[tokio::test(start_paused = true)]
    async fn population_metrics_retention_expires_while_no_refresh_is_due() {
        let now = tokio::time::Instant::now().into_std();
        let mut snapshot = Summary::default();
        snapshot.publish(BTreeMap::from([(bucket("idle", "none"), 1)]), attention(), now, 1).unwrap();
        snapshot.publish(BTreeMap::new(), attention(), now, 2).unwrap();
        let summary = Arc::new(RwLock::new(snapshot));
        let waiting = {
            let summary = Arc::clone(&summary);
            tokio::spawn(async move {
                wait_for_refresh(&tokio::sync::Notify::new(), &summary, None, REFRESH_PAUSE).await;
            })
        };
        tokio::task::yield_now().await;
        tokio::time::advance(EMPTY_RETENTION).await;
        tokio::task::yield_now().await;
        assert!(summary.read().unwrap().buckets.is_empty());
        assert_eq!(summary.read().unwrap().last_success, Some(2));
        assert!(!waiting.is_finished(), "retiring zeros is not a store refresh");
        waiting.abort();
    }

    #[test]
    fn population_metrics_over_allocation_never_freezes_counts_or_retention() {
        let now = Instant::now();
        let mut summary = Summary::default();
        let first = bucket("idle", "none");
        summary.publish(BTreeMap::from([(first.clone(), 3)]), attention(), now, 1).unwrap();
        let occupied = (0..513).map(|index| {
            let mut key = bucket("working", "unattributed");
            key.repo = format!("example/repository-{index}");
            (key, 1)
        }).collect::<BTreeMap<_, _>>();
        summary.publish(occupied.clone(), attention(), now, 2).unwrap();
        assert_eq!(summary.buckets[&first].0, 0);
        assert_eq!(summary.buckets.values().map(|(count, _, _)| count).sum::<u64>(), 513);
        summary.publish(occupied, attention(), now + EMPTY_RETENTION, 3).unwrap();
        assert!(!summary.buckets.contains_key(&first));
        assert_eq!(summary.last_success, Some(3));
    }

    #[test]
    fn population_metrics_host_lost_is_not_an_authoritative_end() {
        let mut card = json!({"runtime_state":"running","incarnation_id":"one",
            "harness_state":"working","reachability":"unreachable","end_reason":{"kind":"host-lost"}});
        assert!(live(&card));
        assert_eq!(classify(&card), ("unreachable", "stale"));
        card["fault"] = json!("not_started");
        assert_eq!(classify(&card), ("unreachable", "stale"),
            "host-loss diagnosis must not turn the last-live incarnation into a current fault");
        for end in ["completed", "crashed", "harness-exited", "stopped-by-person", "retired"] {
            card["end_reason"]["kind"] = json!(end);
            assert!(!live(&card), "{end}");
        }
        card["end_reason"]["kind"] = json!("host-lost");
        card["harness_state"] = json!("ended");
        assert!(!live(&card), "authoritative harness end wins over stale host-loss presentation");
    }

    #[tokio::test(start_paused = true)]
    async fn population_metrics_time_deadline_wakes_without_any_commit() {
        let until = tokio::time::Instant::now() + Duration::from_secs(45);
        let waiting = tokio::spawn(async move {
            wait_for_refresh(&tokio::sync::Notify::new(), &RwLock::new(Summary::default()),
                Some(until), REFRESH_PAUSE).await;
        });
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(44)).await;
        assert!(!waiting.is_finished());
        tokio::time::advance(Duration::from_secs(1)).await;
        waiting.await.unwrap();
    }

    #[test]
    fn population_metrics_classification_deadline_uses_shared_strict_age_boundaries() {
        let store = Store::open_memory("example").unwrap();
        let card = json!({"id":"agent/example.worker", "state":"starting", "_launch_since_ms":1000,
            "_status_source":{"state":"idle", "incarnation_id":"one", "claim":"claim/example",
                "since_unix_ms":1000, "observed_at_unix_ms":2000}});
        let freshness = fractal_drivers::harness_state::HARNESS_OBSERVATION_FRESHNESS.as_millis();
        assert_eq!(classification_deadline(&store, std::slice::from_ref(&card), 2000).unwrap(), Some(121_001));
        assert_eq!(classification_deadline(&store, std::slice::from_ref(&card), 121_001).unwrap(), Some(2000 + freshness + 1));
        assert_eq!(classification_deadline(&store, &[card], 2000 + freshness + 1).unwrap(), Some(663_001));
        let working = json!({"id":"agent/example.worker", "state":"running",
            "_status_source":{"state":"working", "incarnation_id":"one", "claim":"claim/example",
                "since_unix_ms":1000, "observed_at_unix_ms":2000}});
        assert_eq!(classification_deadline(&store, &[working], 2000).unwrap(), None);
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
        assert_eq!(summary.attention.len(), 11);
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
        assert_eq!(fold_with_deadline(&store, &[]).unwrap().2, Some(u128::from(now + 60_000)),
            "the earliest owned child lease is a refresh deadline");
        append("agent/example.worker", "workspace.observed", json!({
            "workspace":"/tmp", "host":"example",
            "repository":"/srv/checkouts/github.com/example/repo/refs/heads/main"
        }));
        let attributed = fold(&store, &["example/repo".to_owned()]).unwrap().0;
        assert_eq!(attributed.values().sum::<u64>(), 2);
        assert!(attributed.keys().all(|bucket| bucket.repo == "example/repo"));
        assert_eq!(count(), 2, "parent plus live leased child, without inventing a parent wait");
        let buckets = fold(&store, &[]).unwrap().0;
        assert!(buckets.keys().any(|bucket| bucket.state == "working" && bucket.reason == "unattributed"));
        append("agent/example.worker", "runtime.observed", json!({
            "status":"running", "incarnation_id":"one", "runtime_id":"wrapper", "reachability":"unreachable"
        }));
        let lost = fold(&store, &[]).unwrap().0;
        assert_eq!(lost.values().sum::<u64>(), 2, "host-loss retains the last-live parent");
        assert!(lost.keys().any(|bucket| bucket.state == "unreachable" && bucket.reason == "stale"));
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
