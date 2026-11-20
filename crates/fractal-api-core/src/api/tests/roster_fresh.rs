use super::*;

#[test]
fn harness_cards_use_the_shared_observation_freshness_window() {
    let store = Store::open_memory("example").unwrap();
    let observed = 1_000_000_u128;
    let freshness = HARNESS_OBSERVATION_FRESHNESS.as_millis();
    for reported in ["idle", "ready"] {
        for runtime in ["running", "starting"] {
            for age in [150_000, freshness, 331_000] {
                let mut cards = vec![json!({
                    "id":"agent/example.worker", "state":runtime, "harness_state":reported,
                    "_status_source":{"state":reported, "incarnation_id":"one", "claim":"claim/example",
                        "since_unix_ms":observed, "observed_at_unix_ms":observed},
                })];
                overlay_agent_resources_at(&store, &mut cards, "now", observed + age, Some(Vec::new())).unwrap();
                let current = age <= freshness;
                assert_eq!(cards[0]["observation"], if current { "current" } else { "stale" }, "{runtime}/{reported}/{age}");
                assert_eq!(cards[0]["harness_state"], if current { reported } else { "indeterminate" }, "{runtime}/{reported}/{age}");
                assert_eq!(cards[0]["state"], if current { "running" } else { "waiting" }, "{runtime}/{reported}/{age}");
            }
        }
    }
}

fn assert_harnessless_card_stays_missing(mut card: Value) {
    let store = Store::open_memory("example").unwrap();
    let state = card["state"].clone();
    let expected = serde_json::to_value(fractal_ui_model::agents::present(
        &fractal_ui_model::agents::Facts::deserialize(&card).unwrap(),
    )).unwrap();
    overlay_agent_resources_at(&store, std::slice::from_mut(&mut card), "now", 1_000_000, Some(Vec::new())).unwrap();
    assert_eq!(card["observation"], "missing");
    assert_eq!(card["state"], state);
    assert_eq!(card["presentation"], expected);
    assert!(card["incarnation_id"].is_null());
    assert!(card["harness_state"].is_null());
}

#[test]
fn suspended_card_without_an_incarnation_stays_missing() {
    assert_harnessless_card_stays_missing(json!({
        "id":"agent/example.paused", "state":"suspended", "incarnation_id":null,
        "harness_state":null, "_status_source":null,
        "suspension":{"action":"suspend", "phase":"suspended"},
    }));
}

#[test]
fn service_card_without_a_harness_stays_missing() {
    assert_harnessless_card_stays_missing(json!({
        "id":"agent/example.web-services", "state":"running", "driver":null,
        "incarnation_id":null, "session":null, "harness":null, "harness_state":null,
        "_status_source":null,
    }));
}

fn fixture() -> (tempfile::TempDir, AppState) {
    let root = tempfile::tempdir().unwrap();
    let mut state = state(root.path());
    state.store = Arc::new(roster_followup_store());
    (root, state)
}

fn unrelated(store: &Store, key: &str) {
    store
        .append_claim(&ClaimInput {
            subject: format!("custom/fresh-proof/{key}"),
            kind: "custom.test.marker".into(),
            actor: None,
            fields: BTreeMap::new(),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
}

fn update_card(store: &Store) -> ClaimRecord {
    store
        .append_claim(&roster_local_observation(
            "runtime.observed",
            json!({"status":"running", "runtime_id":"fresh-new", "incarnation_id":"amber-1"}),
        ))
        .unwrap()
}

/// A seat that has not polled its mailbox is described by the daemon's live uptime. Pages read
/// on this thread see the same uptime and keep their own state clocks, so two reads of an unchanged
/// roster are equal.
fn pinned_uptime() -> crate::delivery_presence::uptime_pin::Pin {
    crate::delivery_presence::uptime_pin::pin(Duration::from_secs(5))
}

#[tokio::test]
async fn unrelated_claims_verify_the_exact_older_response_and_later_wait_target() {
    let _uptime = pinned_uptime();
    for history in [false, true] {
        let (_root, state) = fixture();
        let store = &state.store;
        let _wake = store.start_agent_roster_refresher().unwrap();
        refresh_agent_roster(store, history).unwrap();
        let query = ClientListQuery {
            history,
            fresh: true,
            ..ClientListQuery::default()
        };
        let (cut, cards, published_at) = store
            .published_agent_roster(store.index().unwrap(), history)
            .unwrap();
        let (Extension(before_snapshot), Json(before)) =
            client_agents_published_page(&state, &query, cut, None)
                .unwrap()
                .unwrap();
        unrelated(store, "request");
        let requested = store.index().unwrap();
        unrelated(store, "wait-start");
        let wanted = store.index().unwrap();
        assert!(cut < requested && requested < wanted);
        let refolds = store.agent_resources_refolded_cards_for_test();
        let writes = smallclaims::touched::writes();
        let waited = wait_for_agent_roster(store, history).await;
        assert_eq!(waited.wanted, Some(wanted));
        assert_eq!(waited.outcome, RosterFreshWaitOutcome::Unchanged);
        assert_eq!(
            roster_fresh_served_proof(store, history, waited, Some((cut, published_at, &cards))),
            RosterUnchangedProof::Verified
        );
        let (Extension(snapshot), Json(page)) =
            client_agents_published_page(&state, &query, requested, Some(waited))
                .unwrap()
                .unwrap();
        assert_eq!(snapshot.store_index, before_snapshot.store_index);
        assert_eq!(snapshot.store_index, cut);
        assert_eq!(snapshot.published_at, before_snapshot.published_at);
        assert_eq!(page.items, before.items);
        assert_eq!(
            store.agent_resources_refolded_cards_for_test(),
            refolds,
            "fresh reads never fold"
        );
        assert_eq!(
            smallclaims::touched::writes(),
            writes,
            "proof/page never write"
        );
    }
}

#[tokio::test]
async fn a_daemon_that_ages_between_reads_changes_only_the_unpolled_delivery_reason() {
    // The control for the pin above: the same published page read a second later differs, and
    // only in the live age of a seat that has not polled.
    let uptime = pinned_uptime();
    let (_root, state) = fixture();
    let store = &state.store;
    let _wake = store.start_agent_roster_refresher().unwrap();
    refresh_agent_roster(store, false).unwrap();
    let query = ClientListQuery { fresh: true, ..ClientListQuery::default() };
    let (cut, _, _) = store.published_agent_roster(store.index().unwrap(), false).unwrap();
    let read = || client_agents_published_page(&state, &query, cut, None).unwrap().unwrap().1.0.items;
    let before = read();
    uptime.set(Duration::from_secs(6));
    let after = read();
    assert_ne!(after, before);
    let reasons = |items: &[Value]| {
        items.iter().filter_map(|card| card.pointer("/delivery/reason").and_then(Value::as_str).map(str::to_owned)).collect::<Vec<_>>()
    };
    assert!(reasons(&before).iter().any(|reason| reason.contains("started 5s ago")), "{before:?}");
    assert!(reasons(&after).iter().any(|reason| reason.contains("started 6s ago")), "{after:?}");
    let same_age = |items: Vec<Value>| serde_json::to_string(&items).unwrap().replace("started 6s ago", "started 5s ago");
    assert_eq!(same_age(after), same_age(before));
}

#[tokio::test]
async fn a_card_write_requires_a_new_publication_with_the_new_card() {
    for history in [false, true] {
        let (_root, state) = fixture();
        let store = &state.store;
        let _wake = store.start_agent_roster_refresher().unwrap();
        refresh_agent_roster(store, history).unwrap();
        let old = store
            .published_agent_roster(store.index().unwrap(), history)
            .unwrap();
        let changed = update_card(store);
        let requested = store.index().unwrap();
        assert!(changed.store_index > old.0);
        assert!(
            !store
                .served_agent_roster_unchanged_through(requested, history, old.0, old.2, &old.1)
                .unwrap()
        );
        let reader = Arc::clone(store);
        let waiter = tokio::spawn(async move { wait_for_agent_roster(&reader, history).await });
        store.fresh_agent_roster_wanted().await;
        assert!(
            !waiter.is_finished(),
            "changed cards cannot finish as unchanged"
        );
        refresh_agent_roster(store, history).unwrap();
        let waited = waiter.await.unwrap();
        // A publication racing the unchanged probe can satisfy that probe at cut==wanted.
        assert!(matches!(
            waited.outcome,
            RosterFreshWaitOutcome::Published | RosterFreshWaitOutcome::Unchanged
        ));
        let query = ClientListQuery {
            history,
            fresh: true,
            ..ClientListQuery::default()
        };
        let (Extension(snapshot), Json(page)) =
            client_agents_published_page(&state, &query, requested, Some(waited))
                .unwrap()
                .unwrap();
        assert!(snapshot.store_index >= changed.store_index);
        let card = page
            .items
            .iter()
            .find(|card| card["id"] == "agent/node.amber")
            .unwrap();
        assert_eq!(card["runtime_ids"], json!(["runtime/fresh-new"]));
    }
}

#[tokio::test]
async fn failed_refresh_remains_distinct_from_unchanged_and_keeps_the_old_response() {
    let _uptime = pinned_uptime();
    for history in [false, true] {
        let (_root, state) = fixture();
        let store = &state.store;
        let _wake = store.start_agent_roster_refresher().unwrap();
        refresh_agent_roster(store, history).unwrap();
        let query = ClientListQuery {
            history,
            fresh: true,
            ..ClientListQuery::default()
        };
        let old = store
            .published_agent_roster(store.index().unwrap(), history)
            .unwrap();
        let (Extension(before_snapshot), Json(before)) =
            client_agents_published_page(&state, &query, old.0, None)
                .unwrap()
                .unwrap();
        update_card(store);
        assert!(
            store
                .answer_agent_roster_requests::<()>(|| anyhow::bail!("private refresh failure"))
                .is_err()
        );
        let waited = wait_for_agent_roster(store, history).await;
        assert_eq!(waited.outcome, RosterFreshWaitOutcome::RefreshFailed);
        assert_ne!(
            roster_fresh_served_proof(store, history, waited, Some((old.0, old.2, &old.1))),
            RosterUnchangedProof::Verified
        );
        let (Extension(snapshot), Json(page)) =
            client_agents_published_page(&state, &query, store.index().unwrap(), Some(waited))
                .unwrap()
                .unwrap();
        assert_eq!(snapshot.store_index, before_snapshot.store_index);
        assert_eq!(snapshot.published_at, before_snapshot.published_at);
        assert_eq!(
            page.items, before.items,
            "observing failure must not change fallback policy"
        );
    }
}

#[tokio::test]
async fn closed_fixture_publication_channel_is_a_disconnection_not_unchanged() {
    let (_root, state) = fixture();
    let store = &state.store;
    let _wake = store.start_agent_roster_refresher().unwrap();
    refresh_agent_roster(store, true).unwrap();
    update_card(store);
    let (sender, receiver) = tokio::sync::watch::channel(0);
    drop(sender);
    let result =
        wait_for_agent_roster_publications(store, true, store.index().unwrap(), receiver).await;
    assert_eq!(result, RosterFreshWaitOutcome::Disconnected);
    // The closed receiver is a private branch control, not a production Store lifetime claim.
}

#[tokio::test(start_paused = true)]
async fn unsatisfied_fresh_wait_times_out_at_the_unchanged_virtual_deadline() {
    let (_root, state) = fixture();
    let store = &state.store;
    let _wake = store.start_agent_roster_refresher().unwrap();
    refresh_agent_roster(store, true).unwrap();
    let old = store
        .published_agent_roster(store.index().unwrap(), true)
        .unwrap();
    update_card(store);
    let started = tokio::time::Instant::now();
    let waited = wait_for_agent_roster(store, true).await;
    assert_eq!(waited.outcome, RosterFreshWaitOutcome::Timeout);
    assert_eq!(started.elapsed(), AGENT_ROSTER_READ_WAIT);
    assert_ne!(
        roster_fresh_served_proof(store, true, waited, Some((old.0, old.2, &old.1))),
        RosterUnchangedProof::Verified
    );
}

// Capture every attribute write rather than an SDK's final deduplicated map: two calls
// that write the same value must fail the exactly-once assertion too. No global tracer.
struct AttributeWrites {
    context: opentelemetry::trace::SpanContext,
    values: Arc<std::sync::Mutex<Vec<opentelemetry::KeyValue>>>,
}

impl opentelemetry::trace::Span for AttributeWrites {
    fn add_event_with_timestamp<T: Into<std::borrow::Cow<'static, str>>>(
        &mut self,
        _: T,
        _: std::time::SystemTime,
        _: Vec<opentelemetry::KeyValue>,
    ) {
    }
    fn span_context(&self) -> &opentelemetry::trace::SpanContext {
        &self.context
    }
    fn is_recording(&self) -> bool {
        true
    }
    fn set_attribute(&mut self, attribute: opentelemetry::KeyValue) {
        self.values.lock().unwrap().push(attribute);
    }
    fn set_status(&mut self, _: opentelemetry::trace::Status) {}
    fn update_name<T: Into<std::borrow::Cow<'static, str>>>(&mut self, _: T) {}
    fn add_link(&mut self, _: opentelemetry::trace::SpanContext, _: Vec<opentelemetry::KeyValue>) {}
    fn end_with_timestamp(&mut self, _: std::time::SystemTime) {}
}

fn with_attribute_writes<T>(work: impl FnOnce() -> T) -> (T, Vec<opentelemetry::KeyValue>) {
    use opentelemetry::trace::TraceContextExt as _;
    let values = Arc::new(std::sync::Mutex::new(Vec::new()));
    let context = opentelemetry::Context::new().with_span(AttributeWrites {
        context: opentelemetry::trace::SpanContext::empty_context(),
        values: Arc::clone(&values),
    });
    let _context = context.attach();
    let _export = crate::otel::fixture_export_scope();
    let result = crate::otel::roster_scope(|| {
        assert!(crate::otel::roster_recording());
        work()
    });
    let attributes = values.lock().unwrap().clone();
    (result, attributes)
}

#[test]
fn fresh_wait_attributes_are_recorded_once_on_each_page_return_path() {
    // Exercise real page selection/rendering. Missing, filtered/history not-ready,
    // partial head, full and filtered complete, plus an error during page rendering.
    for path in [
        "missing_head",
        "history_not_ready",
        "filtered_not_ready",
        "head",
        "complete",
        "filtered_complete",
        "page_error",
    ] {
        let (_root, state) = fixture();
        let store = &state.store;
        let _wake = store.start_agent_roster_refresher().unwrap();
        let mut query = ClientListQuery {
            fresh: true,
            ..Default::default()
        };
        if path == "history_not_ready" {
            query.history = true;
        }
        if matches!(path, "filtered_not_ready" | "filtered_complete") {
            query.status = Some("running".into());
        }
        if path == "head" {
            store
                .read_snapshot(|index| client_agent_roster_head(store, index))
                .unwrap();
            assert!(
                store
                    .published_agent_roster(store.index().unwrap(), false)
                    .is_none()
            );
            assert!(
                store
                    .published_agent_roster_head(store.index().unwrap(), 1)
                    .is_some()
            );
        } else if matches!(path, "complete" | "filtered_complete" | "page_error") {
            refresh_agent_roster(store, false).unwrap();
        }
        if path == "page_error" {
            // Direct helper branch fixture: real continuations use their separate handler.
            query.cursor = Some("invalid-private-cursor".into());
        }
        let cut = store.index().unwrap();
        let waited = RosterFreshWait {
            wanted: Some(cut),
            outcome: RosterFreshWaitOutcome::Unchanged,
        };
        for fresh in [true, false] {
            query.fresh = fresh;
            let (result, attributes) = with_attribute_writes(|| {
                client_agents_published_page(&state, &query, cut, fresh.then_some(waited))
            });
            if path == "page_error" {
                assert!(result.is_err());
            } else {
                assert_eq!(
                    result.unwrap().is_some(),
                    matches!(path, "head" | "complete" | "filtered_complete"),
                    "{path}"
                );
            }
            if fresh {
                let expected = if matches!(path, "complete" | "filtered_complete") {
                    "verified"
                } else {
                    "refused"
                };
                let proof = attributes
                    .iter()
                    .find(|attribute| attribute.key.as_str() == "st.roster.fresh_proof")
                    .unwrap();
                assert_eq!(proof.value.as_str(), expected, "{path}");
            }
            for key in [
                "st.roster.fresh_wait",
                "st.roster.fresh_wait_cut",
                "st.roster.fresh_proof",
                "st.roster.fresh_unchanged_verified",
            ] {
                assert_eq!(
                    attributes
                        .iter()
                        .filter(|attribute| attribute.key.as_str() == key)
                        .count(),
                    usize::from(fresh),
                    "{path}: {key}: {attributes:?}"
                );
            }
        }
    }
}

#[tokio::test]
async fn traced_unchanged_proof_reports_work_for_each_churn_interval() {
    for history in [false, true] {
        for churn in [0, 64, 1024] {
            let (_root, state) = fixture();
            let store = &state.store;
            let _wake = store.start_agent_roster_refresher().unwrap();
            refresh_agent_roster(store, history).unwrap();
            let (cut, cards, published_at) = store
                .published_agent_roster(store.index().unwrap(), history)
                .unwrap();
            for index in 0..churn {
                unrelated(store, &format!("cost-{index}"));
            }
            let through = store.index().unwrap();
            assert_eq!(through - cut, churn);
            let waited = wait_for_agent_roster(store, history).await;
            // At zero churn the ordinary numeric publication already satisfies the wait.
            assert_eq!(
                waited.outcome,
                if churn == 0 {
                    RosterFreshWaitOutcome::Published
                } else {
                    RosterFreshWaitOutcome::Unchanged
                }
            );
            let writes = smallclaims::touched::writes();
            let refolds = store.agent_resources_refolded_cards_for_test();
            let proof_scope = smallclaims::sqlite::work::SqliteWorkScope::start();
            let started = Instant::now();
            let verified = store
                .served_agent_roster_unchanged_through(through, history, cut, published_at, &cards)
                .unwrap();
            let proof_elapsed = started.elapsed();
            let proof_work = proof_scope.finish();
            assert!(verified);
            let query = ClientListQuery {
                history,
                fresh: true,
                ..Default::default()
            };
            let page_scope = smallclaims::sqlite::work::SqliteWorkScope::start();
            let started = Instant::now();
            let (page, attributes) = with_attribute_writes(|| {
                client_agents_published_page(&state, &query, through, Some(waited))
            });
            let page_elapsed = started.elapsed();
            let page_work = page_scope.finish();
            assert_eq!(page.unwrap().unwrap().0.0.store_index, cut);
            let proof = attributes
                .iter()
                .find(|attribute| attribute.key.as_str() == "st.roster.fresh_proof")
                .unwrap();
            assert_eq!(
                proof.value.as_str(),
                if churn == 0 {
                    "not_checked"
                } else {
                    "verified"
                }
            );
            assert_eq!(smallclaims::touched::writes(), writes);
            assert_eq!(store.agent_resources_refolded_cards_for_test(), refolds);
            eprintln!(
                "fresh-proof-cost history={history} claims={churn} cut={cut} through={through} isolated_proof={proof_work:?} proof_wall_us={} traced_page={page_work:?} page_wall_us={}",
                proof_elapsed.as_micros(),
                page_elapsed.as_micros()
            );
        }
    }
}

#[test]
fn roster_large_delta_reassembles_bounded_chunks_without_losing_late_membership() {
    use crate::store::seats::AGENT_ROSTER_DELTA_ROWS;

    for history in [false, true] {
        let (_root, state) = fixture();
        let store = &state.store;
        let _wake = store.start_agent_roster_refresher().unwrap();
        let count = AGENT_ROSTER_WARM_CHUNK * 2 + 1;
        for n in 0..count {
            roster_resilience_runtime(store, &format!("agent/delta-{n:04}"), "old");
        }
        refresh_agent_roster(store, history).unwrap();
        let (old_cut, old_cards, _) = store.published_agent_roster(store.index().unwrap(), history).unwrap();
        for n in 0..=AGENT_ROSTER_DELTA_ROWS {
            unrelated(store, &format!("overflow-{n}"));
        }
        // Both a changed card and a newly visible agent lie beyond the row ceiling.
        roster_resilience_runtime(store, "agent/delta-0000", "new");
        roster_resilience_runtime(store, "agent/delta-late", "new");
        let cut = store.index().unwrap();
        let reason = store.read_snapshot(|index| {
            store.agent_roster_unbounded_because(index, history, AGENT_ROSTER_WARM_CHUNK)
        }).unwrap();
        assert_eq!(reason.as_deref(), Some("claim delta exceeds row budget"));
        assert_eq!(store.published_agent_roster(cut, history).unwrap().0, old_cut);
        let before = store.agent_resources_refolded_cards_for_test();
        refresh_agent_roster(store, history).unwrap();
        let (published, cards, _) = store.published_agent_roster(cut, history).unwrap();
        assert_eq!((published, cards.len()), (cut, old_cards.len() + 1));
        assert!(store.agent_resources_refolded_cards_for_test() - before >= count);
        assert!(store.agent_resources_largest_fold_for_test() <= AGENT_ROSTER_WARM_CHUNK);
        assert_eq!(store.agent_roster_chunked_assemblies()["claim delta exceeds row budget"], 1);
        let mut expected = store.read_snapshot(|index| {
            let mut rows = client_agent_resources_uncached(store, history, index)?;
            add_agent_todos(store, &mut rows, index)?;
            Ok(rows)
        }).unwrap();
        expected.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str())
            .then_with(|| a["id"].as_str().cmp(&b["id"].as_str())));
        assert_eq!(*cards, expected, "overflow must rebuild all dependencies, not truncate the delta");
    }
}

#[test]
fn roster_expired_snapshot_releases_reader_and_preserves_complete_publication() {
    use std::sync::atomic::{AtomicBool, Ordering};

    for history in [false, true] {
        for phase in ["sql", "cache"] {
            let (_root, state) = fixture();
            let store = &state.store;
            let _wake = store.start_agent_roster_refresher().unwrap();
            refresh_agent_roster(store, history).unwrap();
            let before = store.published_agent_roster(store.index().unwrap(), history).unwrap();
            unrelated(store, "deadline");
            update_card(store);
            let expired = Arc::new(AtomicBool::new(false));
            let clock_expired = Arc::clone(&expired);
            let start = Instant::now();
            let budget = smallclaims::read_budget::ReadBudget::with_clock_for_test(
                "roster/refresh", Duration::from_secs(1), Arc::new(move || {
                    start + if clock_expired.load(Ordering::SeqCst) {
                        Duration::from_secs(2)
                    } else { Duration::ZERO }
                }),
            );
            let result = smallclaims::read_budget::with(Some(budget), || {
                agent_roster_snapshot(store, |index| {
                    assert!(store.readers.has_pinned_reader());
                    if phase == "sql" {
                        expired.store(true, Ordering::SeqCst);
                        // Exercise the real reader progress handler, not just a Rust check.
                        store.readers.get().query_row(
                            "WITH RECURSIVE n(x) AS (VALUES(0) UNION ALL SELECT x+1 FROM n WHERE x<100000)
                             SELECT sum(x) FROM n", [], |row| row.get::<_, i64>(0),
                        )?;
                    } else {
                        store.cached_agent_resources(index, history, |_| {
                            expired.store(true, Ordering::SeqCst);
                            Ok(Vec::new())
                        })?;
                    }
                    Ok(())
                })
            });
            assert!(result.is_err(), "{phase} must observe wall expiry");
            assert!(!store.readers.has_pinned_reader(), "expiry must end the snapshot");
            let after = store.published_agent_roster(store.index().unwrap(), history).unwrap();
            assert_eq!((after.0, after.2), (before.0, before.2));
            assert!(Arc::ptr_eq(&after.1, &before.1));
            // The released connection remains usable and the next pass can complete.
            refresh_agent_roster(store, history).unwrap();
            assert_eq!(store.published_agent_roster(store.index().unwrap(), history).unwrap().0,
                store.index().unwrap());
        }
    }
}

#[test]
fn roster_short_attempts_resume_membership_and_reconcile_interleaved_changes() {
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    for history in [false, true] {
        let (_root, state) = fixture();
        let store = &state.store;
        let _wake = store.start_agent_roster_refresher().unwrap();
        refresh_agent_roster(store, history).unwrap();
        let previous = store.published_agent_roster(store.index().unwrap(), history).unwrap();
        for n in 0..(AGENT_ROSTER_WARM_CHUNK * 2 + 1) {
            roster_resilience_runtime(store, &format!("agent/resume-{n:04}"), "old");
        }
        let mut completed = false;
        let mut failures = 0;
        for attempt in 0..32 {
            let clock_store = Arc::clone(store);
            let pinned = AtomicBool::new(false);
            let snapshots = AtomicU64::new(0);
            let start = Instant::now();
            // Spend one second only when a pinned snapshot has been released. Every
            // attempt can finish a page but cannot finish the whole roster. No sleeps.
            let budget = smallclaims::read_budget::ReadBudget::with_clock_for_test(
                "roster/resume-test", Duration::from_secs(2), Arc::new(move || {
                    if clock_store.readers.has_pinned_reader() {
                        pinned.store(true, Ordering::SeqCst);
                    } else if pinned.swap(false, Ordering::SeqCst) {
                        snapshots.fetch_add(1, Ordering::SeqCst);
                    }
                    start + Duration::from_secs(snapshots.load(Ordering::SeqCst))
                }),
            );
            let result = smallclaims::read_budget::with(Some(budget), || refresh_agent_roster(store, history));
            assert!(!store.readers.has_pinned_reader());
            if result.is_ok() {
                assert!(attempt > 0, "one short attempt cannot scan this roster");
                completed = true;
                break;
            }
            failures += 1;
            let retained = store.published_agent_roster(store.index().unwrap(), history).unwrap();
            assert_eq!(retained.0, previous.0);
            assert!(Arc::ptr_eq(&retained.1, &previous.1));
            if attempt == 0 {
                // Changes behind and ahead of the saved subject cursor. A paged claim
                // backlog also exceeds the fast-path delta ceiling between attempts.
                roster_resilience_runtime(store, "agent/aaa-behind-cursor", "new");
                roster_resilience_runtime(store, "agent/zzz-ahead-of-cursor", "new");
                roster_resilience_runtime(store, "agent/resume-0000", "changed");
                for n in 0..=crate::store::seats::AGENT_ROSTER_DELTA_ROWS {
                    unrelated(store, &format!("resume-backlog-{n}"));
                }
            }
        }
        assert!(completed, "saved pages must converge despite repeated wall expiry");
        assert!(failures >= 2, "exercise expiry at several assembly phases");
        let (cut, cards, _) = store.published_agent_roster(store.index().unwrap(), history).unwrap();
        assert_eq!(cut, store.index().unwrap());
        let mut expected = store.read_snapshot(|index| {
            let mut rows = client_agent_resources_uncached(store, history, index)?;
            add_agent_todos(store, &mut rows, index)?;
            Ok(rows)
        }).unwrap();
        expected.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str())
            .then_with(|| a["id"].as_str().cmp(&b["id"].as_str())));
        assert_eq!(*cards, expected);
        assert!(store.agent_resources_largest_fold_for_test() <= AGENT_ROSTER_WARM_CHUNK);
    }
}
