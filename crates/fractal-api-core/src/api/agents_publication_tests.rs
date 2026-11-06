use super::*;

type PublicationArc = Arc<crate::store::agents_publication::AgentsPublication>;

fn publication_current(store: &Store) -> PublicationArc {
    store.agents_publication().expect("the complete roster must be published")
}

fn assert_publication_envelope(publication: &PublicationArc) {
    let envelope: Value = serde_json::from_slice(publication.encoded()).unwrap();
    assert_eq!(envelope["publication"], serde_json::to_value(publication.metadata()).unwrap());
    assert_eq!(envelope["items"].as_array().unwrap().as_slice(), publication.rows());
    assert_eq!(envelope["order"], json!(publication.order()));
    assert_eq!(envelope["has_more"], false);
    let ids = publication.rows().iter().map(|row| row["id"].as_str().unwrap()).collect::<Vec<_>>();
    assert_eq!(publication.order(), ids);
}

#[test]
fn agents_publication_completes_a_roster_beyond_the_startup_head() {
    let root = tempfile::tempdir().unwrap();
    let app = state(root.path());
    let store = &app.store;
    let _wake = store.start_agent_roster_refresher().unwrap();
    for agent in 0..401 {
        roster_resilience_runtime(store, &format!("agent/publication-{agent:04}"), "old");
    }
    store.read_snapshot(|index| client_agent_roster_head(store, index)).unwrap();
    assert!(store.agents_publication().is_none(), "a bounded startup head is not complete");
    refresh_agent_roster(store, false).unwrap();
    let publication = publication_current(store);
    assert_eq!(publication.rows().len(), 401);
    assert_eq!(publication.metadata().status_watermark.store_index, store.index().unwrap());
    assert_publication_envelope(&publication);
}

#[test]
fn agents_publication_does_not_replace_current_rows_with_history() {
    let root = tempfile::tempdir().unwrap();
    let app = state(root.path());
    let store = &app.store;
    let _wake = store.start_agent_roster_refresher().unwrap();
    roster_resilience_runtime(store, "agent/publication-current", "old");
    refresh_agent_roster(store, true).unwrap();
    assert!(store.agents_publication().is_none());
    refresh_agent_roster(store, false).unwrap();
    let current = publication_current(store);
    refresh_agent_roster(store, true).unwrap();
    assert!(Arc::ptr_eq(&current, &publication_current(store)));
}

#[test]
fn agents_publication_retained_rows_metadata_and_encoding_are_immutable() {
    let root = tempfile::tempdir().unwrap();
    let app = state(root.path());
    let store = &app.store;
    let _wake = store.start_agent_roster_refresher().unwrap();
    roster_resilience_runtime(store, "agent/publication-current", "old");
    refresh_agent_roster(store, false).unwrap();
    let before = publication_current(store);
    let encoded = Arc::clone(before.encoded());
    let metadata = *before.metadata();
    roster_resilience_runtime(store, "agent/publication-added", "new");
    refresh_agent_roster(store, false).unwrap();
    let after = publication_current(store);
    assert_eq!(before.rows().len(), 1);
    assert_eq!(after.rows().len(), 2);
    assert_eq!(before.metadata(), &metadata);
    assert!(Arc::ptr_eq(before.encoded(), &encoded));
    assert_eq!(after.metadata().node_epoch, metadata.node_epoch);
    assert!(after.metadata().revision > metadata.revision);
    assert_publication_envelope(&before);
    assert_publication_envelope(&after);
}

#[test]
fn agents_publication_forget_fences_the_old_generation_and_changes_epoch() {
    let root = tempfile::tempdir().unwrap();
    let app = state(root.path());
    let store = &app.store;
    let _wake = store.start_agent_roster_refresher().unwrap();
    roster_resilience_runtime(store, "agent/publication-current", "old");
    refresh_agent_roster(store, false).unwrap();
    let before = publication_current(store);
    let generation = store.agents_publication_build().unwrap();
    let source = store.read_snapshot(|index| store.agents_publication_source(index)).unwrap();
    store.forget_current_views();
    assert!(store.agents_publication().is_none());
    assert!(store.publish_presented_agents(generation, source, 1, None, Vec::new()).is_err());
    refresh_agent_roster(store, false).unwrap();
    let after = publication_current(store);
    assert_ne!(before.metadata().node_epoch, after.metadata().node_epoch);
    assert_eq!(before.rows(), after.rows());
}

#[test]
fn agents_publication_subscription_is_demand_and_uses_the_shared_owner() {
    let root = tempfile::tempdir().unwrap();
    let app = state(root.path());
    let store = &app.store;
    assert!(!store.agents_publication_demand());
    let mut changed = store.subscribe_agents_publications();
    assert!(store.agents_publication_demand());
    let _wake = store.start_agent_roster_refresher().unwrap();
    roster_resilience_runtime(store, "agent/publication-current", "old");
    refresh_agent_roster(store, false).unwrap();
    assert!(changed.has_changed().unwrap());
    changed.borrow_and_update();
    let first = publication_current(store);
    assert!(Arc::ptr_eq(&first, &publication_current(store)));
    assert!(!changed.has_changed().unwrap());
    drop(changed);
    assert!(!store.agents_publication_demand());
}

#[tokio::test]
async fn agents_publication_queued_wake_cannot_refresh_without_demand() {
    let root = tempfile::tempdir().unwrap();
    let app = state(root.path());
    let store = &app.store;
    let wake = store.start_agent_roster_refresher().unwrap();
    let subscriber = store.subscribe_agents_publications();
    drop(subscriber);
    wake.notify_one();
    let mut waiting = std::pin::pin!(wait_for_agent_roster_demand(store, &wake));
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    assert!(Future::poll(waiting.as_mut(), &mut context).is_pending());
    assert!(!store.agents_publication_demand());
    let _subscriber = store.subscribe_agents_publications();
    assert!(Future::poll(waiting.as_mut(), &mut context).is_ready());
}

#[tokio::test]
async fn agents_publication_summary_request_after_idle_registers_demand_and_recovers_reset() {
    let root = tempfile::tempdir().unwrap();
    let app = state(root.path());
    let store = &app.store;
    let _wake = store.start_agent_roster_refresher().unwrap();
    roster_resilience_runtime(store, "agent/summary-demand", "old");
    refresh_agent_roster(store, false).unwrap();
    let request = serde_json::from_value(json!({
        "kind":"subscribe", "id":"summary-demand", "collection":"summary"
    })).unwrap();
    let session = client_v0::ClientSession::local(None).unwrap();
    for reset in [false, true] {
        store.fractal.agent_roster_read_at.store(
            (client_now_ms() as u64).saturating_sub(300_001),
            std::sync::atomic::Ordering::Release,
        );
        roster_resilience_runtime(store, "agent/summary-demand", if reset { "reset" } else { "new" });
        if reset {
            store.forget_current_views();
        }
        assert!(!store.agents_publication_demand());
        let summary = || {
            client_v0::summary::shared(&app, &session, &request,
                &client_snapshot_at(&app, store.index().unwrap()), client_now_ms(), None, None, None)
        };
        let before = summary();
        assert_eq!(before.is_err(), reset);
        assert!(store.agents_publication_demand(), "a summary-style refresh request is demand");
        refresh_agent_roster_admitted(Arc::clone(store), false).await.unwrap().unwrap();
        assert!(summary().is_ok(), "the summary recovers after a projection reset");
        let publication = publication_current(store);
        assert_eq!(publication.metadata().status_watermark.store_index, store.index().unwrap());
        assert_eq!(publication.rows()[0]["runtime_ids"],
            json!([if reset { "runtime/reset" } else { "runtime/new" }]));
    }
}

#[tokio::test]
async fn agents_publication_last_subscriber_leaving_during_admission_skips_materialization() {
    let root = tempfile::tempdir().unwrap();
    let app = state(root.path());
    let store = &app.store;
    let _wake = store.start_agent_roster_refresher().unwrap();
    roster_resilience_runtime(store, "agent/admission-demand", "new");
    let subscriber = store.subscribe_agents_publications();
    let held = store.admit_agent_resources("other").await;
    let before = store.agent_resources_builds_for_test();
    let mut refresh = std::pin::pin!(refresh_agent_roster_admitted(Arc::clone(store), false));
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    assert!(Future::poll(refresh.as_mut(), &mut context).is_pending(), "the real owner iteration waits for admission");
    drop(subscriber);
    drop(held);
    refresh.await.unwrap().unwrap();
    assert_eq!(store.agent_resources_builds_for_test(), before);
    assert!(store.agents_publication().is_none());
}

#[test]
fn agents_publication_last_subscriber_leaving_in_blocking_queue_skips_materialization() {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all()
        .max_blocking_threads(1).build().unwrap();
    runtime.block_on(async {
        let root = tempfile::tempdir().unwrap();
        let app = state(root.path());
        let store = &app.store;
        let _wake = store.start_agent_roster_refresher().unwrap();
        roster_resilience_runtime(store, "agent/worker-demand", "new");
        let subscriber = store.subscribe_agents_publications();
        let (entered, started) = std::sync::mpsc::channel();
        let (release, blocked) = std::sync::mpsc::channel();
        let occupying = tokio::task::spawn_blocking(move || {
            entered.send(()).unwrap();
            blocked.recv().unwrap();
        });
        started.recv_timeout(Duration::from_secs(5)).unwrap();
        let before = store.agent_resources_builds_for_test();
        let mut refresh = std::pin::pin!(refresh_agent_roster_admitted(Arc::clone(store), false));
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        assert!(Future::poll(refresh.as_mut(), &mut context).is_pending(), "the real owner iteration queues behind the occupied blocking worker");
        assert!(store.try_admit_agent_resources("other").is_none(), "the queued worker owns admission");
        drop(subscriber);
        release.send(()).unwrap();
        refresh.await.unwrap().unwrap();
        occupying.await.unwrap();
        assert_eq!(store.agent_resources_builds_for_test(), before);
        assert!(store.agents_publication().is_none());
    });
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agents_publication_startup_completes_without_subscriber_or_read_grace() {
    let root = tempfile::tempdir().unwrap();
    let app = state(root.path());
    roster_resilience_runtime(&app.store, "agent/startup-publication", "startup");
    // The legacy publication watch does not contribute revisioned subscriber demand.
    let mut published = app.store.subscribe_agent_roster();
    start_agent_roster(&app);
    tokio::time::timeout(Duration::from_secs(5), async {
        while app.store.agents_publication().is_none() {
            published.changed().await.unwrap();
        }
    }).await.expect("startup must publish the complete roster without a reader");
    assert!(!app.store.agents_publication_demand(), "startup grants one refresh, not a read grace");
}

#[tokio::test]
async fn agents_publication_activity_request_survives_subscriber_loss_and_admission() {
    let root = tempfile::tempdir().unwrap();
    let app = state(root.path());
    let store = &app.store;
    let _wake = store.start_agent_roster_refresher().unwrap();
    roster_resilience_runtime(store, "agent/activity-demand", "new");
    let subscriber = store.subscribe_agents_publications();
    let held = store.admit_agent_resources("other").await;
    let mut refresh = std::pin::pin!(refresh_agent_roster_admitted(Arc::clone(store), false));
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    assert!(Future::poll(refresh.as_mut(), &mut context).is_pending());
    // A projection invalidation uses the real local-activity wake and records pending demand.
    store.forget_current_views();
    drop(subscriber);
    assert!(store.agents_publication_demand());
    drop(held);
    refresh.await.unwrap().unwrap();
    assert!(store.agents_publication().is_some());
    assert!(!store.agents_publication_demand(), "the activity request is acknowledged once");
}

#[tokio::test]
async fn agents_publication_explicit_request_outlives_the_read_grace_while_queued() {
    let root = tempfile::tempdir().unwrap();
    let app = state(root.path());
    let store = &app.store;
    let _wake = store.start_agent_roster_refresher().unwrap();
    roster_resilience_runtime(store, "agent/explicit-demand", "new");
    store.request_agent_roster_refresh();
    let held = store.admit_agent_resources("other").await;
    let mut refresh = std::pin::pin!(refresh_agent_roster_admitted(Arc::clone(store), false));
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    assert!(Future::poll(refresh.as_mut(), &mut context).is_pending());
    store.fractal.agent_roster_read_at.store(
        (client_now_ms() as u64).saturating_sub(300_001),
        std::sync::atomic::Ordering::Release,
    );
    assert!(store.agents_publication_demand(), "an unanswered request remains demand");
    drop(held);
    refresh.await.unwrap().unwrap();
    assert!(store.agents_publication().is_some());
    assert!(!store.agents_publication_demand());
}

#[test]
fn agents_publication_failed_old_generation_does_not_restore_cancelled_demand() {
    let root = tempfile::tempdir().unwrap();
    let app = state(root.path());
    let store = &app.store;
    let _wake = store.start_agent_roster_refresher().unwrap();
    store.fractal.request_roster_refresh();
    let result: anyhow::Result<()> = store.answer_agent_roster_requests(|| {
        store.forget_current_views();
        anyhow::bail!("the old-generation fold was invalidated")
    });
    assert!(result.is_err());
    assert!(!store.agents_publication_demand(), "an invalidated fold cannot resurrect old demand");
}

#[test]
fn agents_publication_reset_between_restore_check_and_cas_wins() {
    let root = tempfile::tempdir().unwrap();
    let app = state(root.path());
    let store = &app.store;
    let _wake = store.start_agent_roster_refresher().unwrap();
    store.fractal.request_roster_refresh();
    let checked = Arc::new(std::sync::Barrier::new(2));
    let release = Arc::new(std::sync::Barrier::new(2));
    *store.fractal.agent_roster_restore_checked.lock() = Some(Arc::new({
        let checked = Arc::clone(&checked);
        let release = Arc::clone(&release);
        move || { checked.wait(); release.wait(); }
    }));
    std::thread::scope(|scope| {
        let attempt = scope.spawn(|| {
            let result: anyhow::Result<()> = store.answer_agent_roster_requests(|| {
                anyhow::bail!("failed fold with an unanswered request")
            });
            assert!(result.is_err());
        });
        checked.wait();
        let generation = store.fractal.presented_agents.generation();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let reset = scope.spawn(move || {
            started_tx.send(()).unwrap();
            store.forget_current_views();
        });
        started_rx.recv().unwrap();
        let reset_serialized = store.fractal.agent_roster_request_generation.try_lock().is_none();
        let generation_held = store.fractal.presented_agents.generation() == generation;
        release.wait();
        attempt.join().unwrap();
        reset.join().unwrap();
        assert!(reset_serialized, "the checked restore must serialize against the reset");
        assert!(generation_held, "reset cannot advance the generation before restore finishes");
        assert_ne!(store.fractal.presented_agents.generation(), generation);
    });
    *store.fractal.agent_roster_restore_checked.lock() = None;
    assert!(!store.agents_publication_demand(), "reset clears even a request restored after the check");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delivery_metrics_worker_keeps_roster_fresh_without_external_readers() {
    use opentelemetry::metrics::MeterProvider as _;
    use opentelemetry_sdk::metrics::data::{AggregatedMetrics, MetricData};
    use opentelemetry_sdk::metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider};
    use std::sync::RwLock;

    async fn expect_state(exporter: &InMemoryMetricExporter, provider: &SdkMeterProvider, state: &str) {
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                provider.force_flush().unwrap();
                let finished = exporter.get_finished_metrics().unwrap();
                let matches = finished.iter().rev().flat_map(|resource| resource.scope_metrics())
                    .flat_map(|scope| scope.metrics()).find(|metric| metric.name() == "st.delivery.state_age")
                    .is_some_and(|metric| {
                        let AggregatedMetrics::F64(MetricData::Gauge(gauge)) = metric.data() else { return false; };
                        gauge.data_points().any(|point| point.attributes().any(|attribute|
                            *attribute == opentelemetry::KeyValue::new("st.delivery.state", state.to_owned())))
                    });
                if matches { return; }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }).await.unwrap_or_else(|_| panic!("metrics did not observe {state} without external roster readers"));
    }

    const SEAT: &str = "agent/example/metrics-no-readers";
    let root = tempfile::tempdir().unwrap();
    let app = state(root.path());
    let intent = parse_intent(r#"version 2
agent "example/metrics-no-readers" { workspace "/tmp"; harness "omp" {} }
"#, app.store.origin()).unwrap();
    app.store.apply_internal(&intent, "metrics-no-readers").unwrap();
    roster_resilience_runtime(&app.store, SEAT, "live");
    app.store.append_claim(&ClaimInput {
        subject: SEAT.into(), kind: "harness.observed".into(), actor: None,
        fields: serde_json::from_value(json!({"state":"idle", "driver":"omp", "incarnation_id":"one"})).unwrap(),
        evidence: vec![], expected_subject: None, idempotency_key: None,
    }).unwrap();
    let fence = crate::mailbox::Fence::new(SEAT, "one", "delivery");
    let heartbeat = || delivery_presence::record_fenced(&fence, &json!({
        "transport":"omp-channel", "ready":true, "pid":std::process::id(),
        "image":fractal_drivers::reexec::running_identity().map(|identity| identity.token()),
    }).to_string());
    heartbeat();
    assert!(!app.store.agents_publication_demand());
    let exporter = InMemoryMetricExporter::default();
    let provider = SdkMeterProvider::builder()
        .with_reader(PeriodicReader::builder(exporter.clone()).build()).build();
    let worker = delivery_metrics::spawn_worker(&app.store,
        Arc::new(RwLock::new(delivery_metrics::Summary::default())),
        provider.meter("st3"), Duration::from_millis(20));
    assert!(app.store.agents_publication_demand(), "the metrics worker owns the only demand receiver");
    start_agent_roster(&app);
    expect_state(&exporter, &provider, "live").await;
    // Advance the heartbeat beyond the real stale threshold without waiting 45 seconds.
    delivery_presence::backdate(SEAT, false, Duration::from_secs(46));
    expect_state(&exporter, &provider, "stale").await;
    delivery_presence::bind_refused(&fence, "mailbox-runtime-indeterminate");
    expect_state(&exporter, &provider, "refused").await;
    delivery_presence::bind_recovered(&fence);
    heartbeat();
    expect_state(&exporter, &provider, "live").await;
    worker.abort();
    let _ = worker.await;
    assert!(!app.store.agents_publication_demand(), "no external reader was keeping the owner alive");
    provider.shutdown().unwrap();
}

#[tokio::test]
async fn successor_delivery_does_not_report_a_predecessor_bind_refusal_on_messages() {
    let root = tempfile::tempdir().unwrap();
    let app = state(root.path());
    let seat = "agent/example/message-successor";
    let old = crate::mailbox::Fence::new(seat, "old", "delivery");
    let new = crate::mailbox::Fence::new(seat, "new", "delivery");
    delivery_presence::bind_refused(&old, "mailbox-runtime-indeterminate");
    delivery_presence::bind_recovered(&new);
    delivery_presence::record_fenced(&new, &json!({
        "transport":"omp-channel", "ready":true, "pid":std::process::id(),
        "image":fractal_drivers::reexec::running_identity().map(|identity| identity.token()),
    }).to_string());
    app.store.append_claim(&ClaimInput {
        subject: "message/successor-bind-refusal".into(), kind: "message.sent".into(), actor: Some("person/example".into()),
        fields: serde_json::from_value(json!({"from":"person/example", "to":seat,
            "status":"sent", "content":"successor must not inherit refusal"})).unwrap(),
        evidence: vec![], expected_subject: None, idempotency_key: None,
    }).unwrap();
    let known = delivery_presence::known(seat).unwrap();
    assert_ne!(known.state, "refused");
    let (status, view) = get_request(router(app), "/v1/messages/delivery/message/successor-bind-refusal").await;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert!(!view.to_string().contains("mailbox-runtime-indeterminate"), "{view}");
}

/// A message sent through `store`: a claim that node writes.
fn roster_pause_note(store: &Store, subject: &str) {
    store.append_claim(&ClaimInput {
        subject: subject.into(), kind: "message.sent".into(), actor: Some("person/example".into()),
        fields: serde_json::from_value(json!({"from": "person/example", "to": "agent/example",
            "status": "sent", "content": "a write someone waits to see"})).unwrap(),
        evidence: vec![], expected_subject: None, idempotency_key: None,
    }).unwrap();
}

const ROSTER_PAUSE_FLEET: &str = "5b4b1a8e-7c3d-4e2f-9a1b-0c6d5e4f3a2b";

/// Two members of one fleet: a writer, and a reader that receives its claims.
fn roster_pause_fleet(root: &Path) -> (Arc<Store>, Arc<Store>) {
    let open = |name: &str| {
        let dir = root.join(name);
        fs::create_dir_all(&dir).unwrap();
        let store = Store::open(&dir.join("graph.db"), name).unwrap();
        store.bind_fleet(ROSTER_PAUSE_FLEET).unwrap();
        Arc::new(store)
    };
    (open("writer"), open("reader"))
}

/// Everything `from` holds, admitted by `to` the way a peer exchange does it.
fn roster_pause_replicate(from: &Store, to: &Store) {
    let exchange = from
        .export_replication_exchange(ROSTER_PAUSE_FLEET, &to.replication_inventory().unwrap())
        .unwrap();
    to.receive_replication_exchange(from.origin(), ROSTER_PAUSE_FLEET, &exchange).unwrap();
    to.validate_replication_backlog().unwrap();
    to.apply_replication_repairs().unwrap();
    assert!(to.project_replication_backlog().unwrap());
}

/// The refresher's own wake: every commit notifies, nothing reads on the writer.
fn roster_pause_commits(store: &Store) -> (Arc<tokio::sync::Notify>, smallclaims::sqlite::CommitObserver) {
    let commits = Arc::new(tokio::sync::Notify::new());
    let wake = Arc::clone(&commits);
    let observer = store.observe_commits(move |_| wake.notify_one());
    (commits, observer)
}

/// A claim this node commits mid-pause ends the pause at its floor, not the full second, so its
/// writer sees it in the roster after one fold.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_claim_this_node_commits_cuts_the_agents_roster_pause_to_its_floor() {
    let root = tempfile::tempdir().unwrap();
    let store = state(root.path()).store;
    let (commits, _observer) = roster_pause_commits(&store);
    let through = store.index().unwrap();
    let started = Instant::now();
    let pause = tokio::spawn({
        let store = Arc::clone(&store);
        async move { agent_roster_pause(&store, &commits, &mut AgentRosterLook::default(), Duration::ZERO, through).await }
    });
    tokio::time::sleep(Duration::from_millis(150)).await;
    roster_pause_note(&store, "message/roster-pause-local");
    assert_eq!(pause.await.unwrap(), AgentRosterPauseEnd::LocalClaim);
    let paused = started.elapsed();
    assert!(paused >= Duration::from_millis(150), "{paused:?}");
    assert!(paused < Duration::from_millis(150) + Duration::from_millis(200), "{paused:?}");
}

/// Claims another node wrote move the cut all the time on a busy fleet: they keep the full pause,
/// so replication does not fold the roster more often than before.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cut_of_only_replicated_claims_keeps_the_full_agents_roster_pause() {
    let root = tempfile::tempdir().unwrap();
    let (writer, reader) = roster_pause_fleet(root.path());
    let (commits, _observer) = roster_pause_commits(&reader);
    let through = reader.index().unwrap();
    let started = Instant::now();
    let pause = tokio::spawn({
        let reader = Arc::clone(&reader);
        async move { agent_roster_pause(&reader, &commits, &mut AgentRosterLook::default(), Duration::ZERO, through).await }
    });
    for round in 0..3 {
        tokio::time::sleep(Duration::from_millis(150)).await;
        roster_pause_note(&writer, &format!("message/roster-pause-replicated-{round}"));
        roster_pause_replicate(&writer, &reader);
    }
    assert!(reader.index().unwrap() > through, "the writer's claims reached the reader");
    assert_eq!(reader.first_local_commit_after(through, reader.index().unwrap()).unwrap(), None);
    assert_eq!(pause.await.unwrap(), AgentRosterPauseEnd::Full);
    assert!(started.elapsed() >= AGENT_ROSTER_REFRESH_PAUSE, "{:?}", started.elapsed());
}

/// However fast this node writes, the refresher pauses at least its floor, and at least as long
/// as the last fold took, so a burst of local writes stays within the half-core bound.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_burst_of_local_commits_never_folds_the_agents_roster_faster_than_its_floor() {
    let root = tempfile::tempdir().unwrap();
    let store = state(root.path()).store;
    let (commits, _observer) = roster_pause_commits(&store);
    let writing = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let burst = std::thread::spawn({
        let (store, writing) = (Arc::clone(&store), Arc::clone(&writing));
        move || {
            let (mut written, mut last_write) = (0, None);
            while writing.load(std::sync::atomic::Ordering::Acquire) {
                roster_pause_note(&store, &format!("message/roster-pause-burst-{written}"));
                last_write = Some(Instant::now());
                written += 1;
                std::thread::sleep(Duration::from_millis(5));
            }
            last_write
        }
    });
    let mut look = AgentRosterLook::default();
    let mut last_started = Instant::now();
    for folded in [Duration::ZERO, Duration::ZERO, Duration::from_millis(250), Duration::ZERO] {
        let started = Instant::now();
        last_started = started;
        let through = store.index().unwrap();
        assert_eq!(agent_roster_pause(&store, &commits, &mut look, folded, through).await, AgentRosterPauseEnd::LocalClaim);
        let paused = started.elapsed();
        assert!(paused >= folded.max(AGENT_ROSTER_FRESH_PAUSE), "{folded:?}: {paused:?}");
        assert!(paused < folded.max(AGENT_ROSTER_FRESH_PAUSE) + Duration::from_millis(200), "{folded:?}: {paused:?}");
    }
    writing.store(false, std::sync::atomic::Ordering::Release);
    // The burst was still writing when the last pause began, however slowly each write went.
    let last_write = burst.join().unwrap().expect("the burst wrote");
    assert!(last_write > last_started, "the burst kept writing into the last pause");
}

/// A local write behind a run of replicated claims longer than one look still ends the pause
/// early: the run is read a look at a time, none of it skipped, from one coalesced wake.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_local_claim_behind_a_long_replicated_run_still_cuts_the_agents_roster_pause() {
    let root = tempfile::tempdir().unwrap();
    let (writer, reader) = roster_pause_fleet(root.path());
    let through = reader.index().unwrap();
    for claim in 0..5 {
        roster_pause_note(&writer, &format!("message/roster-pause-run-{claim}"));
    }
    roster_pause_replicate(&writer, &reader);
    let replicated = reader.index().unwrap();
    assert!(replicated - through > 2, "the run is longer than one look of 2 claims");
    roster_pause_note(&reader, "message/roster-pause-after-the-run");
    // Every commit above has already notified: one permit stands for all of them.
    let commits = tokio::sync::Notify::new();
    commits.notify_one();
    let started = Instant::now();
    let end = agent_roster_pause_scanning(&reader, &commits, &mut AgentRosterLook::default(), Duration::ZERO, through, 2).await;
    assert_eq!(end, AgentRosterPauseEnd::LocalClaim);
    assert!(started.elapsed() < AGENT_ROSTER_REFRESH_PAUSE, "{:?}", started.elapsed());
}

/// The look runs on the blocking pool, but the pause never waits on it: with every blocking
/// thread taken, a commit's wake still lets a fresh read, or the timer, end the pause on time.
/// Pause after pause, the one queued look is kept, never joined by another; once the pool frees,
/// it is the one that reads the claim and ends the pause.
#[test]
fn a_busy_blocking_pool_never_holds_the_agents_roster_pause_past_its_end() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2).max_blocking_threads(1).enable_all().build().unwrap();
    runtime.block_on(async {
        let root = tempfile::tempdir().unwrap();
        let store = state(root.path()).store;
        let _wake = store.start_agent_roster_refresher().unwrap();
        let commits = tokio::sync::Notify::new();
        let mut look = AgentRosterLook::default();
        let (release, held) = std::sync::mpsc::channel::<()>();
        let occupied = tokio::task::spawn_blocking(move || held.recv().unwrap());
        let through = store.index().unwrap();
        roster_pause_note(&store, "message/roster-pause-busy-pool");
        for _ in 0..2 {
            commits.notify_one();
            let started = Instant::now();
            let full = agent_roster_pause(&store, &commits, &mut look, Duration::ZERO, through).await;
            assert_eq!(full, AgentRosterPauseEnd::Full, "the look could not run");
            let paused = started.elapsed();
            assert!(paused >= AGENT_ROSTER_REFRESH_PAUSE, "{paused:?}");
            assert!(paused < AGENT_ROSTER_REFRESH_PAUSE + Duration::from_millis(300), "{paused:?}");
        }
        commits.notify_one();
        let started = Instant::now();
        let fresh = {
            let store = Arc::clone(&store);
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(300)).await;
                store.request_fresh_agent_roster(false);
            })
        };
        let pause = agent_roster_pause(&store, &commits, &mut look, Duration::ZERO, through).await;
        assert_eq!(pause, AgentRosterPauseEnd::FreshRead);
        assert!(started.elapsed() < Duration::from_millis(300) + Duration::from_millis(300), "{:?}", started.elapsed());
        fresh.await.unwrap();
        assert_eq!(look.started, 1, "three pauses kept the one queued look");
        assert!(look.running.is_some(), "the queued look is kept for the next pause");
        release.send(()).unwrap();
        occupied.await.unwrap();
        commits.notify_one();
        let freed = agent_roster_pause(&store, &commits, &mut look, Duration::ZERO, through).await;
        assert_eq!(freed, AgentRosterPauseEnd::LocalClaim);
        assert_eq!(look.started, 1, "the kept look read the claim");
    });
}
