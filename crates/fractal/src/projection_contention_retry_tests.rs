use super::*;
use fractal_daemon::PublishedListStore as _;
#[test]
fn quiet_node_retry_coalesces_and_exhausts_without_self_rearming() {
    let now = tokio::time::Instant::now();
    let mut retry = ProjectionContentionRetry::default();
    assert_eq!(retry.poll(now, 1, true), None);
    // Repeated receives do not push the first deadline back.
    assert_eq!(retry.poll(now + Duration::from_secs(10), 50, true), None);
    assert_eq!(retry.poll(now + Duration::from_secs(30), 50, true), Some(1));
    retry.finish(now + Duration::from_secs(30), 51, true);
    assert_eq!(retry.poll(now + Duration::from_secs(90), 51, true), Some(2));
    retry.finish(now + Duration::from_secs(90), 52, true);
    assert_eq!(
        retry.poll(now + Duration::from_secs(210), 52, true),
        Some(3)
    );
    retry.finish(now + Duration::from_secs(210), 53, true);
    assert_eq!(retry.poll(now + Duration::from_secs(500), 53, true), None);
    // Only a fresh external contention generation rearms an exhausted episode.
    assert_eq!(retry.poll(now + Duration::from_secs(500), 54, true), None);
    assert_eq!(
        retry.poll(now + Duration::from_secs(530), 54, true),
        Some(1)
    );
}
#[test]
fn recovered_or_nonbusy_work_cancels_retry_and_no_contention_means_no_schedule() {
    let now = tokio::time::Instant::now();
    let mut retry = ProjectionContentionRetry::default();
    assert_eq!(retry.poll(now, 0, true), None);
    assert_eq!(retry.poll(now, 1, true), None);
    assert_eq!(retry.poll(now + Duration::from_secs(30), 1, true), Some(1));
    retry.finish(now + Duration::from_secs(30), 1, false);
    assert_eq!(retry.poll(now + Duration::from_secs(500), 1, true), None);
    retry.poll(now + Duration::from_secs(500), 2, true);
    assert_eq!(retry.poll(now + Duration::from_secs(501), 2, false), None);
    assert_eq!(retry.poll(now + Duration::from_secs(550), 2, true), None);
}
#[test]
fn external_contention_racing_success_is_not_lost() {
    let now = tokio::time::Instant::now();
    let mut retry = ProjectionContentionRetry::default();
    retry.poll(now, 1, true);
    assert_eq!(retry.poll(now + Duration::from_secs(30), 1, true), Some(1));
    // The successful call reports its pre-call watermark, not a newer external busy.
    retry.finish(now + Duration::from_secs(30), 1, false);
    assert_eq!(retry.poll(now + Duration::from_secs(31), 2, true), None);
    assert_eq!(retry.poll(now + Duration::from_secs(61), 2, true), Some(1));
}
#[tokio::test(start_paused = true)]
async fn quiet_node_recovers_automatically_after_busy_writer_releases() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("claims.sqlite3");
    const FLEET: &str = "018f6f0d-4a5d-7b8c-9d0e-123456789abc";
    let source = Store::open_memory("source").unwrap();
    source
        .append_client_claim(&smallclaims::ClaimInput {
            subject: "resource/quiet-busy".into(),
            kind: "resource.observed".into(),
            actor: None,
            fields: BTreeMap::from([("kind".into(), json!("custom.test.replication"))]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
    source.bind_fleet(FLEET).unwrap();
    let store = Arc::new(Store::open(&path, "node").unwrap());
    store.bind_fleet(FLEET).unwrap();
    store.project_replication_backlog().unwrap();
    let frontier =
        || {
            store.readers.get().query_row(
        "SELECT last_good_store_index FROM projection_health WHERE aggregate='graph'",
        [], |row| row.get::<_, u64>(0)).unwrap()
        };
    let before = frontier();
    let inventory = store.export_replication_summary(FLEET).unwrap().inventory;
    let exchange = source
        .export_replication_exchange(FLEET, &inventory)
        .unwrap();
    store
        .receive_replication_exchange("source", FLEET, &exchange)
        .unwrap();
    assert!(store.validate_replication_backlog().unwrap().changed);
    let target = store.index().unwrap();
    assert!(target > before);
    assert_eq!(frontier(), before);
    let visible = || {
        store.readers.get().query_row(
        "SELECT COUNT(*) FROM events WHERE subject='resource/quiet-busy' AND kind='resource.observed'",
        [], |row| row.get::<_, i64>(0)).unwrap()
    };
    assert_eq!(visible(), 0);
    store
        .connection
        .write()
        .busy_timeout(Duration::ZERO)
        .unwrap();
    let other = rusqlite::Connection::open(&path).unwrap();
    other.execute_batch(/* sqlite-large-value: allow-for-now lv-hbef187f7188a owner=fractal::projection_contention_retry_tests TODO=trace-statement-and-value-representation */ "BEGIN IMMEDIATE").unwrap();
    assert!(store.project_replication_backlog().is_err());
    assert!(store.replication_projection_deferred());
    assert_eq!(frontier(), before);
    other.execute_batch(/* sqlite-large-value: allow-for-now lv-h97f308cbe046 owner=fractal::projection_contention_retry_tests TODO=trace-statement-and-value-representation */ "ROLLBACK").unwrap();
    // No receive, wake or manual projection follows this release: the real worker
    // must publish the admitted claim and its projected event by itself.
    let (events, mut received) = watch::channel(0u64);
    let (polls, mut polled) = watch::channel(false);
    let worker = retry_projection_contention_observed(
        Arc::downgrade(&store),
        Arc::new(Notify::new()),
        events,
        dir.path().to_owned(),
        move |backoff| { polls.send_replace(backoff); },
    );
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(1)).await;
    loop { let backoff = *polled.borrow_and_update(); if backoff { break; } polled.changed().await.unwrap(); }
    tokio::time::advance(Duration::from_secs(30)).await;
    let observed = store.clone();
    let recovered = tokio::task::spawn_blocking(move || {
        let deadline = Instant::now() + Duration::from_secs(2);
        while observed.replication_projection_deferred() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(2));
        }
        !observed.replication_projection_deferred()
    })
    .await
    .unwrap();
    if recovered {
        received.changed().await.unwrap();
    }
    worker.abort();
    assert!(
        recovered,
        "scheduled retry must finish without a receive/wake"
    );
    assert_eq!(*received.borrow(), 1);
    assert_eq!(frontier(), target);
    assert_eq!(visible(), 1);
}

#[tokio::test(start_paused = true)]
async fn gate_deferred_admission_projects_once_without_another_peer_page() {
    let dir = tempfile::tempdir().unwrap();
    const FLEET: &str = "018f6f0d-4a5d-7b8c-9d0e-123456789abc";
    let source = Store::open_memory("source").unwrap();
    source.append_client_claim(&smallclaims::ClaimInput {
        subject: "resource/quiet-gate".into(),
        kind: "resource.observed".into(),
        actor: None,
        fields: BTreeMap::from([("kind".into(), json!("custom.test.replication"))]),
        evidence: Vec::new(),
        expected_subject: None,
        idempotency_key: None,
    }).unwrap();
    source.bind_fleet(FLEET).unwrap();
    let store = Arc::new(Store::open(&dir.path().join("claims.sqlite3"), "node").unwrap());
    store.bind_fleet(FLEET).unwrap();
    store.project_replication_backlog().unwrap();
    let before = store.projection_frontier().unwrap();
    let inventory = store.export_replication_summary(FLEET).unwrap().inventory;
    let exchange = source.export_replication_exchange(FLEET, &inventory).unwrap();
    let gate = store.projection.lock();
    let receipt = store.receive_replication_exchange("source", FLEET, &exchange).unwrap();
    assert!(receipt.received > 0);
    assert!(store.validate_replication_backlog().unwrap().changed);
    let target = store.index().unwrap();
    assert!(target > before);
    let generation = store.projection_contention_generation();
    assert_eq!(store.try_project_replication_backlog_unless_catching_up().unwrap(), None);
    assert!(store.projection_contention_generation() > generation);
    assert!(store.replication_projection_deferred());
    assert_eq!(store.projection_frontier().unwrap(), before);
    let visible = || store.readers.get().query_row(
        "SELECT COUNT(*) FROM events WHERE subject='resource/quiet-gate' AND kind='resource.observed'",
        [], |row| row.get::<_, i64>(0),
    ).unwrap();
    assert_eq!(visible(), 0);
    let (events, mut received) = watch::channel(0u64);
    let (polls, mut polled) = watch::channel(false);
    let worker = retry_projection_contention_observed(
        Arc::downgrade(&store),
        Arc::new(Notify::new()),
        events,
        dir.path().to_owned(),
        move |backoff| { polls.send_replace(backoff); },
    );
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(1)).await;
    loop {
        let backoff = *polled.borrow_and_update();
        if backoff { break; }
        polled.changed().await.unwrap();
    }
    // Entering backoff must not clear deferred health or advance the good frontier.
    assert!(store.replication_projection_deferred());
    assert_eq!(store.projection_frontier().unwrap(), before);
    assert_eq!(visible(), 0);
    drop(gate);
    assert!(store.replication_projection_deferred());
    // Only the existing background worker runs after this release: no new page,
    // receive notification, or direct projection call can finish the admission.
    tokio::time::advance(Duration::from_secs(30)).await;
    received.changed().await.unwrap();
    assert_eq!(*received.borrow_and_update(), 1);
    assert!(!store.replication_projection_deferred());
    assert_eq!(store.projection_frontier().unwrap(), target);
    assert_eq!(visible(), 1);
    assert_eq!(store.claims_for("resource/quiet-gate", Some("resource.observed")).unwrap().len(), 1);
    polled.borrow_and_update();
    tokio::time::advance(Duration::from_secs(500)).await;
    polled.changed().await.unwrap();
    worker.abort();
    assert_eq!(*received.borrow(), 1, "recovery must not schedule another projection");
    assert_eq!(store.projection_frontier().unwrap(), target);
    assert_eq!(visible(), 1);
}

#[tokio::test(start_paused = true)]
async fn quiet_receive_rearms_exhausted_retry_budget_only_after_cooldown() {
    use axum::{Json, extract::State};
    use fractal_daemon::api::replication::replication_receive;
    use smallclaims::replication::ReplicationReceiveRequest;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("claims.sqlite3");
    const FLEET: &str = "018f6f0d-4a5d-7b8c-9d0e-123456789abc";
    let source = Store::open_memory("quiet-source").unwrap();
    source.append_client_claim(&smallclaims::ClaimInput {
        subject: "resource/exhausted-quiet".into(),
        kind: "resource.observed".into(),
        actor: None,
        fields: BTreeMap::from([("kind".into(), json!("custom.test.replication"))]),
        evidence: Vec::new(),
        expected_subject: None,
        idempotency_key: None,
    }).unwrap();
    source.bind_fleet(FLEET).unwrap();
    let store = Arc::new(Store::open(&path, "quiet-target").unwrap());
    store.bind_fleet(FLEET).unwrap();
    store.project_replication_backlog().unwrap();
    let state = AppState {
        browser_pairing_login: None,
        seat_admission: Default::default(),
        store: store.clone(),
        notify: Arc::new(Notify::new()),
        event_notify: watch::channel(0u64).0,
        node: "quiet-target".into(),
        api_socket: dir.path().join("api.sock"),
        state_dir: dir.path().into(),
        pty_root: dir.path().join("pty"),
        pty_binary: dir.path().join("unused"),
        fleet_id: None,
        configured_peers: vec![],
        client_relay: None,
        native_session_home: None,
        planner_default: Default::default(),
        harness_question_sink: None,
        owning_person: None,
        assistant_receipts: Default::default(),
        private_notes: Default::default(),
    };
    let inventory = store.export_replication_summary(FLEET).unwrap().inventory;
    let exchange = source.export_replication_exchange(FLEET, &inventory).unwrap();
    let request = ReplicationReceiveRequest {
        peer: exchange.peer.clone(), fleet_id: exchange.fleet_id.clone(), exchange,
        round_trip_ms: None, carrier: Default::default(), authenticated_member_key: None,
    };
    let before = store.projection_frontier().unwrap();
    let gate = store.projection.lock();
    let result = tokio::time::timeout(Duration::from_secs(5), replication_receive(
        State(state.clone()), Default::default(), Json(request.clone()),
    )).await;
    drop(gate);
    assert!(result.expect("seed receive waited for gate").unwrap().0.receipt.received > 0);
    assert!(store.replication_projection_deferred());

    // Drive the real retry schedule and its real projection calls without a timer race.
    // A separate SQLite writer holds every scheduled attempt busy through the budget.
    store.connection.write().busy_timeout(Duration::ZERO).unwrap();
    let mut writer = rusqlite::Connection::open(&path).unwrap();
    let held_writer = writer.transaction_with_behavior(
        rusqlite::TransactionBehavior::Immediate,
    ).unwrap();
    let now = tokio::time::Instant::now();
    let mut schedule = ProjectionContentionRetry::default();
    store.projection_retry_episode_started();
    assert_eq!(schedule.poll(now, store.projection_contention_generation(), true), None);
    for (attempt, seconds) in [(1, 30), (2, 90), (3, 210)] {
        let at = now + Duration::from_secs(seconds);
        assert_eq!(schedule.poll(at, store.projection_contention_generation(), true), Some(attempt));
        assert!(store.project_replication_backlog_unless_catching_up().is_err());
        schedule.finish(at, store.projection_contention_generation(), true);
    }
    tokio::time::advance(Duration::from_secs(210)).await;
    store.projection_retry_episode_finished(tokio::time::Instant::now().into_std());
    let exhausted_at = now + Duration::from_secs(500);
    let generation = store.projection_contention_generation();
    assert_eq!(schedule.poll(exhausted_at, generation, true), None);
    assert!(schedule.due.is_none());
    assert_eq!(store.projection_frontier().unwrap(), before);
    held_writer.rollback().unwrap();

    let quiet = ReplicationReceiveRequest {
        exchange: source.export_replication_summary(FLEET).unwrap(),
        ..request
    };
    // Neither the end of the episode nor another quiet receive bypasses cooldown.
    for advance in [Duration::ZERO, Duration::from_secs(119)] {
        tokio::time::advance(advance).await;
        let response = replication_receive(
            State(state.clone()), Default::default(), Json(quiet.clone()),
        ).await.unwrap().0;
        assert_eq!(response.receipt.received, 0);
        assert!(store.replication_projection_deferred());
        assert_eq!(store.projection_frontier().unwrap(), before);
        assert_eq!(store.projection_contention_generation(), generation);
    }
    tokio::time::advance(Duration::from_secs(1)).await;
    let gate = store.projection.lock();
    let result = tokio::time::timeout(Duration::from_secs(5), replication_receive(
        State(state.clone()), Default::default(), Json(quiet.clone()),
    )).await;
    drop(gate);
    let response = result.expect("quiet receive waited for gate").unwrap().0;
    assert_eq!(response.receipt.received, 0);
    assert_eq!(store.projection_frontier().unwrap(), before);
    assert!(store.replication_projection_deferred());
    let rearmed_generation = store.projection_contention_generation();
    assert!(rearmed_generation > generation);
    assert_eq!(schedule.poll(exhausted_at, rearmed_generation, true), None);
    assert_eq!(schedule.poll(exhausted_at + Duration::from_secs(30), rearmed_generation, true), Some(1));

    // Reservation coalesces further quiet traffic even after the gate is free.
    for _ in 0..4 {
        let response = replication_receive(
            State(state.clone()), Default::default(), Json(quiet.clone()),
        ).await.unwrap().0;
        assert_eq!(response.receipt.received, 0);
        assert!(store.replication_projection_deferred());
        assert_eq!(store.projection_contention_generation(), rearmed_generation);
        assert_eq!(store.projection_frontier().unwrap(), before);
    }
    assert_eq!(store.project_replication_backlog_unless_catching_up().unwrap(), Some(true));
    store.projection_retry_episode_finished(tokio::time::Instant::now().into_std());
    assert!(!store.replication_projection_deferred());
    assert!(store.projection_frontier().unwrap() > before);
    let visible: i64 = store.readers.get().query_row(
        "SELECT COUNT(*) FROM events WHERE subject='resource/exhausted-quiet' AND kind='resource.observed'",
        [], |row| row.get(0),
    ).unwrap();
    assert_eq!(visible, 1);
    assert_eq!(store.claims_for("resource/exhausted-quiet", Some("resource.observed")).unwrap().len(), 1);
}
#[tokio::test(start_paused = true)]
async fn retry_worker_does_not_keep_store_alive_during_scheduled_backoff() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(&dir.path().join("claims.sqlite3"), "node").unwrap());
    store.project_replication_backlog().unwrap();
    store
        .connection
        .write()
        .busy_timeout(Duration::ZERO)
        .unwrap();
    let other = rusqlite::Connection::open(dir.path().join("claims.sqlite3")).unwrap();
    other.execute_batch(/* sqlite-large-value: allow-for-now lv-hf54eaf6da9df owner=fractal::projection_contention_retry_tests TODO=trace-statement-and-value-representation */ "BEGIN IMMEDIATE").unwrap();
    assert!(store.project_replication_backlog().is_err());
    other.execute_batch(/* sqlite-large-value: allow-for-now lv-h605b32403e38 owner=fractal::projection_contention_retry_tests TODO=trace-statement-and-value-representation */ "ROLLBACK").unwrap();
    let weak = Arc::downgrade(&store);
    let (events, _) = watch::channel(0u64);
    let (polls, mut polled) = watch::channel(false);
    let worker = retry_projection_contention_observed(
        weak.clone(),
        Arc::new(Notify::new()),
        events,
        dir.path().to_owned(),
        move |backoff| { polls.send_replace(backoff); },
    );
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(1)).await;
    loop { let backoff = *polled.borrow_and_update(); if backoff { break; } polled.changed().await.unwrap(); }
    // The worker has observed contention and waits for its first 30s deadline.
    assert_eq!(weak.strong_count(), 1);
    drop(store);
    assert!(weak.upgrade().is_none());
    worker.abort();
}
#[test]
fn only_the_terminal_provider_refusal_counts_as_an_ended_provider() {
    assert!(provider_ended_refusal(
        "st API returned 422 stale-mailbox-session: the provider authority is terminal"
    ));
    for other in [
        "st API returned 422 stale-mailbox-session: the provider session was superseded",
        "st API returned 422 mailbox-session-starting: waiting for provider ownership",
        "st API returned 500 internal: the provider authority is terminal",
    ] {
        assert!(!provider_ended_refusal(other), "{other}");
    }
}
#[tokio::test(start_paused = true)]
async fn a_provider_task_that_has_returned_is_settled_and_one_that_has_not_is_not() {
    let done = tokio::spawn(async { Ok(()) });
    assert!(provider_task_settled(&done).await);
    let running = tokio::spawn(std::future::pending::<Result<()>>());
    assert!(!provider_task_settled(&running).await);
    running.abort();
}
