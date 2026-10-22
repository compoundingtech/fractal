#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::tests_shared_missions::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::tests_shared_missions::{CARRY_SOURCE, carry_request, listed, misproject_a_step, publish_carry, request_fault};
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::tests_shared_seats::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::tests_shared_seats::{TAKEOVER_SOURCE, TREE_SHAPE_RUNS, TreeShapes, full_canonical_claim_id_oracle, graph_digest_of, insert_raw_claims, local_observation_for_test};
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::tests_shared_seats::{local_timeline_observation_for_test, observe_runtime, projection_replayed, publish_mission, publish_takeover, query_plan, replicate_fully, roster_cache_store, simple};
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::tests_shared_seats::{exchange_of, harness_state, takeover_step, timeline_observation};
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::tests_shared::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::tests_shared::{TEST_FLEET, exchange_from, receive_and_project};
use super::*;
use crate::graph::parse_test_intent as parse_intent;
use proptest::prelude::*;

mod resource_pending_receive_tests {
    
}

mod terminal_projection_date_tests {
    
}

mod mission_eval_reuse_tests {
}

mod subject_reference_readmission_tests {
    use super::*;
    /* sqlite-large-value: allow-for-now lv-hd729ec9593f5 owner=fractal_store::store::tests TODO=trace-statement-and-value-representation */ include!("subject_reference_readmission_tests.rs");
}

mod run_capacity_tests {
    
}

mod mission_deadline_probe {
    
}

mod stopped_seat_declaration_controls {
    use super::*;
    /* sqlite-large-value: allow-for-now lv-hb3cc9adf2362 owner=fractal_store::store::tests TODO=trace-statement-and-value-representation */ include!("stopped_seat_declaration_controls.rs");
}

mod mission_deadline_tests {
    
}

#[test]
fn projection_times_at_names_every_cut_like_the_single_lookup() {
    let store = roster_cache_store();
    let (_, newest) = store.event_bounds().unwrap();
    assert!(newest > 1, "the fixture needs several claims");
    // Heterogeneous, repeated, unordered, empty-store and past-the-frontier cuts.
    let mut cuts = vec![0, newest + 7, 1, newest, 1, newest / 2, 0, newest];
    cuts.extend(1..=newest);
    let batched = store.projection_times_at(&cuts).unwrap();
    for cut in &cuts {
        assert_eq!(batched[cut], store.projection_time_at(*cut).unwrap(), "cut {cut}");
    }
    assert_eq!(batched.len(), (0..=newest + 7).filter(|c| cuts.contains(c)).count());
    assert!(store.projection_times_at(&[]).unwrap().is_empty());
    // Acceptance times are per claim, not one shared value, once the clock moves.
    assert!(batched[&newest] >= batched[&1]);
}

#[test]
fn projection_times_at_seeks_the_claim_index_once_per_cut() {
    let store = roster_cache_store();
    let connection = store.readers.get();
    let plan = connection
        .prepare(
            "EXPLAIN QUERY PLAN SELECT cut.value,
                        (SELECT accepted_at_unix_ms FROM claims
                         WHERE store_index <= cut.value ORDER BY store_index DESC LIMIT 1)
                 FROM json_each(?1) AS cut",
        )
        .unwrap()
        .query_map(["[1,2]"], |row| row.get::<_, String>(3))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
        .join("\n");
    assert!(plan.contains("SEARCH claims USING INTEGER PRIMARY KEY"), "{plan}");
    assert!(!plan.contains("SCAN claims"), "{plan}");
}

mod mailbox_fence_tests {
    use super::*;
    /* sqlite-large-value: allow-for-now lv-hd43f90378c92 owner=fractal_store::store::tests TODO=trace-statement-and-value-representation */ include!("mailbox_fence_tests.rs");
}

mod canonical_audit {
    
}


mod declarative_revision_timestamp_tests {
    
}

mod subagents_purity_tests {
    use super::*;
    /* sqlite-large-value: allow-for-now lv-h268b21b0f384 owner=fractal_store::store::tests TODO=trace-statement-and-value-representation */ include!("subagents_purity_tests.rs");
}

mod actual_purity_tests {
    
}

mod activity_purity_tests {
    use super::*;
    /* sqlite-large-value: allow-for-now lv-h2c081374795f owner=fractal_store::store::tests TODO=trace-statement-and-value-representation */ include!("activity_purity_tests.rs");
}

mod limits_purity_tests {
    use super::*;
    /* sqlite-large-value: allow-for-now lv-h5244785b99be owner=fractal_store::store::tests TODO=trace-statement-and-value-representation */ include!("limits_purity_tests.rs");
}


mod fault_history_tests {
    
}

mod projection_divergence_tests {
    
}

#[test]
fn declared_launch_lineage_fences_subject_kind_and_launch_changes() {
    let original = parse_intent(
        r#"version 2
    agent "garden/orchard" { command "true" }
    "#,
        "amber",
    )
    .unwrap()
    .subjects
    .remove("agent/garden/orchard")
    .unwrap();
    let previous = serde_json::to_value(&original).unwrap();
    for edit in [r#"env { COLOR "blue"; }"#, r#"name "New label";"#] {
        let next = parse_intent(
            &format!("version 2\nagent \"garden/orchard\" {{ command \"true\"; {edit} }}"),
            "amber",
        )
        .unwrap();
        assert!(same_declared_launch(
            &previous,
            &serde_json::to_value(&next.subjects["agent/garden/orchard"]).unwrap()
        ));
    }
    for field in ["subject", "kind", "launch"] {
        let mut next = original.clone();
        match field {
            "subject" => next.subject = "agent/garden/meadow".into(),
            "kind" => next.kind = "stop".into(),
            "launch" => {
                next.member.as_mut().unwrap().launch =
                    crate::model::LaunchSpec::Shell("false".into())
            }
            _ => unreachable!(),
        }
        assert!(
            !same_declared_launch(&previous, &serde_json::to_value(&next).unwrap()),
            "{field}"
        );
    }
    assert!(!same_declared_launch(&previous, &json!({"kind":"agent"})));
}

#[test]
fn subject_claim_pages_seek_the_subject_index() {
    let store = Store::open_memory("node").unwrap();
    store.claims_page(None, None, 0, None, false, 1).unwrap();
    store
        .claims_page(Some("agent/target"), None, 0, None, false, 1)
        .unwrap();
    let connection = store.readers.get();
    for descending in [false, true] {
        let query = format!("EXPLAIN QUERY PLAN {}", claims_page_query(true, descending));
        let plan = connection
            .prepare(/* sqlite-large-value: allow-for-now lv-h8911c1538a9c owner=fractal_store::store::tests TODO=trace-statement-and-value-representation */ &query)
            .unwrap()
            .query_map(params![0, None::<u64>, "agent/target", 2], |row| {
                row.get::<_, String>(3)
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
            .join("; ");
        assert!(
            plan.contains("claims_subject_index"),
            "a subject page must not scan the whole graph: {plan}"
        );
    }
}

#[test]
fn a_pinned_read_sees_one_snapshot_while_commits_land() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(&directory.path().join("state.sqlite3"), "node").unwrap();
    let observe = |n: u64| {
        store
            .append_claim(&ClaimInput {
                subject: "resource/pinned".into(),
                kind: "resource.observed".into(),
                actor: Some("person/avery".into()),
                fields: BTreeMap::from([
                    ("kind".into(), Value::String("human.review".into())),
                    ("reason".into(), Value::String(format!("change {n}"))),
                ]),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
    };
    observe(0);
    let before = store.index().unwrap();
    let seen = store
        .read_snapshot(|index| {
            assert_eq!(index, before);
            // Another thread commits in the middle of the read. The writer sees it; the read
            // does not.
            std::thread::scope(|scope| {
                scope.spawn(|| observe(1));
            });
            assert!(store.index().unwrap() > index);
            let claims = store.claims_for("resource/pinned", None)?.len();
            let nested = store.read_snapshot(|nested| {
                Ok((nested, store.claims_for("resource/pinned", None)?.len()))
            })?;
            assert_eq!(
                nested,
                (index, claims),
                "a nested read joins the outer snapshot"
            );
            Ok(claims)
        })
        .unwrap();
    assert_eq!(seen, 1);
    assert_eq!(store.claims_for("resource/pinned", None).unwrap().len(), 2);
    // Every pinned connection went back to the pool, even after a failed read.
    for _ in 0..32 {
        let _ = store.read_snapshot(|_| -> Result<()> { anyhow::bail!("a failed read") });
        store.read_snapshot(|_| Ok(())).unwrap();
    }
    assert_eq!(store.claims_for("resource/pinned", None).unwrap().len(), 2);
}

#[test]
fn a_read_opens_another_connection_rather_than_wait_for_one() {
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(&directory.path().join("state.sqlite3"), "node").unwrap());
    let (ready, readied) = std::sync::mpsc::channel();
    let (release, released) = std::sync::mpsc::channel::<()>();
    let holder = {
        let store = store.clone();
        std::thread::spawn(move || {
            store.hold_read_connections_for_test(|| {
                ready.send(()).unwrap();
                released.recv().unwrap();
            });
        })
    };
    readied.recv().unwrap();
    // Every connection the pool kept is busy, and more besides. A read and a pinned read
    // that fans out to worker threads still answer at once.
    let (done, finished) = std::sync::mpsc::channel();
    for _ in 0..8 {
        let (store, done) = (store.clone(), done.clone());
        std::thread::spawn(move || {
            let result = store.read_snapshot(|_| {
                std::thread::scope(|scope| {
                    scope
                        .spawn(|| store.claims_for("resource/busy", None))
                        .join()
                        .unwrap()
                })
            });
            done.send(result.is_ok()).unwrap();
        });
    }
    for _ in 0..8 {
        assert!(
            finished
                .recv_timeout(std::time::Duration::from_secs(5))
                .expect("a read waited for a busy connection")
        );
    }
    release.send(()).unwrap();
    holder.join().unwrap();
    // Idle retention does not bound concurrently open connections.
    assert!(
        store.readers.idle.lock().unwrap().len() <= max_idle_read_connections()
    );
}

#[test]
fn pinned_reads_that_fan_out_never_wait_on_each_other() {
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(&directory.path().join("state.sqlite3"), "node").unwrap());
    // Every pinned read holds its connection while a worker thread, like the agent status
    // reduction's, needs another one from the pool.
    let (done, finished) = std::sync::mpsc::channel();
    let barrier = Arc::new(std::sync::Barrier::new(STATUS_WORKERS));
    for _ in 0..STATUS_WORKERS {
        let (store, done, barrier) = (store.clone(), done.clone(), barrier.clone());
        std::thread::spawn(move || {
            barrier.wait();
            let result = store.read_snapshot(|_| {
                std::thread::sleep(std::time::Duration::from_millis(50));
                std::thread::scope(|scope| {
                    scope
                        .spawn(|| {
                            store
                                .claims_for("resource/pinned", None)
                                .map(|claims| claims.len())
                        })
                        .join()
                        .unwrap()
                })
            });
            done.send(result.is_ok()).unwrap();
        });
    }
    for _ in 0..STATUS_WORKERS {
        assert!(
            finished
                .recv_timeout(std::time::Duration::from_secs(10))
                .expect("a pinned read waited forever for a connection")
        );
    }
}

#[test]
fn an_inbound_exchange_keeps_the_outbound_grant_refusal_until_a_dial_succeeds() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("amber.db");
    let fleet = "94cd11ba-c582-4558-9c84-c3bda922eb6d";
    let store = Store::open(&path, "amber").unwrap();
    store.bind_fleet(fleet).unwrap();
    store
        .record_peer_failure(
            "cobalt",
            "refused",
            "refused by that member's Fabric grants",
        )
        .unwrap();
    drop(store);
    let store = Store::open(&path, "amber").unwrap();
    let peer = Store::open_memory("cobalt").unwrap();
    peer.bind_fleet(fleet).unwrap();
    let exchange = peer
        .export_replication_exchange(fleet, &ReplicationInventory::default())
        .unwrap();
    store
        .receive_replication_exchange("cobalt", fleet, &exchange)
        .unwrap();
    let status = store
        .replication_status(true, Some(fleet), &["cobalt".into()])
        .unwrap();
    assert_eq!(status.peers[0].status, "refused");
    assert!(status.peers[0].last_success_at_unix_ms.is_some());
    assert!(status.peers[0].last_error.is_none());
    store
        .receive_replication_exchange_asking("cobalt", fleet, &exchange, true)
        .unwrap();
    let status = store
        .replication_status(true, Some(fleet), &["cobalt".into()])
        .unwrap();
    assert_eq!(status.peers[0].status, "up");
    assert!(status.peers[0].refusal_reason.is_none());
}

#[test]
fn recent_up_observation_prevents_a_transport_timeout_flap() {
    let store = Store::open_memory("source").unwrap();
    store
        .record_transport_observation("target", "up", None, None)
        .unwrap();
    assert_eq!(store.replication_peer_last_success("target").unwrap(), None);
    assert!(
        !store
            .record_peer_failure("target", "down", "request timed out")
            .unwrap()
    );
    assert_eq!(
        store
            .latest_claim("host/target", Some("transport.observed"))
            .unwrap()
            .unwrap()
            .body["fields"]["status"],
        "up"
    );

    let stale = Store::open_memory("source").unwrap();
    stale
        .record_transport_observation(
            "target",
            "up",
            None,
            Some(now_ms().saturating_sub(91_000)),
        )
        .unwrap();
    assert!(
        stale
            .record_peer_failure("target", "down", "request timed out")
            .unwrap()
    );

    // Once an exchange has recorded a success, that row is the evidence, since every
    // exchange updates it. When it is old, a timeout goes through even though the newest
    // observation, from this node, still looks fresh.
    store
        .connection
        .lock()
        .unwrap()
        .execute(
            /* sqlite-large-value: allow-for-now lv-h5f3a84b52036 owner=fractal_store::store::tests TODO=chunk-or-reference-exact-wire-values */ "INSERT OR REPLACE INTO replication_peers(peer, status, last_success_at_unix_ms, updated_at_unix_ms)
                 VALUES ('target', 'up', ?1, ?2)",
            params![
                now_ms().saturating_sub(91_000).to_string(),
                now_ms().to_string()
            ],
        )
        .unwrap();
    assert!(
        store
            .record_peer_failure("target", "down", "request timed out")
            .unwrap()
    );
}

/// A peer that stops answering (#1020) shows its failure at once, and stops counting as up
/// once it has missed an exchange; a failure within one quiet interval of an exchange in
/// the other direction keeps it up. The next exchange clears the failure.
#[test]
fn a_failed_attempt_after_a_missed_exchange_ends_up() {
    let fleet = "94cd11ba-c582-4558-9c84-c3bda922eb6d";
    let store = Store::open_memory("amber").unwrap();
    store.bind_fleet(fleet).unwrap();
    let last_exchange = |ago: u128| {
        store
            .connection
            .lock()
            .unwrap()
            .execute(
                /* sqlite-large-value: allow-for-now lv-h11eed7b84d0c owner=fractal_store::store::tests TODO=chunk-or-reference-exact-wire-values */ "INSERT OR REPLACE INTO replication_peers(peer, status, last_success_at_unix_ms, updated_at_unix_ms)
                     VALUES ('cobalt', 'up', ?1, ?1)",
                [now_ms().saturating_sub(ago).to_string()],
            )
            .unwrap();
    };
    let peer = || {
        store
            .replication_status(true, Some(fleet), &["cobalt".into()])
            .unwrap()
            .peers
            .remove(0)
    };

    last_exchange(10_000);
    store
        .record_peer_failure("cobalt", "down", "request timed out")
        .unwrap();
    let status = peer();
    assert_eq!(
        status.status, "up",
        "a one-way failure soon after an exchange"
    );
    assert_eq!(status.last_error.as_deref(), Some("request timed out"));
    assert!(status.last_failure_at_unix_ms.is_some());

    last_exchange(40_000);
    assert!(
        !store
            .record_peer_failure("cobalt", "down", "request timed out")
            .unwrap(),
        "the published transport status keeps its grace"
    );
    let status = peer();
    assert_eq!(status.status, "last-seen");
    assert_eq!(status.last_error.as_deref(), Some("request timed out"));
    assert!(!store.replication_peer_up("cobalt").unwrap().0);

    let other = Store::open_memory("cobalt").unwrap();
    other.bind_fleet(fleet).unwrap();
    let exchange = other
        .export_replication_exchange(fleet, &ReplicationInventory::default())
        .unwrap();
    store
        .receive_replication_exchange("cobalt", fleet, &exchange)
        .unwrap();
    let status = peer();
    assert_eq!(status.status, "up");
    assert!(status.last_error.is_none());
    assert!(status.last_failure_at_unix_ms.is_none());
    assert!(store.replication_peer_up("cobalt").unwrap().0);
}

/// A measurement the peer has not refreshed for a quiet interval says what both held then
/// (#1020): status marks it stale and counts the envelopes this node gained since.
#[test]
fn an_old_sync_measurement_is_stale_and_counts_what_was_added_since() {
    let fleet = "94cd11ba-c582-4558-9c84-c3bda922eb6d";
    let store = Store::open_memory("amber").unwrap();
    store.bind_fleet(fleet).unwrap();
    let other = Store::open_memory("cobalt").unwrap();
    other.bind_fleet(fleet).unwrap();
    let exchange = other
        .export_replication_exchange(fleet, &ReplicationInventory::default())
        .unwrap();
    store
        .receive_replication_exchange("cobalt", fleet, &exchange)
        .unwrap();
    let sync = |store: &Store| {
        store
            .replication_status(true, Some(fleet), &["cobalt".into()])
            .unwrap()
            .peers
            .remove(0)
            .sync
            .unwrap()
    };
    let fresh = sync(&store);
    assert!(!fresh.stale);
    assert_eq!(fresh.added_since_measured_envelopes, 0);

    for index in 0..20 {
        store
            .append_claim(&ClaimInput {
                subject: format!("custom/frozen-peer/{index}"),
                kind: "custom.frozen-peer.note".into(),
                actor: Some("person/tester".into()),
                fields: BTreeMap::new(),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
    }
    store
        .replication_sync
        .lock()
        .unwrap()
        .get_mut("cobalt")
        .unwrap()
        .measured
        .as_mut()
        .unwrap()
        .measured_at_unix_ms -= 40_000;
    let old = sync(&store);
    assert!(old.stale);
    assert_eq!(old.local_only_envelopes, fresh.local_only_envelopes);
    assert!(
        old.added_since_measured_envelopes >= 20,
        "{}",
        old.added_since_measured_envelopes
    );
}

/// A start repairs operation rows that drifted (#898). It rewrites only those rows: rebuilding
/// the whole table held the only writer for a minute and more on a populated store.
#[test]
fn an_operation_drift_repair_rewrites_only_the_drifted_rows() {
    let store = Store::open_memory("node").unwrap();
    for index in 0..200 {
        store
            .append_claim(&ClaimInput {
                subject: format!("custom/drift/{index}"),
                kind: "custom.drift.note".into(),
                actor: Some("person/tester".into()),
                fields: BTreeMap::new(),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: Some(format!("drift-{index}")),
            })
            .unwrap();
    }
    let (rows, ids) = {
        let connection = store.connection.lock().unwrap();
        let ids = connection
            .prepare("SELECT id FROM operations ORDER BY id LIMIT 2")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        let rows: i64 = connection
            .query_row("SELECT COUNT(*) FROM operations", [], |row| row.get(0))
            .unwrap();
        (rows, ids)
    };
    assert!(rows >= 200, "{rows} operations");
    assert!(store.operation_projection_drift().unwrap().is_empty());
    // One row changed, one row lost, and one row that no claim backs.
    {
        let connection = store.connection.lock().unwrap();
        connection
            .execute(
                /* sqlite-large-value: allow-for-now lv-h16b065f401e1 owner=fractal_store::store::tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE operations SET state='conflict' WHERE id=?1",
                [&ids[0]],
            )
            .unwrap();
        connection
            .execute(/* sqlite-large-value: allow-for-now lv-hea5945cd6dbb owner=fractal_store::store::tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM operations WHERE id=?1", [&ids[1]])
            .unwrap();
        connection
            .execute(
                /* sqlite-large-value: allow-for-now lv-hf5e4151a8ff2 owner=fractal_store::store::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO operations(id, request_digest, canonical_claim_id, state)
                     SELECT 'op/invented-stray', 'digest', canonical_claim_id, 'active'
                     FROM operations WHERE id=?1",
                [&ids[0]],
            )
            .unwrap();
    }
    assert_eq!(store.operation_projection_drift().unwrap().len(), 3);
    // Rows written by the repair, besides the ones its triggers write.
    let operation_rows = || {
        store
            .connection
            .lock()
            .unwrap()
            .query_row("SELECT total_changes()", [], |row| row.get::<_, i64>(0))
            .unwrap()
    };
    let before = operation_rows();
    assert!(store.repair_operation_projection_drift().unwrap());
    assert!(store.operation_projection_drift().unwrap().is_empty());
    let written = operation_rows() - before;
    assert!(
        written <= 3 * 4,
        "the repair wrote {written} rows for 3 drifted ones"
    );
    assert!(!store.repair_operation_projection_drift().unwrap());
}

/// The drift audit reads every claim an operation row can come from. Walking the whole
/// claims table evaluated json_extract on every claim in the store, seconds of work and
/// hundreds of megabytes of reads on a populated member, while the partial operation index
/// already names exactly the claims the audit keeps.
#[test]
fn the_operation_audit_walks_the_partial_operation_index() {
    let store = Store::open_memory("node").unwrap();
    let connection = store.readers.get();
    let plan = connection
        .prepare(/* sqlite-large-value: allow-for-now lv-h306b2a3e7dec owner=fractal_store::store::tests TODO=trace-statement-and-value-representation */ &format!(
            "EXPLAIN QUERY PLAN {}",
            smallclaims::store::EXPECTED_OPERATIONS_QUERY
        ))
        .unwrap()
        .query_map([], |row| row.get::<_, String>(3))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
        .join("; ");
    assert!(
        plan.contains("USING INDEX claims_operation_index")
            && !plan.contains("USING INDEX sqlite_autoindex_claims_1"),
        "{plan}"
    );
}

#[test]
fn persistent_store_uses_bounded_sqlite_page_caches() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(&directory.path().join("state.sqlite3"), "node").unwrap();
    let writer_cache_kib: i64 = store
        .connection
        .lock()
        .unwrap()
        .query_row("PRAGMA cache_size", [], |row| row.get(0))
        .unwrap();
    let reader_cache_kib: i64 = store
        .readers
        .get()
        .query_row("PRAGMA cache_size", [], |row| row.get(0))
        .unwrap();
    assert_eq!(writer_cache_kib, -32768);
    assert_eq!(reader_cache_kib, -(smallclaims::sqlite::read_cache_kib() as i64));
}

#[test]
fn replication_projection_retries_missing_and_stale_health_only() {
    let store = Store::open_memory("node").unwrap();
    assert!(store.replication_projection_needs_recovery().unwrap());
    assert!(store.project_replication_backlog().unwrap());
    assert!(!store.replication_projection_needs_recovery().unwrap());
    store
        .connection
        .lock()
        .unwrap()
        .execute(
            /* sqlite-large-value: allow-for-now lv-h596defbbc0eb owner=fractal_store::store::tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE projection_health SET status='stale' WHERE aggregate='graph'",
            [],
        )
        .unwrap();
    assert!(store.replication_projection_needs_recovery().unwrap());
}

#[test]
fn simple_replication_advances_events_without_full_replay() {
    let store = Store::open_memory("node").unwrap();
    assert!(store.project_replication_backlog().unwrap());
    let claim = {
        let mut connection = store.connection.lock().unwrap();
        let transaction = connection.transaction().unwrap();
        let claim = append_claim_tx(
            &transaction,
            &store.origin,
            "agent/node.test",
            "harness.observed",
            Some("agent/node.test"),
            &json!({"fields": {"state": "ready"}}),
            &[],
            None,
        )
        .unwrap();
        assert!(try_project_all_simple_replication_tx(&transaction, &store.origin).unwrap());
        transaction.commit().unwrap();
        claim
    };
    let connection = store.connection.lock().unwrap();
    let event_count: u64 = connection
        .query_row(
            "SELECT COUNT(*) FROM events WHERE store_index=?1",
            [claim.store_index],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(event_count, 1);
}

#[test]
fn resource_observation_projection_does_not_rebuild_operation_history() {
    let store = Store::open_memory("node").unwrap();
    assert!(store.project_replication_backlog().unwrap());
    {
        let mut connection = store.connection.lock().unwrap();
        let transaction = connection.transaction().unwrap();
        append_claim_tx(
            &transaction,
            &store.origin,
            "agent/node.test",
            "harness.observed",
            Some("agent/node.test"),
            &json!({"fields": {"state": "ready"}, "_operation": {
                "id": "op/existing", "request_digest": "digest-one"
            }}),
            &[],
            None,
        )
        .unwrap();
        transaction.commit().unwrap();
    }
    assert!(store.project_replication_backlog().unwrap());
    {
        let mut connection = store.connection.lock().unwrap();
        let transaction = connection.transaction().unwrap();
        append_claim_tx(
            &transaction,
            &store.origin,
            "resource/repo",
            "resource.observed",
            None,
            &json!({"fields": {"kind": "vcs.repository", "state": "active"}}),
            &[],
            None,
        )
        .unwrap();
        transaction.commit().unwrap();
    }
    let before = store.connection.lock().unwrap().total_changes();
    assert!(store.project_replication_backlog().unwrap());
    let connection = store.connection.lock().unwrap();
    assert!(connection.total_changes() - before <= 2);
    assert!(operation_tx(&connection, "op/existing").unwrap().is_some());
}

/// A full replay projects each claim in its own savepoint and quarantines one it cannot
/// project. Once the transaction has changed the schema, as a checkpoint proof's does with
/// its temporary tables, SQLite aborts every open read when a savepoint rolls back. The
/// replay must not be reading then, or one malformed claim fails every proof on that node.
#[test]
fn a_replay_quarantines_claims_it_cannot_project_and_projects_the_rest() {
    pub const FLEET: &str = "5e3c1a9b-2d4f-4b6e-8a7c-0f1e2d3c4b5a";
    let source = Store::open_memory("source").unwrap();
    for n in 0..3 {
        source
            .append_claim(&ClaimInput {
                subject: format!("mission-run/broken-{n}"),
                kind: "mission-run.created".into(),
                actor: None,
                fields: BTreeMap::new(),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: Some(format!("broken-{n}")),
            })
            .unwrap();
    }
    let later = source
        .append_claim(&ClaimInput {
            subject: "daemon/source".into(),
            kind: "daemon.started".into(),
            actor: None,
            fields: BTreeMap::from([("status".into(), json!("running"))]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: Some("started".into()),
        })
        .unwrap();
    source.bind_fleet(FLEET).unwrap();
    let target = Store::open_memory("target").unwrap();
    target.bind_fleet(FLEET).unwrap();
    let exchange = source
        .export_replication_exchange(FLEET, &target.replication_inventory().unwrap())
        .unwrap();
    target
        .receive_replication_exchange("source", FLEET, &exchange)
        .unwrap();
    target.validate_replication_backlog().unwrap();
    target.project_replication_backlog().unwrap();
    let connection = target.readers.get();
    let quarantined: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM projection_health
                 WHERE aggregate LIKE 'projection:runs:%' AND status='stale'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(quarantined, 3);
    let healthy: String = connection
        .query_row(
            "SELECT status FROM projection_health WHERE aggregate='graph'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(healthy, "healthy");
    assert!(
        target
            .latest_claim("daemon/source", Some("daemon.started"))
            .unwrap()
            .is_some_and(|claim| claim.id == later.id)
    );
    drop(connection);
    // The proof replays the same claims after creating its temporary tables.
    let scratch = tempfile::tempdir().unwrap();
    let (_, _, proof) = target
        .plan_checkpoint_through(now_ms() + 1_000, None, scratch.path())
        .unwrap();
    assert!(proof.passed, "{proof:?}");
}

/// Writes that arrive together share one transaction and one commit, and each caller hears
/// back only once that commit is on disk.
#[test]
fn concurrent_claims_share_the_writer_s_commits() {
    let directory = tempfile::tempdir().unwrap();
    let store =
        Arc::new(Store::open(&directory.path().join("claims.sqlite3"), "node").unwrap());
    let start = Arc::new(std::sync::Barrier::new(32));
    let writers = (0..32)
        .map(|index| {
            let (store, start) = (store.clone(), start.clone());
            std::thread::spawn(move || {
                start.wait();
                store
                    .append_claim(&ClaimInput {
                        subject: format!("agent/node.writer-{index}"),
                        kind: "harness.usage".into(),
                        actor: Some(format!("agent/node.writer-{index}")),
                        fields: serde_json::from_value(json!({
                            "driver": "codex", "incarnation_id": "current",
                            "semantics": "response", "total_tokens": index
                        }))
                        .unwrap(),
                        evidence: Vec::new(),
                        expected_subject: None,
                        idempotency_key: Some(format!("usage-{index}")),
                    })
                    .unwrap()
            })
        })
        .collect::<Vec<_>>();
    let claims = writers
        .into_iter()
        .map(|writer| writer.join().unwrap())
        .collect::<Vec<_>>();
    let (batches, writes) = (
        store.connection.batches.0.load(Ordering::Relaxed),
        store.connection.batches.1.load(Ordering::Relaxed),
    );
    assert_eq!(writes, 32, "{batches} batches");
    assert!(batches < 32, "32 concurrent claims took {batches} commits");
    // Each claim is committed when its writer hears back.
    for claim in claims {
        assert!(store.claim_by_id(&claim.id).unwrap().is_some());
    }
}

/// A batched write that fails or panics undoes only its own savepoint: the writes that share
/// its transaction still commit, the failed caller gets its error and the panic reaches the
/// caller that panicked.
#[test]
fn a_failed_batched_write_undoes_only_itself() {
    let store = Arc::new(Store::open_memory("node").unwrap());
    let insert = |transaction: &Transaction<'_>, key: &str| {
        transaction
            .execute(/* sqlite-large-value: allow-for-now lv-h0842fb52bbb7 owner=fractal_store::store::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta(key, value) VALUES (?1, 'written')", [key])
            .map(drop)
    };
    // Hold the writer so the three writes queue behind it and share the next batch.
    let held = store.connection.write();
    let writers = ["succeeds", "fails", "panics"].map(|outcome| {
        let store = store.clone();
        std::thread::spawn(move || {
            store
                .connection
                .batched(|transaction| -> Result<(), String> {
                    insert(transaction, &format!("batch-test-{outcome}"))
                        .map_err(|error| error.to_string())?;
                    match outcome {
                        "fails" => Err("refused".into()),
                        "panics" => panic!("a batched write panicked"),
                        _ => Ok(()),
                    }
                })
                .unwrap()
        })
    });
    std::thread::sleep(std::time::Duration::from_millis(100));
    drop(held);
    let [succeeds, fails, panics] = writers.map(|writer| writer.join());
    assert_eq!(succeeds.unwrap(), Ok(()));
    assert_eq!(fails.unwrap(), Err("refused".into()));
    assert!(panics.is_err());
    let reader = store.readers.get();
    let written = |key: &str| {
        reader
            .query_row("SELECT 1 FROM meta WHERE key=?1", [key], |_| Ok(()))
            .optional()
            .unwrap()
            .is_some()
    };
    assert!(written("batch-test-succeeds"));
    assert!(!written("batch-test-fails"));
    assert!(!written("batch-test-panics"));
    // The writer keeps working after both.
    store
        .connection
        .batched(|transaction| insert(transaction, "batch-test-after"))
        .unwrap()
        .unwrap();
    assert!(written("batch-test-after"));
}

/// A batch that cannot begin, because another process holds SQLite's write lock past the busy
/// timeout, answers each of its writes with that error instead of losing them.
#[test]
fn a_batch_that_cannot_begin_answers_its_writes_with_the_error() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("claims.sqlite3");
    let store = Store::open(&path, "node").unwrap();
    store
        .connection
        .write()
        .execute_batch("PRAGMA busy_timeout = 50")
        .unwrap();
    let other = Connection::open(&path).unwrap();
    other.execute_batch(/* sqlite-large-value: allow-for-now lv-hab9c96c5ee3a owner=fractal_store::store::tests TODO=trace-statement-and-value-representation */ "BEGIN IMMEDIATE").unwrap();
    let refused = store.connection.batched(|transaction| {
        transaction.execute(
            /* sqlite-large-value: allow-for-now lv-h26cd1e644d89 owner=fractal_store::store::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta(key, value) VALUES ('refused', 'written')",
            [],
        )
    });
    let error = refused.expect_err("the batch could not begin");
    assert!(error.contains("locked"), "{error}");
    other.execute_batch(/* sqlite-large-value: allow-for-now lv-hb5738141bc7b owner=fractal_store::store::tests TODO=trace-statement-and-value-representation */ "ROLLBACK").unwrap();
    store
        .connection
        .batched(|transaction| {
            transaction.execute(
                /* sqlite-large-value: allow-for-now lv-hcd414f6eb87d owner=fractal_store::store::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta(key, value) VALUES ('after', 'written')",
                [],
            )
        })
        .unwrap()
        .unwrap();
}

/// A panic while the writer is held leaves no half-written transaction behind, so it must not
/// turn every later write into a panic while the daemon keeps running.
#[test]
fn a_panic_while_the_writer_is_held_does_not_disable_the_store() {
    let store = Arc::new(Store::open_memory("node").unwrap());
    let panicking = store.clone();
    let panicked = std::thread::spawn(move || {
        let _writer = panicking.connection.write();
        panic!("a panic while the writer is held");
    })
    .join();
    assert!(panicked.is_err());
    store
        .append_claim(&ClaimInput {
            subject: "daemon/node".into(),
            kind: "daemon.diagnostic".into(),
            actor: None,
            fields: BTreeMap::from([
                ("severity".into(), Value::String("error".into())),
                ("code".into(), Value::String("after-a-panic".into())),
                (
                    "reason".into(),
                    Value::String("the store still writes".into()),
                ),
            ]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
}

/// A claim this build cannot project, such as a desired body from another build, is
/// quarantined and named. Every other claim still reaches the graph.
#[test]
fn one_claim_that_cannot_be_projected_is_quarantined_while_the_rest_project() {
    let store = Store::open_memory("node").unwrap();
    let source = "version 2\nagent \"bad\" { workspace \"/tmp\"; command \"true\" }\nagent \"good\" { workspace \"/tmp\"; command \"true\" }\n";
    let intent = parse_intent(source, "node").unwrap();
    let preview = store
        .mission(
            &intent,
            IntentInput {
                kdl: source.into(),
                source_name: None,
            },
        )
        .unwrap();
    store
        .apply(&intent, &preview.subject_tokens, "quarantine")
        .unwrap();
    let bad_claim = {
        let connection = store.connection.lock().unwrap();
        let bad_claim: String = connection
            .query_row(
                "SELECT claim_id FROM desired WHERE subject='agent/node.bad'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        connection
            .execute(
                /* sqlite-large-value: allow-for-now lv-h9dc5b78dba6b owner=fractal_store::store::tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET body='{\"from\":\"another build\"}' WHERE id=?1",
                [&bad_claim],
            )
            .unwrap();
        connection.execute(/* sqlite-large-value: allow-for-now lv-h1dd8fac6de93 owner=fractal_store::store::tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM desired", []).unwrap();
        connection
            .execute(
                /* sqlite-large-value: allow-for-now lv-hbff5fa02c357 owner=fractal_store::store::tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE projection_health SET status='stale' WHERE aggregate='graph'",
                [],
            )
            .unwrap();
        bad_claim
    };
    assert!(store.project_replication_backlog().unwrap());
    let desired = store.desired_subjects().unwrap();
    assert!(
        desired
            .iter()
            .any(|subject| subject.subject == "agent/node.good")
    );
    assert!(
        !desired
            .iter()
            .any(|subject| subject.subject == "agent/node.bad")
    );
    let status = store.replication_status(false, None, &[]).unwrap();
    assert_eq!(status.unhealthy_projections, 1);
    assert_eq!(
        status.unhealthy[0].aggregate,
        format!("projection:base:{bad_claim}")
    );
    assert!(
        status.unhealthy[0]
            .error_message
            .as_deref()
            .is_some_and(|message| message.contains("agent/node.bad"))
    );
    // A later full replay decides the claim again instead of keeping a stale quarantine.
    {
        let connection = store.connection.lock().unwrap();
        connection
            .execute(/* sqlite-large-value: allow-for-now lv-h4b37c84f1ea4 owner=fractal_store::store::tests TODO=prepare-body-and-predecessor-values */ "DELETE FROM claims WHERE id=?1", [&bad_claim])
            .unwrap();
        connection
            .execute(
                /* sqlite-large-value: allow-for-now lv-h2eccd4ddae86 owner=fractal_store::store::tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE projection_health SET status='stale' WHERE aggregate='graph'",
                [],
            )
            .unwrap();
    }
    assert!(store.project_replication_backlog().unwrap());
    assert_eq!(
        store
            .replication_status(false, None, &[])
            .unwrap()
            .unhealthy_projections,
        0
    );
}

/// One repair that cannot be applied never stops the others, replication, or startup.
#[test]
fn one_repair_that_cannot_be_applied_is_named_and_the_rest_apply() {
    let store = Store::open_memory("node").unwrap();
    let source = "version 2\nagent \"one\" { workspace \"/tmp\"; command \"true\" }\nagent \"two\" { workspace \"/tmp\"; command \"true\" }\n";
    let intent = parse_intent(source, "node").unwrap();
    let preview = store
        .mission(
            &intent,
            IntentInput {
                kdl: source.into(),
                source_name: None,
            },
        )
        .unwrap();
    store
        .apply(&intent, &preview.subject_tokens, "repairs")
        .unwrap();
    let broken = {
        let mut connection = store.connection.lock().unwrap();
        let transaction = connection.transaction().unwrap();
        let mut repairs = Vec::new();
        for (position, agent) in ["one", "two"].into_iter().enumerate() {
            let subject = format!("agent/node.{agent}");
            let repaired: String = transaction
                .query_row(
                    "SELECT claim_id FROM desired WHERE subject=?1",
                    [&subject],
                    |row| row.get(0),
                )
                .unwrap();
            let body: String = transaction
                .query_row("SELECT body FROM claims WHERE id=?1", [&repaired], |row| {
                    row.get(0)
                })
                .unwrap();
            let replacement = append_claim_tx(
                &transaction,
                &store.origin,
                &subject,
                "intent.desired",
                None,
                &serde_json::from_str::<Value>(&body).unwrap(),
                &[],
                None,
            )
            .unwrap()
            .id;
            transaction
                .execute(
                    /* sqlite-large-value: allow-for-now lv-h49d8c4c59659 owner=fractal_store::store::tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO replica_records(record_ref, writer, sequence, envelope_hash,
                            position, raw, state, claim_id, updated_at_unix_ms)
                         VALUES (?1, 'peer', 1, 'envelope', ?2, x'00', 'valid', ?3, '0')",
                    params![format!("record/{agent}"), position as i64, repaired],
                )
                .unwrap();
            repairs.push((
                replacement.clone(),
                append_claim_tx(
                    &transaction,
                    &store.origin,
                    &format!("repair/{agent}"),
                    "record.repaired",
                    None,
                    &json!({"fields": {
                        "record": format!("record/{agent}"),
                        "replacement": replacement,
                        "reason": "a test repair"
                    }}),
                    &[],
                    None,
                )
                .unwrap()
                .id,
            ));
        }
        // The first replacement names a desired body that this build cannot read.
        transaction
            .execute(
                /* sqlite-large-value: allow-for-now lv-h07f33c1fd3d6 owner=fractal_store::store::tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET body='{\"from\":\"another build\"}' WHERE id=?1",
                [&repairs[0].0],
            )
            .unwrap();
        transaction.commit().unwrap();
        repairs[0].1.clone()
    };
    store.apply_replication_repairs().unwrap();
    let status = store.replication_status(false, None, &[]).unwrap();
    assert_eq!(
        status
            .unhealthy
            .iter()
            .map(|projection| projection.aggregate.clone())
            .collect::<Vec<_>>(),
        [format!("repair:{broken}")]
    );
    assert_eq!(
        status.repaired_records, 1,
        "the second repair still applied"
    );
}

#[test]
fn structural_replication_advances_from_healthy_frontier() {
    let store = Store::open_memory("node").unwrap();
    assert!(store.project_replication_backlog().unwrap());
    {
        let mut connection = store.connection.lock().unwrap();
        let transaction = connection.transaction().unwrap();
        for id in 0..20 {
            append_claim_tx(
                &transaction,
                &store.origin,
                "agent/node.test",
                "harness.observed",
                Some("agent/node.test"),
                &json!({"fields": {"state": "ready"}, "_operation": {
                    "id": format!("op/structural-test/{id}"),
                    "request_digest": format!("digest-{id}")
                }}),
                &[],
                None,
            )
            .unwrap();
        }
        transaction.commit().unwrap();
    }
    assert!(store.project_replication_backlog().unwrap());
    store
        .connection
        .lock()
        .unwrap()
        .execute(
            /* sqlite-large-value: allow-for-now lv-hb85fcc64bc20 owner=fractal_store::store::tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET accepted_at_unix_ms='1' WHERE kind='harness.observed'",
            [],
        )
        .unwrap();
    {
        let mut connection = store.connection.lock().unwrap();
        let transaction = connection.transaction().unwrap();
        transaction
            .execute(
                /* sqlite-large-value: allow-for-now lv-h2464b2b78923 owner=fractal_store::store::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO blobs(hash, bytes, size) VALUES ('sha256:abc', x'01', 1)",
                [],
            )
            .unwrap();
        append_claim_tx(
            &transaction,
            &store.origin,
            "doc/test/incremental",
            "doc.bound",
            Some("agent/node.test"),
            &json!({"name": "doc/test/incremental", "hash": "sha256:abc"}),
            &[],
            None,
        )
        .unwrap();
        transaction.commit().unwrap();
    }
    {
        let mut connection = store.connection.lock().unwrap();
        let transaction = connection.transaction().unwrap();
        assert!(try_project_all_simple_replication_tx(&transaction, &store.origin).unwrap());
        transaction.rollback().unwrap();
    }
    let before = store.connection.lock().unwrap().total_changes();
    assert!(store.project_replication_backlog().unwrap());
    let connection = store.connection.lock().unwrap();
    let changes = connection.total_changes() - before;
    assert!(
        changes < 10,
        "incremental projection changed {changes} rows"
    );
    let hash: String = connection
        .query_row(
            "SELECT hash FROM documents WHERE name='doc/test/incremental'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(hash, "sha256:abc");
}

#[test]
fn one_divergent_envelope_exchanges_only_its_inventory_range() {
    let observe = |store: &Store, subject: &str, number: usize| {
        store
            .append_claim(&ClaimInput {
                subject: subject.into(),
                kind: "transport.observed".into(),
                actor: None,
                fields: BTreeMap::from([
                    ("status".into(), Value::String("up".into())),
                    ("reason".into(), Value::String(format!("sample {number}"))),
                ]),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
    };
    let remote = Store::open_memory("remote").unwrap();
    for number in 0..1_100 {
        observe(&remote, "host/remote", number);
    }
    let local = Store::open_memory("local").unwrap();
    for after in [0, 512, 1_024] {
        local
            .import_replication("remote", &remote.export_replication(after).unwrap())
            .unwrap();
    }
    observe(&remote, "host/remote", 1_100);

    // The local peer's summary starts the exchange. The remote lists one range, not the
    // 1,100 identities both peers already share.
    let summary = local.export_replication_summary(TEST_FLEET).unwrap();
    let response = remote
        .export_replication_exchange(TEST_FLEET, &summary.inventory)
        .unwrap();
    assert!(!response.inventory.envelopes.is_empty());
    assert!(response.inventory.envelopes.len() <= REPLICATION_BUCKET_WIDTH as usize);
    assert!(serde_json::to_vec(&response).unwrap().len() < 65_536);
    local
        .receive_replication_exchange("remote", TEST_FLEET, &response)
        .unwrap();
    let push = local
        .export_replication_exchange(TEST_FLEET, &response.inventory)
        .unwrap();
    assert!(push.inventory.envelopes.len() <= REPLICATION_BUCKET_WIDTH as usize);
    remote
        .receive_replication_exchange("local", TEST_FLEET, &push)
        .unwrap();
    let pull = remote
        .export_replication_exchange(TEST_FLEET, &push.inventory)
        .unwrap();
    assert_eq!(pull.envelopes.len(), 1);
    local
        .receive_replication_exchange("remote", TEST_FLEET, &pull)
        .unwrap();
    assert_eq!(
        local
            .export_replication_summary(TEST_FLEET)
            .unwrap()
            .inventory
            .digest,
        remote
            .export_replication_summary(TEST_FLEET)
            .unwrap()
            .inventory
            .digest
    );

    // An explicit full-inventory request still gets every identity.
    let explicit = local
        .export_replication_exchange(TEST_FLEET, &ReplicationInventory::default())
        .unwrap();
    assert!(explicit.inventory.envelopes.len() > 1_100);
}

#[test]
fn replay_progress_reports_work_without_advancing_an_uncommitted_frontier() {
    let store = Store::open_memory("node").unwrap();
    store.project_replication_backlog().unwrap();
    {
        let mut connection = store.connection.lock().unwrap();
        let transaction = connection.transaction().unwrap();
        for number in 0..2501 {
            append_claim_record_tx(
                &transaction,
                "node",
                "agent/node.test",
                "harness.observed",
                None,
                &json!({"fields":{"state":"ready", "sequence":number}}),
                &[],
                None,
            )
            .unwrap();
        }
        transaction
            .execute(
                /* sqlite-large-value: allow-for-now lv-h802c6caf7c77 owner=fractal_store::store::tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE projection_health SET status='stale' WHERE aggregate='graph'",
                [],
            )
            .unwrap();
        transaction.commit().unwrap();
    }
    let target = store.index().unwrap();
    let mut events = Vec::new();
    assert!(
        store
            .project_replication_backlog_with_progress("test", |event| events.push(event))
            .unwrap()
    );
    let claims = events
        .iter()
        .filter(|event| event.phase == "full-replay/base-claims")
        .collect::<Vec<_>>();
    assert_eq!(
        claims
            .iter()
            .map(|event| event.processed.unwrap())
            .collect::<Vec<_>>(),
        [0]
    );
    assert!(claims.iter().all(|event| event.frontier == 0
        && event.target == target
        && event.total == Some(0)));
    let committed = events.last().unwrap();
    assert_eq!(committed.phase, "projection-committed");
    assert_eq!(committed.frontier, target);
    let digest = store
        .replication_status(true, None, &[])
        .unwrap()
        .graph_digest;
    store.replay_replication_graph().unwrap();
    assert_eq!(
        store
            .replication_status(true, None, &[])
            .unwrap()
            .graph_digest,
        digest
    );
}

#[test]
fn base_replay_batches_selected_bodies_and_preserves_event_and_quarantine_answers() {
    let store = Store::open_memory("node").unwrap();
    let mut connection = store.connection.write();
    let tx = connection.transaction().unwrap();
    for number in 0..263 {
        let subject = format!("exec/base-{number}");
        append_claim_record_tx(
            &tx,
            "node",
            &subject,
            "intent.desired",
            None,
            &json!({"subject":subject,"kind":"exec","desired":{"number":number}}),
            &[],
            None,
        )
        .unwrap();
    }
    let malformed = append_claim_record_tx(
        &tx,
        "node",
        "exec/bad",
        "intent.desired",
        None,
        &json!({}),
        &[],
        None,
    )
    .unwrap();
    let repaired = append_claim_record_tx(
        &tx,
        "node",
        "exec/repaired",
        "intent.desired",
        None,
        &json!({"subject":"exec/repaired","kind":"exec","desired":{}}),
        &[],
        None,
    )
    .unwrap();
    tx.execute(/* sqlite-large-value: allow-for-now lv-h52765d8ed64f owner=fractal_store::store::tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO replica_records(record_ref,writer,sequence,envelope_hash,position,raw,state,claim_id,updated_at_unix_ms)
            VALUES ('record/repaired','node',1,'test',0,X'','repaired',?1,'1')", [&repaired.id]).unwrap();
    for number in 0..1500 {
        append_claim_record_tx(
            &tx,
            "node",
            "agent/events",
            "harness.observed",
            None,
            &json!({"fields":{"number":number,"padding":"x".repeat(1024)}}),
            &[],
            None,
        )
        .unwrap();
    }
    for subject in ["glass/hidden", "Glass/visible", "glass-visible"] {
        append_claim_record_tx(
            &tx,
            "node",
            subject,
            "harness.observed",
            None,
            &json!({"fields":{}}),
            &[],
            None,
        )
        .unwrap();
    }
    // Numeric time and batch/position ties cross more than one decode page.
    tx.execute(/* sqlite-large-value: allow-for-now lv-hbc878ec34d4d owner=fractal_store::store::tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET accepted_at_unix_ms='1'", [])
        .unwrap();
    let expected = tx.prepare("SELECT claims.store_index, claims.subject FROM claims JOIN batches ON batches.id=claims.batch_id
            WHERE substr(claims.subject,1,6)<>'glass/' COLLATE BINARY
            AND claims.id<>?1 ORDER BY claims.store_index").unwrap()
        .query_map([&repaired.id], |row| Ok((row.get::<_,u64>(0)?,row.get::<_,String>(1)?)))
        .unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
    let mut progress = Vec::new();
    project_replicated_base_claims_with_progress(&tx, &mut |stage| progress.push(stage)).unwrap();
    assert_eq!(
        progress
            .iter()
            .map(|stage| stage.processed.unwrap())
            .collect::<Vec<_>>(),
        [0, 256, 264]
    );
    assert!(progress.iter().all(|stage| stage.total == Some(264)));
    let actual = tx
        .prepare("SELECT store_index,subject FROM event_positions ORDER BY store_index")
        .unwrap()
        .query_map([], |row| {
            Ok((row.get::<_, u64>(0)?, row.get::<_, String>(1)?))
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(actual, expected);
    assert_eq!(
        tx.query_row("SELECT COUNT(*) FROM desired", [], |row| row
            .get::<_, u64>(0))
            .unwrap(),
        263
    );
    let quarantined = tx
        .query_row(
            "SELECT aggregate FROM projection_health WHERE aggregate LIKE 'projection:base:%'",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap();
    assert_eq!(quarantined, format!("projection:base:{}", malformed.id));
    // Repeating the same fold retains the same event positions and quarantine identity.
    project_replicated_base_claims_with_progress(&tx, &mut |_| {}).unwrap();
    assert_eq!(
        tx.query_row("SELECT COUNT(*) FROM event_positions", [], |row| row
            .get::<_, usize>(0))
            .unwrap(),
        expected.len()
    );
    tx.commit().unwrap();
}

#[test]
fn base_replay_body_page_work_does_not_grow_with_selected_order_size() {
    let store = Store::open_memory("node").unwrap();
    let mut connection = store.connection.write();
    let tx = connection.transaction().unwrap();
    tx.execute_batch(
        /* sqlite-large-value: allow-for-now lv-hdf54adb9112e owner=fractal_store::store::tests TODO=chunk-or-reference-variable-columns */ "CREATE TEMP TABLE heal_replay_base_order(
            ordinal INTEGER PRIMARY KEY, claim_id TEXT NOT NULL)",
    )
    .unwrap();
    let mut costs = Vec::new();
    for (start, end) in [(0, 512), (512, 4096)] {
        for number in start..end {
            let subject = format!("exec/page-{number}");
            let claim = append_claim_record_tx(
                &tx,
                "node",
                &subject,
                "intent.desired",
                None,
                &json!({"subject":subject,"kind":"exec","desired":{}}),
                &[],
                None,
            )
            .unwrap();
            tx.execute(
                /* sqlite-large-value: allow-for-now lv-h499587b8ff13 owner=fractal_store::store::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO temp.heal_replay_base_order(claim_id) VALUES (?1)",
                [&claim.id],
            )
            .unwrap();
        }
        let mut statement = tx.prepare(/* sqlite-large-value: allow-for-now lv-h6ab472a6b75d owner=fractal_store::store::tests TODO=trace-statement-and-value-representation */ BASE_REPLAY_PAGE_SQL).unwrap();
        let page = statement
            .query_map(params![0, 256], claim_from_row)
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(page.len(), 256);
        assert_eq!(page[0].subject, "exec/page-0");
        assert_eq!(page[255].subject, "exec/page-255");
        costs.push((
            statement.get_status(rusqlite::StatementStatus::VmStep),
            statement.get_status(rusqlite::StatementStatus::FullscanStep),
            statement.get_status(rusqlite::StatementStatus::Sort),
        ));
    }
    assert_eq!(
        costs[0], costs[1],
        "body page work must depend on its selected page, not the remaining canonical order"
    );
    assert_eq!(costs[0].1, 0, "ordinal lookup must not scan the order");
    assert_eq!(
        costs[0].2, 0,
        "body pages must not repeat the canonical sort"
    );
    // This measures the actual body-page statement, not the once-per-replay canonical
    // sort, the full runtime fold or writer latency.
    tx.rollback().unwrap();
}

#[test]
fn full_replay_logs_every_guard_and_incremental_error_once() {
    for reason in [
        "missing-health",
        "unhealthy-projection",
        "frontier-ahead",
        "non-incremental-kind",
        "malformed-operation",
        "operation-conflict",
        "incremental-error:internal",
    ] {
        FULL_REPLAYS.with(|count| count.set(0));
        let store = Store::open_memory("node").unwrap();
        store
            .project_replication_backlog_with_log("test/setup", |_| {})
            .unwrap();
        let frontier = store.index().unwrap();
        {
            let mut connection = store.connection.lock().unwrap();
            let transaction = connection.transaction().unwrap();
            let operation = json!({"id":"op/replay-test", "request_digest":"digest-a"});
            let (kind, body) = match reason {
                "non-incremental-kind" => ("work.unknown", json!({"fields":{}})),
                "malformed-operation" => (
                    "harness.observed",
                    json!({"fields":{"state":"ready"}, "_operation":{"id":"op/malformed"}}),
                ),
                "operation-conflict" => (
                    "harness.observed",
                    json!({"fields":{"state":"ready"}, "_operation":operation}),
                ),
                "incremental-error:internal" => ("intent.desired", json!({})),
                _ => ("harness.observed", json!({"fields":{"state":"ready"}})),
            };
            // Seed historical input without pre-projecting it or requiring a current schema.
            let claim = append_claim_record_tx(
                &transaction,
                "node",
                "agent/node.test",
                kind,
                None,
                &body,
                &[],
                None,
            )
            .unwrap();
            if reason == "operation-conflict" {
                transaction.execute(/* sqlite-large-value: allow-for-now lv-h5c80b3d61773 owner=fractal_store::store::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO operations(id, request_digest, canonical_claim_id, state) VALUES ('op/replay-test', 'digest-b', ?1, 'active')", [&claim.id]).unwrap();
            }
            match reason {
                "missing-health" => {
                    transaction
                        .execute(/* sqlite-large-value: allow-for-now lv-h789522dbacf2 owner=fractal_store::store::tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM projection_health WHERE aggregate='graph'", [])
                        .unwrap();
                }
                "unhealthy-projection" => {
                    transaction
                        .execute(
                            /* sqlite-large-value: allow-for-now lv-h74360fa29d0f owner=fractal_store::store::tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE projection_health SET status='stale' WHERE aggregate='graph'",
                            [],
                        )
                        .unwrap();
                }
                "frontier-ahead" => {
                    transaction.execute(/* sqlite-large-value: allow-for-now lv-h8beb67dd4555 owner=fractal_store::store::tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE projection_health SET last_good_store_index=999999 WHERE aggregate='graph'", []).unwrap();
                }
                _ => {}
            }
            transaction.commit().unwrap();
        }
        let target = store.index().unwrap();
        let mut lines = Vec::new();
        assert!(
            store
                .project_replication_backlog_with_log(
                    "startup/project-replication-backlog",
                    |line| {
                        // The log sink runs before any full-replay tables are changed.
                        assert_eq!(store.index().unwrap(), target);
                        lines.push(line.to_owned());
                    }
                )
                .unwrap(),
            "{reason}"
        );
        let expected_frontier = match reason {
            "missing-health" => 0,
            "frontier-ahead" => 999999,
            _ => frontier,
        };
        assert_eq!(
            lines
                .iter()
                .filter(|line| line.starts_with("st: projection full replay "))
                .cloned()
                .collect::<Vec<_>>(),
            [format!(
                "st: projection full replay phase=startup/project-replication-backlog reason={reason} frontier={expected_frontier} target={target}"
            )],
            "{reason}"
        );
        let details = lines
            .iter()
            .filter_map(|line| line.strip_prefix("st: projection failure detail "))
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        if matches!(reason, "incremental-error:internal" | "operation-conflict"
            | "malformed-operation" | "non-incremental-kind") {
            assert_eq!(details.len(), 1, "{reason}");
            assert_eq!(details[0]["subject"], "agent/node.test");
            assert!(
                details[0]["claim_id"]
                    .as_str()
                    .is_some_and(|id| id.len() == 64)
            );
            assert_eq!(details[0]["error_truncated"], false);
            if reason == "operation-conflict" {
                assert_eq!(details[0]["operation_id"], "op/replay-test");
                assert!(details[0]["error"].as_str().unwrap().contains("digest_matches=false"));
            } else if reason == "incremental-error:internal" {
                assert!(
                    details[0]["error"]
                        .as_str()
                        .unwrap()
                        .contains("missing field")
                );
            } else {
                assert_eq!(details[0]["stage"], "incremental-guard");
                assert_eq!(details[0]["code"], reason);
                let expected_kind = match reason {
                    "malformed-operation" => "harness.observed",
                    _ => "work.unknown",
                };
                assert_eq!(details[0]["claim_kind"], expected_kind);
                let detail_position = lines.iter().position(|line|
                    line.starts_with("st: projection failure detail ")).unwrap();
                let replay_position = lines.iter().position(|line|
                    line.starts_with("st: projection full replay ")).unwrap();
                assert!(detail_position < replay_position);
            }
        } else {
            assert!(details.is_empty(), "{reason}");
        }
        assert_eq!(
            FULL_REPLAYS.with(std::cell::Cell::get),
            2,
            "one setup replay and one fallback: {reason}"
        );
        let digest = store
            .replication_status(true, None, &[])
            .unwrap()
            .graph_digest;
        store.replay_replication_graph().unwrap();
        assert_eq!(
            store
                .replication_status(true, None, &[])
                .unwrap()
                .graph_digest,
            digest,
            "{reason}"
        );
        lines.clear();
        assert!(
            store
                .project_replication_backlog_with_log("replication", |line| lines
                    .push(line.to_owned()))
                .unwrap()
        );
        assert!(
            lines.is_empty(),
            "a healthy frontier must not log a fallback"
        );
    }
}

#[test]
fn simple_replication_rejects_structural_and_malformed_operation_claims() {
    assert!(Store::simple_replication_kind("harness.observed"));
    assert!(Store::simple_replication_kind("work.renewed"));
    assert!(Store::simple_replication_kind("work.claimed"));
    assert!(Store::simple_replication_kind("work.progress"));
    assert!(Store::simple_replication_kind(WORK_NUDGED_KIND));
    assert!(!Store::simple_replication_kind("work.unknown"));
    assert!(!Store::simple_replication_kind("mission.published"));
    let store = Store::open_memory("node").unwrap();
    assert!(store.project_replication_backlog().unwrap());
    let mut connection = store.connection.lock().unwrap();
    let transaction = connection.transaction().unwrap();
    append_claim_tx(
        &transaction,
        &store.origin,
        "agent/node.test",
        "harness.observed",
        Some("agent/node.test"),
        &json!({"fields": {"state": "ready"}, "_operation": {"id": "test"}}),
        &[],
        None,
    )
    .unwrap();
    assert!(!try_project_all_simple_replication_tx(&transaction, &store.origin).unwrap());
}

#[test]
fn simple_replication_registers_unambiguous_event_operations() {
    let store = Store::open_memory("node").unwrap();
    assert!(store.project_replication_backlog().unwrap());
    {
        let mut connection = store.connection.lock().unwrap();
        let transaction = connection.transaction().unwrap();
        append_claim_tx(
            &transaction,
            &store.origin,
            "agent/node.test",
            "harness.observed",
            Some("agent/node.test"),
            &json!({"fields": {"state": "ready"}, "_operation": {
                "id": "op/simple-event", "request_digest": "digest-one"
            }}),
            &[],
            None,
        )
        .unwrap();
        assert!(try_project_all_simple_replication_tx(&transaction, &store.origin).unwrap());
        transaction.commit().unwrap();
    }
    assert!(store.operation_projection_drift().unwrap().is_empty());
    assert!(store.project_replication_backlog().unwrap());
    {
        let mut connection = store.connection.lock().unwrap();
        let transaction = connection.transaction().unwrap();
        append_claim_tx(
            &transaction,
            &store.origin,
            "agent/node.test",
            "harness.observed",
            Some("agent/node.test"),
            &json!({"fields": {"state": "idle"}, "_operation": {
                "id": "op/simple-event", "request_digest": "digest-two"
            }}),
            &[],
            None,
        )
        .unwrap();
        assert!(!try_project_all_simple_replication_tx(&transaction, &store.origin).unwrap());
        transaction.commit().unwrap();
    }
    assert!(store.project_replication_backlog().unwrap());
    assert!(store.operation_projection_drift().unwrap().is_empty());
    let connection = store.connection.lock().unwrap();
    assert_eq!(
        operation_tx(&connection, "op/simple-event")
            .unwrap()
            .unwrap()
            .2,
        "conflict"
    );
}

pub(super) fn rewrite_envelope(
    envelope: &ReplicaEnvelope,
    update: impl FnOnce(&mut ReplicaEnvelopePayload),
) -> ReplicaEnvelope {
    let bytes = envelope.payload.bytes().expect("envelope base64");
    let mut payload: ReplicaEnvelopePayload =
        ciborium::from_reader(bytes).expect("envelope CBOR");
    update(&mut payload);
    let mut bytes = Vec::new();
    ciborium::into_writer(&payload, &mut bytes).expect("updated envelope CBOR");
    ReplicaEnvelope {
        hash: replica_envelope_hash(
            &envelope.writer,
            envelope.sequence,
            envelope.previous_hash.as_deref(),
            envelope.accepted_at_unix_ms,
            &bytes,
        ),
        payload: bytes.into(),
        ..envelope.clone()
    }
}

#[test]
fn desired_revision_does_not_depend_on_json_object_insertion_order() {
    let mut left_value = serde_json::Map::new();
    left_value.insert("workspace".into(), Value::String("/workspace".into()));
    left_value.insert("restart".into(), Value::String("always".into()));
    let mut right_value = serde_json::Map::new();
    right_value.insert("restart".into(), Value::String("always".into()));
    right_value.insert("workspace".into(), Value::String("/workspace".into()));
    let left = DesiredSubject {
        accountable: None,
        subject: "agent/example".into(),
        kind: "agent".into(),
        desired: Value::Object(left_value),
        member: None,
        owner_run: Some("mission-run/example".into()),
        owner_generation: Some("run-generation/example".into()),
        owner_step: None,
    };
    let right = DesiredSubject {
        accountable: None,
        desired: Value::Object(right_value),
        ..left.clone()
    };

    assert_eq!(desired_revision(&left), desired_revision(&right));
}

#[test]
fn state_transition_claims_replace_their_previous_fields() {
    let store = Store::open_memory("node").unwrap();
    store
        .append_claim(&ClaimInput {
            subject: "observer/recovery".into(),
            kind: "observer.state".into(),
            actor: None,
            fields: BTreeMap::from([
                ("state".into(), Value::String("unreachable".into())),
                ("reason".into(), Value::String("the provider failed".into())),
            ]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
    store
        .append_claim(&ClaimInput {
            subject: "observer/recovery".into(),
            kind: "observer.state".into(),
            actor: None,
            fields: BTreeMap::from([("state".into(), Value::String("healthy".into()))]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();

    let actual = store
        .latest_actual_value("observer/recovery")
        .unwrap()
        .unwrap();
    assert_eq!(actual["state"], "healthy");
    assert!(actual.get("reason").is_none());
}

#[test]
fn common_reads_do_not_wait_for_the_writer_mutex() {
    let store = Arc::new(Store::open_memory("node").unwrap());
    let writer = store.connection.lock().unwrap();
    let other = store.clone();
    let (send, receive) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        send.send((other.index().unwrap(), other.status(None).unwrap()))
            .unwrap();
    });

    let (index, status) = receive
        .recv_timeout(std::time::Duration::from_secs(1))
        .expect("a common read must not wait for the writer mutex");
    assert_eq!(index, 0);
    assert_eq!(status.store_index, 0);
    drop(writer);
}

#[test]
fn an_exited_gate_runtime_leaves_the_current_runtime_list() {
    let store = Store::open_memory("node").unwrap();
    let subject = "gate-operation/step-run.run.check/0123456789abcdef01234567";
    for status in ["running", "exited"] {
        store
            .append_claim(&ClaimInput {
                subject: subject.into(),
                kind: "runtime.observed".into(),
                actor: None,
                fields: BTreeMap::from([
                    ("status".into(), Value::String(status.into())),
                    (
                        "runtime_id".into(),
                        Value::String(subject.replace('/', ".")),
                    ),
                ]),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
    }
    let current = store
        .status_for_claim_kind_at(
            "runtime.observed",
            None,
            false,
            ClaimsReduction::Summary,
        )
        .unwrap();
    assert!(
        current.subjects.iter().all(|item| item.subject != subject),
        "{:?}",
        current.subjects
    );
    let history = store
        .status_for_claim_kind_at("runtime.observed", None, true, ClaimsReduction::Summary)
        .unwrap();
    let gate = history
        .subjects
        .iter()
        .find(|item| item.subject == subject)
        .unwrap();
    assert_eq!(gate.projection.layer, "history");
    assert_eq!(gate.projection.reasons, ["stopped"]);
}

fn current_runtimes(store: &Store) -> (Vec<String>, usize) {
    SUBJECT_REDUCTIONS.with(|reductions| reductions.set(0));
    let view = store
        .status_for_claim_kind_at(
            "runtime.observed",
            None,
            false,
            ClaimsReduction::Summary,
        )
        .unwrap();
    (
        view.subjects
            .into_iter()
            .map(|status| status.subject)
            .collect(),
        SUBJECT_REDUCTIONS.with(std::cell::Cell::get),
    )
}

#[test]
fn agent_card_status_cache_reduces_only_changed_subjects_and_keeps_old_cuts() {
    let store = Store::open_memory("node").unwrap();
    observe_runtime(&store, "agent/cache/first", "running", "one");
    observe_runtime(&store, "agent/cache/second", "running", "two");
    let first_cut = store.index().unwrap();
    let view = |at| serde_json::to_value(store.agent_card_status_at(None, at, true).unwrap()).unwrap();
    let original = view(first_cut);
    SUBJECT_REDUCTIONS.with(|count| count.set(0));
    assert_eq!(view(first_cut), original);
    assert_eq!(SUBJECT_REDUCTIONS.with(std::cell::Cell::get), 0);
    observe_runtime(&store, "exec/unrelated", "running", "other");
    SUBJECT_REDUCTIONS.with(|count| count.set(0));
    let newer = view(store.index().unwrap());
    assert_eq!(newer["subjects"], original["subjects"]);
    assert_eq!(SUBJECT_REDUCTIONS.with(std::cell::Cell::get), 0);
    observe_runtime(&store, "agent/cache/first", "stopped", "three");
    SUBJECT_REDUCTIONS.with(|count| count.set(0));
    let changed = view(store.index().unwrap());
    assert_eq!(SUBJECT_REDUCTIONS.with(std::cell::Cell::get), 1);
    assert_eq!(view(first_cut), original, "a newer entry must not serve an old cut");
    store.forget_current_views();
    assert_eq!(view(store.index().unwrap()), changed, "cache and fresh reducer must agree");
}

#[test]
fn agent_card_status_cache_has_a_fixed_entry_cap_and_repair_clears_it() {
    let store = Store::open_memory("node").unwrap();
    observe_runtime(&store, "agent/cache/cold", "running", "one");
    let cut = store.index().unwrap();
    let first = store.agent_card_status_at(None, cut, true).unwrap();
    {
        let mut cache = store.smalltalk.subject_cache.lock().unwrap();
        cache.card_statuses.clear();
        for n in 0..AGENT_CARD_STATUS_LIMIT {
            cache.card_statuses.insert(format!("agent/fill/{n}"), StatusEntry {
                read_at: cut, owners: Vec::new(), claims: ClaimsReduction::AgentCard,
                status: first.subjects[0].clone(), action: None,
            });
        }
    }
    let cold = store.agent_card_status_at(None, cut, true).unwrap();
    assert_eq!(serde_json::to_value(cold).unwrap(), serde_json::to_value(first).unwrap());
    let cache = store.smalltalk.subject_cache.lock().unwrap();
    assert_eq!(cache.card_statuses.len(), AGENT_CARD_STATUS_LIMIT);
    assert!(!cache.card_statuses.contains_key("agent/cache/cold"));
    drop(cache);
    store.forget_current_views();
    assert!(store.smalltalk.subject_cache.lock().unwrap().card_statuses.is_empty());
    store.agent_card_status_at(None, cut, true).unwrap();
    assert_eq!(store.smalltalk.subject_cache.lock().unwrap().card_statuses.len(), 1);
}

#[test]
fn status_projection_threads_inherit_cancellation_and_release_their_readers() {
    let store = Store::open_memory("node").unwrap();
    let names = (0..65).map(|n| format!("agent/budget/seat-{n}")).collect::<BTreeSet<_>>();
    let index = store.index().unwrap();
    let expired = smallclaims::read_budget::ReadBudget::new("/parallel-status", std::time::Duration::ZERO);
    smallclaims::read_budget::with(Some(expired), || {
        let error = store.status_for_subject_names_at(names.clone(), index, true, ClaimsReduction::Full).unwrap_err();
        assert_eq!(smallclaims::error::typed(error).code, "read-deadline");
    });
    let eligible = smallclaims::read_budget::ReadBudget::new("/parallel-status", std::time::Duration::from_secs(15));
    let result = smallclaims::read_budget::with(Some(eligible.clone()), || {
        store.status_for_subject_names_at(names, index, true, ClaimsReduction::Full)
    }).unwrap();
    assert_eq!(result.subjects.len(), 65);
    assert!(!eligible.expired());
    assert!(store.readers.get().is_autocommit());
}

#[test]
fn bound_one_nested_large_status_reuses_its_snapshot_reader() {
    let store = Arc::new(smallclaims::sqlite::with_read_limit_for_test(
        1, || Store::open_memory("node").unwrap(),
    ));
    let names = (0..80).map(|n| format!("agent/admitted/seat-{n:03}"))
        .collect::<BTreeSet<_>>();
    for name in &names {
        observe_runtime(&store, name, "running", "incarnation");
    }
    let budget = smallclaims::read_budget::ReadBudget::new(
        "/nested-status", std::time::Duration::from_secs(15),
    );
    let worker_budget = budget.clone();
    let worker_store = store.clone();
    let expected = names.iter().cloned().collect::<Vec<_>>();
    let (done, finished) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let result = smallclaims::read_budget::with(Some(worker_budget), || {
            worker_store.readers.request_read(|| {
                worker_store.read_snapshot(|cut| {
                    worker_store.readers.request_read(|| {
                        worker_store.status_for_subject_names_at(names, cut, true, ClaimsReduction::Full)
                    })?
                })
            })?
        });
        done.send(result).unwrap();
    });
    let completed = finished.recv_timeout(std::time::Duration::from_secs(5));
    if completed.is_err() {
        budget.cancel();
    }
    let result = completed.expect("bound-one nested status fanout deadlocked").unwrap();
    worker.join().unwrap();
    assert_eq!(result.subjects.iter().map(|subject| subject.subject.clone())
        .collect::<Vec<_>>(), expected);
    assert_eq!(store.readers.usage().peak, 1);
    assert_eq!(store.readers.usage().idle, 1);
    assert!(!budget.expired());
}

#[test]
fn admitted_status_fanout_and_inline_reduction_have_identical_values_and_digests() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("status.sqlite3");
    let inline = smallclaims::sqlite::with_read_limit_for_test(
        1, || Store::open(&path, "node").unwrap(),
    );
    let names = (0..80).map(|n| format!("agent/admitted/seat-{n:03}"))
        .collect::<BTreeSet<_>>();
    for name in &names {
        observe_runtime(&inline, name, "stopped", "old");
        observe_runtime(&inline, name, "running", "current");
    }
    // Two fresh pools over the same real claims keep provenance and accepted times
    // identical, making the entire serialized result (not a normalized subset) comparable.
    let parallel = smallclaims::sqlite::with_read_limit_for_test(
        32, || Store::open(&path, "node").unwrap(),
    );
    let cut = inline.index().unwrap();
    assert_eq!(parallel.index().unwrap(), cut);
    for history in [false, true] {
        let values = [&inline, &parallel].map(|store| {
            let budget = smallclaims::read_budget::ReadBudget::new(
                "/admitted-status", std::time::Duration::from_secs(15),
            );
            let result = smallclaims::read_budget::with(Some(budget), || {
                store.readers.request_read(|| {
                    store.status_for_subject_names_at(names.clone(), cut, history, ClaimsReduction::Full)
                })
            }).unwrap().unwrap();
            assert_eq!(result.subjects.len(), names.len());
            assert!(result.subjects.windows(2).all(|pair| pair[0].subject < pair[1].subject));
            serde_json::to_value(result).unwrap()
        });
        assert_eq!(values[0], values[1]);
        let digests = values.map(|value| {
            hex::encode(Sha256::digest(serde_json::to_vec(&value).unwrap()))
        });
        assert_eq!(digests[0], digests[1]);
    }
    assert_eq!(inline.readers.usage().peak, 1);
    assert!(parallel.readers.usage().peak > 1);
    assert!(parallel.readers.usage().peak <= 32);
}

#[test]
fn current_runtime_views_reduce_what_they_show_not_every_runtime_ever_held() {
    let store = Store::open_memory("node").unwrap();
    observe_runtime(&store, "agent/bench/live", "running", "live-1");
    let retire = |from: usize, to: usize| {
        for number in from..to {
            let subject = format!("agent/bench/old-{number:03}");
            observe_runtime(&store, &subject, "running", "old");
            observe_runtime(&store, &subject, "stopped", "old");
        }
    };
    retire(0, 40);
    let live = vec!["agent/bench/live".to_owned()];
    assert_eq!(current_runtimes(&store), (live.clone(), 1));
    // Nothing changed, so nothing is reduced again.
    assert_eq!(current_runtimes(&store), (live.clone(), 0));
    // Three times the history leaves the work where it was.
    retire(40, 120);
    assert_eq!(current_runtimes(&store), (live.clone(), 0));
    // A claim about the live runtime reduces it again, and only it.
    observe_runtime(&store, "agent/bench/live", "running", "live-2");
    assert_eq!(current_runtimes(&store), (live.clone(), 1));
    // The view is the current layer of the full reduction.
    let full = store
        .status_for_claim_kind_at("runtime.observed", None, true, ClaimsReduction::Summary)
        .unwrap();
    assert_eq!(full.subjects.len(), 121);
    let current_layer = full
        .subjects
        .into_iter()
        .filter(|status| status.projection.layer == "current")
        .map(|status| status.subject)
        .collect::<Vec<_>>();
    assert_eq!(current_layer, live);
}

#[test]
fn kept_statuses_match_a_fresh_reduction_as_claims_arrive() {
    let store = Store::open_memory("node").unwrap();
    let mut seed = 0x5eed_u64;
    let mut next = || {
        seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (seed >> 33) as usize
    };
    let statuses = ["running", "stopped", "starting", "exited", "idle"];
    let states = ["idle", "working", "ready"];
    for step in 0..240 {
        let seat = next() % 10;
        let subject = format!("agent/bench/seat-{seat}");
        let incarnation = format!("seat-{seat}-{}", next() % 2);
        if next() % 2 == 0 {
            observe_runtime(
                &store,
                &subject,
                statuses[next() % statuses.len()],
                &incarnation,
            );
        } else {
            store
                .append_claim(&ClaimInput {
                    subject: subject.clone(),
                    kind: "harness.observed".into(),
                    actor: Some(subject.clone()),
                    fields: BTreeMap::from([
                        (
                            "state".into(),
                            Value::String(states[next() % states.len()].into()),
                        ),
                        ("incarnation_id".into(), Value::String(incarnation)),
                        ("driver".into(), Value::String("claude".into())),
                    ]),
                    evidence: Vec::new(),
                    expected_subject: None,
                    idempotency_key: None,
                })
                .unwrap();
        }
        if step % 6 != 5 {
            continue;
        }
        for history in [false, true] {
            let view = |store: &Store| {
                serde_json::to_value(
                    store
                        .status_for_claim_kind_at(
                            "runtime.observed",
                            None,
                            history,
                            ClaimsReduction::Summary,
                        )
                        .unwrap(),
                )
                .unwrap()
            };
            let kept = view(&store);
            store.forget_current_views();
            assert_eq!(kept, view(&store), "step {step}, history {history}");
        }
    }
}

#[test]
fn a_kept_status_follows_work_its_agent_does_on_other_subjects() {
    let store = Store::open_memory("node").unwrap();
    let agent = "agent/bench/worker";
    observe_runtime(&store, agent, "running", "worker-1");
    store
        .append_claim(&ClaimInput {
            subject: agent.into(),
            kind: "harness.observed".into(),
            actor: Some(agent.into()),
            fields: BTreeMap::from([
                ("state".into(), Value::String("idle".into())),
                ("incarnation_id".into(), Value::String("worker-1".into())),
            ]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
    let harness_state = |store: &Store| {
        store
            .status_for_claim_kind_at(
                "runtime.observed",
                None,
                false,
                ClaimsReduction::Summary,
            )
            .unwrap()
            .subjects
            .into_iter()
            .find(|status| status.subject == agent)
            .and_then(|status| status.harness)
            .map(|harness| harness.state)
    };
    assert_eq!(harness_state(&store).as_deref(), Some("idle"));
    // Work the agent reports on a step, a claim about another subject, makes it working.
    {
        let mut connection = store.connection.lock().unwrap();
        let transaction = connection.transaction().unwrap();
        append_claim_tx(
            &transaction,
            &store.origin,
            "step-run/bench/work",
            "work.progress",
            Some(agent),
            &json!({"fields": {
                "attempt": 1,
                "status": "working",
                "worker_reported": true,
                "claimant": agent,
                "claim_incarnation": "worker-1",
                "claim_expires_at_unix_ms": now_ms().saturating_add(60_000),
                "readiness_epoch": 1,
            }}),
            &[],
            None,
        )
        .unwrap();
        transaction.commit().unwrap();
    }
    assert_eq!(harness_state(&store).as_deref(), Some("working"));
}

#[test]
fn the_newest_message_to_a_recipient_is_found_by_index() {
    let store = Store::open_memory("node").unwrap();
    let connection = store.readers.get();
    let plan = connection
        .prepare(
            "EXPLAIN QUERY PLAN SELECT accepted_at_unix_ms FROM claims
                 WHERE kind='message.sent' AND json_extract(body, '$.fields.to')=?1
                   AND store_index<=?2 ORDER BY store_index DESC LIMIT 1",
        )
        .unwrap()
        .query_map(params!["agent/bench/reader", 10], |row| {
            row.get::<_, String>(3)
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert!(
        plan.iter()
            .any(|step| step.contains("claims_message_to_order_index")),
        "{plan:?}"
    );
}

/// The replication projection's two frontier checks seek an index instead of reading every
/// claim, and answer as the scans they replace at every snapshot.
#[test]
fn replication_frontier_checks_seek_instead_of_scanning() {
    let store = Store::open_memory("node").unwrap();
    let mut snapshots = vec![0];
    {
        let connection = store.connection.lock().unwrap();
        connection
            .execute_batch("PRAGMA foreign_keys = OFF")
            .unwrap();
        for (subject, batch, accepted) in [
            ("mission-run/one", "batch/left/1/a", "9"),
            ("run-generation/gen", "batch/left/2/a", "10"),
            ("step-run/gen/build", "batch/right/1/a", "10"),
            // Just outside the steps of generation `gen`.
            ("step-run/gen0", "batch/right/2/a", "100"),
            ("step-run/ge/test", "batch/left/3/a", "11"),
        ] {
            connection
                .execute(
                    /* sqlite-large-value: allow-for-now lv-h472ae4c7b78f owner=fractal_store::store::tests TODO=prepare-body-and-predecessor-values */ "INSERT INTO claims(id, batch_id, subject, kind, origin, actor, body,
                                            predecessors, accepted_at_unix_ms)
                         VALUES (?1, ?2, ?3, 'harness.observed', 'node', NULL, '{}', '[]', ?4)",
                    params![Uuid::now_v7().to_string(), batch, subject, accepted],
                )
                .unwrap();
            snapshots.push(
                connection
                    .query_row("SELECT MAX(store_index) FROM claims", [], |row| row.get(0))
                    .unwrap(),
            );
        }
        connection
            .execute_batch("PRAGMA foreign_keys = ON")
            .unwrap();
    }
    let batches_scan = "SELECT batch_id FROM claims
                            WHERE store_index<=?1 AND accepted_at_unix_ms=?2";
    let prior_scan = "SELECT EXISTS(SELECT 1 FROM claims WHERE store_index<=?1
                          AND (subject=?2 OR subject=?3 OR subject LIKE ?4))";
    let exists = |sql: &str, params: &[&dyn rusqlite::ToSql]| -> bool {
        store
            .readers
            .get()
            .query_row(/* sqlite-large-value: allow-for-now lv-hc63c381fa4ab owner=fractal_store::store::tests TODO=trace-statement-and-value-representation */ sql, params, |row| row.get(0))
            .unwrap()
    };
    for &snapshot in &snapshots {
        for accepted in ["1", "9", "10", "11", "100"] {
            let mut sought = listed(&store, LAST_ACCEPTED_BATCHES, params![snapshot, accepted]);
            let mut scanned = listed(&store, batches_scan, params![snapshot, accepted]);
            sought.sort();
            scanned.sort();
            assert_eq!(sought, scanned, "accepted {accepted} at {snapshot}");
        }
        for (run, generation, id) in [
            ("mission-run/one", "run-generation/none", "none"),
            ("mission-run/none", "run-generation/gen", "gen"),
            ("mission-run/none", "run-generation/other", "gen"),
            ("mission-run/none", "run-generation/other", "ge"),
            ("mission-run/none", "run-generation/other", "other"),
        ] {
            let (from, to) = (format!("step-run/{id}/"), format!("step-run/{id}0"));
            let pattern = format!("step-run/{id}/%");
            assert_eq!(
                exists(
                    RUN_HAS_PRIOR_CLAIMS,
                    params![snapshot, run, generation, from, to]
                ),
                exists(prior_scan, params![snapshot, run, generation, pattern]),
                "{run} {generation} {id} at {snapshot}"
            );
        }
    }
    let plan = query_plan(&store, LAST_ACCEPTED_BATCHES, params![10, "10"]);
    assert!(
        plan.iter()
            .any(|step| step.contains("claims_accepted_order_index")),
        "{plan:?}"
    );
    let plan = query_plan(
        &store,
        RUN_HAS_PRIOR_CLAIMS,
        params![
            10,
            "mission-run/one",
            "run-generation/gen",
            "step-run/gen/",
            "step-run/gen0"
        ],
    );
    let reads = plan
        .iter()
        .filter(|step| step.contains("claims"))
        .collect::<Vec<_>>();
    assert_eq!(reads.len(), 3, "{plan:?}");
    assert!(
        reads
            .iter()
            .all(|step| step.contains("INDEX claims_subject_index (subject")),
        "{plan:?}"
    );
}

/// The runtimes' subjects are found one index seek per subject, not by reading every runtime
/// observation, and they are the subjects a scan finds at every snapshot.
#[test]
fn runtime_subjects_are_read_one_seek_per_subject() {
    let store = Store::open_memory("node").unwrap();
    let mut snapshots = Vec::new();
    for round in 0..4 {
        let subjects = (0..=round)
            .map(|number| format!("agent/kind/runtime-{number}"))
            .collect::<Vec<_>>();
        let mut claims = subjects
            .iter()
            .flat_map(|subject| {
                [
                    (subject.as_str(), "runtime.observed"),
                    (subject.as_str(), "harness.observed"),
                ]
            })
            .collect::<Vec<_>>();
        let other = format!("agent/kind/other-{round}");
        claims.push((&other, "harness.observed"));
        snapshots.push(insert_raw_claims(&store, &claims));
    }
    let scan = "SELECT DISTINCT subject FROM claims WHERE kind='runtime.observed'
                    AND store_index<=?1 ORDER BY subject";
    for snapshot in [0, 1, snapshots[0], snapshots[1], snapshots[3]] {
        assert_eq!(
            listed(&store, RUNTIME_SUBJECTS, [snapshot]),
            listed(&store, scan, [snapshot]),
            "at {snapshot}"
        );
    }
    let plan = query_plan(&store, RUNTIME_SUBJECTS, [10]);
    assert!(
        plan.iter()
            .any(|step| step.contains("claims_runtime_subject_index (subject>?)")),
        "{plan:?}"
    );
    assert!(
        plan.iter().all(|step| !step.contains("TEMP B-TREE")),
        "{plan:?}"
    );
}

/// A prefix's subjects are found one index seek per subject, and they are the subjects a
/// GLOB scan finds, including the neighbours just outside the prefix's range.
#[test]
fn a_prefixs_subjects_are_read_one_seek_per_subject() {
    assert_eq!(prefix_upper_bound("agent/").as_deref(), Some("agent0"));
    assert_eq!(prefix_upper_bound("host/").as_deref(), Some("host0"));
    for unbounded in ["", "agent*", "a?", "[a]/", "agent/\u{e9}", "agent\u{7f}"] {
        assert_eq!(prefix_upper_bound(unbounded), None, "{unbounded:?}");
    }
    let store = Store::open_memory("node").unwrap();
    let first = insert_raw_claims(
        &store,
        &[
            ("agen/x", "harness.observed"),
            ("agent", "harness.observed"),
            ("agent/", "harness.observed"),
            ("agent/a", "harness.observed"),
            ("agent/a", "runtime.observed"),
            ("agent/a/b", "harness.observed"),
            ("agent0", "harness.observed"),
            ("agents/x", "harness.observed"),
            ("Agent/x", "harness.observed"),
            ("host/one", "transport.observed"),
        ],
    );
    let last = insert_raw_claims(
        &store,
        &[
            ("agent/a", "harness.observed"),
            ("agent/late", "harness.observed"),
            ("agent/z", "harness.observed"),
            ("host/two", "transport.observed"),
        ],
    );
    let scan = "SELECT DISTINCT subject FROM claims WHERE store_index<=?1 AND subject GLOB ?2
                    ORDER BY subject";
    for snapshot in [0, 1, 4, first, last] {
        for prefix in ["agent/", "host/", "agent/a/"] {
            let bound = prefix_upper_bound(prefix).unwrap();
            assert_eq!(
                listed(&store, RANGE_SUBJECTS, params![snapshot, prefix, bound]),
                listed(&store, scan, params![snapshot, format!("{prefix}*")]),
                "{prefix} at {snapshot}"
            );
        }
    }
    let plan = query_plan(&store, RANGE_SUBJECTS, params![last, "agent/", "agent0"]);
    assert!(
        plan.iter()
            .any(|step| step.contains("claims_subject_index (subject>? AND subject<?)")),
        "{plan:?}"
    );
    assert!(
        plan.iter().all(|step| !step.contains("TEMP B-TREE")),
        "{plan:?}"
    );
}

/// A seat's held and ready steps are read from the fleet's open steps, not from every step the
/// store has run.
#[test]
fn seat_steps_are_read_from_the_open_steps() {
    let store = Store::open_memory("node").unwrap();
    for agent in [None, Some("agent/seat")] {
        let steps = query_plan(&store, SEAT_STEP_ROWS, params![agent, now_ms().to_string()]);
        assert!(
            steps
                .iter()
                .any(|step| step.contains("step_runs_open_index")),
            "{steps:?}"
        );
    }
}

/// Each listed agent's member fault is its newest member-reconcile decision, read per agent.
#[test]
fn member_faults_are_read_for_the_listed_agents() {
    let store = Store::open_memory("node").unwrap();
    let decide = |subject: &str, decision: &str, reason: &str| {
        store
            .append_claim(&ClaimInput {
                subject: subject.into(),
                kind: "runtime.reconcile-decision".into(),
                actor: None,
                fields: BTreeMap::from([
                    ("key".into(), Value::String("member-reconcile".into())),
                    ("decision".into(), Value::String(decision.into())),
                    ("reason".into(), Value::String(reason.into())),
                ]),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap()
    };
    decide("agent/faults/healed", "member-fault", "it failed once");
    let failed = decide("agent/faults/failed", "member-fault", "it cannot start");
    decide("agent/faults/healed", "member-started", "it started");
    decide("agent/faults/failed", "member-started", "it started");
    let subjects = [
        "agent/faults/healed",
        "agent/faults/failed",
        "agent/faults/none",
    ]
    .map(str::to_owned)
    .to_vec();
    let index = store.index().unwrap();
    for at in [failed.store_index, index] {
        let expected = subjects
            .iter()
            .filter_map(|subject| {
                store
                    .member_reconcile_fault(subject, Some(at))
                    .unwrap()
                    .map(|fault| (subject.clone(), fault))
            })
            .collect::<BTreeMap<_, _>>();
        assert_eq!(
            store.member_reconcile_faults_for(&subjects, at).unwrap(),
            expected
        );
    }
    assert_eq!(
        store
            .member_reconcile_faults_for(&subjects, failed.store_index)
            .unwrap()
            .keys()
            .collect::<Vec<_>>(),
        ["agent/faults/failed", "agent/faults/healed"]
    );
}

#[test]
fn the_actual_cache_follows_each_subjects_newest_claim() {
    let store = Store::open_memory("node").unwrap();
    let observe = |subject: &str, status: &str| {
        store
            .append_claim(&ClaimInput {
                subject: subject.into(),
                kind: "runtime.observed".into(),
                actor: None,
                fields: BTreeMap::from([("status".into(), Value::String(status.into()))]),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap()
    };
    let subject = "agent/run/worker";
    let stopped = observe(subject, "stopped");
    assert_eq!(
        store.latest_actual_value(subject).unwrap().unwrap()["status"],
        "stopped"
    );
    let cached = |store: &Store| {
        store
            .smalltalk
            .actual_cache
            .lock()
            .unwrap()
            .get(subject)
            .cloned()
    };
    assert_eq!(cached(&store).unwrap().0, stopped.store_index);

    // Other subjects' writes leave a stopped runtime's folded state valid.
    observe("agent/run/other", "running");
    let before = cached(&store).unwrap();
    store.latest_actual_value(subject).unwrap();
    assert_eq!(
        cached(&store).unwrap(),
        before,
        "an unrelated write must not refold this subject's history"
    );

    let running = observe(subject, "running");
    assert_eq!(
        store.latest_actual_value(subject).unwrap().unwrap()["status"],
        "running"
    );
    assert_eq!(cached(&store).unwrap().0, running.store_index);
}

#[test]
fn declaration_restart_observation_waits_on_an_older_registry() {
    let store = Store::open_memory("node").unwrap();
    let record = store.append_claim(&ClaimInput {
        subject: "agent/example/relaunch".into(), kind: "runtime.observed".into(), actor: None,
        fields: serde_json::from_value(serde_json::json!({
            "status":"running", "incarnation_id":"old", "restarting_from":"old"
        })).unwrap(), evidence: vec![], expected_subject: None, idempotency_key: None,
    }).unwrap();
    let mut older = fractal_schema::registry().clone();
    older.claims.get_mut("runtime.observed").unwrap().fields.remove("restarting_from").unwrap();
    assert!(matches!(classify_replicated_claim_with_registry(&record, &older).unwrap(),
        ReplicatedClaimAdmission::UnknownField));
    assert!(matches!(classify_replicated_claim_with_registry(&record, fractal_schema::registry()).unwrap(),
        ReplicatedClaimAdmission::Valid));
}

#[test]
fn a_subject_with_no_actual_or_desired_state_has_unknown_reachability() {
    let store = Store::open_memory("node").unwrap();
    let subject = "agent/diagnostic-run/sig.hub";
    store
        .append_claim(&ClaimInput {
            subject: subject.into(),
            kind: "harness.observed".into(),
            actor: Some(subject.into()),
            fields: BTreeMap::from([("state".into(), Value::String("idle".into()))]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
    let status = store.status(Some(subject)).unwrap();
    assert!(status.subjects[0].actual.is_none());
    assert!(status.subjects[0].desired.is_none());
    assert_eq!(status.subjects[0].reachability, "unknown");
}

#[test]
fn a_same_origin_restart_window_does_not_create_runtime_conflict() {
    let store = Store::open_memory("node").unwrap();
    let subject = "agent/run/worker";
    let observe = |status: &str, incarnation: &str| {
        store
            .append_claim(&ClaimInput {
                subject: subject.into(),
                kind: "runtime.observed".into(),
                actor: None,
                fields: BTreeMap::from([
                    ("status".into(), Value::String(status.into())),
                    ("incarnation_id".into(), Value::String(incarnation.into())),
                ]),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
    };
    observe("running", "one");
    store
        .append_claim(&ClaimInput {
            subject: subject.into(),
            kind: "runtime.restart-window-reset".into(),
            actor: None,
            fields: BTreeMap::from([
                ("desired_token".into(), Value::String("desired-one".into())),
                ("incarnation_id".into(), Value::String("one".into())),
                (
                    "reason".into(),
                    Value::String("the stable interval elapsed".into()),
                ),
            ]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
    observe("running", "two");

    let status = store.status(Some(subject)).unwrap();
    assert_eq!(status.subjects[0].reachability, "reachable");
    assert_ne!(
        status.subjects[0].reason.as_deref(),
        Some("concurrent runtime observations have indeterminate authority")
    );
}

#[test]
fn concurrent_cross_origin_runtime_observations_are_indeterminate() {
    let left = Store::open_memory("left").unwrap();
    let right = Store::open_memory("right").unwrap();
    for (store, incarnation) in [(&left, "left-one"), (&right, "right-one")] {
        store
            .append_claim(&ClaimInput {
                subject: "agent/run/worker".into(),
                kind: "runtime.observed".into(),
                actor: None,
                fields: BTreeMap::from([
                    ("status".into(), Value::String("running".into())),
                    ("incarnation_id".into(), Value::String(incarnation.into())),
                ]),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
    }
    left.import_replication("right", &right.export_replication(0).unwrap())
        .unwrap();

    let status = left.status(Some("agent/run/worker")).unwrap();
    assert_eq!(status.subjects[0].reachability, "indeterminate");
    assert_eq!(
        status.subjects[0].reason.as_deref(),
        Some("concurrent runtime observations have indeterminate authority")
    );
}

#[test]
fn a_rival_origins_latest_observation_determines_runtime_authority() {
    let subject = "agent/run/worker";
    for terminal_status in ["stopped", "absent", "exited", "vanished"] {
        let left = Store::open_memory("left").unwrap();
        let right = Store::open_memory("right").unwrap();
        left.set_write_clock_at(1_800_000_000_000).unwrap();
        right.set_write_clock_at(1_800_000_000_001).unwrap();
        let observe = |store: &Store, status: &str, host: &str| {
            store
                .append_claim(&ClaimInput {
                    subject: subject.into(),
                    kind: "runtime.observed".into(),
                    actor: None,
                    fields: BTreeMap::from([
                        ("status".into(), json!(status)),
                        ("host".into(), json!(host)),
                        ("incarnation_id".into(), json!(host)),
                    ]),
                    evidence: Vec::new(),
                    expected_subject: None,
                    idempotency_key: None,
                })
                .unwrap()
        };
        observe(&left, "running", "left");
        observe(&left, terminal_status, "left");
        // The new owner has no causal path through the old owner's runtime branch.
        let selected = observe(&right, "running", "right");
        right.import_replication("left", &left.export_replication(0).unwrap())
            .unwrap();
        let view = right.status(Some(subject)).unwrap().subjects.remove(0);
        assert_eq!(view.actual_claim.as_deref(), Some(selected.id.as_str()));
        assert_eq!(view.reachability, "reachable", "{terminal_status}: {view:?}");
        // A later live rival is not hidden by its previous terminal observation.
        left.set_write_clock_at(1_800_000_000_002).unwrap();
        observe(&left, "running", "left");
        right.import_replication("left", &left.export_replication(0).unwrap())
            .unwrap();
        assert_eq!(
            right.status(Some(subject)).unwrap().subjects[0].reachability,
            "indeterminate"
        );
    }
}

/// A subject's harness reports neither change which observation its status selects nor hide
/// the causal path between two hosts' observations, although the reads that select skip them.
#[test]
fn harness_reports_neither_select_nor_hide_a_runtime_observation() {
    let subject = "agent/run/worker";
    let append = |store: &Store, kind: &str, fields: &[(&str, &str)]| {
        store
            .append_claim(&ClaimInput {
                subject: subject.into(),
                kind: kind.into(),
                actor: Some(subject.into()),
                fields: fields
                    .iter()
                    .map(|(name, value)| ((*name).into(), Value::String((*value).into())))
                    .collect(),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap()
    };
    let report = |store: &Store, count: usize| {
        for _ in 0..count {
            append(store, "harness.observed", &[("state", "working")]);
        }
    };
    let observe = |store: &Store, incarnation: &str| {
        append(
            store,
            "runtime.observed",
            &[("status", "running"), ("incarnation_id", incarnation)],
        )
    };

    // The left host observes the runtime after it has taken in the right host's
    // observation, with harness reports between the two: no conflict.
    let left = Store::open_memory("left").unwrap();
    let right = Store::open_memory("right").unwrap();
    observe(&right, "right-one");
    report(&right, 3);
    left.import_replication("right", &right.export_replication(0).unwrap())
        .unwrap();
    report(&left, 3);
    let selected = observe(&left, "left-two");
    report(&left, 3);
    let status = left.status(Some(subject)).unwrap();
    assert_eq!(status.subjects[0].reachability, "reachable");
    assert_eq!(
        status.subjects[0].actual_claim.as_deref(),
        Some(selected.id.as_str())
    );
    assert_eq!(
        status.subjects[0].actual.as_ref().unwrap()["incarnation_id"],
        "left-two"
    );

    // Two hosts observe it without seeing each other, harness reports on both sides.
    let other = Store::open_memory("other").unwrap();
    observe(&other, "other-one");
    report(&other, 3);
    left.import_replication("other", &other.export_replication(0).unwrap())
        .unwrap();
    let status = left.status(Some(subject)).unwrap();
    assert_eq!(status.subjects[0].reachability, "indeterminate");

    // The selecting reads seek past the harness reports instead of reading them.
    let connection = left.readers.get();
    let plan = connection
        .prepare(/* sqlite-large-value: allow-for-now lv-h38eff0d38ab8 owner=fractal_store::store::tests TODO=trace-statement-and-value-representation */ &format!(
            "EXPLAIN QUERY PLAN SELECT claims.id FROM claims
                 INDEXED BY claims_subject_kind_index
                 JOIN batches ON batches.id=claims.batch_id
                 WHERE claims.subject=?1 AND {ACTUAL_STATE_CLAIM} AND claims.store_index<=?2
                 ORDER BY {CANONICAL_ORDER}"
        ))
        .unwrap()
        .query_map(params![subject, i64::MAX], |row| row.get::<_, String>(3))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert!(
        plan.iter()
            .any(|step| step.contains("claims_subject_kind_index (subject=? AND kind<?)")),
        "{plan:?}"
    );
    assert!(
        plan.iter()
            .any(|step| step.contains("claims_subject_kind_index (subject=? AND kind>?)")),
        "{plan:?}"
    );
}

#[test]
fn a_remote_stop_without_process_authority_cannot_poison_the_running_owner() {
    let owner = json!({"fields": {"status": "running", "host": "ExampleMac"}});
    let remote_stop = json!({"fields": {"status": "stopped"}});
    let remote_running = json!({"fields": {"status": "running", "host": "example-linux"}});
    assert!(nonowner_terminal_observation(
        None,
        "ExampleMac",
        &owner,
        "example-linux",
        &remote_stop,
    ));
    assert!(!nonowner_terminal_observation(
        Some("example-linux"),
        "ExampleMac",
        &owner,
        "example-linux",
        &remote_stop,
    ));
    assert!(!nonowner_terminal_observation(
        None,
        "ExampleMac",
        &owner,
        "example-linux",
        &remote_running,
    ));
}

#[test]
fn apply_is_subject_cas_and_idempotent() {
    let store = Store::open_memory("node").expect("store");
    let intent = simple("true");
    let mission = store
        .mission(
            &intent,
            IntentInput {
                kdl: "test".into(),
                source_name: None,
            },
        )
        .expect("mission");
    let first = store
        .apply(&intent, &mission.subject_tokens, "one")
        .expect("apply");
    let repeated = store
        .apply(&intent, &mission.subject_tokens, "one")
        .expect("repeat");
    assert_eq!(first.batch_id, repeated.batch_id);
    let unchanged_mission = store
        .mission(
            &intent,
            IntentInput {
                kdl: "test".into(),
                source_name: None,
            },
        )
        .unwrap();
    let unchanged = store
        .apply(&intent, &unchanged_mission.subject_tokens, "unchanged")
        .unwrap();
    assert!(!unchanged.changed);
    assert!(unchanged.batch_id.is_none());
    assert_eq!(unchanged.store_index, first.store_index);

    let changed = simple("false");
    let error = store
        .apply(&changed, &mission.subject_tokens, "two")
        .expect_err("stale token");
    assert_eq!(error.code, "stale-subject");
    assert_eq!(error.details["subject"], "exec/work");
    assert_eq!(error.details["expected_heads"], json!([]));
    assert_eq!(error.details["current_heads"].as_array().unwrap().len(), 1);
}

#[test]
fn direct_publication_is_additive_atomic_and_operation_ids_are_immutable() {
    let store = Store::open_memory("node").unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let mission = publish_mission(
        &store,
        "version 2\nmission \"lifecycle\" state=\"ready\" { goal \"Remain open.\" }",
        "publish-lifecycle",
    );
    let creation = format!(
        "version 2\nmission-run \"lifecycle/demo\" {{\n  mission {:?}\n  workspace {:?}\n  requester \"person/operator\"\n}}\n",
        format!("mission/lifecycle@{}", mission.revision),
        workspace.path().display().to_string(),
    );
    let intent = crate::graph::parse_intent(&creation, "node").unwrap();
    let preview = store
        .mission(
            &intent,
            IntentInput {
                kdl: creation.clone(),
                source_name: None,
            },
        )
        .unwrap();
    let created = store
        .apply_as(
            &intent,
            &preview.subject_tokens,
            "create-lifecycle-run",
            Some("person/operator"),
        )
        .unwrap();
    assert!(created.changed);
    assert!(
        created
            .operations
            .iter()
            .any(|operation| operation.action == "start-mission-run")
    );

    let replay_preview = store
        .mission(
            &intent,
            IntentInput {
                kdl: creation,
                source_name: None,
            },
        )
        .unwrap();
    let replay = store
        .apply_as(
            &intent,
            &replay_preview.subject_tokens,
            "replay-lifecycle-run",
            Some("person/operator"),
        )
        .unwrap();
    assert!(!replay.changed);

    let omitted_source =
        "version 2\nmission \"unrelated\" state=\"ready\" { goal \"Do other work.\" }\n";
    let omitted = crate::graph::parse_intent(omitted_source, "node").unwrap();
    let omitted_preview = store
        .mission(
            &omitted,
            IntentInput {
                kdl: omitted_source.into(),
                source_name: None,
            },
        )
        .unwrap();
    let omission = store
        .apply_as(
            &omitted,
            &omitted_preview.subject_tokens,
            "omit-existing-run",
            Some("person/operator"),
        )
        .unwrap();
    assert!(omission.changed);
    assert!(store.mission_run("lifecycle/demo").unwrap().is_some());

    let atomic_failure = r#"version 2
mission-run "lifecycle/demo" {
  cancellation "stop" { reason "stop the run" }
}
resource "missing" {
  refresh "now" { timeout "1s" }
}
"#;
    let atomic_intent = crate::graph::parse_intent(atomic_failure, "node").unwrap();
    let atomic_preview = store
        .mission(
            &atomic_intent,
            IntentInput {
                kdl: atomic_failure.into(),
                source_name: None,
            },
        )
        .unwrap();
    assert!(!atomic_preview.blockers.is_empty());
    let error = store
        .apply_as(
            &atomic_intent,
            &atomic_preview.subject_tokens,
            "atomic-failure",
            Some("person/operator"),
        )
        .unwrap_err();
    assert_eq!(error.code, "resource-not-observed");
    assert_eq!(
        store.mission_run("lifecycle/demo").unwrap().unwrap().phase,
        "normal"
    );

    let cancellation = r#"version 2
mission-run "lifecycle/demo" {
  cancellation "stop" { reason "stop the run" }
}
"#;
    let cancel_intent = crate::graph::parse_intent(cancellation, "node").unwrap();
    let cancel_preview = store
        .mission(
            &cancel_intent,
            IntentInput {
                kdl: cancellation.into(),
                source_name: None,
            },
        )
        .unwrap();
    let cancelled = store
        .apply_as(
            &cancel_intent,
            &cancel_preview.subject_tokens,
            "cancel-lifecycle-run",
            Some("person/operator"),
        )
        .unwrap();
    assert!(cancelled.changed);
    let replay_preview = store
        .mission(
            &cancel_intent,
            IntentInput {
                kdl: cancellation.into(),
                source_name: None,
            },
        )
        .unwrap();
    let replay = store
        .apply_as(
            &cancel_intent,
            &replay_preview.subject_tokens,
            "replay-cancellation",
            Some("person/operator"),
        )
        .unwrap();
    assert!(!replay.changed);

    let changed_id = r#"version 2
mission-run "lifecycle/demo" {
  cancellation "stop" { reason "a different reason" }
}
"#;
    let changed_intent = crate::graph::parse_intent(changed_id, "node").unwrap();
    let changed_preview = store
        .mission(
            &changed_intent,
            IntentInput {
                kdl: changed_id.into(),
                source_name: None,
            },
        )
        .unwrap();
    assert!(
        changed_preview
            .blockers
            .iter()
            .any(|blocker| blocker.contains("different content"))
    );
}

#[test]
fn a_mission_response_reports_the_server_normalized_revision() {
    let source = r#"
version 2

  mission "portable" state="ready" {
    goal "Keep one worker available."
     agent "worker" { workspace "."; command "true" }
  }

"#;
    let server_intent = crate::graph::parse_intent(source, "server-node").unwrap();
    let client_intent = crate::graph::parse_intent(source, "local").unwrap();
    let server_mission = server_intent.missions.values().next().unwrap();
    let client_mission = client_intent.missions.values().next().unwrap();
    assert_eq!(server_mission.revision, client_mission.revision);

    let store = Store::open_memory("server-node").unwrap();
    let response = store
        .mission(
            &server_intent,
            IntentInput {
                kdl: source.into(),
                source_name: None,
            },
        )
        .unwrap();
    assert_eq!(
        response.mission_revisions["mission/portable"],
        server_mission.revision
    );
}

#[test]
fn terminal_operations_do_not_replace_member_status() {
    let store = Store::open_memory("node").expect("store");
    let subject = "pty/demo";
    store
        .append_claim(&ClaimInput {
            subject: subject.into(),
            kind: "runtime.observed".into(),
            actor: None,
            fields: BTreeMap::from([("status".into(), Value::String("running".into()))]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
        .expect("member observation");
    store
        .append_claim(&ClaimInput {
            subject: subject.into(),
            kind: "terminal.input.result".into(),
            actor: None,
            fields: BTreeMap::from([("result".into(), Value::String("written".into()))]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
        .expect("input result");

    let actual = store
        .latest_actual_value(subject)
        .expect("actual state")
        .expect("subject state");
    assert_eq!(actual["status"], "running");
    assert_eq!(actual["result"], "written");
}

#[test]
fn old_document_versions_remain_valid() {
    let store = Store::open_memory("node").expect("store");
    let old = store
        .put_document("doc/task", b"old", &None, "old")
        .expect("old");
    let new = store
        .put_document(
            "doc/task",
            b"new",
            &Some(old.binding_claim_id.clone()),
            "new",
        )
        .expect("new");
    assert_ne!(old.hash, new.hash);
    let selected = store
        .list_documents(Some("doc/task"), false, 10)
        .expect("selected documents");
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].hash, new.hash);
    let history = store
        .list_documents(Some("doc/task"), true, 10)
        .expect("document history");
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].hash, new.hash);
    assert_eq!(history[1].hash, old.hash);
    assert_eq!(
        store
            .list_documents(Some("doc/task"), true, 1)
            .expect("bounded history")
            .len(),
        1
    );
    let intent = parse_intent(
        &format!(
            "version 2\n person \"worker\"; message \"task\" {{ to \"person/worker\"; content \"doc/task@{}\"; }} ",
            old.hash
        ),
        "node",
    )
    .expect("intent");
    let mission = store
        .mission(
            &intent,
            IntentInput {
                kdl: "test".into(),
                source_name: None,
            },
        )
        .expect("mission");
    assert!(mission.blockers.is_empty());
    assert_eq!(mission.warnings.len(), 1);
}

#[test]
fn historical_mission_and_status_use_the_selected_index() {
    let store = Store::open_memory("node").expect("store");
    let first = simple("true");
    let first_mission = store
        .mission(
            &first,
            IntentInput {
                kdl: "first".into(),
                source_name: None,
            },
        )
        .unwrap();
    let first_apply = store
        .apply(&first, &first_mission.subject_tokens, "first")
        .unwrap();
    let first_index = first_apply.store_index;

    let second = simple("false");
    let second_mission = store
        .mission(
            &second,
            IntentInput {
                kdl: "second".into(),
                source_name: None,
            },
        )
        .unwrap();
    store
        .apply(&second, &second_mission.subject_tokens, "second")
        .unwrap();

    let historical = store
        .status_at(Some("exec/work"), None, Some(first_index))
        .unwrap();
    assert_eq!(historical.store_index, first_index);
    assert_eq!(
        historical.subjects[0].desired_token,
        first_apply.subject_tokens["exec/work"].first().cloned()
    );
    let historical_mission = store
        .mission_at(
            &second,
            IntentInput {
                kdl: "historical".into(),
                source_name: None,
            },
            Some(first_index),
        )
        .unwrap();
    assert_eq!(
        historical_mission.subject_tokens["exec/work"],
        first_apply.subject_tokens["exec/work"]
    );
    assert_eq!(historical_mission.changes.len(), 1);
}

#[test]
fn replication_carries_document_bytes_and_name_versions() {
    let source = Store::open_memory("source").unwrap();
    let document = source
        .put_document("doc/task", b"replicated", &None, "document")
        .unwrap();
    let batch = source.export_replication(0).unwrap();
    let target = Store::open_memory("target").unwrap();
    target.import_replication("source", &batch).unwrap();
    assert_eq!(
        target
            .get_document("doc/task", &document.hash)
            .unwrap()
            .unwrap(),
        b"replicated"
    );
}

#[test]
fn replication_carries_loop_state_and_round_results() {
    let source = Store::open_memory("source").unwrap();
    let subject = "loop-run/01990000000070008000000000000000/improve";
    source
        .append_claim(&ClaimInput {
            subject: subject.into(),
            kind: "loop.state".into(),
            actor: None,
            fields: BTreeMap::from([
                ("status".into(), Value::String("running".into())),
                ("round".into(), Value::from(1)),
            ]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: Some("replicate-loop-state".into()),
        })
        .unwrap();
    source
        .append_claim(&ClaimInput {
            subject: subject.into(),
            kind: "loop.round-result".into(),
            actor: None,
            fields: BTreeMap::from([
                ("round".into(), Value::from(1)),
                ("status".into(), Value::String("completed".into())),
                (
                    "mission_run".into(),
                    Value::String("mission-run/01990000000070008000000000000001".into()),
                ),
                ("metrics".into(), serde_json::json!({"quality": 1})),
                ("token_usage".into(), Value::from(10)),
            ]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: Some("replicate-loop-result".into()),
        })
        .unwrap();

    let target = Store::open_memory("target").unwrap();
    target
        .import_replication("source", &source.export_replication(0).unwrap())
        .unwrap();

    assert_eq!(
        target
            .latest_claim(subject, Some("loop.state"))
            .unwrap()
            .unwrap()
            .body
            .pointer("/fields/round")
            .and_then(Value::as_u64),
        Some(1)
    );
    assert_eq!(
        target
            .claims_for(subject, Some("loop.round-result"))
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn a_host_document_must_exist_before_its_declaration_is_applied() {
    let store = Store::open_memory("node").unwrap();
    let bytes = b"host facts\n";
    let hash = hex::encode(sha2::Sha256::digest(bytes));
    let source = format!("version 2\nhost \"node\" {{ document \"doc/hosts/node@{hash}\" }}\n");
    let intent = crate::graph::parse_test_intent(&source, "node").unwrap();
    let preview = store
        .mission(
            &intent,
            IntentInput {
                kdl: source.clone(),
                source_name: None,
            },
        )
        .unwrap();
    let error = store
        .apply(&intent, &preview.subject_tokens, "missing-host-document")
        .unwrap_err();
    assert_eq!(error.code, "missing-document");

    store
        .put_document("doc/hosts/node", bytes, &None, "host-document")
        .unwrap();
    let preview = store
        .mission(
            &intent,
            IntentInput {
                kdl: source,
                source_name: None,
            },
        )
        .unwrap();
    store
        .apply(&intent, &preview.subject_tokens, "apply-host-document")
        .unwrap();
}

#[test]
fn work_nudged_incremental_guards_and_rollback_preserve_the_committed_frontier() {
    for refusal in ["none", "malformed-operation", "operation-conflict", "unhealthy-projection"] {
        let store = Store::open_memory("node").unwrap();
        assert!(store.project_replication_backlog().unwrap());
        let before = store.index().unwrap();
        let digest = graph_digest_of(&store);
        let mut connection = store.connection.lock().unwrap();
        let transaction = connection.transaction().unwrap();
        let frontier: u64 = transaction.query_row(
            "SELECT last_good_store_index FROM projection_health WHERE aggregate='graph'",
            [], |row| row.get(0),
        ).unwrap();
        let mut body = json!({"fields": {
            "attempt": 1, "agent": "agent/worker.one", "message": "message/invented-nudge",
            "idle_since_unix_ms": "1", "reason": "invented control", "waits": []
        }, "_operation": {"id": "op/nudge-guard", "request_digest": "digest-a"}});
        if refusal == "malformed-operation" {
            body["_operation"] = json!({"id": "op/nudge-guard"});
        }
        let claim = append_claim_record_tx(&transaction, "node", "step-run/invented",
            WORK_NUDGED_KIND, Some("daemon/runtime"), &body, &[], None).unwrap();
        if refusal == "operation-conflict" {
            transaction.execute(
                /* sqlite-large-value: allow-for-now lv-h130c10b1685b owner=fractal_store::store::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO operations(id,request_digest,canonical_claim_id,state)
                     VALUES ('op/nudge-guard','digest-b',?1,'active')", [&claim.id],
            ).unwrap();
        }
        if refusal == "unhealthy-projection" {
            transaction.execute(/* sqlite-large-value: allow-for-now lv-h7ba1308bc642 owner=fractal_store::store::tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE projection_health SET status='stale' WHERE aggregate='graph'", []).unwrap();
        }
        let result = try_project_simple_replication_tx(&transaction, &store.origin, claim.store_index).unwrap();
        match result {
            IncrementalProjection::Projected => {
                assert_eq!(refusal, "none");
                assert_eq!(operation_tx(&transaction, "op/nudge-guard").unwrap().unwrap().2, "active");
            }
            IncrementalProjection::Replay(reason)
            | IncrementalProjection::ReplayWithContext { reason, .. } => assert_eq!(reason, refusal),
        }
        // A projection attempt never advertises the uncommitted index as good.
        assert_eq!(transaction.query_row(
            "SELECT last_good_store_index FROM projection_health WHERE aggregate='graph'",
            [], |row| row.get::<_, u64>(0),
        ).unwrap(), frontier);
        transaction.rollback().unwrap();
        assert_eq!(operation_tx(&connection, "op/nudge-guard").unwrap(), None);
        drop(connection);
        assert_eq!(store.index().unwrap(), before);
        assert_eq!(graph_digest_of(&store), digest);
        assert!(!store.replication_projection_needs_recovery().unwrap());
    }
}

#[test]
fn lanes_runtime_actions_and_intake_extend_the_graph_incrementally() {
    for kind in [
        "lane.marked",
        "lane.joined",
        "runtime.action.requested",
        "runtime.action.succeeded",
        "subscription.mission-requested",
        "subscription.mission-started",
        "observer.refresh-requested",
        "publication.operation",
    ] {
        assert!(Store::simple_replication_kind(kind), "{kind}");
    }
    // Projections read loop and attention claims, or rebuild from checkpoints and repairs.
    for kind in [
        "planning-session.started",
        "loop.round-result",
        "attention.resolved",
        "checkpoint.sealed",
        "record.repaired",
    ] {
        assert!(!Store::simple_replication_kind(kind), "{kind}");
    }
}

#[test]
fn a_run_started_with_a_reporter_records_it_and_an_unset_run_records_nothing() {
    let store = Store::open_memory("node").unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let plain = publish_mission(
        &store,
        "version 2\nmission \"plain\" state=\"ready\" { concurrent-runs; goal \"Plain.\"; step \"work\" { agentless } }",
        "publish-plain",
    );
    let watched = publish_mission(
        &store,
        "version 2\nmission \"watched\" state=\"ready\" report-to=\"agent/ops/mission-reporter\" stalled-after=\"15m\" { concurrent-runs; goal \"Watched.\"; step \"work\" { agentless } }",
        "publish-watched",
    );
    let declare = |run: &str, mission: &MissionSpec, extra: &str| {
        let source = format!(
            "version 2\nmission-run {run:?} {{\n  mission \"mission/{}@{}\"\n  workspace {:?}\n  requester \"person/operator\"\n{extra}}}\n",
            mission.id,
            mission.revision,
            workspace.path().display().to_string(),
        );
        let intent = crate::graph::parse_intent(&source, "node").unwrap();
        let preview = store
            .mission(
                &intent,
                IntentInput {
                    kdl: source,
                    source_name: None,
                },
            )
            .unwrap();
        (intent, preview)
    };
    let start = |run: &str, mission: &MissionSpec, extra: &str| {
        let (intent, preview) = declare(run, mission, extra);
        assert!(preview.blockers.is_empty(), "{:?}", preview.blockers);
        store
            .apply_as(
                &intent,
                &preview.subject_tokens,
                &format!("start-{run}"),
                Some("person/operator"),
            )
            .unwrap();
        store
            .latest_claim(&format!("mission-run/{run}"), Some("mission-run.created"))
            .unwrap()
            .unwrap()
            .body["fields"]
            .clone()
    };

    let unset = start("plain/1", &plain, "");
    for field in ["report_to", "stalled_after_ms", "report_completed"] {
        assert!(unset.get(field).is_none(), "{field}");
    }
    // Whoever starts a run names the reporter, and the default limit applies.
    let named = start(
        "plain/2",
        &plain,
        "  report-to \"agent/ops/starter\"\n  report-completed \"true\"\n",
    );
    assert_eq!(named["report_to"], "agent/ops/starter");
    assert_eq!(named["stalled_after_ms"], 30 * 60_000);
    assert_eq!(named["report_completed"], true);
    // The mission's own reporter and limit apply when the start names none.
    let inherited = start("watched/1", &watched, "");
    assert_eq!(inherited["report_to"], "agent/ops/mission-reporter");
    assert_eq!(inherited["stalled_after_ms"], 15 * 60_000);
    assert!(inherited.get("report_completed").is_none());
    // The start's reporter replaces the mission's.
    let replaced = start("watched/2", &watched, "  report-to \"agent/ops/starter\"\n");
    assert_eq!(replaced["report_to"], "agent/ops/starter");
    assert_eq!(replaced["stalled_after_ms"], 15 * 60_000);

    // Declaring the same run again with different reporting is not a retry: the stored
    // creation claim would be left unchanged while the operator is told it was accepted.
    let different = "mission run `mission-run/plain/2` already exists with different creation fields";
    for (what, extra) in [
        ("another reporter", "  report-to \"agent/ops/other\"\n  report-completed \"true\"\n"),
        ("no reporting at all", ""),
        ("the reporter alone, dropping completion", "  report-to \"agent/ops/starter\"\n"),
    ] {
        let (_, changed) = declare("plain/2", &plain, extra);
        assert_eq!(changed.blockers, [different], "{what}");
    }
    // Adding completion to a run that was started without it is not a retry either.
    let (_, added) = declare(
        "watched/2",
        &watched,
        "  report-to \"agent/ops/starter\"\n  report-completed \"true\"\n",
    );
    assert_eq!(
        added.blockers,
        ["mission run `mission-run/watched/2` already exists with different creation fields"]
    );
    // The exact declarations, and one that leaves the mission's own reporter to apply, are.
    for (run, mission, extra) in [
        (
            "plain/2",
            &plain,
            "  report-to \"agent/ops/starter\"\n  report-completed \"true\"\n",
        ),
        ("watched/1", &watched, ""),
        ("watched/2", &watched, "  report-to \"agent/ops/starter\"\n"),
        ("plain/1", &plain, ""),
    ] {
        let (_, same) = declare(run, mission, extra);
        assert!(same.blockers.is_empty(), "{run}: {:?}", same.blockers);
    }
}

#[test]
fn a_run_declared_after_another_keeps_its_wait_through_preview_and_replication() {
    let controller = Store::open_memory("controller").unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let build = publish_mission(
        &controller,
        "version 2\nmission \"build\" state=\"ready\" { goal \"Build.\"; step \"build\" { agentless } }",
        "publish-build",
    );
    let ship = publish_mission(
        &controller,
        "version 2\nmission \"ship\" state=\"ready\" { goal \"Ship.\"; step \"ship\" { agentless } }",
        "publish-ship",
    );
    let declare = |run: &str, mission: &MissionSpec, after: Option<&str>| {
        let after = after.map_or(String::new(), |after| format!("  after {after:?}\n"));
        let source = format!(
            "version 2\nmission-run {run:?} {{\n  mission \"mission/{}@{}\"\n  workspace {:?}\n  requester \"person/operator\"\n{after}}}\n",
            mission.id,
            mission.revision,
            workspace.path().display().to_string(),
        );
        let intent = crate::graph::parse_intent(&source, "node").unwrap();
        let preview = controller
            .mission(
                &intent,
                IntentInput {
                    kdl: source,
                    source_name: None,
                },
            )
            .unwrap();
        (intent, preview)
    };

    let (_, early) = declare("ship/1", &ship, Some("build/1"));
    assert_eq!(
        early.blockers,
        [
            "mission run `mission-run/ship/1` waits for `mission-run/build/1`, which does not exist"
        ]
    );
    for (run, mission, after) in [
        ("build/1", &build, None),
        ("ship/1", &ship, Some("build/1")),
    ] {
        let (intent, preview) = declare(run, mission, after);
        assert!(preview.blockers.is_empty(), "{:?}", preview.blockers);
        controller
            .apply_as(
                &intent,
                &preview.subject_tokens,
                &format!("start-{run}"),
                Some("person/operator"),
            )
            .unwrap();
    }
    let (_, changed) = declare("ship/1", &ship, Some("build/2"));
    assert_eq!(
        changed.blockers,
        ["mission run `mission-run/ship/1` already exists with different creation fields"]
    );

    let local = controller.mission_run("ship/1").unwrap().unwrap();
    assert_eq!(local.after.as_deref(), Some("mission-run/build/1"));
    let worker = Store::open_memory("worker").unwrap();
    receive_and_project(
        &worker,
        "controller",
        &exchange_from(&controller, &ReplicationInventory::default()),
    );
    let remote = worker.mission_run("ship/1").unwrap().unwrap();
    assert_eq!(remote.after, local.after);
    assert_eq!(
        remote
            .steps
            .iter()
            .map(|step| (step.subject.as_str(), step.agentless))
            .collect::<Vec<_>>(),
        local
            .steps
            .iter()
            .map(|step| (step.subject.as_str(), step.agentless))
            .collect::<Vec<_>>()
    );
    assert_eq!(remote.steps[0].step, "after-run");
}

#[test]
fn one_invalid_or_unknown_claim_does_not_block_its_valid_sibling_or_later_envelopes() {
    for unknown_kind in [false, true] {
        let source = Store::open_memory("source").unwrap();
        let valid = source
            .append_claim(&ClaimInput {
                subject: "host/source".into(),
                kind: "transport.observed".into(),
                actor: None,
                fields: BTreeMap::from([("status".into(), Value::String("up".into()))]),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: Some("valid-sibling".into()),
            })
            .unwrap();
        let mut exchange = exchange_from(&source, &ReplicationInventory::default());
        let candidate = rewrite_envelope(&exchange.envelopes[0], |payload| {
            let mut invalid = payload.batch.claims[0].clone();
            if unknown_kind {
                // A newer transport kind could decide the host's reachability.
                invalid.kind = "transport.future-observed".into();
            } else {
                invalid.body["fields"]["status"] = Value::Bool(true);
            }
            invalid.id = claim_hash(
                &invalid.batch_id,
                &invalid.subject,
                &invalid.kind,
                &invalid.origin,
                invalid.actor.as_deref(),
                &invalid.body,
                &invalid.predecessors,
            )
            .unwrap();
            payload.batch.claims.push(invalid);
        });
        exchange.envelopes = vec![candidate.clone()];
        exchange.inventory.envelopes = vec![ReplicaEnvelopeId {
            writer: candidate.writer.clone(),
            sequence: candidate.sequence,
            hash: candidate.hash.clone(),
        }];

        let target = Store::open_memory("target").unwrap();
        let admission = receive_and_project(&target, "source", &exchange);
        assert_eq!(admission.valid, 1);
        assert_eq!(admission.invalid, usize::from(!unknown_kind));
        assert_eq!(admission.unknown, usize::from(unknown_kind));
        assert_eq!(
            target
                .latest_claim("host/source", Some("transport.observed"))
                .unwrap()
                .unwrap()
                .id,
            valid.id
        );
        assert_eq!(
            target.status(Some("host/source")).unwrap().subjects[0].reachability,
            "indeterminate"
        );

        source
            .append_claim(&ClaimInput {
                subject: "host/later".into(),
                kind: "transport.observed".into(),
                actor: None,
                fields: BTreeMap::from([("status".into(), Value::String("up".into()))]),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: Some("later-envelope".into()),
            })
            .unwrap();
        let later = exchange_from(&source, &target.replication_inventory().unwrap());
        receive_and_project(&target, "source", &later);
        assert!(
            target
                .latest_claim("host/later", Some("transport.observed"))
                .unwrap()
                .is_some()
        );
        assert_eq!(target.replica_records(true).unwrap().len(), 1);
    }
}

#[test]
fn review_reader_old_peer_quarantines_and_retries_original_bytes() {
    for kind in std::iter::once("gate.requested").chain(fractal_schema::review::KINDS) {
        let current = Store::open_memory("ada").unwrap();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("review.sqlite");
        let older = Store::open(&path, "tester").unwrap();
        let mut registry = fractal_schema::registry().clone();
        for new_kind in fractal_schema::review::KINDS {
            registry.claims.remove(new_kind);
        }
        for name in fractal_schema::review::REQUEST_FIELDS {
            registry
                .claims
                .get_mut("gate.requested")
                .unwrap()
                .fields
                .remove(name);
        }
        assert_ne!(registry.digest(), fractal_schema::registry().digest());
        older.set_claim_registry(registry);
        let observe = |store: &Store, subject: &str, status: &str| {
            store
                .append_claim(&ClaimInput {
                    subject: subject.into(),
                    kind: "transport.observed".into(),
                    actor: None,
                    fields: BTreeMap::from([("status".into(), json!(status))]),
                    evidence: Vec::new(),
                    expected_subject: None,
                    idempotency_key: None,
                })
                .unwrap()
        };
        let bootstrap = observe(&current, "host/ada", "up");
        receive_and_project(
            &older,
            "ada",
            &exchange_from(&current, &ReplicationInventory::default()),
        );
        current.project_replication_backlog().unwrap();
        assert!(!current.replication_projection_needs_recovery().unwrap());
        // An empty store's initial replay is legitimate. Everything after this
        // healthy baseline, including reopening with the new registry, must be incremental.
        assert!(!older.replication_projection_needs_recovery().unwrap());
        FULL_REPLAYS.with(|count| count.set(0));
        const DAY_MS: u128 = 24 * 60 * 60 * 1_000;
        current
            .set_write_clock_at(bootstrap.accepted_at_unix_ms + DAY_MS)
            .unwrap();
        let spec = fractal_schema::registry().claim(kind).unwrap();
        let mut fields = BTreeMap::new();
        for (name, field) in &spec.fields {
            if !field.required { continue; }
            let value = match field.value_type {
                fractal_schema::ValueType::Integer => json!(1),
                fractal_schema::ValueType::Array => json!([]),
                fractal_schema::ValueType::Object => json!({}),
                fractal_schema::ValueType::SubjectReference => json!(format!("{}/tester", field.reference_families.first().map_or("mission-run", String::as_str))),
                _ => json!(field.values.first().map_or("receipt", String::as_str)),
            };
            fields.insert(name.clone(), value);
        }
        if matches!(kind, "gate.head-bound" | "gate.review-verdict" | "gate.requested") {
            fields.insert("change".into(), json!({"kind":"github.pull-request","subject":"resource/project/review"}));
        }
        if matches!(kind, "gate.head-bound" | "gate.requested") {
            fields.insert("head".into(), json!({"kind":"git.commit","sha":"a".repeat(40)}));
        }
        if matches!(kind, "gate.review-verdict" | "gate.remediation-attached") {
            fields.insert("reviewed_head".into(), json!({"kind":"git.commit","sha":"a".repeat(40)}));
        }
        if kind == "gate.requested" {
            fields.extend(BTreeMap::from([
                ("gate_subject".into(),json!("gate-operation/tester")),
                ("generation".into(),json!("run-generation/tester")), ("episode".into(),json!(1)),
                ("scope".into(),json!({"coverage":["implementation"]})),
                ("author".into(),json!("agent/alex")), ("binding_claim".into(),json!("binding")),
                ("delta_ref".into(),json!("doc/project/delta")),
            ]));
        }
        let claim = current.append_claim(&ClaimInput {
            subject:"gate-operation/tester".into(), kind:kind.into(), actor:None,
            fields, evidence:Vec::new(), expected_subject:None, idempotency_key:None,
        }).unwrap();
        let exchange = exchange_from(&current, &older.replication_inventory().unwrap());
        assert_eq!(exchange.envelopes.len(), 1);
        let original_envelope = &exchange.envelopes[0];
        // Hold the old review envelope while the old peer makes ninety days of
        // unrelated progress. No sleeps or wall-clock timing can weaken this lag.
        for day in 2..=90 {
            older
                .set_write_clock_at(bootstrap.accepted_at_unix_ms + day * DAY_MS)
                .unwrap();
            observe(&older, "host/tester", "up");
        }
        older.project_replication_backlog().unwrap();
        let admission = receive_and_project(&older, "ada", &exchange);
        assert_eq!((admission.valid, admission.unknown, admission.invalid), (0,1,0), "{kind}");
        let records = older.replica_records(true).unwrap();
        assert_eq!(records.len(),1);
        assert_eq!(records[0].state,"unknown");
        assert_eq!(records[0].error_code.as_deref(),Some(if kind == "gate.requested" { "unknown-claim-field" } else { "unknown-claim-kind" }));
        let raw: Vec<u8> = older.readers.get().query_row(
            "SELECT raw FROM replica_records WHERE claim_id=?1",[&claim.id],|row|row.get(0),
        ).unwrap();
        assert!(!raw.is_empty());
        assert!(older.claims_for(&claim.subject,Some(kind)).unwrap().is_empty());
        assert_eq!(FULL_REPLAYS.with(std::cell::Cell::get), 0, "{kind}: lagged receipt");
        assert!(!older.replication_projection_needs_recovery().unwrap());
        // Known remote work after the unknown record must still cross the same
        // writer chain and advance the projection over several inventory pages.
        for day in 2..=25 {
            current
                .set_write_clock_at(bootstrap.accepted_at_unix_ms + day * DAY_MS)
                .unwrap();
            observe(&current, "host/ada", if day == 25 { "down" } else { "up" });
        }
        let mut pages = 0;
        loop {
            let mut inventory = older.replication_inventory().unwrap();
            inventory.accepts = Some(4);
            let page = exchange_from(&current, &inventory);
            if page.envelopes.is_empty() {
                break;
            }
            assert!(pages < 6, "{kind}: catch-up inventory must advance");
            pages += 1;
            let admission = receive_and_project(&older, "ada", &page);
            assert_eq!(admission.invalid, 0, "{kind}");
            assert!(admission.valid > 0, "{kind}");
            assert_eq!(FULL_REPLAYS.with(std::cell::Cell::get), 0, "{kind}: catch-up page");
            assert!(!older.replication_projection_needs_recovery().unwrap());
        }
        assert_eq!(pages, 6, "{kind}: exercise paginated catch-up");
        let latest = older.latest_claim("host/ada", Some("transport.observed")).unwrap().unwrap();
        assert_eq!(latest.body["fields"]["status"], "down", "{kind}: unrelated work progressed");
        assert_eq!(older.replica_records(true).unwrap().len(), 1, "{kind}");
        assert!(older.claims_for(&claim.subject, Some(kind)).unwrap().is_empty());
        drop(older);
        let upgraded = Store::open(&path,"tester").unwrap();
        assert!(!upgraded.replication_projection_needs_recovery().unwrap());
        let admitted = upgraded.validate_replication_backlog().unwrap();
        assert_eq!((admitted.valid,admitted.unknown,admitted.invalid),(1,0,0));
        upgraded.project_replication_backlog().unwrap();
        let retained = upgraded.claims_for(&claim.subject,Some(kind)).unwrap();
        assert_eq!(retained.len(),1);
        assert_eq!(retained[0].id,claim.id);
        assert_eq!(retained[0].body,claim.body);
        let retried_raw: Vec<u8> = upgraded.readers.get().query_row(
            "SELECT raw FROM replica_records WHERE claim_id=?1",[&claim.id],|row|row.get(0),
        ).unwrap();
        assert_eq!(raw,retried_raw);
        assert!(upgraded.replica_records(true).unwrap().is_empty(), "{kind}");
        assert_eq!(
            upgraded.latest_claim("host/ada", Some("transport.observed")).unwrap().unwrap().id,
            latest.id,
            "{kind}: retry preserves unrelated projection"
        );
        let relayed = exchange_from(&upgraded, &ReplicationInventory::default());
        let retained_envelope = relayed.envelopes.iter().find(|envelope| {
            envelope.writer == original_envelope.writer
                && envelope.sequence == original_envelope.sequence
        }).unwrap();
        assert_eq!(retained_envelope.hash, original_envelope.hash, "{kind}");
        assert_eq!(
            retained_envelope.payload.bytes().unwrap(),
            original_envelope.payload.bytes().unwrap(),
            "{kind}: retry and relay retain the original hash-authenticated envelope bytes"
        );
        assert!(!upgraded.replication_projection_needs_recovery().unwrap());
        assert_eq!(FULL_REPLAYS.with(std::cell::Cell::get), 0, "{kind}: upgrade retry");
    }
}

#[test]
fn an_unknown_field_is_retryable_and_an_old_schema_rejection_is_readmitted() {
    let source = Store::open_memory("source").unwrap();
    let claim = source
        .append_claim(&ClaimInput {
            subject: "host/source".into(),
            kind: "transport.observed".into(),
            actor: None,
            fields: BTreeMap::from([("status".into(), Value::String("up".into()))]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: Some("schema-upgrade-source".into()),
        })
        .unwrap();
    let exchange = exchange_from(&source, &ReplicationInventory::default());
    let envelope = &exchange.envelopes[0];

    let target = Store::open_memory("target").unwrap();
    target.bind_fleet(TEST_FLEET).unwrap();
    target
        .receive_replication_exchange("source", TEST_FLEET, &exchange)
        .unwrap();
    let record_ref = replica_record_ref(&envelope.writer, envelope.sequence, &envelope.hash, 0);
    {
        let connection = target.connection.lock().unwrap();
        connection
            .execute(
                /* sqlite-large-value: allow-for-now lv-h86ed99698958 owner=fractal_store::store::tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_envelopes SET receipt_state='degraded'\n                     WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3",
                params![envelope.writer, envelope.sequence, envelope.hash],
            )
            .unwrap();
        connection
            .execute(
                /* sqlite-large-value: allow-for-now lv-h15e9cb0e46e3 owner=fractal_store::store::tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO replica_records(\n                         record_ref, writer, sequence, envelope_hash, position, raw, state,\n                         claim_id, subject_hint, kind_hint, error_code, error_message, updated_at_unix_ms\n                     ) VALUES (?1, ?2, ?3, ?4, 0, X'', 'invalid', ?5, ?6, ?7,\n                               'invalid-replicated-claim', ?8, '1')",
                params![
                    record_ref,
                    envelope.writer,
                    envelope.sequence,
                    envelope.hash,
                    claim.id,
                    claim.subject,
                    claim.kind,
                    format!(
                        "replicated claim `{}` violates unknown-claim-field: field `future` is unknown",
                        claim.id
                    ),
                ],
            )
            .unwrap();
    }

    let admission = target.validate_replication_backlog().unwrap();
    assert_eq!(admission.valid, 1);
    assert_eq!(admission.unknown, 0);
    assert_eq!(admission.invalid, 0);
    assert!(target.replica_records(true).unwrap().is_empty());
    assert_eq!(
        target
            .latest_claim("host/source", Some("transport.observed"))
            .unwrap()
            .unwrap()
            .id,
        claim.id
    );
}

#[test]
fn a_claim_field_from_a_newer_schema_is_unknown_instead_of_invalid() {
    let source = Store::open_memory("source").unwrap();
    source
        .append_claim(&ClaimInput {
            subject: "host/source".into(),
            kind: "transport.observed".into(),
            actor: None,
            fields: BTreeMap::from([("status".into(), Value::String("up".into()))]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: Some("future-field-source".into()),
        })
        .unwrap();
    let mut exchange = exchange_from(&source, &ReplicationInventory::default());
    let candidate = rewrite_envelope(&exchange.envelopes[0], |payload| {
        let claim = &mut payload.batch.claims[0];
        claim.body["fields"]["future"] = Value::Bool(true);
        claim.id = claim_hash(
            &claim.batch_id,
            &claim.subject,
            &claim.kind,
            &claim.origin,
            claim.actor.as_deref(),
            &claim.body,
            &claim.predecessors,
        )
        .unwrap();
    });
    exchange.envelopes = vec![candidate.clone()];
    exchange.inventory.envelopes = vec![ReplicaEnvelopeId {
        writer: candidate.writer.clone(),
        sequence: candidate.sequence,
        hash: candidate.hash.clone(),
    }];

    let target = Store::open_memory("target").unwrap();
    let admission = receive_and_project(&target, "source", &exchange);
    assert_eq!(admission.valid, 0);
    assert_eq!(admission.unknown, 1);
    assert_eq!(admission.invalid, 0);
    let unresolved = target.replica_records(true).unwrap();
    assert_eq!(unresolved.len(), 1);
    assert_eq!(unresolved[0].state, "unknown");
    assert_eq!(
        unresolved[0].error_code.as_deref(),
        Some("unknown-claim-field")
    );
}

#[test]
fn future_arrangement_vocabulary_waits_on_replication_but_is_refused_locally() {
    let subject = "arrangement/person/ada/019a0000-0000-7000-8000-000000000001";
    for (operation, code) in [
        (json!({"op":"future.rename","name":"new"}), "invalid-arrangement-operations"),
        (json!({"op":"subject.place","subject":"future/item","folder":null,"key":"a0"}), "invalid-subject-reference"),
    ] {
        let source = Store::open_memory("source").unwrap();
        let mut input = ClaimInput {
            subject: subject.into(), kind: "arrangement.edited".into(),
            actor: Some("person/ada".into()),
            fields: BTreeMap::from([
                ("owner".into(), json!("person/ada")),
                ("operations".into(), json!([{"op":"create","name":"original"}])),
            ]),
            evidence: Vec::new(), expected_subject: None, idempotency_key: None,
        };
        source.append_claim(&input).unwrap();
        input.fields.insert("operations".into(), json!([operation.clone()]));
        assert_eq!(source.append_claim(&input).unwrap_err().code, code);
        let mut exchange = exchange_from(&source, &ReplicationInventory::default());
        let candidate = rewrite_envelope(&exchange.envelopes[0], |payload| {
            let claim = &mut payload.batch.claims[0];
            claim.body["fields"]["operations"] = json!([operation]);
            claim.id = claim_hash(&claim.batch_id, &claim.subject, &claim.kind,
                &claim.origin, claim.actor.as_deref(), &claim.body, &claim.predecessors).unwrap();
        });
        exchange.envelopes = vec![candidate.clone()];
        exchange.inventory.envelopes = vec![ReplicaEnvelopeId {
            writer: candidate.writer.clone(), sequence: candidate.sequence, hash: candidate.hash.clone(),
        }];
        let target = Store::open_memory("target").unwrap();
        let admission = receive_and_project(&target, "source", &exchange);
        assert_eq!((admission.valid, admission.unknown, admission.invalid), (0, 1, 0));
        let records = target.replica_records(true).unwrap();
        assert_eq!(records[0].state, "unknown");
        assert_eq!(records[0].error_code.as_deref(), Some("unknown-claim-field"));
        assert!(target.latest_claim(subject, None).unwrap().is_none());
    }
}

#[test]
fn one_invalid_blob_does_not_block_an_unrelated_valid_claim() {
    let source = Store::open_memory("source").unwrap();
    let valid = source
        .append_claim(&ClaimInput {
            subject: "host/source".into(),
            kind: "transport.observed".into(),
            actor: None,
            fields: BTreeMap::from([("status".into(), Value::String("up".into()))]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: Some("valid-beside-invalid-blob".into()),
        })
        .unwrap();
    let exchange = exchange_from(&source, &ReplicationInventory::default());
    let candidate = rewrite_envelope(&exchange.envelopes[0], |payload| {
        payload
            .blobs
            .insert("0".repeat(64), b"wrong bytes".to_vec());
    });
    let input = ReplicationExchange {
        build: None,
        projection_digests: Default::default(),
        peer: "source".into(),
        fleet_id: TEST_FLEET.into(),
        schema_digest: runtime::compatibility_digest(&fractal_schema::registry().digest()),
        projection_registry_digest: None,
        projection_versions: Default::default(),
        authority_digest: String::new(),
        graph_digest: String::new(),
        inventory: ReplicationInventory {
            digest: String::new(),
            envelopes: vec![ReplicaEnvelopeId {
                writer: candidate.writer.clone(),
                sequence: candidate.sequence,
                hash: candidate.hash.clone(),
            }],
            buckets: Vec::new(),
            accepts: None,
            checkpoint: None,
        },
        envelopes: vec![candidate],
        signature_requests: Vec::new(),
        signatures: Vec::new(),
    };

    let target = Store::open_memory("target").unwrap();
    let admission = receive_and_project(&target, "source", &input);
    assert_eq!(admission.valid, 1);
    assert_eq!(admission.invalid, 1);
    assert_eq!(
        target
            .latest_claim("host/source", Some("transport.observed"))
            .unwrap()
            .unwrap()
            .id,
        valid.id
    );
    let records = target.replica_records(true).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].kind.as_deref(), Some("blob"));
    assert_eq!(records[0].error_code.as_deref(), Some("blob-hash-mismatch"));
}

#[test]
fn same_sequence_candidates_converge_in_any_receipt_order() {
    let source = Store::open_memory("source").unwrap();
    source
        .append_claim(&ClaimInput {
            subject: "host/source".into(),
            kind: "transport.observed".into(),
            actor: None,
            fields: BTreeMap::from([("status".into(), Value::String("up".into()))]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: Some("candidate-source".into()),
        })
        .unwrap();
    let original =
        exchange_from(&source, &ReplicationInventory::default()).envelopes[0].clone();
    let fork = rewrite_envelope(&original, |payload| {
        let claim = &mut payload.batch.claims[0];
        claim.kind = "future.transport-observed".into();
        claim.id = claim_hash(
            &claim.batch_id,
            &claim.subject,
            &claim.kind,
            &claim.origin,
            claim.actor.as_deref(),
            &claim.body,
            &claim.predecessors,
        )
        .unwrap();
    });
    let make_exchange = |envelopes: Vec<ReplicaEnvelope>| ReplicationExchange {
        build: None,
        projection_digests: Default::default(),
        peer: "source".into(),
        fleet_id: TEST_FLEET.into(),
        schema_digest: runtime::compatibility_digest(&fractal_schema::registry().digest()),
        projection_registry_digest: None,
        projection_versions: Default::default(),
        authority_digest: String::new(),
        graph_digest: String::new(),
        inventory: ReplicationInventory {
            digest: String::new(),
            envelopes: envelopes
                .iter()
                .map(|envelope| ReplicaEnvelopeId {
                    writer: envelope.writer.clone(),
                    sequence: envelope.sequence,
                    hash: envelope.hash.clone(),
                })
                .collect(),
            buckets: Vec::new(),
            accepts: None,
            checkpoint: None,
        },
        envelopes,
        signature_requests: Vec::new(),
        signatures: Vec::new(),
    };
    let left = Store::open_memory("left").unwrap();
    let right = Store::open_memory("right").unwrap();
    receive_and_project(
        &left,
        "source",
        &make_exchange(vec![original.clone(), fork.clone()]),
    );
    receive_and_project(&right, "source", &make_exchange(vec![fork, original]));
    let left_status = left
        .replication_status(true, Some(TEST_FLEET), &[])
        .unwrap();
    let right_status = right
        .replication_status(true, Some(TEST_FLEET), &[])
        .unwrap();
    assert_eq!(left_status.received_envelopes, 2);
    assert_eq!(left_status.authority_digest, right_status.authority_digest);
    assert_eq!(left_status.graph_digest, right_status.graph_digest);
    assert_eq!(left_status.unknown_records, 1);
    assert_eq!(right_status.unknown_records, 1);
}


/// Status, the harness view and the latest claim of a kind fold in canonical order. Two nodes
/// that hold the same claims therefore agree even when the claims reached them in different
/// orders, which a checkpoint relies on when it drops claims that a later one replaced.
#[test]
fn nodes_that_received_the_same_claims_in_different_orders_agree() {
    let source = Store::open_memory("source").unwrap();
    let agent = "agent/source.worker";
    for (status, incarnation) in [
        ("running", "inc-0"),
        ("exited", "inc-0"),
        ("running", "inc-1"),
    ] {
        source
            .append_claim(&ClaimInput {
                subject: agent.into(),
                kind: "runtime.observed".into(),
                actor: None,
                fields: BTreeMap::from([
                    ("status".into(), Value::String(status.into())),
                    ("incarnation_id".into(), Value::String(incarnation.into())),
                ]),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: Some(format!("runtime:{status}:{incarnation}")),
            })
            .unwrap();
    }
    for (state, observed_at) in [("idle", 1_000), ("working", 2_000), ("idle", 3_000)] {
        source
            .append_claim_outcome(&harness_state(agent, state, observed_at))
            .unwrap();
    }
    let envelopes = exchange_from(&source, &ReplicationInventory::default()).envelopes;
    assert!(envelopes.len() >= 6);

    let in_order = Store::open_memory("in-order").unwrap();
    receive_and_project(
        &in_order,
        "source",
        &exchange_of("source", envelopes.clone()),
    );
    // The other node admits every envelope in its own exchange, newest first, so its store
    // indexes run backwards.
    let reversed = Store::open_memory("reversed").unwrap();
    for envelope in envelopes.iter().rev() {
        receive_and_project(
            &reversed,
            "source",
            &exchange_of("source", vec![envelope.clone()]),
        );
    }
    let first = |store: &Store| {
        store
            .claims_for(agent, Some("runtime.observed"))
            .unwrap()
            .first()
            .map(|claim| claim.store_index)
    };
    assert!(
        first(&reversed) > first(&in_order),
        "the reversed node must have admitted the oldest claim last"
    );

    for store in [&in_order, &reversed] {
        let actual = store.latest_actual_value(agent).unwrap().unwrap();
        assert_eq!(actual["status"], "running");
        assert_eq!(actual["incarnation_id"], "inc-1");
        let harness = store.current_harness(agent).unwrap().unwrap();
        assert_eq!(harness.state, "idle");
    }
    assert_eq!(
        in_order
            .latest_claim(agent, Some("runtime.observed"))
            .unwrap()
            .unwrap()
            .id,
        reversed
            .latest_claim(agent, Some("runtime.observed"))
            .unwrap()
            .unwrap()
            .id,
    );
    let ids = |store: &Store| {
        store
            .claims_for(agent, None)
            .unwrap()
            .into_iter()
            .map(|claim| claim.id)
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(&in_order), ids(&reversed));
    assert_eq!(
        in_order
            .latest_claim(agent, Some("harness.observed"))
            .unwrap()
            .unwrap()
            .body["fields"]["state"],
        "idle"
    );
    assert_eq!(
        reversed
            .latest_claim(agent, Some("harness.observed"))
            .unwrap()
            .unwrap()
            .body["fields"]["state"],
        "idle"
    );
}

#[test]
fn the_same_envelopes_projecting_different_graphs_diverge() {
    let left = Store::open_memory("left").unwrap();
    let right = Store::open_memory("right").unwrap();
    left.capture_divergences(true);
    right.capture_divergences(true);
    let intent = simple("true");
    let preview = left
        .mission(
            &intent,
            IntentInput {
                kdl: "work".into(),
                source_name: None,
            },
        )
        .unwrap();
    left.apply(&intent, &preview.subject_tokens, "work")
        .unwrap();
    receive_and_project(
        &right,
        "left",
        &exchange_from(&left, &right.replication_inventory().unwrap()),
    );
    receive_and_project(
        &left,
        "right",
        &exchange_from(&right, &left.replication_inventory().unwrap()),
    );
    // Each exchange a worker starts carries its node's inventory and graph digest.
    let summary = |from: &Store, to: &Store, relay: &str| {
        let exchange = from.export_replication_summary(TEST_FLEET).unwrap();
        to.receive_replication_exchange(relay, TEST_FLEET, &exchange)
            .unwrap();
        to.replication_peer_sync(&[relay.to_owned()])
            .remove(relay)
            .unwrap()
    };
    let synced = summary(&left, &right, "left");
    assert_eq!(
        (synced.peer_only_envelopes, synced.local_only_envelopes),
        (0, 0)
    );
    assert!(synced.graph_compared_at_unix_ms.is_some());
    assert_eq!(synced.graph_differs_since_unix_ms, None);
    assert!(!synced.diverged);

    // Right loses the desired claim while keeping its envelope, as a node that drops
    // received claims would: the inventories still match, the graphs no longer do.
    {
        let connection = right.connection.lock().unwrap();
        connection
            .execute_batch(
                /* sqlite-large-value: allow-for-now lv-hfb8131659b25 owner=fractal_store::store::tests TODO=chunk-or-reference-variable-columns */ "CREATE TEMP TABLE dropped_claims AS
                         SELECT * FROM claims WHERE subject='exec/work' AND kind='intent.desired';
                     CREATE TEMP TABLE dropped_desired AS
                         SELECT * FROM desired WHERE subject='exec/work';
                     PRAGMA foreign_keys=OFF;
                     DELETE FROM desired WHERE subject='exec/work';
                     DELETE FROM claims WHERE subject='exec/work' AND kind='intent.desired';
                     PRAGMA foreign_keys=ON;",
            )
            .unwrap();
    }
    assert_eq!(
        left.replication_inventory().unwrap().digest,
        right.replication_inventory().unwrap().digest
    );
    let differs = summary(&left, &right, "left");
    let since = differs.graph_differs_since_unix_ms.expect("graphs differ");
    assert_eq!(differs.graph_compared_at_unix_ms, Some(since));
    assert!(
        !differs.diverged,
        "one comparison can catch a peer mid-projection"
    );
    assert!(!right.replication_diverged());
    assert!(
        summary(&right, &left, "right")
            .graph_differs_since_unix_ms
            .is_some(),
        "the peer sees the difference too"
    );

    // A difference that outlasts any projection delay is divergence.
    right
        .replication_sync
        .lock()
        .unwrap()
        .get_mut("left")
        .unwrap()
        .graph_differs_since_unix_ms = Some(since - REPLICATION_DIVERGED_AFTER_MS);
    let diverged = summary(&left, &right, "left");
    assert!(diverged.diverged);
    assert_eq!(
        diverged.graph_differs_since_unix_ms,
        Some(since - REPLICATION_DIVERGED_AFTER_MS)
    );
    assert!(right.replication_diverged());
    let status = right
        .replication_status(true, Some(TEST_FLEET), &["left".to_owned()])
        .unwrap();
    assert!(status.peers[0].sync.as_ref().unwrap().diverged);
    assert_ne!(
        status.peers[0].graph_digest.as_deref(),
        Some(status.graph_digest.as_str())
    );

    // Different envelopes cannot be compared, so they leave the last comparison standing.
    let compared = diverged.graph_compared_at_unix_ms;
    left.record_transport_observation("right", "up", None, None)
        .unwrap();
    let moved = summary(&left, &right, "left");
    assert_eq!(moved.peer_only_envelopes, 1);
    assert_eq!(moved.graph_compared_at_unix_ms, compared);
    assert!(moved.diverged);

    // Once the same envelopes project the same graph again, the divergence clears.
    receive_and_project(
        &right,
        "left",
        &exchange_from(&left, &right.replication_inventory().unwrap()),
    );
    {
        let connection = right.connection.lock().unwrap();
        connection
            .execute_batch(
                /* sqlite-large-value: allow-for-now lv-hb7696ce642f6 owner=fractal_store::store::tests TODO=prepare-body-and-predecessor-values */ "PRAGMA foreign_keys=OFF;
                     INSERT INTO claims SELECT * FROM dropped_claims;
                     INSERT INTO desired SELECT * FROM dropped_desired;
                     PRAGMA foreign_keys=ON;",
            )
            .unwrap();
    }
    let healed = summary(&left, &right, "left");
    assert_eq!(
        (healed.peer_only_envelopes, healed.local_only_envelopes),
        (0, 0)
    );
    assert_eq!(healed.graph_differs_since_unix_ms, None);
    assert!(!healed.diverged);
    assert!(!right.replication_diverged());
}

#[test]
fn concurrent_graph_writes_converge_after_a_partition() {
    let left = Store::open_memory("left").unwrap();
    let right = Store::open_memory("right").unwrap();
    let initial = simple("true");
    let initial_preview = left
        .mission(
            &initial,
            IntentInput {
                kdl: "initial".into(),
                source_name: None,
            },
        )
        .unwrap();
    left.apply(&initial, &initial_preview.subject_tokens, "initial")
        .unwrap();
    let initial_exchange = exchange_from(&left, &ReplicationInventory::default());
    receive_and_project(&right, "left", &initial_exchange);

    let left_change = simple("printf-left");
    let left_preview = left
        .mission(
            &left_change,
            IntentInput {
                kdl: "left".into(),
                source_name: None,
            },
        )
        .unwrap();
    left.apply(&left_change, &left_preview.subject_tokens, "left-change")
        .unwrap();

    let right_change = simple("printf-right");
    let right_preview = right
        .mission(
            &right_change,
            IntentInput {
                kdl: "right".into(),
                source_name: None,
            },
        )
        .unwrap();
    right
        .apply(&right_change, &right_preview.subject_tokens, "right-change")
        .unwrap();

    let to_right = exchange_from(&left, &right.replication_inventory().unwrap());
    receive_and_project(&right, "left", &to_right);
    let to_left = exchange_from(&right, &left.replication_inventory().unwrap());
    receive_and_project(&left, "right", &to_left);

    let left_status = left
        .replication_status(true, Some(TEST_FLEET), &[])
        .unwrap();
    let right_status = right
        .replication_status(true, Some(TEST_FLEET), &[])
        .unwrap();
    assert_eq!(left_status.authority_digest, right_status.authority_digest);
    assert_eq!(left_status.graph_digest, right_status.graph_digest);
    assert_eq!(
        left.selected_desired_revision("exec/work").unwrap(),
        right.selected_desired_revision("exec/work").unwrap()
    );
}

#[test]
fn an_explicit_repair_keeps_the_bad_record_and_names_its_replacement() {
    let source = Store::open_memory("source").unwrap();
    let replacement = source
        .append_claim(&ClaimInput {
            subject: "host/source".into(),
            kind: "transport.observed".into(),
            actor: None,
            fields: BTreeMap::from([("status".into(), Value::String("up".into()))]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: Some("repair-replacement".into()),
        })
        .unwrap();
    let mut exchange = exchange_from(&source, &ReplicationInventory::default());
    let original = exchange.envelopes[0].clone();
    let broken = rewrite_envelope(&original, |payload| {
        let claim = &mut payload.batch.claims[0];
        claim.body["fields"]["unexpected"] = Value::Bool(true);
        claim.id = claim_hash(
            &claim.batch_id,
            &claim.subject,
            &claim.kind,
            &claim.origin,
            claim.actor.as_deref(),
            &claim.body,
            &claim.predecessors,
        )
        .unwrap();
    });
    exchange.envelopes = vec![original, broken];
    let target = Store::open_memory("target").unwrap();
    receive_and_project(&target, "source", &exchange);
    let record = target.replica_records(true).unwrap().remove(0);
    let first = target
        .repair_replica_record(
            &record.record_ref,
            &replacement.id,
            "replace the malformed observation",
            "person/operator",
            "repair-once",
        )
        .unwrap();
    let retry = target
        .repair_replica_record(
            &record.record_ref,
            &replacement.id,
            "replace the malformed observation",
            "person/operator",
            "repair-once",
        )
        .unwrap();
    assert_eq!(first.id, retry.id);
    let repaired = target.replica_record(&record.record_ref).unwrap().unwrap();
    assert_eq!(repaired.state, "repaired");
    assert_eq!(repaired.replacement_claim_id, Some(replacement.id));
    assert!(target.replica_records(true).unwrap().is_empty());

    let repair_exchange = exchange_from(&target, &source.replication_inventory().unwrap());
    receive_and_project(&source, "target", &repair_exchange);
    let replicated = source.replica_record(&record.record_ref).unwrap().unwrap();
    assert_eq!(replicated.state, "repaired");
    assert!(source.replica_records(true).unwrap().is_empty());
}

#[test]
fn a_replicated_repair_replaces_a_record_that_the_source_had_accepted() {
    let source = Store::open_memory("source").unwrap();
    let target = Store::open_memory("target").unwrap();

    let initial = simple("true");
    let preview = source
        .mission(
            &initial,
            IntentInput {
                kdl: "initial".into(),
                source_name: None,
            },
        )
        .unwrap();
    source
        .apply(&initial, &preview.subject_tokens, "initial")
        .unwrap();
    receive_and_project(
        &target,
        "source",
        &exchange_from(&source, &target.replication_inventory().unwrap()),
    );
    let replacement = source.selected_desired_token("exec/work").unwrap().unwrap();

    let changed = simple("printf changed");
    let preview = source
        .mission(
            &changed,
            IntentInput {
                kdl: "changed".into(),
                source_name: None,
            },
        )
        .unwrap();
    source
        .apply(&changed, &preview.subject_tokens, "changed")
        .unwrap();
    receive_and_project(
        &target,
        "source",
        &exchange_from(&source, &target.replication_inventory().unwrap()),
    );
    let repaired_claim = source.selected_desired_token("exec/work").unwrap().unwrap();
    assert_ne!(repaired_claim, replacement);

    let record_ref = {
        let connection = target.connection.lock().unwrap();
        let record_ref = connection
            .query_row(
                "SELECT record_ref FROM replica_records WHERE claim_id=?1",
                [&repaired_claim],
                |row| row.get::<_, String>(0),
            )
            .unwrap();
        connection
            .execute(
                /* sqlite-large-value: allow-for-now lv-he3371a6eb157 owner=fractal_store::store::tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_records SET state='invalid' WHERE record_ref=?1",
                [&record_ref],
            )
            .unwrap();
        record_ref
    };
    target
        .repair_replica_record(
            &record_ref,
            &replacement,
            "the receiver rejects this record after an upgrade",
            "person/operator",
            "repair-version-skew",
        )
        .unwrap();
    assert_eq!(
        target.selected_desired_token("exec/work").unwrap().unwrap(),
        replacement
    );
    // A build that rejected the original never put it in claims. Keep the raw repaired
    // record, but model that admission difference explicitly.
    target
        .connection
        .lock()
        .unwrap()
        .execute(/* sqlite-large-value: allow-for-now lv-h2f237fd3f1aa owner=fractal_store::store::tests TODO=prepare-body-and-predecessor-values */ "DELETE FROM claims WHERE id=?1", [&repaired_claim])
        .unwrap();

    receive_and_project(
        &source,
        "target",
        &exchange_from(&target, &source.replication_inventory().unwrap()),
    );
    assert_eq!(
        source.selected_desired_token("exec/work").unwrap().unwrap(),
        replacement
    );
    let repaired = source
        .replica_records(false)
        .unwrap()
        .into_iter()
        .find(|record| record.claim_id.as_deref() == Some(repaired_claim.as_str()))
        .unwrap();
    assert_eq!(repaired.state, "repaired");
    assert_eq!(
        repaired.replacement_claim_id.as_deref(),
        Some(replacement.as_str())
    );

    let source_status = source
        .replication_status(true, Some(TEST_FLEET), &[])
        .unwrap();
    let target_status = target
        .replication_status(true, Some(TEST_FLEET), &[])
        .unwrap();
    assert_eq!(
        source_status.authority_digest,
        target_status.authority_digest
    );
    assert_eq!(source_status.graph_digest, target_status.graph_digest);
}

proptest! {
    #[test]
    fn envelope_sets_converge_after_sparse_duplicate_delivery(
        reverse in any::<bool>(),
        copies in 1usize..4,
        split_seed in 0usize..8,
    ) {
        let source = Store::open_memory("source").unwrap();
        for index in 0..4 {
            source
                .append_claim(&ClaimInput {
                    subject: format!("host/source-{index}"),
                    kind: "transport.observed".into(),
                    actor: None,
                    fields: BTreeMap::from([("status".into(), Value::String("up".into()))]),
                    evidence: Vec::new(),
                    expected_subject: None,
                    idempotency_key: Some(format!("source-{index}")),
                })
                .unwrap();
        }
        let complete = exchange_from(&source, &ReplicationInventory::default());
        let mut delivered = complete.envelopes.clone();
        if reverse {
            delivered.reverse();
        }
        let split = split_seed % (delivered.len() + 1);
        let second = delivered.split_off(split);
        let duplicate = |part: &[ReplicaEnvelope]| {
            let mut envelopes = Vec::new();
            for envelope in part {
                envelopes.extend(std::iter::repeat_n(envelope.clone(), copies));
            }
            ReplicationExchange {
                envelopes,
                ..complete.clone()
            }
        };
        let target = Store::open_memory("target").unwrap();
        receive_and_project(&target, "source", &duplicate(&delivered));
        receive_and_project(&target, "source", &duplicate(&second));
        receive_and_project(&target, "source", &complete);
        let source_status = source
            .replication_status(true, Some(TEST_FLEET), &[])
            .unwrap();
        let target_status = target
            .replication_status(true, Some(TEST_FLEET), &[])
            .unwrap();
        prop_assert_eq!(source_status.authority_digest, target_status.authority_digest);
        prop_assert_eq!(source_status.graph_digest, target_status.graph_digest);
        prop_assert_eq!(target_status.pending_records, 0);
        prop_assert_eq!(target_status.invalid_records, 0);
        prop_assert_eq!(target_status.unknown_records, 0);
    }
}

#[test]
fn replication_rejects_tampered_claims() {
    let source = Store::open_memory("source").unwrap();
    source
        .append_claim(&ClaimInput {
            subject: "host/source".into(),
            kind: "transport.observed".into(),
            actor: None,
            fields: BTreeMap::from([("status".into(), Value::String("up".into()))]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
    let mut batch = source.export_replication(0).unwrap();
    batch.batches[0].claims[0].body["fields"]["status"] = Value::String("down".into());
    let target = Store::open_memory("target").unwrap();
    let error = target
        .import_replication("source", &batch)
        .expect_err("tampering must fail");
    assert_eq!(error.code, "claim-hash-mismatch");
}

/// One source claim whose body lists its fields in reverse order and whose ID is the hash of
/// that order, the way builds between eaec66a and 2537978d wrote claims on 2026-09-16.
fn insertion_order_hashed_batch() -> ReplicationBatch {
    let source = Store::open_memory("source").unwrap();
    source
        .append_claim(&ClaimInput {
            subject: "host/source".into(),
            kind: "transport.observed".into(),
            actor: None,
            fields: BTreeMap::from([
                ("protocol".into(), Value::String("http-replication".into())),
                ("status".into(), Value::String("up".into())),
            ]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
    let mut batch = source.export_replication(0).unwrap();
    let claim = &mut batch.batches[0].claims[0];
    let reversed = |value: &Value| {
        let mut fields = value.as_object().unwrap().iter().collect::<Vec<_>>();
        fields.sort_by(|left, right| right.0.cmp(left.0));
        Value::Object(
            fields
                .into_iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
        )
    };
    let mut body = reversed(&claim.body);
    body["fields"] = reversed(&claim.body["fields"]);
    claim.body = body;
    claim.id = canonical_hash(&(
        &claim.batch_id,
        &claim.subject,
        &claim.kind,
        &claim.origin,
        claim.actor.as_deref(),
        &claim.body,
        &claim.predecessors,
    ))
    .unwrap();
    batch
}

#[test]
fn a_claim_hashed_in_field_insertion_order_still_verifies() {
    let batch = insertion_order_hashed_batch();
    let claim = &batch.batches[0].claims[0];
    let canonical = claim_hash(
        &claim.batch_id,
        &claim.subject,
        &claim.kind,
        &claim.origin,
        claim.actor.as_deref(),
        &claim.body,
        &claim.predecessors,
    )
    .unwrap();
    assert_ne!(canonical, claim.id, "the fixture must not be canonical");
    assert!(claim_id_is_content_hash(claim).unwrap());

    // A new node decodes the body from CBOR. It must keep the writer's field order.
    let mut bytes = Vec::new();
    ciborium::into_writer(claim, &mut bytes).unwrap();
    let decoded: ClaimRecord = ciborium::from_reader(bytes.as_slice()).unwrap();
    assert!(claim_id_is_content_hash(&decoded).unwrap());

    let mut tampered = decoded;
    tampered.body["fields"]["status"] = Value::String("down".into());
    assert!(!claim_id_is_content_hash(&tampered).unwrap());

    let target = Store::open_memory("target").unwrap();
    target.import_replication("source", &batch).unwrap();
    assert!(target.claim_by_id(&claim.id).unwrap().is_some());
}

#[test]
fn a_node_that_rejected_insertion_order_hashes_admits_them_once_upgraded() {
    pub const FLEET: &str = "018f6f0d-4a5d-7b8c-9d0e-123456789abc";
    let envelope = |batch: &ReplicaBatch| {
        let mut bytes = Vec::new();
        ciborium::into_writer(
            &ReplicaEnvelopePayload {
                batch: batch.clone(),
                blobs: BTreeMap::new(),
                claim_signatures: BTreeMap::new(),
            },
            &mut bytes,
        )
        .unwrap();
        ReplicaEnvelope {
            writer: batch.origin.clone(),
            sequence: batch.replica_sequence,
            previous_hash: batch.previous_hash.clone(),
            hash: replica_envelope_hash(
                &batch.origin,
                batch.replica_sequence,
                batch.previous_hash.as_deref(),
                batch.accepted_at_unix_ms,
                &bytes,
            ),
            accepted_at_unix_ms: batch.accepted_at_unix_ms,
            payload: bytes.into(),
            member_key: None,
            signature: None,
        }
    };
    let receive = |store: &Store, envelope: ReplicaEnvelope| {
        store
            .receive_replication_exchange(
                "source",
                FLEET,
                &ReplicationExchange {
                    build: None,
                    projection_digests: Default::default(),
                    peer: "source".into(),
                    fleet_id: FLEET.into(),
                    schema_digest: String::new(),
                    projection_registry_digest: None,
                    projection_versions: Default::default(),
                    authority_digest: String::new(),
                    graph_digest: String::new(),
                    inventory: ReplicationInventory::default(),
                    envelopes: vec![envelope],
                    signature_requests: Vec::new(),
                    signatures: Vec::new(),
                },
            )
            .unwrap();
    };
    let batch = insertion_order_hashed_batch().batches.remove(0);
    let legacy = envelope(&batch);
    let target = Store::open_memory("target").unwrap();
    receive(&target, legacy.clone());
    // An older build rejected the genuine claim as a hash mismatch.
    record_invalid_replica_envelope(
        &target.connection.lock().unwrap(),
        &legacy,
        &St3Error::new("claim-hash-mismatch", "failed verification"),
    )
    .unwrap();
    assert_eq!(target.replica_records(true).unwrap().len(), 1);

    let admission = target.validate_replication_backlog().unwrap();
    assert_eq!(admission.invalid, 0);
    assert!(target.replica_records(true).unwrap().is_empty());
    assert!(target.claim_by_id(&batch.claims[0].id).unwrap().is_some());

    // A claim that fails both hashes stays invalid and is not checked again on every wake.
    let mut tampered = batch.clone();
    tampered.replica_sequence += 1;
    tampered.hash = batch_header_hash(
        &tampered.origin,
        tampered.replica_sequence,
        tampered.previous_hash.as_deref(),
        tampered.accepted_at_unix_ms,
    )
    .unwrap();
    tampered.id = format!(
        "batch/{}/{}/{}",
        tampered.origin, tampered.replica_sequence, tampered.hash
    );
    tampered.claims[0].batch_id = tampered.id.clone();
    tampered.claims[0].body["fields"]["status"] = Value::String("down".into());
    receive(&target, envelope(&tampered));
    assert_eq!(target.validate_replication_backlog().unwrap().invalid, 1);
    assert_eq!(target.validate_replication_backlog().unwrap().invalid, 0);
    assert_eq!(target.replica_records(true).unwrap().len(), 1);
}

#[test]
fn replication_rejects_a_registered_claim_that_violates_the_schema() {
    let source = Store::open_memory("source").unwrap();
    source
        .append_claim(&ClaimInput {
            subject: "host/source".into(),
            kind: "transport.observed".into(),
            actor: None,
            fields: BTreeMap::from([("status".into(), Value::String("up".into()))]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
    let mut batch = source.export_replication(0).unwrap();
    let claim = &mut batch.batches[0].claims[0];
    claim.body["fields"]["unexpected"] = Value::Bool(true);
    claim.id = claim_hash(
        &claim.batch_id,
        &claim.subject,
        &claim.kind,
        &claim.origin,
        claim.actor.as_deref(),
        &claim.body,
        &claim.predecessors,
    )
    .unwrap();

    let target = Store::open_memory("target").unwrap();
    let error = target
        .import_replication("source", &batch)
        .expect_err("a registered claim must satisfy the shared schema");
    assert_eq!(error.code, "invalid-replicated-claim");
    assert_eq!(target.index().unwrap(), 0);
}

#[test]
fn an_unknown_replicated_claim_marks_its_subject_indeterminate() {
    let source = Store::open_memory("source").unwrap();
    source
        .append_claim(&ClaimInput {
            subject: "resource/future".into(),
            kind: "resource.observed".into(),
            actor: None,
            fields: BTreeMap::from([
                ("kind".into(), Value::String("custom.st3.future".into())),
                ("status".into(), Value::String("active".into())),
            ]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: Some("future".into()),
        })
        .unwrap();
    let mut batch = source.export_replication(0).unwrap();
    let claim = &mut batch.batches[0].claims[0];
    claim.kind = "future.resource-observed".into();
    claim.id = claim_hash(
        &claim.batch_id,
        &claim.subject,
        &claim.kind,
        &claim.origin,
        claim.actor.as_deref(),
        &claim.body,
        &claim.predecessors,
    )
    .unwrap();

    let target = Store::open_memory("target").unwrap();
    target.import_replication("source", &batch).unwrap();
    let status = target.status(Some("resource/future")).unwrap();
    assert_eq!(status.subjects[0].reachability, "indeterminate");
    assert!(
        status.subjects[0]
            .reason
            .as_deref()
            .unwrap()
            .contains("future.resource-observed")
    );
}

#[test]
fn replication_relays_each_origin_across_multiple_peers() {
    let source = Store::open_memory("source").unwrap();
    source
        .append_claim(&ClaimInput {
            subject: "host/source".into(),
            kind: "transport.observed".into(),
            actor: None,
            fields: BTreeMap::from([("status".into(), Value::String("up".into()))]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: Some("source-up".into()),
        })
        .unwrap();

    let middle = Store::open_memory("middle").unwrap();
    middle
        .import_replication("source", &source.export_replication(0).unwrap())
        .unwrap();
    let relayed = middle
        .export_replication_for_heads(&BTreeMap::new())
        .unwrap();
    assert_eq!(relayed.peer, "middle");
    assert_eq!(relayed.batches[0].origin, "source");

    let target = Store::open_memory("target").unwrap();
    target.import_replication("middle", &relayed).unwrap();
    assert!(
        target
            .latest_claim("host/source", Some("transport.observed"))
            .unwrap()
            .is_some()
    );
    assert_eq!(target.replica_heads().unwrap().get("source"), Some(&1));
}

#[test]
fn a_later_publish_cites_and_resolves_all_concurrent_intent_heads() {
    let left = Store::open_memory("left").unwrap();
    let initial = simple("true");
    let mission = left
        .mission(
            &initial,
            IntentInput {
                kdl: "initial".into(),
                source_name: None,
            },
        )
        .unwrap();
    left.apply(&initial, &mission.subject_tokens, "initial")
        .unwrap();

    let right = Store::open_memory("right").unwrap();
    right
        .import_replication("left", &left.export_replication(0).unwrap())
        .unwrap();
    let left_change = simple("false");
    let right_change = simple("printf right");
    let left_mission = left
        .mission(
            &left_change,
            IntentInput {
                kdl: "left".into(),
                source_name: None,
            },
        )
        .unwrap();
    let right_mission = right
        .mission(
            &right_change,
            IntentInput {
                kdl: "right".into(),
                source_name: None,
            },
        )
        .unwrap();
    left.apply(&left_change, &left_mission.subject_tokens, "left")
        .unwrap();
    right
        .apply(&right_change, &right_mission.subject_tokens, "right")
        .unwrap();

    let left_heads = left.replica_heads().unwrap();
    let right_heads = right.replica_heads().unwrap();
    left.import_replication(
        "right",
        &right.export_replication_for_heads(&left_heads).unwrap(),
    )
    .unwrap();
    right
        .import_replication(
            "left",
            &left.export_replication_for_heads(&right_heads).unwrap(),
        )
        .unwrap();
    assert_eq!(
        left.status(Some("exec/work")).unwrap().subjects[0]
            .conflicts
            .len(),
        1
    );
    assert_eq!(
        right.status(Some("exec/work")).unwrap().subjects[0]
            .conflicts
            .len(),
        1
    );

    let resolved = simple("printf resolved");
    let resolution_mission = left
        .mission(
            &resolved,
            IntentInput {
                kdl: "resolved".into(),
                source_name: None,
            },
        )
        .unwrap();
    assert_eq!(resolution_mission.subject_tokens["exec/work"].len(), 2);
    left.apply(&resolved, &resolution_mission.subject_tokens, "resolved")
        .unwrap();
    let right_heads = right.replica_heads().unwrap();
    right
        .import_replication(
            "left",
            &left.export_replication_for_heads(&right_heads).unwrap(),
        )
        .unwrap();
    assert!(
        left.status(Some("exec/work")).unwrap().subjects[0]
            .conflicts
            .is_empty()
    );
    assert!(
        right.status(Some("exec/work")).unwrap().subjects[0]
            .conflicts
            .is_empty()
    );
    assert_eq!(
        left.status(Some("exec/work")).unwrap().subjects[0].desired_revision,
        right.status(Some("exec/work")).unwrap().subjects[0].desired_revision
    );
}

#[test]
fn capabilities_are_one_use_and_expire() {
    let store = Store::open_memory("node").unwrap();
    let (secret, _) = store
        .issue_capability("terminal", "agent/node.worker", Some("one"), 1_000)
        .unwrap();
    assert!(!store.consume_capability(&secret, "terminal").unwrap().used);
    assert!(store.consume_capability(&secret, "terminal").unwrap().used);

    let (expired, _) = store
        .issue_capability("terminal", "agent/node.worker", Some("one"), 0)
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(2));
    assert_eq!(
        store
            .consume_capability(&expired, "terminal")
            .expect_err("expired capability")
            .code,
        "expired-capability"
    );
}

/// The message index follows the claim log: each message's first claim, closed once a claim
/// closes it. A message whose claims a checkpoint removes is indexed again from the rest, and
/// a store written before the index existed fills it once from its claims.
#[test]
fn the_message_index_follows_the_message_claims() {
    let store = Store::open_memory("node").unwrap();
    let append = |subject: &str, kind: &str, status: &str, actor: &str| {
        store
            .append_claim(&ClaimInput {
                subject: subject.into(),
                kind: kind.into(),
                actor: Some(actor.into()),
                fields: BTreeMap::from([("status".into(), Value::String(status.into()))]),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: Some(format!("{subject}-{kind}")),
            })
            .unwrap()
    };
    for message in 0..3 {
        append(
            &format!("message/index-{message}"),
            "message.sent",
            "sent",
            "requester",
        );
    }
    for (kind, status) in [
        ("message.delivered", "delivered"),
        ("message.read", "read"),
        ("message.closed", "closed"),
    ] {
        append("message/index-0", kind, status, "agent/worker");
    }
    let indexed = |store: &Store| {
        let connection = store.readers.get();
        let rows = |sql: &str| {
            connection
                .prepare(/* sqlite-large-value: allow-for-now lv-h9eb319f5de00 owner=fractal_store::store::tests TODO=trace-statement-and-value-representation */ sql)
                .unwrap()
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, u64>(1)?,
                        row.get::<_, bool>(2)?,
                    ))
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap()
        };
        (
            rows("SELECT subject, created_index, closed FROM message_index ORDER BY subject"),
            rows(
                "SELECT subject, MIN(store_index), MAX(kind = 'message.closed') FROM claims
                     WHERE subject LIKE 'message/%' GROUP BY subject ORDER BY subject",
            ),
        )
    };
    let (index, claims) = indexed(&store);
    assert_eq!(index, claims);
    assert_eq!(index.iter().filter(|(_, _, closed)| *closed).count(), 1);
    let open = store
        .messages(None, false)
        .unwrap()
        .into_iter()
        .map(|message| message.subject)
        .collect::<Vec<_>>();
    assert_eq!(open, ["message/index-1", "message/index-2"]);
    assert_eq!(store.messages(None, true).unwrap().len(), 3);

    // A checkpoint removes the rows that refer to a claim before the claim itself.
    store
        .connection
        .write()
        .execute_batch(
            /* sqlite-large-value: allow-for-now lv-h8fe114b2ad5c owner=fractal_store::store::tests TODO=prepare-body-and-predecessor-values */ "PRAGMA foreign_keys = OFF;
                 DELETE FROM claims WHERE subject='message/index-0' AND kind='message.closed';
                 PRAGMA foreign_keys = ON;",
        )
        .unwrap();
    let (index, claims) = indexed(&store);
    assert_eq!(index, claims);
    assert_eq!(store.messages(None, false).unwrap().len(), 3);

    {
        let connection = store.connection.write();
        connection
            .execute_batch(
                /* sqlite-large-value: allow-for-now lv-hb345d2d573cd owner=fractal_store::store::tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM message_index; DELETE FROM meta WHERE key='message_index';",
            )
            .unwrap();
        backfill_message_index(&connection).unwrap();
    }
    let (index, claims) = indexed(&store);
    assert_eq!(index, claims);
}

#[test]
fn message_lifecycle_exposes_native_staging_without_breaking_direct_delivery() {
    let store = Store::open_memory("node").unwrap();
    let subject = "message/demo";
    let append = |kind: &str, status: &str, actor: Option<&str>, key: &str| {
        store.append_claim(&ClaimInput {
            subject: subject.into(),
            kind: kind.into(),
            actor: actor.map(str::to_owned),
            fields: BTreeMap::from([("status".into(), Value::String(status.into()))]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: Some(key.into()),
        })
    };
    append("message.sent", "sent", Some("requester"), "sent").unwrap();
    assert_eq!(
        append("message.closed", "closed", Some("agent/worker"), "early")
            .expect_err("close must not skip states")
            .code,
        "invalid-message-transition"
    );
    append("message.staged", "staged", Some("agent/worker"), "staged").unwrap();
    assert_eq!(store.messages(None, true).unwrap()[0].status, "staged");
    append(
        "message.delivered",
        "delivered",
        Some("agent/worker"),
        "delivered",
    )
    .unwrap();
    let read = append("message.read", "read", Some("agent/worker"), "read").unwrap();
    let repeated_read =
        append("message.read", "read", Some("agent/worker"), "read-again").unwrap();
    assert_eq!(repeated_read.id, read.id);
    let closed = append("message.closed", "closed", Some("agent/worker"), "closed").unwrap();
    let repeated_close = append(
        "message.closed",
        "closed",
        Some("agent/worker"),
        "closed-again",
    )
    .unwrap();
    assert_eq!(repeated_close.id, closed.id);
    assert_eq!(store.messages(None, true).unwrap()[0].status, "closed");
    assert!(store.messages(None, false).unwrap().is_empty());

    let direct = "message/direct";
    let append_direct = |kind: &str, status: &str, key: &str| {
        store.append_claim(&ClaimInput {
            subject: direct.into(),
            kind: kind.into(),
            actor: Some("agent/worker".into()),
            fields: BTreeMap::from([("status".into(), Value::String(status.into()))]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: Some(key.into()),
        })
    };
    append_direct("message.sent", "sent", "direct-sent").unwrap();
    append_direct("message.delivered", "delivered", "direct-delivered").unwrap();
}

#[test]
fn only_the_person_recipient_can_read_before_native_delivery() {
    let store = Store::open_memory("person-read").unwrap();
    for (id, to) in [
        ("sent", "person/avery"),
        ("staged", "person/avery"),
        ("native", "agent/example/keeper"),
    ] {
        let subject = format!("message/{id}");
        store
            .append_claim(&ClaimInput {
                subject: subject.clone(),
                kind: "message.sent".into(),
                actor: Some("agent/example/sender".into()),
                fields: BTreeMap::from([
                    ("status".into(), Value::String("sent".into())),
                    ("from".into(), Value::String("agent/example/sender".into())),
                    ("to".into(), Value::String(to.into())),
                ]),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
        let receipt = |actor: &str, status: &str| {
            store.append_claim(&ClaimInput {
                subject: subject.clone(),
                kind: format!("message.{status}"),
                actor: Some(actor.into()),
                fields: BTreeMap::from([("status".into(), Value::String(status.into()))]),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: None,
            })
        };
        for actor in ["person/other", "agent/example/sender"] {
            assert_eq!(
                receipt(actor, "read").unwrap_err().code,
                "invalid-message-transition"
            );
            assert_eq!(store.message(&subject).unwrap().unwrap().status, "sent");
        }
        if id == "native" {
            assert_eq!(
                receipt(to, "read").unwrap_err().code,
                "invalid-message-transition"
            );
            continue;
        }
        if id == "staged" {
            receipt(to, "staged").unwrap();
        }
        let first = receipt(to, "read").unwrap();
        assert_eq!(receipt(to, "read").unwrap().id, first.id);
        assert_eq!(store.message(&subject).unwrap().unwrap().status, "read");
    }
}

#[test]
fn concurrent_message_close_converges_to_one_claim() {
    let store = Arc::new(Store::open_memory("node").unwrap());
    let subject = "message/concurrent-close";
    for (kind, status, key) in [
        ("message.sent", "sent", "concurrent-sent"),
        ("message.delivered", "delivered", "concurrent-delivered"),
        ("message.read", "read", "concurrent-read"),
    ] {
        store
            .append_claim(&ClaimInput {
                subject: subject.into(),
                kind: kind.into(),
                actor: Some("agent/worker".into()),
                fields: BTreeMap::from([("status".into(), Value::String(status.into()))]),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: Some(key.into()),
            })
            .unwrap();
    }
    let barrier = Arc::new(std::sync::Barrier::new(8));
    let threads = (0..8)
        .map(|index| {
            let store = store.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                store.append_claim(&ClaimInput {
                    subject: subject.into(),
                    kind: "message.closed".into(),
                    actor: Some("agent/worker".into()),
                    fields: BTreeMap::from([("status".into(), Value::String("closed".into()))]),
                    evidence: Vec::new(),
                    expected_subject: None,
                    idempotency_key: Some(format!("concurrent-close-{index}")),
                })
            })
        })
        .collect::<Vec<_>>();
    let records = threads
        .into_iter()
        .map(|thread| thread.join().unwrap().unwrap())
        .collect::<Vec<_>>();
    assert!(records.iter().all(|record| record.id == records[0].id));
    assert_eq!(
        store
            .claims_for(subject, Some("message.closed"))
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn an_old_nonempty_database_is_rejected() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.sqlite3");
    let connection = Connection::open(&path).unwrap();
    connection
        .execute(/* sqlite-large-value: allow-for-now lv-h634aebf81624 owner=fractal_store::store::tests TODO=chunk-or-reference-variable-columns */ "CREATE TABLE desired(subject TEXT PRIMARY KEY)", [])
        .unwrap();
    drop(connection);

    let error = Store::open(&path, "node")
        .err()
        .expect("the old schema must be rejected");
    assert!(error.to_string().contains("unsupported st schema"));
}

#[test]
fn schema_version_nine_requires_fresh_state_for_version_ten() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.sqlite3");
    let connection = Connection::open(&path).unwrap();
    connection
        .execute(/* sqlite-large-value: allow-for-now lv-hb2b8598e33bd owner=fractal_store::store::tests TODO=chunk-or-reference-variable-columns */ "CREATE TABLE marker(value TEXT)", [])
        .unwrap();
    connection.pragma_update(None, "user_version", 9).unwrap();
    drop(connection);

    let error = Store::open(&path, "node")
        .err()
        .expect("schema version 9 must be rejected");
    assert!(error.to_string().contains("unsupported st schema"));
}

#[test]
fn schema_version_ten_upgrades_additively_for_mission_deadlines() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.sqlite3");
    drop(Store::open(&path, "node").unwrap());

    let connection = Connection::open(&path).unwrap();
    connection
        .execute(/* sqlite-large-value: allow-for-now lv-h31e6b9a5b373 owner=fractal_store::store::tests TODO=bound-or-reference-values-and-reclamation */ "DROP TABLE mission_run_deadlines", [])
        .unwrap();
    connection.pragma_update(None, "user_version", 10).unwrap();
    drop(connection);

    drop(Store::open(&path, "node").unwrap());
    let connection = Connection::open(&path).unwrap();
    assert_eq!(
        connection
            .query_row("PRAGMA user_version", [], |row| row.get::<_, u32>(0))
            .unwrap(),
        fractal_schema::STORAGE_VERSION
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='mission_run_deadlines'",
                [],
                |row| row.get::<_, u32>(0),
            )
            .unwrap(),
        1
    );
}

#[test]
fn schema_version_eleven_removes_writer_sequence_and_pending_proposal_indexes() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.sqlite3");
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(
            /* sqlite-large-value: allow-for-now lv-h49bd0f5926e3 owner=fractal_store::store::tests TODO=chunk-or-reference-variable-columns */ "PRAGMA foreign_keys=ON;
                 CREATE TABLE batches (
                   id TEXT PRIMARY KEY,
                   origin TEXT NOT NULL,
                   replica_sequence INTEGER NOT NULL,
                   previous_hash TEXT,
                   hash TEXT NOT NULL,
                   accepted_at_unix_ms TEXT NOT NULL,
                   UNIQUE(origin, replica_sequence)
                 );
                 CREATE TABLE claims (
                   store_index INTEGER PRIMARY KEY AUTOINCREMENT,
                   id TEXT NOT NULL UNIQUE,
                   batch_id TEXT NOT NULL REFERENCES batches(id),
                   subject TEXT NOT NULL,
                   kind TEXT NOT NULL,
                   origin TEXT NOT NULL,
                   actor TEXT,
                   body TEXT NOT NULL,
                   predecessors TEXT NOT NULL,
                   accepted_at_unix_ms TEXT NOT NULL
                 );
                 PRAGMA user_version=11;",
        )
        .unwrap();
    drop(connection);

    let store = Store::open(&path, "node").unwrap();
    let connection = store.connection.lock().unwrap();
    connection
        .execute(
            /* sqlite-large-value: allow-for-now lv-h662fba43a1a8 owner=fractal_store::store::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO batches VALUES ('a','writer',1,NULL,'a','1')",
            [],
        )
        .unwrap();
    connection
        .execute(
            /* sqlite-large-value: allow-for-now lv-h96d94e7ed715 owner=fractal_store::store::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO batches VALUES ('b','writer',1,NULL,'b','1')",
            [],
        )
        .unwrap();
    let unique_indexes: u64 = connection
        .query_row(
            "SELECT COUNT(*) FROM pragma_index_list('batches') WHERE [unique]=1 AND origin='u'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(unique_indexes, 0, "the writer sequence index is not unique");
    assert_eq!(
        connection
            .query_row("PRAGMA user_version", [], |row| row.get::<_, u32>(0))
            .unwrap(),
        fractal_schema::STORAGE_VERSION
    );
}

#[test]
fn schema_version_twelve_adds_durable_planner_configuration() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.sqlite3");
    drop(Store::open(&path, "node").unwrap());
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(
            /* sqlite-large-value: allow-for-now lv-hee99517b937f owner=fractal_store::store::tests TODO=bound-or-reference-values-and-reclamation */ "DROP TRIGGER projection_digest_planning_sessions_insert;
                 DROP TRIGGER projection_digest_planning_sessions_update;
                 DROP TRIGGER projection_digest_planning_sessions_delete;
                 ALTER TABLE planning_sessions DROP COLUMN planner_spec_json;
                 PRAGMA user_version = 12;",
        )
        .unwrap();
    drop(connection);

    let store = Store::open(&path, "node").unwrap();
    let connection = store.connection.lock().unwrap();
    let version: u32 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    let planner_column: u32 = connection
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('planning_sessions') WHERE name='planner_spec_json'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(version, smallclaims::store::CURRENT_SCHEMA_VERSION);
    assert_eq!(planner_column, 1);
}

#[test]
fn the_current_schema_reopens() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.sqlite3");
    drop(Store::open(&path, "node").unwrap());
    Store::open(&path, "node").unwrap();
}

#[test]
fn replication_projection_uses_the_claim_position_index() {
    let store = Store::open_memory("node").unwrap();
    let connection = store.connection.lock().unwrap();
    let mut statement = connection
        .prepare(
            "EXPLAIN QUERY PLAN
                 SELECT claims.id,
                        COALESCE((SELECT MIN(position) FROM replica_records
                                  WHERE replica_records.claim_id=claims.id), 0)
                 FROM claims",
        )
        .unwrap();
    let plan = statement
        .query_map([], |row| row.get::<_, String>(3))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
        .join("\n");
    assert!(
        plan.contains("replica_records_claim"),
        "the projection query must use the claim position index:\n{plan}"
    );
}

#[test]
fn latest_predecessor_seeks_the_newest_time_block_and_preserves_ties() {
    let store = Store::open_memory("alder").unwrap();
    let mut connection = store.connection.write();
    let transaction = connection.transaction().unwrap();
    let mut expected = Vec::new();
    // Arrival order and claim ID deliberately disagree with canonical time and position.
    for (id, time, writer, sequence, batch) in [
        ("old", "99", "z", 9, "old-batch"),
        ("z-first", "100", "a", 1, "a-batch"),
        ("a-second", "100", "a", 1, "a-batch"),
        ("sequence", "100", "a", 2, "a-later"),
        ("writer", "100", "b", 1, "b-batch"),
        ("batch", "100", "b", 1, "z-batch"),
        ("late-arrival-old-time", "99", "z", 10, "older-time"),
    ] {
        transaction.execute(
            /* sqlite-large-value: allow-for-now lv-hb7bee44be324 owner=fractal_store::store::tests TODO=chunk-or-reference-exact-wire-values */ "INSERT OR IGNORE INTO batches(id,origin,replica_sequence,hash,accepted_at_unix_ms)
                 VALUES(?1,?2,?3,?1,?4)", params![batch, writer, sequence, time],
        ).unwrap();
        transaction.execute(
            /* sqlite-large-value: allow-for-now lv-h761ca6be608d owner=fractal_store::store::tests TODO=prepare-body-and-predecessor-values */ "INSERT INTO claims(id,batch_id,subject,kind,origin,body,predecessors,accepted_at_unix_ms)
                 VALUES(?1,?2,'daemon/alder','daemon.diagnostic',?3,'{}','[]',?4)",
            params![id, batch, writer, time],
        ).unwrap();
        expected.push(canonical::claim_key(&transaction, id).unwrap());
        assert_eq!(
            latest_claim_id_tx(&transaction, "daemon/alder").unwrap(),
            expected.iter().max().map(|key| key.5.clone())
        );
    }
    assert_eq!(
        latest_claim_id_tx(&transaction, "daemon/missing").unwrap(),
        None
    );
    let plan = transaction
        .prepare(/* sqlite-large-value: allow-for-now lv-hcd1c481d5eb2 owner=fractal_store::store::tests TODO=trace-statement-and-value-representation */ &format!(
            "EXPLAIN QUERY PLAN {}",
            canonical_sql(LATEST_CLAIM_QUERY)
        ))
        .unwrap()
        .query_map(["daemon/alder"], |row| row.get::<_, String>(3))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
        .join("\n");
    assert!(
        plan.contains("claims_subject_accepted_index (subject=?)"),
        "{plan}"
    );
    assert!(
        !plan.contains("USE TEMP B-TREE FOR ORDER BY"),
        "history must not be sorted: {plan}"
    );
}

#[test]
fn claims_of_one_kind_use_the_subject_kind_index() {
    let store = Store::open_memory("node").unwrap();
    let connection = store.connection.lock().unwrap();
    let mut statement = connection
        .prepare(/* sqlite-large-value: allow-for-now lv-h417aa229a9e1 owner=fractal_store::store::tests TODO=trace-statement-and-value-representation */ &format!(
            "EXPLAIN QUERY PLAN {}",
            claims_for_subject_query(true)
        ))
        .unwrap();
    let plan = statement
        .query_map(params!["agent/example", "harness.diagnostic"], |row| {
            row.get::<_, String>(3)
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
        .join("\n");
    assert!(
        plan.contains("claims_subject_kind_accepted_index (subject=? AND kind=?)")
            || plan.contains("claims_subject_kind_index (subject=? AND kind=?)"),
        "one kind of a subject's claims must not read every claim of the subject:\n{plan}"
    );
    drop(statement);
    drop(connection);

    let mut connection = store.connection.lock().unwrap();
    let transaction = connection.transaction().unwrap();
    for (kind, fields) in (0..20)
        .map(|sequence| {
            (
                "harness.observed",
                json!({"state": "working", "observed_at_ms": sequence}),
            )
        })
        .chain([(
            "harness.diagnostic",
            json!({"code": "provider-capacity", "status": "waiting"}),
        )])
    {
        append_claim_tx(
            &transaction,
            &store.origin,
            "agent/example",
            kind,
            None,
            &json!({ "fields": fields }),
            &[],
            None,
        )
        .unwrap();
    }
    transaction.commit().unwrap();
    drop(connection);
    let diagnostics = store
        .claims_for("agent/example", Some("harness.diagnostic"))
        .unwrap();
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].kind, "harness.diagnostic");
    assert_eq!(store.claims_for("agent/example", None).unwrap().len(), 21);
}

#[test]
fn mailbox_state_read_preserves_ended_diagnostic_work_and_legacy_semantics() {
    let store = Store::open_memory("alder").unwrap();
    let mut connection = store.connection.write();
    let transaction = connection.transaction().unwrap();
    let subject = "agent/example";
    let fence = crate::mailbox::Fence::new(subject, "current", "delivery");
    let mut sequence = 0;
    let mut insert = |kind: &str, fields: Value| {
        sequence += 1;
        let id = format!("claim-{sequence}");
        let time = sequence.to_string();
        transaction
            .execute(
                /* sqlite-large-value: allow-for-now lv-h3235673ff0ac owner=fractal_store::store::tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO batches(id,origin,replica_sequence,hash,accepted_at_unix_ms)
                 VALUES(?1,'alder',?2,?1,?3)",
                params![id, sequence, time],
            )
            .unwrap();
        transaction.execute(
            /* sqlite-large-value: allow-for-now lv-hec4e1414873d owner=fractal_store::store::tests TODO=prepare-body-and-predecessor-values */ "INSERT INTO claims(id,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms)
                 VALUES(?1,?1,?2,?3,'alder',?2,?4,'[]',?5)",
            params![id, subject, kind, json!({"fields": fields}).to_string(), time],
        ).unwrap();
    };
    insert(
        "runtime.observed",
        json!({"status":"running","incarnation_id":"current"}),
    );
    for (kind, fields, expected_ended) in [
        (
            "harness.observed",
            json!({"state":"ended","incarnation_id":"prior"}),
            false,
        ),
        (
            "harness.observed",
            json!({"state":"idle","incarnation_id":"current"}),
            false,
        ),
        (
            "harness.observed",
            json!({"state":"ended","incarnation_id":"current"}),
            true,
        ),
        (
            "harness.observed",
            json!({"driver":"claude","incarnation_id":"current"}),
            true,
        ),
        (
            "harness.diagnostic",
            json!({"code":"provider-auth-expired","incarnation_id":"current"}),
            false,
        ),
        (
            "harness.diagnostic",
            json!({"code":"provider-auth-restored","incarnation_id":"current"}),
            true,
        ),
        (
            "work.progress",
            json!({"claim_incarnation":"current"}),
            false,
        ),
        ("harness.observed", json!({"state":"ended"}), true),
        ("harness.observed", json!({"state":"idle"}), false),
    ] {
        insert(kind, fields);
        let ended = current_harness_at(&transaction, subject, None)
            .unwrap()
            .is_some_and(|harness| harness.state == "ended");
        assert_eq!(ended, expected_ended, "{kind}");
        assert_eq!(
            check_mailbox_incarnation(&transaction, &fence).is_err(),
            ended,
            "{kind}"
        );
    }
}

#[test]
fn subscription_run_lookup_cost_does_not_grow_with_unrelated_deliveries() {
    let store = Store::open_memory("orchid").unwrap();
    let mut connection = store.connection.write();
    let transaction = connection.transaction().unwrap();
    let mut costs = Vec::new();
    for n in 1..=10_000 {
        let id = format!("started-{n}");
        transaction
            .execute(
                /* sqlite-large-value: allow-for-now lv-h2e6ddfef3611 owner=fractal_store::store::tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO batches(id,origin,replica_sequence,hash,accepted_at_unix_ms)
                 VALUES(?1,'orchid',1,?1,'1')",
                [&id],
            )
            .unwrap();
        transaction.execute(
            /* sqlite-large-value: allow-for-now lv-h0702121b0c5d owner=fractal_store::store::tests TODO=prepare-body-and-predecessor-values */ "INSERT INTO claims(id,batch_id,subject,kind,origin,body,predecessors,accepted_at_unix_ms)
                 VALUES(?1,?1,'subscription/reviews','subscription.mission-started','orchid',?2,'[]','1')",
            params![id, json!({"fields":{"mission_run":format!("mission-run/{n}")}}).to_string()],
        ).unwrap();
        if n == 100 || n == 10_000 {
            transaction
                .prepare_cached(/* sqlite-large-value: allow-for-now lv-he29db82d26e6 owner=fractal_store::store::tests TODO=trace-statement-and-value-representation */ SUBSCRIPTION_STARTED_RUN_QUERY)
                .unwrap()
                .reset_status(rusqlite::StatementStatus::VmStep);
            assert!(subscription_started_run_tx(&transaction, "mission-run/1").unwrap());
            assert!(!subscription_started_run_tx(&transaction, "mission-run/authored").unwrap());
            costs.push(
                transaction
                    .prepare_cached(/* sqlite-large-value: allow-for-now lv-h31cbd87efccc owner=fractal_store::store::tests TODO=trace-statement-and-value-representation */ SUBSCRIPTION_STARTED_RUN_QUERY)
                    .unwrap()
                    .get_status(rusqlite::StatementStatus::VmStep),
            );
        }
    }
    assert!(
        costs[0] > 0 && costs[1] <= costs[0] * 2 && costs[1] < 100,
        "positive and absent subscription lookups must seek, not scan history: {costs:?}"
    );
}

#[test]
fn mailbox_fence_cost_does_not_grow_with_sparse_prior_incarnations() {
    let store = Store::open_memory("alder").unwrap();
    let mut connection = store.connection.write();
    let transaction = connection.transaction().unwrap();
    let insert = |id: &str, kind: &str, fields: Value, time: &str| {
        transaction
            .execute(
                /* sqlite-large-value: allow-for-now lv-hb90d9adc1d7c owner=fractal_store::store::tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO batches(id,origin,replica_sequence,hash,accepted_at_unix_ms)
                 VALUES(?1,'alder',1,?1,?2)",
                params![id, time],
            )
            .unwrap();
        transaction.execute(
            /* sqlite-large-value: allow-for-now lv-h8c192c7dff13 owner=fractal_store::store::tests TODO=prepare-body-and-predecessor-values */ "INSERT INTO claims(id,batch_id,subject,kind,origin,body,predecessors,accepted_at_unix_ms)
                 VALUES(?1,?1,'agent/example',?2,'alder',?3,'[]',?4)",
            params![id, kind, json!({"fields": fields}).to_string(), time],
        ).unwrap();
    };
    insert(
        "runtime",
        "runtime.observed",
        json!({"status":"running", "incarnation_id":"current"}),
        "20000",
    );
    insert(
        "current",
        "harness.observed",
        json!({"state":"idle", "incarnation_id":"current"}),
        "20001",
    );
    let fence = crate::mailbox::Fence::new("agent/example", "current", "delivery");
    // Count the actual statements used by the fence, including the previous display fold.
    // Cached statement reset does not reset SQLite's VM step counter.
    let queries = [
        newest_mailbox_harness_state_query(),
        newest_claims_of_kind_query("claims.id, claims.body", "harness.observed"),
        newest_claims_of_kind_query(
            "claims.id, claims.store_index, claims.body, claims.accepted_at_unix_ms",
            "harness.observed",
        ),
    ];
    let mut costs = Vec::new();
    for n in 1..=10_000 {
        insert(
            &format!("old-{n}"),
            "harness.observed",
            json!({"state":"working", "incarnation_id":"prior"}),
            &n.to_string(),
        );
        if n == 100 || n == 10_000 {
            for query in &queries {
                transaction
                    .prepare_cached(/* sqlite-large-value: allow-for-now lv-hdd535e1fd134 owner=fractal_store::store::tests TODO=trace-statement-and-value-representation */ query)
                    .unwrap()
                    .reset_status(rusqlite::StatementStatus::VmStep);
            }
            check_mailbox_incarnation(&transaction, &fence).unwrap();
            let cost: i32 = queries
                .iter()
                .map(|query| {
                    transaction
                        .prepare_cached(/* sqlite-large-value: allow-for-now lv-h4eafaa7fe1d3 owner=fractal_store::store::tests TODO=trace-statement-and-value-representation */ query)
                        .unwrap()
                        .get_status(rusqlite::StatementStatus::VmStep)
                })
                .sum();
            costs.push(cost);
        }
    }
    assert!(
        costs[0] > 0 && costs[1] <= costs[0] * 2 && costs[1] < 1_000,
        "a live fence must read its state, not its optional display history: {costs:?}"
    );
}

/// The reads behind the session list, a mission detail and the missions tree seek or walk an
/// index whose part they read is what they show: one incarnation's claims, the newest
/// observations, the open runs and steps. None reads or sorts every claim of a kind, every
/// claim of a subject, or every step and run the store holds.
/// A mailbox stream asks on every graph change whether its seat has a newer local observation
/// (`mailbox_changed_since`). That seeks the seat's newest observations; by subject and kind
/// it read every observation the seat ever had, 1.3 ms for a seat with 36,000 of them, about
/// 160 times a second on a busy member.
#[test]
fn a_mailbox_stream_seeks_its_seats_newer_local_observations() {
    let store = Store::open_memory("node").unwrap();
    let connection = store.readers.get();
    let plan = connection
        .prepare(
            "EXPLAIN QUERY PLAN SELECT EXISTS(SELECT 1 FROM local_observations
                 WHERE subject=?1 AND id>?2)",
        )
        .unwrap()
        .query_map(params!["agent/example", 0], |row| row.get::<_, String>(3))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
        .join("; ");
    assert!(plan.contains("(subject=? AND id>?)"), "{plan}");
}

/// Foreign keys are on, so deleting or rekeying a parent row looks up every child that names
/// it. A child column without an index made each lookup read the whole child table: deleting
/// a claim read every operation, and a checkpoint trim held the writer for an hour.
#[test]
fn every_foreign_key_column_leads_an_index() {
    let store = Store::open_memory("node").unwrap();
    let connection = store.readers.get();
    let tables = connection
        .prepare("SELECT name FROM sqlite_master WHERE type='table'")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    let mut unindexed = Vec::new();
    for table in tables {
        let references = connection
            .prepare(/* sqlite-large-value: allow-for-now lv-h9c42de773e5d owner=fractal_store::store::tests TODO=trace-statement-and-value-representation */ &format!("PRAGMA foreign_key_list(\"{table}\")"))
            .unwrap()
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>("from")?,
                    row.get::<_, String>("table")?,
                ))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        for (column, parent) in references {
            let leading = connection
                .prepare(/* sqlite-large-value: allow-for-now lv-h018f6e9cebf9 owner=fractal_store::store::tests TODO=trace-statement-and-value-representation */ &format!("PRAGMA index_list(\"{table}\")"))
                .unwrap()
                .query_map([], |row| row.get::<_, String>("name"))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap()
                .into_iter()
                .any(|index| {
                    connection
                        .query_row(
                            /* sqlite-large-value: allow-for-now lv-hfe331bf497fc owner=fractal_store::store::tests TODO=trace-statement-and-value-representation */ &format!(
                                "SELECT name FROM pragma_index_info('{index}') WHERE seqno=0"
                            ),
                            [],
                            |row| row.get::<_, String>(0),
                        )
                        .is_ok_and(|first| first == column)
                });
            if !leading {
                unindexed.push(format!("{table}.{column} -> {parent}"));
            }
        }
    }
    assert!(
        unindexed.is_empty(),
        "unindexed foreign keys: {unindexed:?}"
    );
}

#[test]
fn claude_attachment_lookup_cost_ignores_hook_runtime_and_predecessor_history() {
    let store = Store::open_memory("node").unwrap();
    let append = |kind: &str, incarnation: &str, code: &str, key: &str| {
        let fields = match kind {
            "harness.diagnostic" => json!({"incarnation_id":incarnation,
                "code":code,"status":"blocked","severity":"warning","reason":"cost fixture"}),
            "harness.observed" => json!({"incarnation_id":incarnation,"state":"idle","driver":"claude"}),
            "runtime.observed" => json!({"incarnation_id":incarnation,"status":"running",
                "runtime_id":"grove/cedar","reachability":"local"}),
            _ => unreachable!(),
        };
        store.append_claim(&ClaimInput {
            subject: "agent/grove/cedar".into(),
            kind: kind.into(), actor: None,
            fields: serde_json::from_value(fields).unwrap(),
            evidence: Vec::new(), expected_subject: None,
            idempotency_key: Some(key.into()),
        }).unwrap();
    };
    append("harness.diagnostic", "current", "claude-channel-unattached", "attach");
    let work = || {
        let connection = store.connection.lock().unwrap();
        let mut statement = connection.prepare(/* sqlite-large-value: allow-for-now lv-hf687fbbd30ec owner=fractal_store::store::tests TODO=trace-statement-and-value-representation */ &claude_attachment_query()).unwrap();
        let claim: String = statement.query_row(
            /* sqlite-large-value: allow-for-now lv-h007497d73c53 owner=fractal_store::store::tests TODO=trace-statement-and-value-representation */ params!["agent/grove/cedar", i64::MAX, "current"], |row| row.get(0),
        ).unwrap();
        (claim, statement.get_status(rusqlite::StatementStatus::VmStep))
    };
    let before = work();
    for index in 0..128 {
        for (kind, incarnation, code) in [
            ("harness.observed", "current", ""),
            ("runtime.observed", "current", ""),
            ("harness.diagnostic", "current", "native-delivery-recovered"),
            ("harness.diagnostic", "previous", "claude-channel-attached"),
        ] {
            append(kind, incarnation, code, &format!("history:{kind}:{incarnation}:{index}"));
        }
    }
    let after = work();
    assert_eq!(after.0, before.0);
    assert!(after.1 <= before.1 + 20, "attachment lookup grew with unrelated history: {before:?} -> {after:?}");
}

#[test]
fn unnamed_harness_seek_preserves_legacy_candidates_and_snapshot_cut() {
    let store = Store::open_memory("legacy-seek").unwrap();
    let mut connection = store.connection.write();
    let transaction = connection.transaction().unwrap();
    transaction
        .execute(
            /* sqlite-large-value: allow-for-now lv-ha175ed71501d owner=fractal_store::store::tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO batches(id,origin,replica_sequence,hash,accepted_at_unix_ms)
             VALUES ('legacy-batch','legacy-seek',1,'synthetic','1')",
            [],
        )
        .unwrap();
    let bodies = [
        json!({"fields":{"state":"idle"}}),
        json!({"state":"working","incarnation_id":null}),
        json!({"fields":{"state":"idle","incarnation_id":7}}),
        json!({"fields":{"state":"idle","incarnation_id":false}}),
        json!({"fields":{"state":"idle","incarnation_id":"current"}}),
        json!({"state":"idle","incarnation_id":"current"}),
        json!({"fields":{"state":"idle","incarnation_id":[7]}}),
        json!({"fields":null,"state":"idle","incarnation_id":"current"}),
        json!({"fields":{"reason":"sparse legacy fields"}}),
    ];
    for (position, body) in bodies.iter().enumerate() {
        let index = position + 1;
        transaction.execute(
            /* sqlite-large-value: allow-for-now lv-h3964e2fa19ae owner=fractal_store::store::tests TODO=prepare-body-and-predecessor-values */ "INSERT INTO claims(store_index,id,batch_id,subject,kind,origin,body,predecessors,accepted_at_unix_ms)
                 VALUES (?1,?2,'legacy-batch','agent/legacy-seek','harness.observed','legacy-seek',?3,'[]',?4)",
            params![index as i64,format!("legacy-{index}"),body.to_string(),index.to_string()],
        ).unwrap();
    }
    transaction.commit().unwrap();
    let read = |sql: &str, cut: i64, since: &str| {
        connection
            .prepare(/* sqlite-large-value: allow-for-now lv-hdce7deac19c1 owner=fractal_store::store::tests TODO=trace-statement-and-value-representation */ sql)
            .unwrap()
            .query_map(params!["agent/legacy-seek", cut, since], |row| {
                row.get::<_, String>(0)
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    };
    let query = harness_observations_without_incarnation_query();
    // The retained pre-change SQL is an independent candidate oracle; only its index differs.
    let oracle = query.replace(
        "claims_harness_unnamed_accepted_index",
        "claims_subject_kind_accepted_index",
    );
    for (cut, since, expected) in [
        (
            9,
            "1",
            vec![
                "legacy-9", "legacy-8", "legacy-4", "legacy-3", "legacy-2", "legacy-1",
            ],
        ),
        (4, "1", vec!["legacy-4", "legacy-3", "legacy-2", "legacy-1"]),
        (9, "4", vec!["legacy-9", "legacy-8", "legacy-4"]),
        (9, "10", vec![]),
    ] {
        assert_eq!(read(&query, cut, since), expected);
        assert_eq!(read(&query, cut, since), read(&oracle, cut, since));
    }
    // The partial index also follows updates/deletes; repaired observation bodies cannot
    // leave a stale legacy candidate, and becoming unnamed must make a new candidate.
    connection.execute(/* sqlite-large-value: allow-for-now lv-h5e3e82607ef0 owner=fractal_store::store::tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET body=json_set(body,'$.fields.incarnation_id','current') WHERE id='legacy-3'",[]).unwrap();
    connection.execute(/* sqlite-large-value: allow-for-now lv-h607728b48402 owner=fractal_store::store::tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET body=json_remove(body,'$.fields.incarnation_id') WHERE id='legacy-5'",[]).unwrap();
    connection
        .execute(/* sqlite-large-value: allow-for-now lv-hf5f690296101 owner=fractal_store::store::tests TODO=prepare-body-and-predecessor-values */ "DELETE FROM claims WHERE id='legacy-2'", [])
        .unwrap();
    assert_eq!(
        read(&query, 9, "1"),
        vec!["legacy-9", "legacy-8", "legacy-5", "legacy-4", "legacy-1"]
    );
    assert_eq!(read(&query, 9, "1"), read(&oracle, 9, "1"));
}

#[test]
fn unnamed_harness_seek_cost_ignores_named_heartbeat_growth() {
    let measure = |named: usize| {
        let store = Store::open_memory("legacy-cost").unwrap();
        let mut connection = store.connection.write();
        let transaction = connection.transaction().unwrap();
        transaction
            .execute(
                /* sqlite-large-value: allow-for-now lv-h45face962b63 owner=fractal_store::store::tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO batches(id,origin,replica_sequence,hash,accepted_at_unix_ms)
                VALUES ('cost-batch','legacy-cost',1,'synthetic','1')",
                [],
            )
            .unwrap();
        for index in 1..=named {
            let body = json!({"fields":{"incarnation_id":"current","state":"idle","status_transition":false}});
            transaction.execute(/* sqlite-large-value: allow-for-now lv-h49207ad0162b owner=fractal_store::store::tests TODO=prepare-body-and-predecessor-values */ "INSERT INTO claims(store_index,id,batch_id,subject,kind,origin,body,predecessors,accepted_at_unix_ms)
                    VALUES (?1,?2,'cost-batch','agent/legacy-cost','harness.observed','legacy-cost',?3,'[]',?4)",
                params![index as i64,format!("named-{index}"),body.to_string(),index.to_string()]).unwrap();
        }
        transaction.execute(/* sqlite-large-value: allow-for-now lv-h218c3c3c1c0e owner=fractal_store::store::tests TODO=prepare-body-and-predecessor-values */ "INSERT INTO claims(store_index,id,batch_id,subject,kind,origin,body,predecessors,accepted_at_unix_ms)
                VALUES (100000,'legacy-tail','cost-batch','agent/legacy-cost','harness.observed','legacy-cost',?1,'[]','100000')",[json!({"fields":{"state":"idle"}}).to_string()]).unwrap();
        // Native claims have a recorded canonical position. Without it, the unchanged
        // legacy-position fallback counts this synthetic batch's preceding rows, a
        // separate ordering cost from selecting unnamed candidates.
        transaction.execute(
            /* sqlite-large-value: allow-for-now lv-h5e41ec81b6ee owner=fractal_store::store::tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO replica_records(record_ref,writer,sequence,envelope_hash,position,raw,state,claim_id,updated_at_unix_ms)
                 VALUES ('cost-record','legacy-cost',1,'synthetic',0,X'', 'valid','legacy-tail','1')",
            [],
        ).unwrap();
        transaction.commit().unwrap();
        let query = harness_observations_without_incarnation_query();
        let plans = connection
            .prepare(/* sqlite-large-value: allow-for-now lv-h7d81d766ec47 owner=fractal_store::store::tests TODO=trace-statement-and-value-representation */ &format!("EXPLAIN QUERY PLAN {query}"))
            .unwrap()
            .query_map(params!["agent/legacy-cost", 100000, "1"], |row| {
                row.get::<_, String>(3)
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert!(
            plans
                .iter()
                .any(|p| p.contains("claims_harness_unnamed_accepted_index")),
            "{plans:?}"
        );
        let work = |sql: &str| {
            let mut statement = connection.prepare(/* sqlite-large-value: allow-for-now lv-had2334359419 owner=fractal_store::store::tests TODO=trace-statement-and-value-representation */ sql).unwrap();
            let ids = statement
                .query_map(params!["agent/legacy-cost", 100000, "1"], |row| {
                    row.get::<_, String>(0)
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            assert_eq!(ids, vec!["legacy-tail"]);
            statement.get_status(rusqlite::StatementStatus::VmStep)
        };
        let new = work(&query);
        let old = work(&query.replace(
            "claims_harness_unnamed_accepted_index",
            "claims_subject_kind_accepted_index",
        ));
        println!("unnamed seek: named={named} new_vm_steps={new} old_vm_steps={old}");
        (new, old)
    };
    let small = measure(128);
    let large = measure(4096);
    assert!(
        small.0 > 0 && large.0 <= small.0 + 20,
        "{small:?} -> {large:?}"
    );
    assert!(
        large.1 > small.1 * 8,
        "the retained old query must demonstrate the scan: {small:?} -> {large:?}"
    );
}

/// A claim names its incarnation in its fields, or, when it has none, in its body, exactly as
/// a view that reads the claim finds it; the index expression agrees with that reading.
#[test]
fn the_incarnation_index_reads_a_claim_as_a_view_does() {
    let store = Store::open_memory("node").unwrap();
    let connection = store.connection.lock().unwrap();
    for body in [
        json!({"fields": {"incarnation_id": "current"}}),
        json!({"fields": {"incarnation_id": "current"}, "incarnation_id": "other"}),
        json!({"fields": {"state": "idle"}, "incarnation_id": "other"}),
        json!({"fields": null, "incarnation_id": "other"}),
        json!({"incarnation_id": "current"}),
        json!({"incarnation_id": 7}),
        json!({"state": "idle"}),
        json!(["current"]),
    ] {
        let viewed = body
            .get("fields")
            .unwrap_or(&body)
            .get("incarnation_id")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let indexed = connection
            .query_row(
                /* sqlite-large-value: allow-for-now lv-ha1163b9d4ef7 owner=fractal_store::store::tests TODO=trace-statement-and-value-representation */ &format!("SELECT {INCARNATION_OF_CLAIM} FROM (SELECT ?1 AS body)"),
                [body.to_string()],
                |row| row.get::<_, rusqlite::types::Value>(0),
            )
            .unwrap();
        let indexed = match indexed {
            rusqlite::types::Value::Text(text) => Some(text),
            _ => None,
        };
        assert_eq!(indexed, viewed, "{body}");
    }
}

#[test]
fn replication_inventory_uses_the_batch_indexes() {
    let store = Store::open_memory("node").unwrap();
    let connection = store.connection.lock().unwrap();
    let plan = |query: &str| {
        let mut statement = connection.prepare(/* sqlite-large-value: allow-for-now lv-h7f43c43d37a1 owner=fractal_store::store::tests TODO=trace-statement-and-value-representation */ query).unwrap();
        statement
            .query_map([], |row| row.get::<_, String>(3))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
            .join("\n")
    };
    let missing_envelopes = plan(
        "EXPLAIN QUERY PLAN
             SELECT id FROM batches
             WHERE NOT EXISTS (
                 SELECT 1 FROM replica_envelopes
                 WHERE replica_envelopes.batch_id=batches.id
             )",
    );
    assert!(
        missing_envelopes.contains("replica_envelopes_batch"),
        "the inventory seed query must use the envelope batch index:\n{missing_envelopes}"
    );
    let incremental_seed = plan(
        "EXPLAIN QUERY PLAN
             SELECT id FROM batches
             WHERE batches.rowid>=1
               AND NOT EXISTS (
                   SELECT 1 FROM replica_envelopes
                   WHERE replica_envelopes.batch_id=batches.id
               )
             ORDER BY batches.rowid",
    );
    assert!(
        incremental_seed.contains("USING INTEGER PRIMARY KEY"),
        "an incremental seed must range-scan only new batches:\n{incremental_seed}"
    );
    let batch_claims = plan(
        "EXPLAIN QUERY PLAN
             SELECT id FROM claims WHERE batch_id='batch/example' ORDER BY store_index",
    );
    assert!(
        batch_claims.contains("claims_batch_index"),
        "the inventory seed query must use the claim batch index:\n{batch_claims}"
    );
}

/// A replicated harness observation. Each one is a new transition: a repeat that changed
/// only `observed_at_ms` would stay in the local observation log.
fn observe_harness(store: &Store, observed_at_ms: u64) {
    store
        .append_claim(&ClaimInput {
            subject: "agent/example".into(),
            kind: "harness.observed".into(),
            actor: Some("agent/example".into()),
            fields: BTreeMap::from([
                ("state".into(), json!("working")),
                ("observed_at_ms".into(), json!(observed_at_ms)),
                ("transition_sequence".into(), json!(observed_at_ms)),
            ]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
}

#[test]
fn a_local_write_rehashes_only_its_own_inventory_range() {
    let store = Store::open_memory("example-linux").unwrap();
    for observed_at_ms in 0..600 {
        observe_harness(&store, observed_at_ms);
    }
    let before = store.replication_snapshot().unwrap();
    assert!(before.inventory.envelopes.len() >= 600);
    INVENTORY_IDENTITIES_HASHED.with(|hashed| hashed.set(0));

    observe_harness(&store, 600);
    let after = store.replication_snapshot().unwrap();
    let hashed = INVENTORY_IDENTITIES_HASHED.with(std::cell::Cell::get);
    assert!(
        hashed <= REPLICATION_BUCKET_WIDTH as usize,
        "one new envelope re-hashed {hashed} of {} identities",
        after.inventory.envelopes.len()
    );
    let public = after.inventory.public();
    assert_eq!(
        after.inventory.digest,
        replication_inventory_digest(&public.envelopes)
    );
    let connection = store.connection.lock().unwrap();
    let (full, _) = full_compact_replication_inventory(&connection).unwrap();
    assert_eq!(public.envelopes, full.public().envelopes);
    assert_eq!(after.inventory.digest, full.digest);
}

proptest! {
    #[test]
    fn a_resumed_inventory_digest_matches_a_full_pass(
        initial in proptest::collection::vec((0usize..3, 1u64..2000), 0..300),
        rounds in proptest::collection::vec(
            proptest::collection::vec((0usize..3, 1u64..2000), 1..20),
            1..6,
        ),
    ) {
        let writers = ["ExampleMac", "fleet-node", "example-linux"];
        // Some hashes are not SHA-256 hex, so the digest also covers verbatim hashes.
        let identity = |(writer, sequence): (usize, u64)| ReplicaEnvelopeId {
            writer: writers[writer].into(),
            sequence,
            hash: if sequence % 7 == 0 {
                format!("irregular-{sequence}")
            } else {
                format!("{sequence:064x}")
            },
        };
        let mut all = initial.into_iter().map(identity).collect::<BTreeSet<_>>();
        let mut inventory = CompactReplicationInventory::from_sorted(all.iter().cloned());
        let mut buckets = inventory.buckets();
        let mut prefixes = Vec::new();
        inventory.resume_digest(&buckets, &mut prefixes, 0);
        prop_assert_eq!(
            &inventory.digest,
            &replication_inventory_digest(&all.iter().cloned().collect::<Vec<_>>())
        );
        for round in rounds {
            let before = buckets.clone();
            for identity in round.into_iter().map(identity) {
                if all.insert(identity.clone()) {
                    inventory.insert(identity);
                }
            }
            buckets = inventory.buckets();
            let resume_from = buckets
                .iter()
                .zip(&before)
                .position(|(after, before)| after != before)
                .unwrap_or(before.len().min(buckets.len()));
            inventory.resume_digest(&buckets, &mut prefixes, resume_from);
            prop_assert_eq!(
                &inventory.digest,
                &replication_inventory_digest(&all.iter().cloned().collect::<Vec<_>>())
            );
        }
    }
}

#[test]
fn replication_reuses_legacy_digest_while_complete_source_digest_changes() {
    let store = Store::open_memory("example-linux").unwrap();
    observe_harness(&store, 0);
    let first = store.replication_snapshot().unwrap();
    GRAPH_DIGESTS_COMPUTED.with(|computed| computed.set(0));
    observe_harness(&store, 1);
    let second = store.replication_snapshot().unwrap();
    assert_ne!(first.store_index, second.store_index);
    assert_eq!(
        GRAPH_DIGESTS_COMPUTED.with(std::cell::Cell::get),
        0,
        "a harness observation must not recompute the graph digest"
    );
    assert_eq!(second.legacy_graph_digest, first.legacy_graph_digest);
    assert_ne!(second.graph_digest, first.graph_digest);
    assert_ne!(
        second.projection_digests["claim_sources"],
        first.projection_digests["claim_sources"]
    );

    let generation =
        |store: &Store| graph_generation(&store.connection.lock().unwrap()).unwrap();
    let execute = |store: &Store, sql: &str| {
        store.connection.lock().unwrap().execute(/* sqlite-large-value: allow-for-now lv-h310a23130ecb owner=fractal_store::store::tests TODO=trace-statement-and-value-representation */ sql, []).unwrap();
    };
    let before = generation(&store);
    execute(
        &store,
        /* sqlite-large-value: allow-for-now lv-h74b5be50ed01 owner=fractal_store::store::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO mission_runs(id, mission_id, initial_revision, current_generation_id,
                 root_revision, root_run_id, workspace, requester, inputs, mode, status, phase,
                 created_at_unix_ms, updated_at_unix_ms)
             VALUES ('run-example', 'mission/example', 'revision', 'generation', 'revision',
                 'run-example', '/tmp/example', 'person/example', '{}', 'normal', 'running',
                 'normal', '1', '1')",
    );
    assert_eq!(generation(&store), before + 1);
    execute(
        &store,
        /* sqlite-large-value: allow-for-now lv-he39c47fbf8df owner=fractal_store::store::tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE mission_runs SET updated_at_unix_ms='2' WHERE id='run-example'",
    );
    assert_eq!(
        generation(&store),
        before + 1,
        "an undigested column must not change the generation"
    );
    execute(
        &store,
        /* sqlite-large-value: allow-for-now lv-h1e09917472a1 owner=fractal_store::store::tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE mission_runs SET status='running' WHERE id='run-example'",
    );
    assert_eq!(generation(&store), before + 1);
    execute(
        &store,
        /* sqlite-large-value: allow-for-now lv-h580eec954622 owner=fractal_store::store::tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE mission_runs SET status='completed' WHERE id='run-example'",
    );
    assert_eq!(generation(&store), before + 2);

    observe_harness(&store, 2);
    let third = store.replication_snapshot().unwrap();
    assert_eq!(GRAPH_DIGESTS_COMPUTED.with(std::cell::Cell::get), 1);
    assert_ne!(third.graph_digest, first.graph_digest);
    assert_eq!(
        third.graph_digest,
        graph_digest(&store.connection.lock().unwrap()).unwrap()
    );
    execute(&store, /* sqlite-large-value: allow-for-now lv-ha73927e89741 owner=fractal_store::store::tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM mission_runs WHERE id='run-example'");
    assert_eq!(generation(&store), before + 3);
}

#[test]
fn operation_projection_drift_is_detected_and_repaired() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.sqlite3");
    let store = Store::open(&path, "node").unwrap();
    store
        .append_client_claim(&ClaimInput {
            subject: "resource/projection".into(),
            kind: "resource.observed".into(),
            actor: None,
            fields: BTreeMap::from([(
                "kind".into(),
                Value::String("custom.test.projection".into()),
            )]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: Some("projection-operation".into()),
        })
        .unwrap();
    assert!(store.operation_projection_drift().unwrap().is_empty());
    store
        .connection
        .lock()
        .unwrap()
        .execute(/* sqlite-large-value: allow-for-now lv-hfb5739c87232 owner=fractal_store::store::tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE operations SET state='conflict'", [])
        .unwrap();
    assert_eq!(store.operation_projection_drift().unwrap().len(), 1);
    drop(store);

    // A start keeps it, so it can answer at once; the repair finds and rebuilds it.
    let reopened = Store::open(&path, "node").unwrap();
    assert_eq!(reopened.operation_projection_drift().unwrap().len(), 1);
    assert!(reopened.repair_operation_projection_drift().unwrap());
    assert!(reopened.operation_projection_drift().unwrap().is_empty());
    assert!(!reopened.repair_operation_projection_drift().unwrap());
}

#[test]
fn legacy_human_gate_mission_keeps_its_revision_during_replication() {
    let store = Store::open_memory("node").unwrap();
    assert!(store.project_replication_backlog().unwrap());
    let intent = parse_intent(
        "version 2\nmission \"sample/review\" state=\"ready\" {\n  goal \"Review a sample change.\"\n  step \"approval\" {\n    gate \"review\" type=\"human\" { reviewer \"person/reviewer\" }\n  }\n}\n",
        "node",
    )
    .unwrap();
    let mut mission = intent.missions["sample/review"].clone();
    mission.revision.clear();
    // This is the serialized form from before human gates had a mode field.
    let current_json = serde_json::to_string(&mission).unwrap();
    let legacy_json = current_json.replace(",\"mode\":\"approve\"", "");
    assert_ne!(legacy_json, current_json);
    let revision = hex::encode(Sha256::digest(legacy_json.as_bytes()));
    let mut body: Value = serde_json::from_str(&legacy_json).unwrap();
    body["revision"] = Value::String(revision.clone());
    let claim = {
        let mut connection = store.connection.lock().unwrap();
        let transaction = connection.transaction().unwrap();
        let claim = append_claim_tx(
            &transaction,
            &store.origin,
            "mission/sample/review",
            "mission.published",
            None,
            &body,
            &[],
            None,
        )
        .unwrap();
        transaction.commit().unwrap();
        claim
    };
    assert!(store.project_replication_backlog().unwrap());
    let connection = store.connection.lock().unwrap();
    let selected: String = connection
        .query_row(
            "SELECT revision FROM mission_revisions WHERE mission_id='sample/review'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(selected, revision);
    assert!(store.operation_projection_drift().unwrap().is_empty());
    assert!(!connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM projection_health WHERE aggregate=?1 AND status='stale')",
            [format!("projection:base:{}", claim.id)],
            |row| row.get::<_, bool>(0),
        )
        .unwrap());
}

#[test]
fn operation_projection_drift_does_not_report_consistent_concurrent_writes() {
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(&directory.path().join("state.sqlite3"), "node").unwrap());
    let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let writer = {
        let store = Arc::clone(&store);
        let done = Arc::clone(&done);
        std::thread::spawn(move || {
            for index in 0..64 {
                store
                    .append_client_claim(&ClaimInput {
                        subject: format!("resource/projection-{index}"),
                        kind: "resource.observed".into(),
                        actor: None,
                        fields: BTreeMap::from([(
                            "kind".into(),
                            Value::String("custom.test.projection".into()),
                        )]),
                        evidence: Vec::new(),
                        expected_subject: None,
                        idempotency_key: Some(format!("projection-operation-{index}")),
                    })
                    .unwrap();
            }
            done.store(true, Ordering::Release);
        })
    };
    while !done.load(Ordering::Acquire) {
        assert!(store.operation_projection_drift().unwrap().is_empty());
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    writer.join().unwrap();
    assert!(store.operation_projection_drift().unwrap().is_empty());
}

#[test]
fn agent_activity_ignores_timeline_status_and_tracks_content() {
    let store = Store::open_memory("node").unwrap();
    let agent = "agent/node.worker";
    let mut status = timeline_observation(agent, "inc-1", "idle");
    status
        .fields
        .insert("entry_type".into(), Value::String("status".into()));
    store.append_claim(&status).unwrap();
    assert_eq!(
        store
            .agent_last_activity_at(agent, Some("inc-1"), store.index().unwrap())
            .unwrap(),
        None
    );
    let content = store
        .append_claim(&timeline_observation(agent, "inc-1", "answer"))
        .unwrap();
    assert_eq!(
        store
            .agent_last_activity_at(agent, Some("inc-1"), store.index().unwrap())
            .unwrap(),
        Some(content.accepted_at_unix_ms)
    );
    let mut later_status = timeline_observation(agent, "inc-1", "working");
    later_status
        .fields
        .insert("entry_type".into(), Value::String("status".into()));
    store.append_claim(&later_status).unwrap();
    assert_eq!(
        store
            .agent_last_activity_at(agent, Some("inc-1"), store.index().unwrap())
            .unwrap(),
        Some(content.accepted_at_unix_ms)
    );
}

fn timeline_entries(page: &ClaimsPage) -> Vec<String> {
    page.claims
        .iter()
        .map(|claim| {
            claim.body["fields"]["entry_id"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect()
}

#[test]
fn a_local_timeline_observation_stays_on_the_node_that_made_it() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("local.sqlite3");
    let store = Store::open(&path, "node").unwrap();
    let subject = "agent/node.worker";
    let index_before = store.index().unwrap();
    let inventory_before = store.replication_inventory().unwrap();

    let mut written = Vec::new();
    for entry in ["first", "second", "third"] {
        let (record, appended) = store
            .append_claim_outcome(&timeline_observation(subject, "inc-1", entry))
            .unwrap();
        assert!(appended);
        assert!(
            record.id.starts_with("local-observation/node/"),
            "{}",
            record.id
        );
        assert_eq!(record.store_index, index_before);
        written.push(record);
    }
    let (repeated, appended) = store
        .append_claim_outcome(&timeline_observation(subject, "inc-1", "second"))
        .unwrap();
    assert!(!appended, "a repeated idempotency key records nothing new");
    assert_eq!(repeated.id, written[1].id);
    let mut changed = timeline_observation(subject, "inc-1", "second");
    changed.fields.insert(
        "body".into(),
        json!({"media_type": "text/plain", "text": "a different body"}),
    );
    assert_eq!(
        store.append_claim_outcome(&changed).unwrap_err().code,
        "idempotency-mismatch",
        "a retry with a changed body is not silently dropped"
    );
    let mut elsewhere = timeline_observation("agent/node.other", "inc-1", "second");
    elsewhere.idempotency_key =
        timeline_observation(subject, "inc-1", "second").idempotency_key;
    assert_eq!(
        store.append_claim_outcome(&elsewhere).unwrap_err().code,
        "idempotency-mismatch",
        "a key reused for another subject does not return the first subject's row"
    );
    let mut unkeyed = timeline_observation(subject, "inc-1", "second");
    unkeyed.idempotency_key = None;
    assert!(
        store.append_claim_outcome(&unkeyed).unwrap().1,
        "an observation without a key is always new"
    );

    assert_eq!(store.index().unwrap(), index_before, "no claim was written");
    assert!(
        store
            .claims_for(subject, Some("harness.timeline"))
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        store.replication_inventory().unwrap().envelopes,
        inventory_before.envelopes,
        "a local observation adds no envelope"
    );
    let page = store
        .timeline_claims_for_incarnation_at(subject, "inc-1", None, false, 10)
        .unwrap();
    assert_eq!(
        timeline_entries(&page),
        ["first", "second", "third", "second"]
    );
    assert_eq!(page.next_cursor, None);

    let replica = Store::open_memory("replica").unwrap();
    let exchange = exchange_from(&store, &ReplicationInventory::default());
    receive_and_project(&replica, "node", &exchange);
    assert!(
        replica
            .timeline_claims_for_incarnation_at(subject, "inc-1", None, false, 10)
            .unwrap()
            .claims
            .is_empty(),
        "a peer never receives a local observation"
    );

    drop(store);
    let reopened = Store::open(&path, "node").unwrap();
    let page = reopened
        .timeline_claims_for_incarnation_at(subject, "inc-1", None, false, 10)
        .unwrap();
    assert_eq!(
        timeline_entries(&page),
        ["first", "second", "third", "second"]
    );
    let mut with_head = timeline_observation(subject, "inc-1", "fourth");
    with_head.expected_subject = Some(None);
    assert_eq!(
        reopened.append_claim_outcome(&with_head).unwrap_err().code,
        "local-observation-head"
    );
    let mut missing = timeline_observation(subject, "inc-1", "fifth");
    missing.fields.remove("incarnation_id");
    assert_eq!(
        reopened.append_claim_outcome(&missing).unwrap_err().code,
        "missing-claim-field"
    );
    let mut unnumbered = timeline_observation(subject, "inc-1", "sixth");
    unnumbered.fields.remove("sequence");
    assert_eq!(
        reopened.append_claim_outcome(&unnumbered).unwrap_err().code,
        "missing-claim-field"
    );
}

#[test]

fn agent_usage_rollups_include_latest_slot_cost_without_double_counting() {

    let rows = [

        json!({"semantics":"response","incarnation_id":"inc-one","total_tokens":10,"cost":99.0,"currency":"USD"}),

        json!({"semantics":"session_cumulative","incarnation_id":"inc-one","total_tokens":10,"cost":99.0,"currency":"USD"}),

        json!({"semantics":"response_rollup","incarnation_id":"inc-one","model":"first","total_tokens":10,"unpriced_tokens":10}),

        json!({"semantics":"response_rollup","incarnation_id":"inc-one","model":"first","total_tokens":20,"cost_microusd":6_338_109,"reported_cost_microusd":6_338_109,"unpriced_tokens":0}),

        json!({"semantics":"response_rollup","incarnation_id":"inc-one","model":"second","total_tokens":30,"cost_microusd":500_000,"unpriced_tokens":0}),

        json!({"semantics":"response_rollup","incarnation_id":"inc-two","model":"first","total_tokens":40,"cost_microusd":0,"unpriced_tokens":0}),

    ];

    let rows = rows.into_iter().enumerate().map(|(index, fields)| {

        Ok((index as u64, json!({"fields":fields}).to_string(), "0".into()))

    });

    let summary = Store::usage_summary_from_rows(rows, None).unwrap().unwrap();

    assert_eq!(summary.total_tokens, 90);

    assert_eq!(summary.cost, Some(6.838109));

    assert_eq!(summary.currency.as_deref(), Some("USD"));

}


#[test]

fn agent_usage_rollup_missing_or_unpriced_cost_stays_unknown() {

    for cost_fields in [

        json!({}),

        json!({"cost_microusd":0,"unpriced_tokens":10}),

        json!({"cost_microusd":500_000,"unpriced_tokens":1}),

    ] {

        let mut fields = json!({"semantics":"response_rollup","incarnation_id":"inc-one","model":"unknown","total_tokens":10});

        fields

            .as_object_mut()

            .unwrap()

            .extend(cost_fields.as_object().unwrap().clone());

        let known = json!({"semantics":"response_rollup","incarnation_id":"inc-one","model":"known","total_tokens":20,"cost_microusd":1_000_000,"unpriced_tokens":0});

        let rows = [known, fields].into_iter().enumerate().map(|(index, fields)| {

            Ok((index as u64, json!({"fields":fields}).to_string(), "0".into()))

        });

        let summary = Store::usage_summary_from_rows(rows, None).unwrap().unwrap();

        assert_eq!(summary.total_tokens, 30);

        assert_eq!(summary.cost, None);

        assert_eq!(summary.currency, None);

    }

}


#[test]

fn agent_usage_rollup_and_legacy_cost_require_complete_matching_currency() {

    for semantics in ["session_cumulative", "response"] {

        for (legacy_cost, currency, expected) in [

            (None, Some("USD"), None),

            (Some(2.0), Some("USD"), Some(3.0)),

            (Some(2.0), Some("EUR"), None),

            (Some(2.0), None, None),

        ] {

            for legacy_incarnation in ["a-legacy", "z-legacy"] {

                let rows = [

                    json!({"semantics":"response_rollup","incarnation_id":"m-rollup","total_tokens":20,"cost_microusd":1_000_000,"unpriced_tokens":0}),

                    json!({"semantics":semantics,"incarnation_id":legacy_incarnation,"total_tokens":10,"cost":legacy_cost,"currency":currency}),

                ];

                let rows = rows.into_iter().enumerate().map(|(index, fields)| {

                    Ok((index as u64, json!({"fields":fields}).to_string(), "0".into()))

                });

                let summary = Store::usage_summary_from_rows(rows, None).unwrap().unwrap();

                assert_eq!(summary.total_tokens, 30);

                assert_eq!(summary.cost, expected);

                assert_eq!(summary.currency.as_deref(), expected.map(|_| "USD"));

            }

        }

    }

}


#[test]

fn agent_usage_legacy_response_cost_requires_every_delta_priced_in_one_currency() {

    for second in [

        json!({"total_tokens":10}),

        json!({"total_tokens":10,"cost":2.0,"currency":"EUR"}),

    ] {

        let mut second = second;

        second["semantics"] = json!("response");

        second["incarnation_id"] = json!("legacy");

        let rows = [

            json!({"semantics":"response","incarnation_id":"legacy","total_tokens":20,"cost":1.0,"currency":"USD"}),

            second,

        ];

        let rows = rows.into_iter().enumerate().map(|(index, fields)| {

            Ok((index as u64, json!({"fields":fields}).to_string(), "0".into()))

        });

        let summary = Store::usage_summary_from_rows(rows, None).unwrap().unwrap();

        assert_eq!(summary.total_tokens, 30);

        assert_eq!(summary.cost, None);

        assert_eq!(summary.currency, None);

    }

}

#[test]
fn response_usage_binds_to_the_step_held_when_the_harness_recorded_it() {
    let store = Store::open_memory("host-one").unwrap();
    let subject = "agent/example.worker";
    let activated = now_ms() as u64 - 60_000;
    {
        let connection = store.connection.lock().unwrap();
        connection
            .execute_batch("PRAGMA foreign_keys=OFF;")
            .unwrap();
        connection.execute(
            /* sqlite-large-value: allow-for-now lv-h0c020fd54d91 owner=fractal_store::store::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO step_runs(subject, run_id, generation_id, step_path, definition_hash, status, attempt,
                    goals, lease_owner, lease_incarnation, lease_expires_at_unix_ms, activated_at_unix_ms,
                    created_at_unix_ms, updated_at_unix_ms)
                 VALUES ('step-run/example/build', 'example', 'example', 'build', 'hash', 'working', 1,
                    '[]', ?1, 'inc-one', ?2, ?3, ?3, ?3)",
            params![subject, (activated + 3_600_000).to_string(), activated.to_string()],
        ).unwrap();
        connection.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
    }
    let respond = |entry: &str, at: u64| {
        let mut claim = timeline_observation(subject, "inc-one", entry);
        claim
            .fields
            .insert("entry_type".into(), Value::String("usage".into()));
        claim
            .fields
            .insert("observed_at_unix_ms".into(), Value::from(at));
        claim.fields.insert(
            "body".into(),
            json!({"semantics": "response", "model": "claude-opus-5-5", "input_tokens": 1, "output_tokens": 1, "total_tokens": 2}),
        );
        store.append_claim_outcome(&claim).unwrap().0.body["fields"]["attribution"].clone()
    };
    let during = respond("during", activated + 1);
    assert_eq!(during["step_id"], "step-run/example/build");
    assert_eq!(during["mission_run_id"], "mission-run/example");
    // Replayed later, a response from before the step began is not charged to it.
    let before = respond("before", activated - 1);
    assert!(before["step_id"].is_null(), "{before}");
}

#[test]
fn a_timeline_page_merges_claims_from_older_builds_with_local_observations() {
    let store = Store::open_memory("node").unwrap();
    let subject = "agent/node.worker";
    // An older build wrote timeline entries as replicated claims.
    let legacy = |entry: &str| {
        let input = timeline_observation(subject, "inc-1", entry);
        let mut connection = store.connection.lock().unwrap();
        let transaction = connection.transaction().unwrap();
        append_claim_tx(
            &transaction,
            "older-node",
            subject,
            "harness.timeline",
            Some(subject),
            &json!({ "fields": input.fields, "evidence": [] }),
            &[],
            None,
        )
        .unwrap();
        transaction.commit().unwrap();
    };
    legacy("legacy-1");
    legacy("legacy-2");
    store
        .append_claim(&timeline_observation(subject, "inc-1", "local-1"))
        .unwrap();
    store
        .append_claim(&timeline_observation(subject, "inc-2", "other-incarnation"))
        .unwrap();
    legacy("legacy-3");
    store
        .append_claim(&timeline_observation(subject, "inc-1", "local-2"))
        .unwrap();
    store
        .append_claim(&timeline_observation(subject, "inc-1", "local-3"))
        .unwrap();

    let ascending = store
        .timeline_claims_for_incarnation_at(subject, "inc-1", None, false, 10)
        .unwrap();
    assert_eq!(
        timeline_entries(&ascending),
        [
            "legacy-1", "legacy-2", "local-1", "legacy-3", "local-2", "local-3"
        ]
    );
    let newest = store
        .timeline_claims_for_incarnation_at(subject, "inc-1", None, true, 2)
        .unwrap();
    assert_eq!(timeline_entries(&newest), ["local-3", "local-2"]);
    assert!(newest.next_cursor.is_some());
    let legacy_three = ascending.claims[3].store_index;
    let before = store
        .timeline_claims_for_incarnation_at(subject, "inc-1", Some(legacy_three), true, 10)
        .unwrap();
    assert_eq!(
        timeline_entries(&before),
        ["local-1", "legacy-2", "legacy-1"],
        "a snapshot before a claim excludes the local observations written after it"
    );
    assert_eq!(
        store
            .latest_local_timeline_at(subject, "inc-1", legacy_three)
            .unwrap(),
        Some(ascending.claims[5].accepted_at_unix_ms)
    );
    assert_eq!(
        store
            .latest_local_timeline_at(subject, "inc-1", legacy_three - 1)
            .unwrap(),
        Some(ascending.claims[2].accepted_at_unix_ms)
    );
}

#[test]
fn trimming_local_observations_keeps_the_newest_of_each_subject_and_kind() {
    let store = Store::open_memory("node").unwrap();
    for entry in 0..10 {
        store
            .append_claim(&timeline_observation(
                "agent/node.first",
                "inc",
                &format!("entry-{entry}"),
            ))
            .unwrap();
    }
    for entry in 0..3 {
        store
            .append_claim(&timeline_observation(
                "agent/node.second",
                "inc",
                &format!("entry-{entry}"),
            ))
            .unwrap();
    }
    let positions = |store: &Store| {
        store
            .local_observations_after(0, 100)
            .unwrap()
            .iter()
            .map(|record| local_observation_position(record).unwrap())
            .collect::<Vec<_>>()
    };
    assert_eq!(positions(&store), (1..=13).collect::<Vec<_>>());
    assert_eq!(
        store
            .trim_local_observations(now_ms().saturating_sub(3_600_000), 20_000, 2)
            .unwrap(),
        0,
        "nothing is older than the window yet"
    );
    store
        .connection
        .lock()
        .unwrap()
        .execute(
            /* sqlite-large-value: allow-for-now lv-hfc7b7df88745 owner=fractal_store::store::tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE local_observations SET observed_at_unix_ms=1000 WHERE id <= 12",
            [],
        )
        .unwrap();
    assert_eq!(
        store.trim_local_observations(2_000, 20_000, 2).unwrap(),
        11,
        "each chunk deletes at most two rows until nothing old is left"
    );
    assert_eq!(
        positions(&store),
        [10, 13],
        "the newest observation of each subject and kind stays"
    );
    assert_eq!(store.local_observations_tail(1).unwrap().len(), 1);

    for entry in 0..5 {
        store
            .append_claim(&timeline_observation(
                "agent/node.third",
                "inc",
                &format!("entry-{entry}"),
            ))
            .unwrap();
    }
    assert_eq!(store.trim_local_observations(0, 2, 100).unwrap(), 3);
    assert_eq!(positions(&store), [10, 13, 17, 18]);
    // AUTOINCREMENT never reuses a trimmed position, so event cursors stay ordered.
    let (record, _) = store
        .append_claim_outcome(&timeline_observation("agent/node.third", "inc", "later"))
        .unwrap();
    assert_eq!(local_observation_position(&record), Some(19));
    assert_eq!(store.local_observation_floor_after_claim(0).unwrap(), 9);
    assert_eq!(
        store
            .local_observation_floor_after_claim(store.index().unwrap() + 1)
            .unwrap(),
        19
    );
}

#[test]
fn local_observation_floor_seeks_the_claim_frontier() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.sqlite3");
    {
        let store = Store::open(&path, "node").unwrap();
        assert_eq!(store.local_observation_floor_after_claim(0).unwrap(), 0);
        let connection = store.connection.write();
        connection
            .execute_batch(
                /* sqlite-large-value: allow-for-now lv-hc2b2cb3648c6 owner=fractal_store::store::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO local_observations(id,after_store_index,subject,kind,body,observed_at_unix_ms)
                     VALUES(3,10,'agent/node.a','harness.timeline','{}',1),
                           (7,30,'agent/node.a','harness.timeline','{}',1),
                           (12,20,'agent/node.a','harness.timeline','{}',1);
                     DROP INDEX local_observations_after_store_index;",
            )
            .unwrap();
    }
    // Opening an existing schema adds the index without replaying or changing rows.
    let store = Store::open(&path, "node").unwrap();
    assert_eq!(store.local_observation_floor_after_claim(0).unwrap(), 2);
    assert_eq!(store.local_observation_floor_after_claim(10).unwrap(), 2);
    assert_eq!(store.local_observation_floor_after_claim(11).unwrap(), 6);
    assert_eq!(store.local_observation_floor_after_claim(20).unwrap(), 6);
    assert_eq!(store.local_observation_floor_after_claim(21).unwrap(), 6);
    assert_eq!(store.local_observation_floor_after_claim(31).unwrap(), 12);
    let connection = store.readers.get();
    let plan = connection
        .prepare(/* sqlite-large-value: allow-for-now lv-h61ffbc9870f4 owner=fractal_store::store::tests TODO=trace-statement-and-value-representation */ &format!("EXPLAIN QUERY PLAN {LOCAL_OBSERVATION_FLOOR_QUERY}"))
        .unwrap()
        .query_map([30], |row| row.get::<_, String>(3))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
        .join("; ");
    assert!(
        plan.contains("USING COVERING INDEX local_observations_after_store_index (after_store_index>?)"),
        "{plan}"
    );
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM local_observations", [], |row| row.get::<_, u64>(0))
            .unwrap(),
        3
    );
}

#[test]
fn the_per_subject_cap_trims_every_subject_over_it_and_only_those() {
    let store = Store::open_memory("node").unwrap();
    for (subject, entries) in [("agent/node.a", 7), ("agent/node.b", 5), ("agent/node.c", 2)] {
        for entry in 0..entries {
            store
                .append_claim(&timeline_observation(subject, "inc", &format!("entry-{entry}")))
                .unwrap();
        }
    }
    let remaining = |store: &Store| {
        store
            .local_observations_after(0, 100)
            .unwrap()
            .iter()
            .map(|record| (record.subject.clone(), local_observation_position(record).unwrap()))
            .collect::<Vec<_>>()
    };
    // Nothing is old; the cap alone applies, two rows to a chunk, to a and b but not c.
    assert_eq!(store.trim_local_observations(0, 3, 2).unwrap(), 4 + 2);
    let kept = remaining(&store);
    let of = |name: &str| {
        kept.iter()
            .filter(|(subject, _)| subject == name)
            .map(|(_, position)| *position)
            .collect::<Vec<_>>()
    };
    assert_eq!(of("agent/node.a"), [5, 6, 7], "the three newest of seven");
    assert_eq!(of("agent/node.b"), [10, 11, 12], "the three newest of five");
    assert_eq!(of("agent/node.c"), [13, 14], "a subject under the cap is untouched");
    assert_eq!(store.trim_local_observations(0, 3, 2).unwrap(), 0);
}


fn harness_usage(subject: &str, semantics: &str, tokens: u64, compactions: u64) -> ClaimInput {
    ClaimInput {
        subject: subject.into(),
        kind: "harness.usage".into(),
        actor: Some(subject.into()),
        fields: BTreeMap::from([
            ("driver".into(), Value::String("codex".into())),
            ("incarnation_id".into(), Value::String("inc-1".into())),
            ("semantics".into(), Value::String(semantics.into())),
            ("context_used_tokens".into(), Value::from(tokens)),
            ("compactions".into(), Value::from(compactions)),
            ("model".into(), Value::String("example-model".into())),
        ]),
        evidence: Vec::new(),
        expected_subject: None,
        idempotency_key: Some(format!(
            "usage:{subject}:{semantics}:{tokens}:{compactions}"
        )),
    }
}

#[test]
fn a_harness_state_replicates_only_when_it_changes() {
    let store = Store::open_memory("node").unwrap();
    let subject = "agent/node.worker";
    let inventory_before = store.replication_inventory().unwrap().envelopes.len();
    let mut replicated = Vec::new();
    for (state, observed_at, expect_claim) in [
        ("working", 1_000, true),
        ("working", 301_000, false),
        ("working", 601_000, false),
        ("idle", 602_000, true),
        ("idle", 902_000, false),
        ("working", 903_000, true),
    ] {
        let (record, appended) = store
            .append_claim_outcome(&harness_state(subject, state, observed_at))
            .unwrap();
        assert!(appended);
        assert_eq!(
            local_observation_position(&record).is_none(),
            expect_claim,
            "{state} at {observed_at}"
        );
        if expect_claim {
            replicated.push(record.id);
        }
    }
    let (repeated, appended) = store
        .append_claim_outcome(&harness_state(subject, "idle", 902_000))
        .unwrap();
    assert!(!appended);
    assert!(local_observation_position(&repeated).is_some());

    let claims = store.claims_for(subject, Some("harness.observed")).unwrap();
    assert_eq!(
        claims
            .iter()
            .map(|claim| claim.id.clone())
            .collect::<Vec<_>>(),
        replicated
    );
    assert_eq!(
        store.replication_inventory().unwrap().envelopes.len(),
        inventory_before + 3
    );
    let local = store.local_observations_after(0, 100).unwrap();
    assert_eq!(local.len(), 6, "the local log keeps every observation");
    assert!(local.iter().all(|record| record.kind == "harness.observed"));
    let latest = store
        .latest_claim(subject, Some("harness.observed"))
        .unwrap()
        .unwrap();
    assert_eq!(latest.body["fields"]["state"], "working");
    assert_eq!(latest.body["fields"]["observed_at_ms"], 903_000);

    let replica = Store::open_memory("replica").unwrap();
    receive_and_project(
        &replica,
        "node",
        &exchange_from(&store, &ReplicationInventory::default()),
    );
    assert_eq!(
        replica
            .claims_for(subject, Some("harness.observed"))
            .unwrap()
            .len(),
        3
    );
    assert!(replica.local_observations_after(0, 100).unwrap().is_empty());
}

#[test]
fn harness_usage_replicates_on_a_schedule_while_working_and_at_once_when_it_stops() {
    let store = Store::open_memory("node").unwrap();
    let subject = "agent/node.worker";
    let start = now_ms();
    let minute = 60_000_u128;
    let usage_claims = |store: &Store| {
        store
            .claims_for(subject, Some("harness.usage"))
            .unwrap()
            .iter()
            .map(|claim| {
                (
                    claim.body["fields"]["semantics"]
                        .as_str()
                        .unwrap()
                        .to_owned(),
                    claim.body["fields"]["context_used_tokens"]
                        .as_u64()
                        .unwrap(),
                )
            })
            .collect::<Vec<_>>()
    };
    let occupancy = |tokens: u64| ("context_occupancy".to_owned(), tokens);
    store
        .append_claim_outcome(&harness_state(subject, "working", 1))
        .unwrap();
    let usage = |semantics: &str, tokens: u64, compactions: u64, at: u128| {
        store
            .append_latest_observation(
                &harness_usage(subject, semantics, tokens, compactions),
                at,
            )
            .unwrap()
    };

    assert!(local_observation_position(&usage("context_occupancy", 10, 0, start).0).is_none());
    assert!(
        local_observation_position(&usage("context_occupancy", 20, 0, start + minute).0)
            .is_some()
    );
    assert!(
        local_observation_position(&usage("context_occupancy", 30, 0, start + 2 * minute).0)
            .is_some()
    );
    assert_eq!(
        usage_claims(&store),
        [occupancy(10)],
        "while working, a changed reading waits for the interval"
    );
    assert!(
        local_observation_position(&usage("session_cumulative", 500, 0, start + 2 * minute).0)
            .is_none(),
        "each semantics has its own slot"
    );
    assert!(
        local_observation_position(&usage("context_occupancy", 5, 1, start + 3 * minute).0)
            .is_none(),
        "a compaction replicates at once"
    );
    assert!(
        local_observation_position(&usage("context_occupancy", 15, 1, start + 4 * minute).0)
            .is_some()
    );
    assert!(
        local_observation_position(&usage("context_occupancy", 25, 1, start + 9 * minute).0)
            .is_none(),
        "the interval has passed since the last replicated reading"
    );
    assert!(
        local_observation_position(&usage("context_occupancy", 35, 1, start + 10 * minute).0)
            .is_some()
    );
    assert_eq!(
        usage_claims(&store),
        [
            occupancy(10),
            ("session_cumulative".into(), 500),
            occupancy(5),
            occupancy(25),
        ]
    );

    store
        .append_claim_outcome(&harness_state(subject, "idle", 2))
        .unwrap();
    assert_eq!(
        usage_claims(&store).last(),
        Some(&occupancy(35)),
        "the newest pending reading replicates when the harness stops working"
    );
    let settled = usage_claims(&store).len();
    store
        .append_claim_outcome(&harness_state(subject, "idle", 3))
        .unwrap();
    assert_eq!(usage_claims(&store).len(), settled, "nothing was pending");
    assert!(
        local_observation_position(&usage("context_occupancy", 36, 1, start + 11 * minute).0)
            .is_none(),
        "an idle harness replicates its usage at once"
    );
    assert_eq!(store.local_observations_after(0, 100).unwrap().len(), 12);
}

#[test]
fn a_system_runtime_action_stays_local_while_a_requested_one_replicates() {
    let store = Store::open_memory("node").unwrap();
    let subject = "agent/node.worker";
    let action =
        |action: &str, actor: Option<&str>, evidence: Vec<String>, key: &str| ClaimInput {
            subject: subject.into(),
            kind: if action == "signal-result" {
                "runtime.action.succeeded".into()
            } else {
                "runtime.action.requested".into()
            },
            actor: actor.map(str::to_owned),
            fields: BTreeMap::from([(
                "action".into(),
                Value::String(
                    if action == "signal-result" {
                        "signal"
                    } else {
                        action
                    }
                    .into(),
                ),
            )]),
            evidence,
            expected_subject: None,
            idempotency_key: Some(key.into()),
        };
    let system = store
        .append_claim(&action("terminate", None, Vec::new(), "system-stop"))
        .unwrap();
    assert!(local_observation_position(&system).is_some());
    let deadline = store
        .append_claim(&ClaimInput {
            subject: subject.into(),
            kind: "runtime.action.deadline-reached".into(),
            actor: None,
            fields: BTreeMap::from([("action".into(), Value::String("terminate".into()))]),
            evidence: vec![system.id.clone()],
            expected_subject: None,
            idempotency_key: Some("system-deadline".into()),
        })
        .unwrap();
    assert!(
        local_observation_position(&deadline).is_some(),
        "a local observation may cite another"
    );

    let requested = store
        .append_claim(&action(
            "signal",
            Some("requester"),
            Vec::new(),
            "person-signal",
        ))
        .unwrap();
    assert!(
        local_observation_position(&requested).is_none(),
        "a request that names its actor replicates"
    );
    let result = store
        .append_claim(&action(
            "signal-result",
            Some("requester"),
            vec![requested.id.clone()],
            "person-signal-result",
        ))
        .unwrap();
    assert!(local_observation_position(&result).is_none());
    assert_eq!(
        store
            .append_claim(&action(
                "signal-result",
                Some("requester"),
                vec![system.id.clone()],
                "cites-local",
            ))
            .unwrap_err()
            .code,
        "missing-evidence",
        "a replicated claim never cites a local observation"
    );
    assert_eq!(
        store
            .append_claim(&ClaimInput {
                subject: subject.into(),
                kind: "runtime.action.deadline-reached".into(),
                actor: None,
                fields: BTreeMap::from([("action".into(), Value::String("terminate".into()))]),
                evidence: vec!["local-observation/another-node/1".into()],
                expected_subject: None,
                idempotency_key: Some("foreign-local".into()),
            })
            .unwrap_err()
            .code,
        "missing-evidence",
        "another node's local observation is never here"
    );
    let recorded = store
        .observations_for(subject, "runtime.action.requested")
        .unwrap();
    assert_eq!(
        recorded
            .iter()
            .map(|record| record.id.clone())
            .collect::<Vec<_>>(),
        [system.id, requested.id]
    );
}

#[test]
fn retention_event_tail_work_is_bounded_during_and_after_migration() {
    let connection = rusqlite::Connection::open_in_memory().unwrap();
    connection
        .execute_batch(
            /* sqlite-large-value: allow-for-now lv-h830751fcf154 owner=fractal_store::store::tests TODO=chunk-or-reference-variable-columns */ "CREATE TABLE claims(store_index INTEGER PRIMARY KEY,kind TEXT,body TEXT);
             CREATE TABLE event_positions(store_index INTEGER PRIMARY KEY,subject TEXT);
             CREATE TABLE local_event_payloads(store_index INTEGER PRIMARY KEY,subject TEXT);",
        )
        .unwrap();
    for count in [1_000, 10_000, 100_000] {
        connection.execute_batch(&format!(
            /* sqlite-large-value: allow-for-now lv-hccb926f560be owner=fractal_store::store::tests TODO=prepare-body-and-predecessor-values */ "DELETE FROM claims; DELETE FROM event_positions; DELETE FROM local_event_payloads;
                 WITH RECURSIVE numbers(n) AS (VALUES(1) UNION ALL SELECT n+1 FROM numbers WHERE n<{count})
                 INSERT INTO claims SELECT n,'custom.test.recorded','{{}}' FROM numbers;
                 INSERT INTO event_positions SELECT store_index,'custom/test/tail' FROM claims WHERE store_index%2=0;
                 INSERT INTO local_event_payloads SELECT store_index,'custom/test/tail' FROM claims WHERE store_index%2=1;"
        )).unwrap();
        for migrating in [true, false] {
            let mut statement = connection.prepare(/* sqlite-large-value: allow-for-now lv-hfb3818e1a39a owner=fractal_store::store::tests TODO=trace-statement-and-value-representation */ event_tail_sql(migrating)).unwrap();
            let indices = statement
                .query_map([3], |row| row.get::<_, u64>(0))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            let expected = if migrating {
                vec![count - 2, count - 1, count]
            } else {
                vec![count - 4, count - 2, count]
            };
            assert_eq!(indices, expected);
            assert!(
                statement.get_status(rusqlite::StatementStatus::VmStep) < 1_000,
                "{count} rows, migrating={migrating}"
            );
            assert!(
                statement.get_status(rusqlite::StatementStatus::FullscanStep) <= 12,
                "only bounded membership and result steps"
            );
        }
    }
}

#[test]
fn retention_event_publication_bodies_match_the_old_payload_contract() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("claims.sqlite");
    let store = Store::open(&path, "node").unwrap();
    let source = format!(
        "{TAKEOVER_SOURCE}\nagent \"payload-worker\" {{ workspace \"/tmp\"; command \"true\" }}\n"
    );
    let intent = parse_intent(&source, "node").unwrap();
    let planned = store
        .mission(
            &intent,
            IntentInput {
                kdl: source,
                source_name: None,
            },
        )
        .unwrap();
    store
        .apply(&intent, &planned.subject_tokens, "retention-event-payload")
        .unwrap();
    let admitted = store.events_after(0, None).unwrap();
    let mut old_payloads = Vec::new();
    {
        let connection = store.connection.write();
        // Restore the actual schema-16 payload table. Populate it with the inputs used by
        // the old insert_event, independently of the new join's payload results.
        connection
            .execute_batch(
                /* sqlite-large-value: allow-for-now lv-h4f4d21fb29b9 owner=fractal_store::store::tests TODO=chunk-or-reference-variable-columns */ "DROP VIEW events; DROP TABLE event_positions;
                 CREATE TABLE events (
                     store_index INTEGER PRIMARY KEY, kind TEXT NOT NULL,
                     subject TEXT NOT NULL, body TEXT NOT NULL
                 ); PRAGMA user_version=16;",
            )
            .unwrap();
        for (subject, kind, mut old_body) in [
            (
                "agent/node.payload-worker",
                "intent.desired",
                serde_json::to_value(&intent.subjects["agent/node.payload-worker"]).unwrap(),
            ),
            (
                "mission/takeover",
                "mission.published",
                serde_json::to_value(&intent.missions["takeover"]).unwrap(),
            ),
        ] {
            let event = admitted
                .iter()
                .find(|event| event.subject == subject && event.kind == kind)
                .unwrap();
            if let Some(operation) = event.body.get("_operation") {
                old_body["_operation"] = operation.clone();
            }
            // This is schema-16 insert_event's INSERT and canonicalization, rather than
            // an INSERT SELECT from claims or the new view.
            connection.execute(
                /* sqlite-large-value: allow-for-now lv-h414036e9cfed owner=fractal_store::store::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO events(store_index, kind, subject, body) VALUES (?1, ?2, ?3, ?4)",
                params![event.store_index, kind, subject, canonical_json_text(&old_body).unwrap()],
            ).unwrap();
        }
        let mut statement = connection
            .prepare("SELECT store_index,kind,subject,body FROM events ORDER BY store_index")
            .unwrap();
        old_payloads.extend(
            statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, u64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap(),
        );
        assert_eq!(old_payloads.len(), 2);
    }
    drop(store);
    let store = Store::open(&path, "node").unwrap();
    let read_payloads = || {
        let connection = store.readers.get();
        let mut statement = connection
            .prepare("SELECT store_index,kind,subject,body FROM events ORDER BY store_index")
            .unwrap();
        statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, u64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    };
    assert!(store.event_payload_migration_pending().unwrap());
    assert_eq!(
        read_payloads(),
        old_payloads,
        "upgrade view must match old stored bytes"
    );
    assert_eq!(store.migrate_event_payloads().unwrap(), 2);
    assert!(!store.event_payload_migration_pending().unwrap());
    assert_eq!(
        read_payloads(),
        old_payloads,
        "completed payload-free view must match old stored bytes"
    );
}

#[test]
fn a_mission_selector_or_completion_change_restarts_carried_work() {
    let parse = |selector: &str, completion: &str| {
        let source = format!(
            r#"
version 2

  mission "contract" state="ready" {{
    goal "Use the current mission contract."
    {selector}
    {completion}
    step "work" {{ }}
  }}

"#
        );
        parse_intent(&source, "node").unwrap().missions["contract"].clone()
    };
    let old = parse(
        "assigned-to \"agent/node.one\"",
        "completion { when \"all-steps-exhausted\" }",
    );
    let selector_changed = parse(
        "assigned-to \"agent/node.two\"",
        "completion { when \"all-steps-exhausted\" }",
    );
    let completion_changed = parse(
        "assigned-to \"agent/node.one\"",
        "completion { depends-on { step \"work\" completed } }",
    );
    for candidate in [&selector_changed, &completion_changed] {
        let (compatible, _) =
            analyze_mission_revision(&old, candidate, "person/requester").unwrap();
        if std::ptr::eq(candidate, &selector_changed) {
            assert!(compatible.is_empty());
        } else {
            assert_eq!(compatible, BTreeSet::from(["work".into()]));
        }
    }
}

#[test]
fn preview_recognizes_a_run_scoped_selector_for_a_declared_agent() {
    let store = Store::open_memory("node").unwrap();
    let source = r#"version 2
mission "work" state="ready" {
  goal "Complete the available work."
  agent "worker" {
    identity "fleet.worker"
    workspace "."
    command "true"
  }
  step "do-work" {
    assigned-to "agent/${ST_MISSION_RUN}/fleet.worker"
  }
}"#;
    let intent = parse_intent(source, "node").unwrap();
    let planned = store
        .mission(
            &intent,
            IntentInput {
                kdl: source.into(),
                source_name: None,
            },
        )
        .unwrap();
    assert!(
        planned
            .warnings
            .iter()
            .all(|warning| !warning.contains("missing eligible agent")),
        "{:?}",
        planned.warnings
    );
}

#[test]
fn preview_warns_when_negative_review_would_cancel_its_only_remediation() {
    let store = Store::open_memory("node").unwrap();
    let source = r#"version 2
mission "review-guardrail" state="ready" {
  goal "Review, remediate, and release."
  step "independent-review" { goal "Fail validation when material findings exist." }
  step "fix-and-real-daemon-proof" {
    goal "Remediate every review finding."
    depends-on { step "independent-review" completed }
  }
}
"#;
    let intent = parse_intent(source, "node").unwrap();
    let planned = store
        .mission(
            &intent,
            IntentInput {
                kdl: source.into(),
                source_name: None,
            },
        )
        .unwrap();
    assert!(planned.blockers.is_empty());
    assert!(
        planned.warnings.iter().any(|warning| {
            warning.contains("failure propagation makes remediation unreachable")
                && warning.contains("independent-review")
        }),
        "{:?}",
        planned.warnings
    );
}

#[test]
fn a_graph_wide_idempotency_key_returns_the_original_claim() {
    let store = Store::open_memory("node").unwrap();
    let request = ClaimInput {
        subject: "custom/test/fact".into(),
        kind: "custom.test.observed".into(),
        actor: Some("person/tester".into()),
        fields: BTreeMap::from([("value".into(), json!(1))]),
        evidence: Vec::new(),
        expected_subject: None,
        idempotency_key: Some("one-operation".into()),
    };
    let first = store.append_claim(&request).unwrap();
    let retry = store.append_claim(&request).unwrap();
    assert_eq!(retry.id, first.id);
    assert_eq!(retry.operation_id, first.operation_id);
    assert_eq!(store.claims_for(&request.subject, None).unwrap().len(), 1);

    let mut changed = request.clone();
    changed.fields.insert("value".into(), json!(2));
    let error = store.append_claim(&changed).unwrap_err();
    assert_eq!(error.code, "idempotency-mismatch");
}

#[test]
fn matching_replica_operations_converge_without_a_second_event() {
    let left = Store::open_memory("left").unwrap();
    let right = Store::open_memory("right").unwrap();
    let request = ClaimInput {
        subject: "custom/test/fact".into(),
        kind: "custom.test.observed".into(),
        actor: None,
        fields: BTreeMap::from([("value".into(), json!(1))]),
        evidence: Vec::new(),
        expected_subject: None,
        idempotency_key: Some("replicated-operation".into()),
    };
    let left_claim = left.append_claim(&request).unwrap();
    let right_claim = right.append_claim(&request).unwrap();
    assert_ne!(left_claim.id, right_claim.id);
    assert_eq!(left_claim.operation_id, right_claim.operation_id);

    left.import_replication("right", &right.export_replication(0).unwrap())
        .unwrap();
    let retry = left.append_claim(&request).unwrap();
    assert_eq!(
        retry.id,
        std::cmp::min(left_claim.id.clone(), right_claim.id.clone())
    );
    assert_eq!(left.events_after(0, None).unwrap().len(), 1);
}

#[test]
fn conflicting_replica_operations_make_the_subject_indeterminate() {
    let left = Store::open_memory("left").unwrap();
    let right = Store::open_memory("right").unwrap();
    let request = |value| ClaimInput {
        subject: "custom/test/fact".into(),
        kind: "custom.test.observed".into(),
        actor: None,
        fields: BTreeMap::from([("value".into(), json!(value))]),
        evidence: Vec::new(),
        expected_subject: None,
        idempotency_key: Some("conflicting-operation".into()),
    };
    left.append_claim(&request(1)).unwrap();
    right.append_claim(&request(2)).unwrap();
    left.import_replication("right", &right.export_replication(0).unwrap())
        .unwrap();
    assert!(left.operation_projection_drift().unwrap().is_empty());

    let error = left.append_claim(&request(1)).unwrap_err();
    assert_eq!(error.code, "idempotency-conflict");
    let status = left.status(Some("custom/test/fact")).unwrap();
    assert_eq!(status.subjects[0].reachability, "indeterminate");
    assert!(
        status.subjects[0]
            .reason
            .as_deref()
            .is_some_and(|value| value.contains("idempotency-conflict"))
    );
}

#[test]
fn public_claims_cannot_impersonate_system_producers() {
    let store = Store::open_memory("node").unwrap();
    let error = store
        .append_client_claim(&ClaimInput {
            subject: "host/peer".into(),
            kind: "transport.observed".into(),
            actor: None,
            fields: BTreeMap::from([("status".into(), Value::String("up".into()))]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap_err();
    assert_eq!(error.code, "claim-write-forbidden");
}

#[test]
fn an_agent_can_change_its_account_without_account_existence_coupling() {
    let store = Store::open_memory("node").unwrap();
    let association = |account: &str, actor: &str| ClaimInput {
        subject: "agent/run/worker".into(),
        kind: "agent.account".into(),
        actor: Some(actor.into()),
        fields: BTreeMap::from([("account".into(), Value::String(account.into()))]),
        evidence: Vec::new(),
        expected_subject: None,
        idempotency_key: None,
    };

    store
        .append_client_claim(&association(
            "account/claude/not-declared",
            "agent/run/worker",
        ))
        .unwrap();
    store
        .append_client_claim(&association("account/claude/team-a", "agent/run/worker"))
        .unwrap();
    assert_eq!(
        store
            .latest_actual_value("agent/run/worker")
            .unwrap()
            .unwrap()["account"],
        "account/claude/team-a"
    );
    assert_eq!(
        store
            .append_client_claim(&association("account/claude/team-b", "agent/run/other",))
            .unwrap_err()
            .code,
        "claim-write-forbidden"
    );
}

#[test]
fn once_cardinality_rejects_a_second_local_claim() {
    let store = Store::open_memory("node").unwrap();
    let request = ClaimInput {
        subject: "mission-run/one".into(),
        kind: "mission-run.created".into(),
        actor: None,
        fields: BTreeMap::new(),
        evidence: Vec::new(),
        expected_subject: None,
        idempotency_key: None,
    };
    store.append_claim(&request).unwrap();
    assert_eq!(
        store.append_claim(&request).unwrap_err().code,
        "claim-cardinality"
    );
}

#[test]
fn replicated_once_claims_preserve_concurrent_history() {
    let left = Store::open_memory("left").unwrap();
    let right = Store::open_memory("right").unwrap();
    for (store, verdict) in [(&left, "pass"), (&right, "fail")] {
        store
            .append_claim(&ClaimInput {
                subject: "mission-run/shared-eval".into(),
                kind: "eval.verdict".into(),
                actor: None,
                fields: BTreeMap::from([("verdict".into(), Value::String(verdict.into()))]),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
    }

    let target = Store::open_memory("target").unwrap();
    target
        .import_replication("left", &left.export_replication(0).unwrap())
        .unwrap();
    target
        .import_replication("right", &right.export_replication(0).unwrap())
        .unwrap();
    assert_eq!(
        target
            .claims_for("mission-run/shared-eval", Some("eval.verdict"))
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn resource_kind_and_immutable_facts_are_enforced() {
    let store = Store::open_memory("node").unwrap();
    let observed = |fields| ClaimInput {
        subject: "resource/commit".into(),
        kind: "resource.observed".into(),
        actor: None,
        fields,
        evidence: Vec::new(),
        expected_subject: None,
        idempotency_key: None,
    };
    store
        .append_client_claim(&observed(BTreeMap::from([
            ("kind".into(), Value::String("vcs.commit".into())),
            ("sha".into(), Value::String("abc".into())),
            ("state".into(), Value::String("published".into())),
        ])))
        .unwrap();
    let second = store
        .append_client_claim(&observed(BTreeMap::from([(
            "state".into(),
            Value::String("verified".into()),
        )])))
        .unwrap();
    assert_eq!(second.body["fields"]["kind"], "vcs.commit");
    assert_eq!(
        store
            .append_client_claim(&observed(BTreeMap::from([(
                "sha".into(),
                Value::String("def".into()),
            )])))
            .unwrap_err()
            .code,
        "immutable-resource-field"
    );
    assert_eq!(
        store
            .append_client_claim(&observed(BTreeMap::from([(
                "kind".into(),
                Value::String("ci.run".into()),
            )])))
            .unwrap_err()
            .code,
        "immutable-resource-kind"
    );
}

#[test]
fn explicit_attention_is_idempotent_authorized_and_terminal() {
    let store = Store::open_memory("node").unwrap();
    let request = AttentionRequest {
        reviewer: "alex".into(),
        title: "Fabric needs review".into(),
        reason: "The queue did not recover.".into(),
        severity: "error".into(),
        targets: vec!["resource/fabric/queue".into()],
        actor: "agent/fabric/worker".into(),
        idempotency_key: "attention-fabric-queue".into(),
    };
    let first = store
        .request_attention("attention/fabric-queue", &request)
        .unwrap();
    let retry = store
        .request_attention("attention/fabric-queue", &request)
        .unwrap();
    assert_eq!(first.request, retry.request);
    assert_eq!(first.reviewer, "person/alex");
    assert_eq!(first.status, "pending");
    let items = store.attention_items(Some("person/alex")).unwrap();
    assert!(
        items.is_empty(),
        "historical legacy records do not enter current attention"
    );
    assert!(
        store
            .attention_items(Some("person/someone-else"))
            .unwrap()
            .is_empty()
    );

    let unknown = AttentionResolveRequest {
        delegation: None,
        outcome: "resolved".into(),
        reason: None,
        actor: "client/unknown".into(),
        idempotency_key: "resolve-fabric-unknown".into(),
    };
    assert_eq!(
        store
            .resolve_attention(&first.subject, &unknown)
            .unwrap_err()
            .code,
        "invalid-attention-resolver"
    );
    // Any agent can close an item routed to a person, and is recorded as the resolver.
    let resolution = AttentionResolveRequest {
        delegation: None,
        outcome: "dismissed".into(),
        reason: Some("The fault is expected during maintenance.".into()),
        actor: "agent/fabric/other".into(),
        idempotency_key: "resolve-fabric".into(),
    };
    let closed = store
        .resolve_attention(&first.subject, &resolution)
        .unwrap();
    assert_eq!(closed.status, "dismissed");
    assert_eq!(
        store
            .latest_claim(&first.subject, Some("attention.resolved"))
            .unwrap()
            .unwrap()
            .actor
            .as_deref(),
        Some("agent/fabric/other")
    );
    assert_eq!(closed.outcome.as_deref(), Some("dismissed"));
    assert!(store.attention_items(None).unwrap().is_empty());
    let retry = store
        .resolve_attention(&first.subject, &resolution)
        .unwrap();
    assert_eq!(retry.resolved_at_unix_ms, closed.resolved_at_unix_ms);
}

#[test]
fn a_declared_person_message_stays_in_conversations_and_out_of_attention() {
    let store = Store::open_memory("node").unwrap();
    let source = r#"
version 2
message "human-attention" {
  from "agent/demo/worker"
  to "person/alex"
  title "Please review"
  content "The declarative message is ready."
}
"#;
    let intent = crate::graph::parse_test_intent(source, "node").unwrap();
    let preview = store
        .mission(
            &intent,
            IntentInput {
                kdl: source.into(),
                source_name: None,
            },
        )
        .unwrap();
    store
        .apply(&intent, &preview.subject_tokens, "desired-human-attention")
        .unwrap();

    let mailbox = store.messages(Some("person/alex"), false).unwrap();
    assert_eq!(mailbox.len(), 1);
    assert_eq!(mailbox[0].title.as_deref(), Some("Please review"));
    assert!(
        store
            .attention_items(Some("person/alex"))
            .unwrap()
            .is_empty()
    );
    assert!(store.attention_items(None).unwrap().is_empty());
}

#[test]
fn an_unread_person_message_never_enters_attention() {
    let store = Store::open_memory("node").unwrap();
    let message = store
        .append_claim(&ClaimInput {
            subject: "message/human-attention".into(),
            kind: "message.sent".into(),
            actor: Some("agent/demo/worker".into()),
            fields: BTreeMap::from([
                ("from".into(), Value::String("agent/demo/worker".into())),
                ("to".into(), Value::String("person/alex".into())),
                ("title".into(), Value::String("Can you look?".into())),
                ("content".into(), Value::String("Please read this.".into())),
                ("status".into(), Value::String("sent".into())),
            ]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: Some("human-attention-message".into()),
        })
        .unwrap();
    let quiet = |store: &Store| {
        store
            .attention_items(Some("person/alex"))
            .unwrap()
            .is_empty()
            && store.attention_items(None).unwrap().is_empty()
    };
    assert!(quiet(&store));

    store
        .append_claim(&ClaimInput {
            subject: message.subject.clone(),
            kind: "message.delivered".into(),
            actor: None,
            fields: BTreeMap::from([
                ("status".into(), Value::String("delivered".into())),
                ("recipient".into(), Value::String("person/alex".into())),
            ]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: Some("deliver-human-attention-message".into()),
        })
        .unwrap();
    assert!(quiet(&store));
    assert_eq!(
        store.messages(Some("person/alex"), false).unwrap()[0].status,
        "delivered"
    );
}

#[test]
fn login_attention_skips_healthy_seats_and_retains_legacy_and_runtime_fences() {
    let store = Store::open_memory("node").unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let names = ["native", "legacy", "screen", "stale", "healthy"];
    let source = format!(
        "version 2\n{}",
        names
            .iter()
            .map(|name| format!(
                "agent {name:?} {{ workspace {:?}; harness \"claude\" {{}} }}\n",
                workspace.path().display().to_string()
            ))
            .collect::<String>()
    );
    let intent = parse_intent(&source, "node").unwrap();
    let preview = store
        .mission(
            &intent,
            IntentInput {
                kdl: source,
                source_name: None,
            },
        )
        .unwrap();
    store
        .apply_as(
            &intent,
            &preview.subject_tokens,
            "login-candidates",
            Some("person/avery"),
        )
        .unwrap();
    let append = |name: &str, kind: &str, fields: Value| {
        store
            .append_claim(&ClaimInput {
                subject: format!("agent/node.{name}"),
                kind: kind.into(),
                actor: Some(format!("agent/node.{name}")),
                fields: serde_json::from_value(fields).unwrap(),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
    };
    for name in names {
        append(
            name,
            "runtime.observed",
            json!({"status":"running", "incarnation_id":"current"}),
        );
    }
    append(
        "native",
        "harness.observed",
        json!({"state":"idle", "incarnation_id":"current", "provider_auth":false}),
    );
    append(
        "legacy",
        "harness.observed",
        json!({"state":"blocked", "incarnation_id":"current", "reason":"providerAuth"}),
    );
    append(
        "screen",
        "harness.diagnostic",
        json!({"code":"provider-auth-expired", "incarnation_id":"current", "driver":"claude"}),
    );
    append(
        "stale",
        "harness.observed",
        json!({"state":"idle", "incarnation_id":"prior", "provider_auth":false}),
    );
    append(
        "healthy",
        "harness.observed",
        json!({"state":"idle", "incarnation_id":"current", "provider_auth":null}),
    );
    // These seats share the harness's default login on one host, so one alert names them all.
    let login_subjects = || {
        let logins = store
            .attention_items(Some("person/avery"))
            .unwrap()
            .into_iter()
            .filter(|item| item.kind == "harness-login")
            .collect::<Vec<_>>();
        assert!(logins.len() <= 1, "{logins:?}");
        logins
            .into_iter()
            .flat_map(|item| item.targets)
            .collect::<BTreeSet<_>>()
    };
    let (items, reads) = smallclaims::touched::record(login_subjects);
    assert_eq!(
        items,
        BTreeSet::from([
            "agent/node.native".into(),
            "agent/node.legacy".into(),
            "agent/node.screen".into(),
        ])
    );
    assert!(
        reads.contains("agent/node.stale"),
        "positive evidence still needs its runtime fence"
    );
    assert!(
        !reads.contains("agent/node.healthy"),
        "healthy seats must not rebuild their harness for attention"
    );
    assert!(reads.contains("kind:harness.observed"));
    assert!(reads.contains("kind:harness.diagnostic"));
    for name in ["native", "legacy"] {
        append(
            name,
            "harness.observed",
            json!({"state":"idle", "incarnation_id":"current", "provider_auth":true, "reason":null}),
        );
    }
    append(
        "screen",
        "harness.diagnostic",
        json!({"code":"provider-auth-restored", "incarnation_id":"current", "driver":"claude"}),
    );
    assert!(
        login_subjects().is_empty(),
        "retained negative evidence cannot defeat recovery"
    );
    append(
        "healthy",
        "harness.observed",
        json!({"state":"idle", "incarnation_id":"current", "provider_auth":false}),
    );
    assert_eq!(
        login_subjects(),
        BTreeSet::from(["agent/node.healthy".into()])
    );
}

#[test]
fn one_login_alert_per_login_directory_names_every_seat_that_shares_it() {
    let store = Store::open_memory("node").unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let workspace = workspace.path().display().to_string();
    let source = format!(
        r#"version 2
account "avery/one" {{ provider "anthropic"; owner "person/avery"; plan "max"; login "/logins/one"; }}
account "avery/two" {{ provider "anthropic"; owner "person/avery"; plan "max"; login "/logins/two"; }}
agent "first" {{ workspace {workspace:?}; harness "claude" {{ account "avery/one"; }} }}
agent "second" {{ workspace {workspace:?}; harness "claude" {{ account "avery/one"; }} }}
agent "third" {{ workspace {workspace:?}; harness "claude" {{ account "avery/two"; }} }}
"#
    );
    let intent = parse_intent(&source, "node").unwrap();
    let preview = store
        .mission(
            &intent,
            IntentInput {
                kdl: source,
                source_name: None,
            },
        )
        .unwrap();
    store
        .apply_as(&intent, &preview.subject_tokens, "logins", Some("person/avery"))
        .unwrap();
    for name in ["first", "second", "third"] {
        for (kind, fields) in [
            ("runtime.observed", json!({"status":"running", "incarnation_id":"current"})),
            ("harness.diagnostic", json!({"code":"provider-auth-expired", "incarnation_id":"current", "driver":"claude"})),
        ] {
            store
                .append_claim(&ClaimInput {
                    subject: format!("agent/node.{name}"),
                    kind: kind.into(),
                    actor: Some(format!("agent/node.{name}")),
                    fields: serde_json::from_value(fields).unwrap(),
                    evidence: vec![],
                    expected_subject: None,
                    idempotency_key: None,
                })
                .unwrap();
        }
    }
    let logins = store
        .attention_items(Some("person/avery"))
        .unwrap()
        .into_iter()
        .filter(|item| item.kind == "harness-login")
        .map(|item| (item.targets.clone(), (item.is_alert(), item.conversations())))
        .collect::<BTreeMap<_, _>>();
    let seats = |names: &[&str]| {
        names
            .iter()
            .map(|name| format!("agent/node.{name}"))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        logins,
        BTreeMap::from([
            (seats(&["first", "second"]), (true, seats(&["first", "second"]))),
            (seats(&["third"]), (true, seats(&["third"]))),
        ])
    );
}

/// A native prompt is an alert in its seat's conversation while the harness waits on it, and
/// clears when the harness reports it gone, when the terminal shows it was refused, or when
/// the seat restarts.
#[test]
fn a_native_prompt_is_an_alert_until_its_harness_or_terminal_says_it_is_gone() {
    let store = Store::open_memory("node").unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let source = format!(
        "version 2\nagent \"worker\" {{ workspace {:?}; harness \"claude\" {{}} }}\n",
        workspace.path().display().to_string()
    );
    let intent = parse_intent(&source, "node").unwrap();
    let preview = store
        .mission(
            &intent,
            IntentInput {
                kdl: source,
                source_name: None,
            },
        )
        .unwrap();
    store
        .apply_as(&intent, &preview.subject_tokens, "prompt-seat", Some("person/avery"))
        .unwrap();
    let append = |kind: &str, fields: Value| {
        store
            .append_claim(&ClaimInput {
                subject: "agent/node.worker".into(),
                kind: kind.into(),
                actor: Some("agent/node.worker".into()),
                fields: serde_json::from_value(fields).unwrap(),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap()
    };
    let prompts = || {
        store
            .attention_items(Some("person/avery"))
            .unwrap()
            .into_iter()
            .filter(|item| item.kind == "harness-prompt")
            .collect::<Vec<_>>()
    };
    append("runtime.observed", json!({"status":"running", "incarnation_id":"one"}));
    assert!(prompts().is_empty());
    let asked = append(
        "harness.observed",
        json!({"state":"working", "incarnation_id":"one", "blocked_on":"human",
            "ask":"permission", "reason":"Deploy production?"}),
    );
    let shown = prompts();
    assert_eq!(shown.len(), 1, "{shown:?}");
    assert_eq!(shown[0].episode, asked.id);
    assert!(shown[0].is_alert());
    assert_eq!(shown[0].conversation.as_deref(), Some("agent/node.worker"));
    assert!(shown[0].detail.starts_with("Deploy production?"));
    // Answered in the terminal: the harness reports it is no longer blocked.
    append(
        "harness.observed",
        json!({"state":"working", "incarnation_id":"one", "blocked_on":null, "ask":null}),
    );
    assert!(prompts().is_empty());
    // Asked again, then refused in the terminal, which only the screen shows.
    let again = append(
        "harness.observed",
        json!({"state":"working", "incarnation_id":"one", "blocked_on":"human", "ask":"question"}),
    );
    assert_eq!(prompts()[0].episode, again.id);
    store
        .append_claim(&ClaimInput {
            subject: "agent/node.worker".into(),
            kind: "harness.diagnostic".into(),
            actor: Some("agent/node.worker".into()),
            fields: serde_json::from_value(
                json!({"code":"native-prompt-gone", "incarnation_id":"one"}),
            )
            .unwrap(),
            evidence: vec![again.id.clone()],
            expected_subject: None,
            idempotency_key: Some(attention_snapshot::native_prompt_gone_key(&again.id)),
        })
        .unwrap();
    assert!(prompts().is_empty());
    // A prompt from an incarnation that is no longer running waits on nobody.
    append(
        "harness.observed",
        json!({"state":"working", "incarnation_id":"one", "blocked_on":"human", "ask":"permission"}),
    );
    assert_eq!(prompts().len(), 1);
    append("runtime.observed", json!({"status":"running", "incarnation_id":"two"}));
    assert!(prompts().is_empty());
}

#[test]
fn claude_native_prompt_lookup_window_never_infers_missing_episode_closure() {
    let store = Store::open_memory("node").unwrap();
    let seat = "agent/node.worker";
    let observe = |ownership: u64, transition: u64, state: &str, blocked: &str, ask: &str| {
        store.append_claim(&ClaimInput {
            subject: seat.into(),
            kind: "harness.observed".into(),
            actor: Some(seat.into()),
            fields: serde_json::from_value(json!({
                "driver":"claude", "state":state, "incarnation_id":"one",
                "blocked_on":blocked, "ask":ask,
                "ownership_sequence":ownership, "transition_sequence":transition,
            })).unwrap(),
            evidence: vec![],
            expected_subject: None,
            idempotency_key: None,
        }).unwrap()
    };
    for _ in 0..20 {
        observe(1, 1, "idle", "none", "none");
    }
    // The new writer can precede its publication even with a full retained window.
    assert_eq!(store.native_prompt_state(seat, 1, 2).unwrap(), NativePromptState::Open);
    for _ in 0..20 {
        observe(1, 2, "indeterminate", "unknown", "unknown");
    }
    assert_eq!(store.native_prompt_state(seat, 1, 2).unwrap(), NativePromptState::Open);
    observe(1, 2, "working", "human", "permission");
    assert_eq!(store.native_prompt_state(seat, 1, 2).unwrap(), NativePromptState::Open);
    for _ in 0..20 {
        observe(1, 2, "working", "human", "permission");
    }
    assert_eq!(store.native_prompt_state(seat, 1, 2).unwrap(), NativePromptState::Open);
    // Only positive successor evidence releases a record outside the bounded window.
    for _ in 0..20 {
        observe(1, 3, "idle", "none", "none");
    }
    assert_eq!(store.native_prompt_state(seat, 1, 2).unwrap(), NativePromptState::Gone);
    assert_eq!(store.native_prompt_state(seat, 1, 4).unwrap(), NativePromptState::Open);
}

#[test]
fn claude_native_human_release_is_episode_fenced_durable_and_shared_with_sender_status() {
    for ask in ["permission", "question", "review"] {
        let store = Store::open_memory("node").unwrap();
        let seat = "agent/node.worker";
        let append = |kind: &str, fields: Value, key: Option<String>| {
            store.append_claim(&ClaimInput {
                subject: seat.into(), kind: kind.into(), actor: Some(seat.into()),
                fields: serde_json::from_value(fields).unwrap(), evidence: vec![],
                expected_subject: None, idempotency_key: key,
            }).unwrap()
        };
        append("runtime.observed", json!({"status":"running", "incarnation_id":"one"}), None);
        let blocked = |transition| json!({
            "state":"working", "driver":"claude", "incarnation_id":"one",
            "blocked_on":"human", "ask":ask, "reason":"native prompt",
            "ownership_sequence":1, "transition_sequence":transition
        });
        let asked = append("harness.observed", blocked(3), None);
        let before = store.index().unwrap();
        assert_eq!(store.native_prompt_state(seat, 1, 3).unwrap(), NativePromptState::Open, "{ask}");
        assert_eq!(store.observed_harness_at(seat, before).unwrap().unwrap().blocked_on.as_deref(), Some("human"));
        append("harness.diagnostic", json!({
            "code":"native-prompt-gone", "status":"resolved",
            "driver":"claude", "incarnation_id":"one"
        }), Some(native_prompt_gone_key(&asked.id)));
        assert_eq!(store.native_prompt_state(seat, 1, 3).unwrap(), NativePromptState::Gone, "{ask}");
        assert_eq!(store.observed_harness_at(seat, before).unwrap().unwrap().blocked_on.as_deref(), Some("human"));
        if ask == "permission" {
            // A replicated answer request can arrive after native closure.
            // It must not resurrect the sender's held projection.
            append("harness.diagnostic", json!({
                "code":"native-prompt-answered", "status":"allow",
                "driver":"claude", "incarnation_id":"one"
            }), Some(format!("native-prompt-answer:{}", asked.id)));
            assert_eq!(store.native_prompt_state(seat, 1, 3).unwrap(), NativePromptState::Gone);
            assert!(store.observed_harness_at(seat, store.index().unwrap()).unwrap().unwrap().blocked_on.is_none());
        }
        for _ in 0..20 {
            append("harness.observed", blocked(3), None);
        }
        // Esc/No emits no hook event; the blocked driver file can heartbeat
        // for longer than the bounded recent-observation lookup window.
        assert_eq!(store.native_prompt_state(seat, 1, 3).unwrap(), NativePromptState::Gone, "{ask}");
        let released = store.observed_harness_at(seat, store.index().unwrap()).unwrap().unwrap();
        assert!(released.blocked_on.is_none(), "{ask}");
        assert!(released.ask.is_none(), "{ask}");
        assert_eq!(released.state, "idle", "{ask}");
        // A full history window cannot release a not-yet-published new prompt.
        assert_eq!(store.native_prompt_state(seat, 1, 999).unwrap(), NativePromptState::Open);
        append("harness.observed", blocked(4), None);
        assert_eq!(store.native_prompt_state(seat, 1, 4).unwrap(), NativePromptState::Open, "{ask}");
        assert_eq!(store.observed_harness_at(seat, store.index().unwrap()).unwrap().unwrap().blocked_on.as_deref(), Some("human"));
        // Staleness is uncertainty, not prompt dismissal. Its derived
        // snapshot retains the driver's ownership/transition counters.
        append("harness.observed", json!({
            "state":"indeterminate", "driver":"claude", "incarnation_id":"one",
            "blocked_on":"unknown", "ask":"unknown", "reason":"stale",
            "ownership_sequence":1, "transition_sequence":4
        }), None);
        assert_eq!(store.native_prompt_state(seat, 1, 4).unwrap(), NativePromptState::Open, "{ask}");
        append("harness.observed", blocked(4), None);
        assert_eq!(store.native_prompt_state(seat, 1, 4).unwrap(), NativePromptState::Open, "{ask}");
        assert_eq!(store.observed_harness_at(seat, store.index().unwrap()).unwrap().unwrap().blocked_on.as_deref(), Some("human"));
        // A native timeout that ends the turn, unlike the hook's polling
        // timeout while a dialog stays open, retires the hold.
        append("harness.observed", json!({
            "state":"ended", "driver":"claude", "incarnation_id":"one",
            "ownership_sequence":1, "transition_sequence":5, "exit":"timeout"
        }), None);
        assert_eq!(store.native_prompt_state(seat, 1, 4).unwrap(), NativePromptState::Gone);
        assert!(store.observed_harness_at(seat, store.index().unwrap()).unwrap().unwrap().blocked_on.is_none());
        append("runtime.observed", json!({"status":"running", "incarnation_id":"two"}), None);
        append("harness.observed", json!({
            "state":"ready", "driver":"claude", "incarnation_id":"two",
            "ownership_sequence":2, "transition_sequence":1
        }), None);
        assert!(store.observed_harness_at(seat, store.index().unwrap()).unwrap().unwrap().blocked_on.is_none());
    }
}

/// A Claude permission prompt can be answered from a client: only by the seat's person, only
/// while it waits, and once. The hook sees the answer; the alert goes once it is answered.
#[test]
fn a_claude_permission_prompt_is_answered_once_by_its_person_while_it_waits() {
    let store = Store::open_memory("node").unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let source = format!(
        "version 2\nagent \"worker\" {{ workspace {:?}; harness \"claude\" {{}} }}\n",
        workspace.path().display().to_string()
    );
    let intent = parse_intent(&source, "node").unwrap();
    let preview = store
        .mission(
            &intent,
            IntentInput {
                kdl: source,
                source_name: None,
            },
        )
        .unwrap();
    store
        .apply_as(&intent, &preview.subject_tokens, "answer-seat", Some("person/avery"))
        .unwrap();
    let seat = "agent/node.worker";
    let append = |kind: &str, fields: Value| {
        store
            .append_claim(&ClaimInput {
                subject: seat.into(),
                kind: kind.into(),
                actor: Some(seat.into()),
                fields: serde_json::from_value(fields).unwrap(),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap()
    };
    // The hook waits on its own observation, named by its state record's sequences.
    assert_eq!(
        store.native_prompt_state(seat, 1, 3).unwrap(),
        NativePromptState::Open,
        "the hook's observation is not published yet"
    );
    append("runtime.observed", json!({"status":"running", "incarnation_id":"one"}));
    let asked = append(
        "harness.observed",
        json!({"state":"working", "driver":"claude", "incarnation_id":"one",
            "blocked_on":"human", "ask":"permission", "reason":"Bash",
            "ownership_sequence":1, "transition_sequence":3}),
    );
    let prompt = || {
        store
            .attention_items(Some("person/avery"))
            .unwrap()
            .into_iter()
            .find(|item| item.kind == "harness-prompt")
    };
    assert_eq!(
        prompt().unwrap().answers,
        vec!["allow".to_owned(), "deny".to_owned()]
    );
    assert_eq!(
        store
            .answer_native_prompt(seat, &asked.id, "allow", "person/other")
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(
        store
            .answer_native_prompt(seat, "an-older-prompt", "allow", "person/avery")
            .unwrap_err()
            .code,
        "stale-fence"
    );
    assert_eq!(
        store.native_prompt_state(seat, 1, 3).unwrap(),
        NativePromptState::Open
    );
    store
        .answer_native_prompt(seat, &asked.id, "deny", "person/avery")
        .unwrap();
    assert_eq!(
        store.native_prompt_state(seat, 1, 3).unwrap(),
        NativePromptState::Answered {
            answer: "deny".into()
        }
    );
    let answered = store.observed_harness_at(seat, store.index().unwrap()).unwrap().unwrap();
    assert_eq!(answered.blocked_on.as_deref(), Some("human"));
    assert_eq!(answered.ask.as_deref(), Some("permission"));
    // Recording the answer does not prove the native polling hook applied it.
    assert!(prompt().is_none(), "an answered prompt is no longer an alert");
    assert!(
        store
            .answer_native_prompt(seat, &asked.id, "allow", "person/avery")
            .is_err(),
        "a prompt is answered once"
    );
    // Claude acts on the answer and moves on: a hook still waiting sees it gone.
    append(
        "harness.observed",
        json!({"state":"working", "driver":"claude", "incarnation_id":"one",
            "blocked_on":null, "ask":null, "ownership_sequence":1, "transition_sequence":4}),
    );
    assert_eq!(
        store.native_prompt_state(seat, 1, 3).unwrap(),
        NativePromptState::Gone
    );
    // The next prompt is its own: the earlier prompt's answer never answers it.
    append(
        "harness.observed",
        json!({"state":"working", "driver":"claude", "incarnation_id":"one",
            "blocked_on":"human", "ask":"permission", "reason":"Write",
            "ownership_sequence":1, "transition_sequence":5}),
    );
    assert_eq!(
        store.native_prompt_state(seat, 1, 5).unwrap(),
        NativePromptState::Open
    );
    // Only the tool is stored; the call itself is read from the seat's conversation.
    assert!(prompt().unwrap().detail.starts_with("Claude asks to use Write;"));
    // A later observation of the same waiting prompt: an answer to it is still the hook's.
    let restated = append(
        "harness.observed",
        json!({"state":"working", "driver":"claude", "incarnation_id":"one",
            "blocked_on":"human", "ask":"permission", "reason":"Write",
            "provider_auth": true, "ownership_sequence":1, "transition_sequence":6}),
    );
    assert_eq!(prompt().unwrap().episode, restated.id);
    store
        .answer_native_prompt(seat, &restated.id, "allow", "person/avery")
        .unwrap();
    assert_eq!(
        store.native_prompt_state(seat, 1, 5).unwrap(),
        NativePromptState::Answered {
            answer: "allow".into()
        }
    );
    // A prompt that does not say what it would run can only be answered in the terminal.
    append(
        "harness.observed",
        json!({"state":"working", "driver":"claude", "incarnation_id":"one",
            "blocked_on":"human", "ask":"permission", "reason":"permissionRequest",
            "ownership_sequence":1, "transition_sequence":7}),
    );
    assert!(prompt().unwrap().answers.is_empty());
    // A question is answered in the terminal: it is an alert without answers.
    let question = append(
        "harness.observed",
        json!({"state":"working", "driver":"claude", "incarnation_id":"one",
            "blocked_on":"human", "ask":"question"}),
    );
    assert!(prompt().unwrap().answers.is_empty());
    assert_eq!(
        store
            .answer_native_prompt(seat, &question.id, "allow", "person/avery")
            .unwrap_err()
            .code,
        "stale-fence"
    );
}

/// A driver writes each prompt as its own transition, so a later prompt for the same tool is
/// another prompt: an answer to it never reaches the earlier prompt's waiter. A restatement
/// of the same transition, or a credential edge, is still the same prompt.
#[test]
fn a_later_prompt_for_the_same_tool_is_another_prompt() {
    let store = Store::open_memory("node").unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let source = format!(
        "version 2\nagent \"worker\" {{ workspace {:?}; harness \"codex\" {{}} }}\n",
        workspace.path().display().to_string()
    );
    let intent = parse_intent(&source, "node").unwrap();
    let preview = store
        .mission(
            &intent,
            IntentInput {
                kdl: source,
                source_name: None,
            },
        )
        .unwrap();
    store
        .apply_as(&intent, &preview.subject_tokens, "two-prompts", Some("person/avery"))
        .unwrap();
    let seat = "agent/node.worker";
    let append = |fields: Value| {
        store
            .append_claim(&ClaimInput {
                subject: seat.into(),
                kind: "harness.observed".into(),
                actor: Some(seat.into()),
                fields: serde_json::from_value(fields).unwrap(),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap()
    };
    let waiting = |transition: u64, auth: Value| {
        json!({"state":"working", "driver":"codex", "incarnation_id":"one",
            "blocked_on":"human", "ask":"permission", "reason":"commandExecution",
            "provider_auth": auth, "ownership_sequence":1, "transition_sequence":transition})
    };
    store
        .append_claim(&ClaimInput {
            subject: seat.into(),
            kind: "runtime.observed".into(),
            actor: Some(seat.into()),
            fields: serde_json::from_value(json!({"status":"running", "incarnation_id":"one"}))
                .unwrap(),
            evidence: vec![],
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
    append(waiting(3, Value::Null));
    // Restated: the same prompt.
    append(waiting(3, Value::Null));
    assert_eq!(store.native_prompt_state(seat, 1, 3).unwrap(), NativePromptState::Open);
    // A credential edge moves the transition without a new prompt.
    let restated = append(waiting(4, json!(true)));
    store
        .answer_native_prompt(seat, &restated.id, "allow", "person/avery")
        .unwrap();
    assert_eq!(
        store.native_prompt_state(seat, 1, 3).unwrap(),
        NativePromptState::Answered {
            answer: "allow".into()
        }
    );
    // The next request is its own transition: its answer is never the earlier waiter's.
    let next = append(waiting(5, json!(true)));
    store
        .answer_native_prompt(seat, &next.id, "deny", "person/avery")
        .unwrap();
    assert_eq!(store.native_prompt_state(seat, 1, 3).unwrap(), NativePromptState::Gone);
    assert_eq!(
        store.native_prompt_state(seat, 1, 5).unwrap(),
        NativePromptState::Answered {
            answer: "deny".into()
        }
    );
}

/// Claude's question (AskUserQuestion) is answered from a client with the person's choices, as
/// positions (question to options chosen), or declined. Only positions are recorded; the hook
/// turns them into the call's own labels.
#[test]
fn a_claude_question_is_answered_with_its_choices() {
    let store = Store::open_memory("node").unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let source = format!(
        "version 2\nagent \"worker\" {{ workspace {:?}; harness \"claude\" {{}} }}\n",
        workspace.path().display().to_string()
    );
    let intent = parse_intent(&source, "node").unwrap();
    let preview = store
        .mission(
            &intent,
            IntentInput {
                kdl: source,
                source_name: None,
            },
        )
        .unwrap();
    store
        .apply_as(&intent, &preview.subject_tokens, "question-seat", Some("person/avery"))
        .unwrap();
    let seat = "agent/node.worker";
    let append = |kind: &str, fields: Value| {
        store
            .append_claim(&ClaimInput {
                subject: seat.into(),
                kind: kind.into(),
                actor: Some(seat.into()),
                fields: serde_json::from_value(fields).unwrap(),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap()
    };
    let prompt = || {
        store
            .attention_items(Some("person/avery"))
            .unwrap()
            .into_iter()
            .find(|item| item.kind == "harness-prompt")
    };
    append("runtime.observed", json!({"status":"running", "incarnation_id":"one"}));
    let asked = append(
        "harness.observed",
        json!({"state":"working", "driver":"claude", "incarnation_id":"one",
            "blocked_on":"human", "ask":"question", "reason":"AskUserQuestion",
            "ownership_sequence":1, "transition_sequence":3}),
    );
    let shown = prompt().unwrap();
    assert_eq!(shown.answers, vec!["answer".to_owned(), "deny".to_owned()]);
    assert!(shown.title.ends_with("is waiting for an answer"));
    assert!(shown.detail.starts_with("Claude asks you a question;"), "{}", shown.detail);
    // A question is never allowed, and its choices name each question with a choice.
    // Positions only: never the questions' or options' words, which are conversation content.
    for wrong in ["allow", "{}", r#"{"0":[]}"#, r#"["Blue"]"#, r#"{"Which color?":"Blue"}"#, r#"{"0":[1,1]}"#, r#"{"0":"1"}"#] {
        assert_eq!(
            store
                .answer_native_prompt(seat, &asked.id, wrong, "person/avery")
                .unwrap_err()
                .code,
            "invalid-answer",
            "{wrong}"
        );
    }
    let choices = r#"{"0":[1],"1":[0,2]}"#;
    store
        .answer_native_prompt(seat, &asked.id, choices, "person/avery")
        .unwrap();
    let NativePromptState::Answered { answer } = store.native_prompt_state(seat, 1, 3).unwrap()
    else {
        panic!("the question is answered");
    };
    assert_eq!(
        serde_json::from_str::<Value>(&answer).unwrap(),
        json!({"0": [1], "1": [0, 2]})
    );
    assert!(prompt().is_none());
    // The next question is declined.
    append(
        "harness.observed",
        json!({"state":"working", "driver":"claude", "incarnation_id":"one",
            "blocked_on":null, "ask":null, "ownership_sequence":1, "transition_sequence":4}),
    );
    let next = append(
        "harness.observed",
        json!({"state":"working", "driver":"claude", "incarnation_id":"one",
            "blocked_on":"human", "ask":"question", "reason":"AskUserQuestion",
            "ownership_sequence":1, "transition_sequence":5}),
    );
    store
        .answer_native_prompt(seat, &next.id, "deny", "person/avery")
        .unwrap();
    assert_eq!(
        store.native_prompt_state(seat, 1, 5).unwrap(),
        NativePromptState::Answered {
            answer: "deny".into()
        }
    );
    // A permission is never answered with choices.
    let permission = append(
        "harness.observed",
        json!({"state":"working", "driver":"claude", "incarnation_id":"one",
            "blocked_on":"human", "ask":"permission", "reason":"Bash",
            "ownership_sequence":1, "transition_sequence":6}),
    );
    assert_eq!(
        store
            .answer_native_prompt(seat, &permission.id, choices, "person/avery")
            .unwrap_err()
            .code,
        "invalid-answer"
    );
}

/// A subagent asks while its parent's Task call is still open: the only call the seat's
/// conversation shows unanswered is the parent's, so an allow offered beside it would let
/// through a call the person never saw. st offers only deny for a subagent's prompt.
#[test]
fn a_subagents_prompt_is_answered_only_with_deny() {
    let store = Store::open_memory("node").unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let source = format!(
        "version 2\nagent \"worker\" {{ workspace {:?}; harness \"claude\" {{}} }}\n",
        workspace.path().display().to_string()
    );
    let intent = parse_intent(&source, "node").unwrap();
    let preview = store
        .mission(
            &intent,
            IntentInput {
                kdl: source,
                source_name: None,
            },
        )
        .unwrap();
    store
        .apply_as(&intent, &preview.subject_tokens, "subagent-seat", Some("person/avery"))
        .unwrap();
    let seat = "agent/node.worker";
    let append = |kind: &str, fields: Value| {
        store
            .append_claim(&ClaimInput {
                subject: seat.into(),
                kind: kind.into(),
                actor: Some(seat.into()),
                fields: serde_json::from_value(fields).unwrap(),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap()
    };
    append("runtime.observed", json!({"status":"running", "incarnation_id":"one"}));
    // The seat's own Task call is open (its conversation shows it unanswered), and the
    // subagent it started asks to run Bash.
    let asked = append(
        "harness.observed",
        json!({"state":"working", "driver":"claude", "incarnation_id":"one",
            "blocked_on":"human", "ask":"permission", "reason":"subagent:Bash",
            "ownership_sequence":1, "transition_sequence":3}),
    );
    let shown = store
        .attention_items(Some("person/avery"))
        .unwrap()
        .into_iter()
        .find(|item| item.kind == "harness-prompt")
        .unwrap();
    assert_eq!(shown.answers, vec!["deny".to_owned()]);
    assert!(shown.detail.starts_with("A subagent of this seat asks to use Bash;"), "{}", shown.detail);
    assert_eq!(
        store
            .answer_native_prompt(seat, &asked.id, "allow", "person/avery")
            .unwrap_err()
            .code,
        "invalid-answer"
    );
    store
        .answer_native_prompt(seat, &asked.id, "deny", "person/avery")
        .unwrap();
    assert_eq!(
        store.native_prompt_state(seat, 1, 3).unwrap(),
        NativePromptState::Answered {
            answer: "deny".into()
        }
    );
}

/// A prompt names the exact call it asks about, as its hook published it for that very
/// observation; a later prompt's call is never an earlier one's, and an unnamed call is none.
#[test]
fn a_prompt_names_the_exact_call_its_hook_published() {
    let store = Store::open_memory("node").unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let source = format!(
        "version 2\nagent \"worker\" {{ workspace {:?}; harness \"claude\" {{}} }}\n",
        workspace.path().display().to_string()
    );
    let intent = parse_intent(&source, "node").unwrap();
    let preview = store
        .mission(
            &intent,
            IntentInput {
                kdl: source,
                source_name: None,
            },
        )
        .unwrap();
    store
        .apply_as(&intent, &preview.subject_tokens, "call-seat", Some("person/avery"))
        .unwrap();
    let seat = "agent/node.worker";
    let append = |kind: &str, fields: Value| {
        store
            .append_claim(&ClaimInput {
                subject: seat.into(),
                kind: kind.into(),
                actor: Some(seat.into()),
                fields: serde_json::from_value(fields).unwrap(),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap()
    };
    let call_of_prompt = || {
        store
            .attention_items(Some("person/avery"))
            .unwrap()
            .into_iter()
            .find(|item| item.kind == "harness-prompt")
            .unwrap()
            .prompt_call
    };
    let asked = |transition: u64| {
        append(
            "harness.observed",
            json!({"state":"working", "driver":"claude", "incarnation_id":"one",
                "blocked_on":"human", "ask":"permission", "reason":"Bash",
                "ownership_sequence":1, "transition_sequence":transition}),
        )
    };
    let published = |transition: u64, call: &str| {
        append(
            "harness.diagnostic",
            json!({"severity":"warning", "status":"shown", "code":"native-prompt-call",
                "driver":"claude", "ownership_sequence":1,
                "matched_line": json!({"call_id": call, "transition": transition}).to_string()}),
        )
    };
    append("runtime.observed", json!({"status":"running", "incarnation_id":"one"}));
    asked(3);
    assert_eq!(call_of_prompt(), None, "not named yet: allowed only in the terminal");
    published(3, "toolu_7");
    assert_eq!(call_of_prompt().as_deref(), Some("toolu_7"));
    // The next prompt is its own observation: the earlier call is not its call.
    append(
        "harness.observed",
        json!({"state":"working", "driver":"claude", "incarnation_id":"one",
            "blocked_on":null, "ask":null, "ownership_sequence":1, "transition_sequence":4}),
    );
    asked(5);
    assert_eq!(call_of_prompt(), None);
    published(5, "toolu_9");
    assert_eq!(call_of_prompt().as_deref(), Some("toolu_9"));
}

/// A denied call waits on its person only until the seat moves on: once the seat begins another
/// turn or its session ends, the alert goes and an answer is refused. The turn the denial ended
/// is still observed working for a moment, and that is no new turn; nor is the seat at rest,
/// however often it is observed. The claims stay: only the alert they derived goes.
#[test]
fn a_classifier_denial_goes_once_the_seat_moves_on() {
    let store = Store::open_memory("node").unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let source = format!(
        "version 2\nagent \"worker\" {{ workspace {:?}; harness \"claude\" {{}} }}\n",
        workspace.path().display().to_string()
    );
    let intent = parse_intent(&source, "node").unwrap();
    let preview = store
        .mission(
            &intent,
            IntentInput {
                kdl: source,
                source_name: None,
            },
        )
        .unwrap();
    store
        .apply_as(&intent, &preview.subject_tokens, "denial-seat", Some("person/avery"))
        .unwrap();
    let seat = "agent/node.worker";
    let append = |kind: &str, fields: Value, key: Option<&str>| {
        store
            .append_claim(&ClaimInput {
                subject: seat.into(),
                kind: kind.into(),
                actor: Some(seat.into()),
                fields: serde_json::from_value(fields).unwrap(),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: key.map(str::to_owned),
            })
            .unwrap()
    };
    let denied = |call: &str| {
        append(
            "harness.diagnostic",
            json!({"severity":"warning", "status":"Bash", "code":"permission-denied",
                "reason":"Production Reads", "driver":"claude",
                "session_id":"session-1", "matched_line": json!({"call_id": call}).to_string()}),
            Some(&format!("claude-permission-denied:{seat}:{call}")),
        )
    };
    // The driver names when it saw each state; the store keeps that as the state's start when
    // the state changes, and carries the start over a refresh of the same state.
    let observed = |state: &str, at: Option<u128>| {
        let mut fields = json!({"state": state, "driver": "claude", "blocked_on": "none", "ask": "none"});
        if let Some(at) = at {
            fields["observed_at_ms"] = json!(at as u64);
        }
        append("harness.observed", fields, None)
    };
    let alerts = || {
        store
            .attention_items(Some("person/avery"))
            .unwrap()
            .into_iter()
            .filter(|item| item.kind == "harness-prompt")
            .map(|item| item.episode)
            .collect::<Vec<_>>()
    };
    append("runtime.observed", json!({"status":"running", "incarnation_id":"one"}), None);
    let first = denied("toolu_1");
    // The turn that ended in the denial, then the seat at rest, both seen before the `Stop` hook
    // raised it: still waiting on the person.
    let ending = observed("working", Some(first.accepted_at_unix_ms - 1_000));
    assert_eq!(ending.body["fields"]["observed_since_ms"], (first.accepted_at_unix_ms - 1_000) as u64);
    observed("idle", Some(first.accepted_at_unix_ms - 500));
    assert_eq!(alerts(), vec![first.id.clone()]);
    // However long the seat rests, refreshed as it is, the call still waits.
    for refresh in 1..=40 {
        observed("idle", Some(first.accepted_at_unix_ms + refresh * 300_000));
    }
    assert_eq!(alerts(), vec![first.id.clone()], "a resting seat's call still waits");
    // Another turn begins: the call waits on nobody.
    std::thread::sleep(std::time::Duration::from_millis(5));
    observed("working", None);
    assert!(alerts().is_empty(), "the seat moved on");
    assert_eq!(
        store.answer_native_prompt(seat, &first.id, "approve-once", "person/avery").unwrap_err().code,
        "stale-fence"
    );
    // A session that ends leaves nothing waiting either.
    let second = denied("toolu_2");
    assert_eq!(alerts(), vec![second.id.clone()]);
    observed("ended", None);
    assert!(alerts().is_empty(), "the session ended");
}

/// A Codex approval prompt is answered the same way. The answer names the Codex driver, whose
/// control connection reads it and answers the app-server; a Codex question stays terminal-only.
#[test]
fn a_codex_permission_prompt_is_answered_and_records_its_driver() {
    let store = Store::open_memory("node").unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let source = format!(
        "version 2\nagent \"worker\" {{ workspace {:?}; harness \"codex\" {{}} }}\n",
        workspace.path().display().to_string()
    );
    let intent = parse_intent(&source, "node").unwrap();
    let preview = store
        .mission(
            &intent,
            IntentInput {
                kdl: source,
                source_name: None,
            },
        )
        .unwrap();
    store
        .apply_as(&intent, &preview.subject_tokens, "answer-seat", Some("person/avery"))
        .unwrap();
    let seat = "agent/node.worker";
    let append = |kind: &str, fields: Value| {
        store
            .append_claim(&ClaimInput {
                subject: seat.into(),
                kind: kind.into(),
                actor: Some(seat.into()),
                fields: serde_json::from_value(fields).unwrap(),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap()
    };
    let prompt = || {
        store
            .attention_items(Some("person/avery"))
            .unwrap()
            .into_iter()
            .find(|item| item.kind == "harness-prompt")
    };
    append("runtime.observed", json!({"status":"running", "incarnation_id":"one"}));
    // A wait the driver knows only from the thread's flag has no request st could answer.
    let flagged = append(
        "harness.observed",
        json!({"state":"working", "driver":"codex", "incarnation_id":"one",
            "blocked_on":"human", "ask":"permission", "reason":"waitingOnApproval",
            "ownership_sequence":1, "transition_sequence":2}),
    );
    assert!(prompt().unwrap().answers.is_empty());
    assert!(prompt().unwrap().detail.contains("did not see requested"));
    assert_eq!(
        store
            .answer_native_prompt(seat, &flagged.id, "allow", "person/avery")
            .unwrap_err()
            .code,
        "stale-fence"
    );
    // The driver waits on the observation its approval wrote, named by the record's sequences,
    // and the observation names what the request asks to do.
    let asked = append(
        "harness.observed",
        json!({"state":"working", "driver":"codex", "incarnation_id":"one",
            "blocked_on":"human", "ask":"permission", "reason":"commandExecution",
            "ownership_sequence":1, "transition_sequence":3}),
    );
    assert_eq!(
        prompt().unwrap().answers,
        vec!["allow".to_owned(), "deny".to_owned()]
    );
    assert!(prompt().unwrap().detail.starts_with("Codex asks to run a command;"));
    assert_eq!(
        store.native_prompt_state(seat, 1, 3).unwrap(),
        NativePromptState::Open
    );
    let answer = store
        .answer_native_prompt(seat, &asked.id, "allow", "person/avery")
        .unwrap();
    assert_eq!(answer.body["fields"]["driver"], "codex");
    assert_eq!(
        store.native_prompt_state(seat, 1, 3).unwrap(),
        NativePromptState::Answered {
            answer: "allow".into()
        }
    );
    assert!(prompt().is_none(), "an answered prompt is no longer an alert");
    // The next approval is its own: the earlier answer never answers it.
    append(
        "harness.observed",
        json!({"state":"working", "driver":"codex", "incarnation_id":"one",
            "blocked_on":null, "ask":null, "ownership_sequence":1, "transition_sequence":4}),
    );
    append(
        "harness.observed",
        json!({"state":"working", "driver":"codex", "incarnation_id":"one",
            "blocked_on":"human", "ask":"permission", "reason":"fileChange",
            "ownership_sequence":1, "transition_sequence":5}),
    );
    assert!(prompt().unwrap().detail.starts_with("Codex asks to change files;"));
    assert_eq!(
        store.native_prompt_state(seat, 1, 3).unwrap(),
        NativePromptState::Gone
    );
    assert_eq!(
        store.native_prompt_state(seat, 1, 5).unwrap(),
        NativePromptState::Open
    );
    let question = append(
        "harness.observed",
        json!({"state":"working", "driver":"codex", "incarnation_id":"one",
            "blocked_on":"human", "ask":"question", "reason":"waitingOnUserInput",
            "ownership_sequence":1, "transition_sequence":6}),
    );
    assert!(prompt().unwrap().answers.is_empty());
    assert_eq!(
        store
            .answer_native_prompt(seat, &question.id, "allow", "person/avery")
            .unwrap_err()
            .code,
        "stale-fence"
    );
}

#[test]
fn current_incarnation_work_activity_recovers_a_stale_harness_projection() {
    let store = Store::open_memory("node").unwrap();
    let subject = "agent/node.worker";
    store
        .append_claim(&ClaimInput {
            subject: subject.into(),
            kind: "runtime.observed".into(),
            actor: None,
            fields: BTreeMap::from([
                ("status".into(), Value::String("running".into())),
                ("runtime_id".into(), Value::String("node.worker".into())),
                ("terminal".into(), Value::Bool(true)),
                ("incarnation_id".into(), Value::String("current".into())),
            ]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: Some("activity-runtime".into()),
        })
        .unwrap();
    store
        .append_claim(&ClaimInput {
            subject: subject.into(),
            kind: "harness.observed".into(),
            actor: Some(subject.into()),
            fields: BTreeMap::from([
                ("state".into(), Value::String("indeterminate".into())),
                ("driver".into(), Value::String("codex".into())),
                ("incarnation_id".into(), Value::String("current".into())),
                ("reason".into(), Value::String("stale".into())),
            ]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: Some("activity-stale".into()),
        })
        .unwrap();
    assert_eq!(
        store
            .current_harness(subject)
            .unwrap()
            .unwrap()
            .reason
            .as_deref(),
        Some("stale")
    );

    {
        let mut connection = store.connection.lock().unwrap();
        let transaction = connection.transaction().unwrap();
        append_claim_tx(
            &transaction,
            "node",
            "step-run/activity/work",
            "work.claimed",
            Some(subject),
            &json!({"fields": {
                "status": "claimed",
                "attempt": 1,
                "readiness_epoch": 1,
                "claimant": subject,
                "claim_incarnation": "current",
                "claim_expires_at_unix_ms": now_ms().saturating_add(60_000),
                "worker_reported": false,
                "summary": null,
                "reason": null
            }}),
            &[],
            None,
        )
        .unwrap();
        transaction.commit().unwrap();
    }

    let recovered = store.current_harness(subject).unwrap().unwrap();
    assert_eq!(recovered.state, "working");
    assert!(recovered.is_ready());
    assert_eq!(recovered.reason, None);
    assert_eq!(recovered.driver.as_deref(), Some("codex"));

    // A lease renewal says only that the process is alive enough to extend its lease.
    // It must not erase a newer observation that the harness is stuck in a command.
    store
        .append_claim(&ClaimInput {
            subject: subject.into(),
            kind: "harness.observed".into(),
            actor: Some(subject.into()),
            fields: BTreeMap::from([
                ("state".into(), Value::String("indeterminate".into())),
                ("driver".into(), Value::String("codex".into())),
                ("incarnation_id".into(), Value::String("current".into())),
                ("reason".into(), Value::String("stale-command".into())),
            ]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: Some("activity-command-stale".into()),
        })
        .unwrap();
    {
        let mut connection = store.connection.lock().unwrap();
        let transaction = connection.transaction().unwrap();
        append_claim_tx(
            &transaction,
            "node",
            "step-run/activity/work",
            "work.renewed",
            Some(subject),
            &json!({"fields": {
                "status": "claimed",
                "attempt": 1,
                "readiness_epoch": 1,
                "claimant": subject,
                "claim_incarnation": "current",
                "claim_expires_at_unix_ms": now_ms().saturating_add(60_000),
                "worker_reported": false,
                "summary": null,
                "reason": null
            }}),
            &[],
            None,
        )
        .unwrap();
        transaction.commit().unwrap();
    }
    let renewed = store.current_harness(subject).unwrap().unwrap();
    assert_eq!(renewed.state, "indeterminate");
    assert_eq!(renewed.reason.as_deref(), Some("stale-command"));

    {
        let mut connection = store.connection.lock().unwrap();
        let transaction = connection.transaction().unwrap();
        append_claim_tx(
            &transaction,
            "node",
            "step-run/activity/work",
            "work.progress",
            Some(subject),
            &json!({"fields": {
                "status": "working",
                "attempt": 1,
                "readiness_epoch": 1,
                "claimant": subject,
                "claim_incarnation": "current",
                "claim_expires_at_unix_ms": now_ms().saturating_add(60_000),
                "worker_reported": false,
                "summary": "command finished",
                "reason": null
            }}),
            &[],
            None,
        )
        .unwrap();
        transaction.commit().unwrap();
    }
    assert_eq!(
        store.current_harness(subject).unwrap().unwrap().state,
        "working"
    );
}

#[test]
fn usage_aggregation_separates_occupancy_and_deduplicates_cumulative_incarnations() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(&directory.path().join("usage.sqlite"), "usage-node").unwrap();
    let subject = "agent/usage-owner";
    let append = |semantics: &str,
                  incarnation: &str,
                  total: Option<u64>,
                  context_used: Option<u64>,
                  key: &str| {
        let mut fields = BTreeMap::from([
            ("semantics".into(), Value::String(semantics.into())),
            ("driver".into(), Value::String("codex".into())),
            ("incarnation_id".into(), Value::String(incarnation.into())),
        ]);
        if let Some(total) = total {
            fields.insert("total_tokens".into(), Value::from(total));
        }
        if let Some(used) = context_used {
            fields.insert("context_used_tokens".into(), Value::from(used));
        }
        store
            .append_claim(&ClaimInput {
                subject: subject.into(),
                kind: "harness.usage".into(),
                actor: Some(subject.into()),
                fields,
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: Some(key.into()),
            })
            .unwrap();
    };
    append("session_cumulative", "one", Some(100), None, "one-100");
    append("response", "one", Some(10), None, "one-response");
    append("session_cumulative", "one", Some(150), None, "one-150");
    append("response", "two", Some(20), None, "two-response-1");
    append("response", "two", Some(30), None, "two-response-2");
    append(
        "context_occupancy",
        "two",
        None,
        Some(999),
        "occupancy-arrived-last",
    );

    // The default path must use a SQLite-representable bound rather than u64::MAX.
    let usage = store
        .usage_summary_at(subject, None, None)
        .unwrap()
        .unwrap();
    let batch = store
        .usage_summaries_at(
            &[subject.into(), "agent/missing".into(), subject.into()],
            None,
        )
        .unwrap();
    assert_eq!(batch.get(subject), Some(&usage));
    assert!(!batch.contains_key("agent/missing"));
    assert_eq!(usage.total_tokens, 200);
    assert_eq!(usage.incarnation_count, 2);
    assert_eq!(usage.context.unwrap().used_tokens, Some(999));
    assert_eq!(
        store
            .usage_summary_at(subject, Some("one"), None)
            .unwrap()
            .unwrap()
            .total_tokens,
        150
    );
}

#[test]
fn batched_usage_preserves_canonical_ties_and_snapshot_bound() {
    let store = Store::open_memory("usage-node").unwrap();
    let subjects = ["agent/usage-a", "agent/usage-b"];
    let mut middle = 0;
    for i in 0..602 {
        let subject = subjects[i % 2];
        store
            .append_claim(&ClaimInput {
                subject: subject.into(),
                kind: "harness.usage".into(),
                actor: Some(subject.into()),
                fields: BTreeMap::from([
                    ("driver".into(), json!("codex")),
                    ("incarnation_id".into(), json!("same-incarnation")),
                    (
                        "semantics".into(),
                        json!(if i % 4 < 2 {
                            "session_cumulative"
                        } else {
                            "context_occupancy"
                        }),
                    ),
                    ("total_tokens".into(), json!(100)),
                    ("cost".into(), json!(i as f64)),
                    ("currency".into(), json!("USD")),
                    ("context_used_tokens".into(), json!(i)),
                    ("model".into(), json!("fixture-model-".repeat(320))),
                ]),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
        if i == 299 {
            middle = store.index().unwrap();
        }
    }
    // Reverse timestamps and batch order: tied canonical winners differ from index row order.
    {
        let mut connection = store.connection.write();
        let tx = connection.transaction().unwrap();
        tx.execute(
            /* sqlite-large-value: allow-for-now lv-h7976f7c9b118 owner=fractal_store::store::tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET accepted_at_unix_ms=CAST(1000000-(store_index/8) AS TEXT)
                        WHERE kind='harness.usage'",
            [],
        )
        .unwrap();
        tx.execute(
            /* sqlite-large-value: allow-for-now lv-h18dd59167769 owner=fractal_store::store::tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE batches SET replica_sequence=10000-(SELECT MIN(store_index)
                        FROM claims WHERE claims.batch_id=batches.id)",
            [],
        )
        .unwrap();
        tx.commit().unwrap();
    }
    let selected = [
        subjects[1].into(),
        subjects[0].into(),
        subjects[1].into(),
        "agent/missing".into(),
    ];
    for at in [None, Some(middle)] {
        let batch = store.usage_summaries_at(&selected, at).unwrap();
        assert_eq!(batch.len(), 2);
        assert_eq!(batch[subjects[0]].cost, Some(0.0));
        assert_eq!(batch[subjects[1]].cost, Some(1.0));
        assert_eq!(
            batch[subjects[0]].context.as_ref().unwrap().used_tokens,
            Some(2)
        );
        for subject in subjects {
            assert_eq!(
                batch.get(subject),
                store.usage_summary_at(subject, None, at).unwrap().as_ref()
            );
        }
    }
}

fn claim_id_order_fixture(connection: &Connection) {
    connection
        .execute_batch(
            /* sqlite-large-value: allow-for-now lv-hb3647d01a418 owner=fractal_store::store::tests TODO=chunk-or-reference-variable-columns */ "CREATE TABLE batches(id TEXT PRIMARY KEY, origin TEXT, replica_sequence INTEGER);
             CREATE TABLE claims(id TEXT PRIMARY KEY, subject TEXT, kind TEXT, batch_id TEXT,
                                 store_index INTEGER, accepted_at_unix_ms TEXT);
             CREATE INDEX claims_subject_accepted_index ON claims
                 (subject, length(accepted_at_unix_ms), accepted_at_unix_ms);
             CREATE TABLE replica_records(claim_id TEXT, position INTEGER); CREATE TABLE claim_positions(claim_id TEXT PRIMARY KEY, position INTEGER NOT NULL) WITHOUT ROWID;
             INSERT INTO batches VALUES ('x','writer-a',4),('y','writer-a',3),('z','writer-b',1);
             INSERT INTO claims VALUES
               ('early','agent/target','runtime.observed','x',1,'9'),
               ('legacy','agent/target','harness.usage','x',2,'10'),
               ('record-a','agent/target','work.renewed','x',3,'10'),
               ('other-subject','agent/other','daemon.diagnostic','x',4,'10'),
               ('record-b','agent/target','intent.desired','x',5,'10'),
               ('cross-batch','agent/target','harness.observed','y',6,'10'),
               ('other-writer','agent/target','runtime.observed','z',7,'10'),
               ('u128-time','agent/target','harness.usage','z',8,
                '340282366920938463463374607431768211455');
             INSERT INTO replica_records VALUES ('record-a',7),('record-a',3),('record-b',0);",
        )
        .unwrap();
}

#[test]
fn claim_ids_stream_preserves_full_order_historical_bounds_and_transaction_ownership() {
    let mut connection = Connection::open_in_memory().unwrap();
    claim_id_order_fixture(&connection);
    assert_eq!(
        full_canonical_claim_id_oracle(&connection, "agent/target", None),
        [
            "early",
            "cross-batch",
            "record-b",
            "legacy",
            "record-a",
            "other-writer",
            "u128-time"
        ]
    );
    for at in [None, Some(2), Some(5), Some(6)] {
        let expected = full_canonical_claim_id_oracle(&connection, "agent/target", at);
        let actual = claim_ids_at(&connection, "agent/target", at).unwrap();
        assert_eq!(actual, expected);
        assert_eq!(
            actual.last(),
            expected.last(),
            "the planning revision stays exact"
        );
        assert!(connection.is_autocommit());
    }
    assert!(
        claim_ids_at(&connection, "agent/missing", None)
            .unwrap()
            .is_empty()
    );
    assert!(connection.is_autocommit());
    let tx = connection.transaction().unwrap();
    tx.execute(
        /* sqlite-large-value: allow-for-now lv-hbcf472355ed4 owner=fractal_store::store::tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_records SET position=0 WHERE claim_id='record-a'",
        [],
    )
    .unwrap();
    assert_eq!(
        claim_ids_at(&tx, "agent/target", None).unwrap(),
        full_canonical_claim_id_oracle(&tx, "agent/target", None)
    );
    assert!(
        !tx.is_autocommit(),
        "the caller retains its in-flight writer transaction"
    );
    tx.rollback().unwrap();
    assert!(connection.is_autocommit());
    connection
        .execute_batch("PRAGMA query_only=ON; BEGIN")
        .unwrap();
    assert_eq!(
        claim_ids_at(&connection, "agent/target", None).unwrap(),
        full_canonical_claim_id_oracle(&connection, "agent/target", None)
    );
    assert!(
        !connection.is_autocommit(),
        "the caller retains its read transaction"
    );
    connection
        .execute_batch(/* sqlite-large-value: allow-for-now lv-hb25e08689142 owner=fractal_store::store::tests TODO=trace-statement-and-value-representation */ "ROLLBACK; PRAGMA query_only=OFF")
        .unwrap();
    connection
        .execute_batch(/* sqlite-large-value: allow-for-now lv-h195617ffd7cc owner=fractal_store::store::tests TODO=chunk-or-reference-exact-wire-values */ "DROP TABLE replica_records")
        .unwrap();
    assert!(claim_ids_at(&connection, "agent/target", None).is_err());
    assert!(
        connection.is_autocommit(),
        "a failed owned read must roll back"
    );
    let tx = connection.transaction().unwrap();
    assert!(claim_ids_at(&tx, "agent/target", None).is_err());
    assert!(
        !tx.is_autocommit(),
        "a failed nested read must not roll back its caller"
    );
    tx.rollback().unwrap();
    connection
        .execute_batch("PRAGMA query_only=ON; BEGIN")
        .unwrap();
    assert!(claim_ids_at(&connection, "agent/target", None).is_err());
    assert!(
        !connection.is_autocommit(),
        "a failed read must not close its caller"
    );
    connection
        .execute_batch(/* sqlite-large-value: allow-for-now lv-h07a636fe8088 owner=fractal_store::store::tests TODO=trace-statement-and-value-representation */ "ROLLBACK; PRAGMA query_only=OFF")
        .unwrap();
}

#[test]
fn claim_ids_stream_pins_tie_lookups_across_a_wal_writer_and_unwind() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("claims.sqlite3");
    let connection = Connection::open(&path).unwrap();
    connection.execute_batch("PRAGMA journal_mode=WAL").unwrap();
    claim_id_order_fixture(&connection);
    let old = full_canonical_claim_id_oracle(&connection, "agent/target", None);
    let actual = claim_ids_at_after_first_row(&connection, "agent/target", None, || {
        let writer = Connection::open(&path).unwrap();
        writer
            .execute_batch(
                /* sqlite-large-value: allow-for-now lv-h2be3c50dbcff owner=fractal_store::store::tests TODO=chunk-or-reference-exact-wire-values */ "BEGIN IMMEDIATE;
                 UPDATE replica_records SET position=0 WHERE claim_id='record-a';
                 UPDATE replica_records SET position=99 WHERE claim_id='record-b';
                 COMMIT;",
            )
            .unwrap();
    })
    .unwrap();
    assert_eq!(
        actual, old,
        "the first stream row and later tie keys share one cut"
    );
    assert!(connection.is_autocommit());
    let new = full_canonical_claim_id_oracle(&connection, "agent/target", None);
    assert_ne!(
        new, old,
        "the concurrent commit really changed the canonical order"
    );
    assert_eq!(
        claim_ids_at(&connection, "agent/target", None).unwrap(),
        new
    );
    let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        claim_ids_at_after_first_row(&connection, "agent/target", None, || {
            panic!("injected after stream pin")
        })
    }));
    assert!(unwind.is_err());
    assert!(
        connection.is_autocommit(),
        "unwind must release the locally owned read"
    );
    assert_eq!(
        claim_ids_at(&connection, "agent/target", None).unwrap(),
        new
    );
}
#[test]
fn claim_ids_stream_keeps_real_store_status_claims_at_current_and_historical_cuts() {
    let store = Store::open_memory("claim-id-node").unwrap();
    let subject = "resource/claim-id-order";
    let mut historical = 0;
    for i in 0..4 {
        store
            .append_claim(&ClaimInput {
                subject: subject.into(),
                kind: "resource.observed".into(),
                actor: Some("person/avery".into()),
                fields: BTreeMap::from([
                    ("kind".into(), json!("human.review")),
                    ("reason".into(), json!(format!("change {i}"))),
                ]),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
        if i == 1 {
            historical = store.index().unwrap();
        }
    }
    for cut in [historical, store.index().unwrap()] {
        let status = store.status_for_subject_names_at(
            BTreeSet::from([subject.into()]),
            cut,
            true,
            ClaimsReduction::Full,
        )
        .unwrap();
        let actual = &status
            .subjects
            .iter()
            .find(|row| row.subject == subject)
            .unwrap()
            .claims;
        let expected = full_canonical_claim_id_oracle(&store.readers.get(), subject, Some(cut));
        assert_eq!(actual, &expected);
        assert_eq!(actual.last(), expected.last());
    }
}

/// `count` claims for one subject over a handful of writers, batches and replica
/// positions, with accepted times drawn from a set far smaller than the claim count so
/// canonical tie groups appear everywhere, including across the session-dating window.
fn seeded_claim_summary_fixture(connection: &Connection, seed: u64, count: usize) {
    let mut create = String::from(
        /* sqlite-large-value: allow-for-now lv-h78aa93c4fbad owner=fractal_store::store::tests TODO=chunk-or-reference-variable-columns */ "CREATE TABLE batches(id TEXT PRIMARY KEY, origin TEXT, replica_sequence INTEGER);
             CREATE TABLE claims(id TEXT PRIMARY KEY, subject TEXT, kind TEXT, batch_id TEXT,
                                 store_index INTEGER, accepted_at_unix_ms TEXT);
             CREATE INDEX claims_subject_accepted_index ON claims
                 (subject, length(accepted_at_unix_ms), accepted_at_unix_ms);
             CREATE TABLE replica_records(claim_id TEXT, position INTEGER); CREATE TABLE claim_positions(claim_id TEXT PRIMARY KEY, position INTEGER NOT NULL) WITHOUT ROWID;",
    );
    for writer in 0..3 {
        create.push_str(&format!(
            /* sqlite-large-value: allow-for-now lv-h433788055431 owner=fractal_store::store::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO batches VALUES ('batch-{writer}','writer-{writer}',{writer});"
        ));
    }
    let mut state = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    let mut next = || {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        state >> 33
    };
    let times = [
        "9", "10", "11", "99", "100", "1000",
        "340282366920938463463374607431768211455",
    ];
    for index in 0..count {
        let time = times[(next() % times.len() as u64) as usize];
        let batch = format!("batch-{}", next() % 3);
        create.push_str(&format!(
            /* sqlite-large-value: allow-for-now lv-h523a4524bd5b owner=fractal_store::store::tests TODO=prepare-body-and-predecessor-values */ "INSERT INTO claims VALUES ('claim-{index:06}','agent/target','runtime.observed','{batch}',{},'{time}');",
            index + 1
        ));
        if next() % 2 == 0 {
            create.push_str(&format!(
                /* sqlite-large-value: allow-for-now lv-h94cfc5e51e8c owner=fractal_store::store::tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO replica_records VALUES ('claim-{index:06}',{});",
                next() % 7
            ));
        }
    }
    // Claims of other subjects must never leak into the subject's answers.
    create.push_str(
        /* sqlite-large-value: allow-for-now lv-h5b19d7ca2c85 owner=fractal_store::store::tests TODO=prepare-body-and-predecessor-values */ "INSERT INTO claims VALUES ('elsewhere','agent/other','runtime.observed','batch-0',0,'10');",
    );
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-h10f5d4269a59 owner=fractal_store::store::tests TODO=trace-statement-and-value-representation */ &create).unwrap();
}

#[test]
fn claims_summary_equals_the_full_canonical_vector_at_every_cut() {
    for seed in 0..24 {
        let counts = [0, 1, 2, 7, 40];
        for count in counts {
            let connection = Connection::open_in_memory().unwrap();
            seeded_claim_summary_fixture(&connection, seed, count);
            for cut in [None, Some(1), Some(count as u64 / 2), Some(count as u64)] {
                let full = full_canonical_claim_id_oracle(&connection, "agent/target", cut);
                assert_eq!(
                    claim_ids_at(&connection, "agent/target", cut).unwrap(),
                    full,
                    "seed {seed}, count {count}"
                );
                let summary =
                    claims_summary_at(&connection, "agent/target", cut).unwrap();
                assert_eq!(
                    summary.latest,
                    full.last().cloned(),
                    "latest, seed {seed}, count {count}"
                );
                assert_eq!(
                    summary.first,
                    full.first().cloned(),
                    "first, seed {seed}, count {count}"
                );
                assert_eq!(
                    summary.count,
                    full.len(),
                    "count, seed {seed}, count {count}"
                );
                for window in 1..=(count + 2) {
                    assert_eq!(
                        window_boundary_at(
                            &connection,
                            "agent/target",
                            cut.unwrap_or(i64::MAX as u64),
                            window,
                        )
                        .unwrap(),
                        (full.len() >= window)
                            .then(|| full[window - 1].clone()),
                        "window {window}, seed {seed}, count {count}"
                    );
                }
            }
        }
    }
}

#[test]
fn claims_summary_resolves_the_real_session_window_inside_a_crossing_tie() {
    let connection = Connection::open_in_memory().unwrap();
    // The window edge lands inside a wide tie group: only canonical ordering decides the
    // boundary, exactly what the full vector resolves claim by claim.
    let mut fixture = String::from(
        /* sqlite-large-value: allow-for-now lv-ha71a57e6ad3f owner=fractal_store::store::tests TODO=chunk-or-reference-variable-columns */ "CREATE TABLE batches(id TEXT PRIMARY KEY, origin TEXT, replica_sequence INTEGER);
             CREATE TABLE claims(id TEXT PRIMARY KEY, subject TEXT, kind TEXT, batch_id TEXT,
                                 store_index INTEGER, accepted_at_unix_ms TEXT);
             CREATE INDEX claims_subject_accepted_index ON claims
                 (subject, length(accepted_at_unix_ms), accepted_at_unix_ms);
             CREATE TABLE replica_records(claim_id TEXT, position INTEGER); CREATE TABLE claim_positions(claim_id TEXT PRIMARY KEY, position INTEGER NOT NULL) WITHOUT ROWID;
             INSERT INTO batches VALUES ('a','writer-a',1),('b','writer-b',2);",
    );
    for index in 0..(SESSION_CLAIMS + 4) {
        // One claim before the window, a tie group spanning it, one claim after.
        let (time, batch) = if index < SESSION_CLAIMS - 3 {
            ("10", "a")
        } else if index < SESSION_CLAIMS + 2 {
            ("12", if index % 2 == 0 { "a" } else { "b" })
        } else {
            ("340282366920938463463374607431768211455", "a")
        };
        fixture.push_str(&format!(
            /* sqlite-large-value: allow-for-now lv-ha5f57e1148f4 owner=fractal_store::store::tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO claims VALUES ('claim-{index:06}','agent/target','runtime.observed','{batch}',{},'{time}');
                 INSERT INTO replica_records VALUES ('claim-{index:06}',{});",
            index + 1,
            (index % 5) as u64
        ));
    }
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-h4ed31e2a3f89 owner=fractal_store::store::tests TODO=trace-statement-and-value-representation */ &fixture).unwrap();
    let full = full_canonical_claim_id_oracle(&connection, "agent/target", None);
    assert_eq!(full.len(), SESSION_CLAIMS + 4);
    let summary = claims_summary_at(&connection, "agent/target", None).unwrap();
    assert_eq!(summary.count, full.len());
    assert_eq!(summary.latest, full.last().cloned());
    assert_eq!(
        summary.session_boundary,
        Some(full[SESSION_CLAIMS - 1].clone()),
        "the boundary is the window's own canonical id, not a physical neighbor"
    );
}

#[test]
fn summary_status_matches_full_status_for_every_internal_decision() {
    let store = Store::open_memory("node").unwrap();
    let subject = "agent/node.summary";
    let mut historical = 0;
    for round in 0..6 {
        store
            .append_claim(&ClaimInput {
                subject: subject.into(),
                kind: "runtime.observed".into(),
                actor: Some("person/avery".into()),
                fields: BTreeMap::from([(
                    "status".into(),
                    json!(if round % 2 == 0 { "running" } else { "waiting" }),
                )]),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
        if round == 2 {
            historical = store.index().unwrap();
        }
    }
    for cut in [historical, store.index().unwrap()] {
        let full = store
            .status_for_subject_names_at(
                BTreeSet::from([subject.into()]),
                cut,
                true,
                ClaimsReduction::Full,
            )
            .unwrap()
            .subjects
            .remove(0);
        let summary = store
            .status_for_subject_names_at(
                BTreeSet::from([subject.into()]),
                cut,
                true,
                ClaimsReduction::Summary,
            )
            .unwrap()
            .subjects
            .remove(0);
        assert!(
            summary.claims.is_empty(),
            "a summary reduction carries no vector"
        );
        // The agent card's fallback revision.
        assert_eq!(
            summary
                .desired_revision
                .clone()
                .or_else(|| summary.claims_summary.latest.clone()),
            full.desired_revision
                .clone()
                .or_else(|| full.claims.last().cloned())
        );
        // The session list's dating boundary, including its gate on the claim count.
        assert_eq!(
            summary.claims_summary.session_boundary,
            (full.claims.len() > SESSION_CLAIMS)
                .then(|| full.claims[SESSION_CLAIMS - 1].clone())
        );
        assert_eq!(summary.claims_summary.count, full.claims.len());
        assert_eq!(
            summary.claims_summary.latest,
            full.claims.last().cloned()
        );
    }
}


/// Run against a reflink-copied live store only:
/// `CLAIM_SUMMARY_BENCH=/path/to/copy.sqlite3 cargo test -p fractal-daemon --lib claim_summary -- --ignored --nocapture`.
/// Alternates full and summary reductions of the same subject at the same cut so both
/// answers measure one store, one index and one page cache.
#[test]
#[ignore = "points at a copied live store through CLAIM_SUMMARY_BENCH"]
fn claim_summary_bench_subject_status_at_on_a_copied_store() {
    let path = std::env::var("CLAIM_SUMMARY_BENCH").unwrap_or_default();
    if path.is_empty() {
        return;
    }
    let store = Store::open(std::path::Path::new(&path), "bench-node").unwrap();
    let index = store.index().unwrap();
    let subjects = std::env::var("CLAIM_SUMMARY_BENCH_SUBJECTS")
        .unwrap_or_else(|_| {
            "agent/dotfiles/manager,agent/dotfiles/deployer,agent/dotfiles/smalltalk-graph,daemon/mbp2025".into()
        })
        .split(',')
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let iterations = 30;
    let percentile = |samples: &mut Vec<f64>, fraction: f64| {
        samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
        samples[((samples.len() as f64 - 1.0) * fraction) as usize]
    };
    println!("subject\tclaims\tfull p50 ms\tfull p95 ms\tsummary p50 ms\tsummary p95 ms");
    for subject in subjects {
        let connection = store.readers.get();
        let claims = connection
            .query_row(
                "SELECT COUNT(*) FROM claims WHERE subject=?1",
                [&subject],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();
        let mut full = Vec::new();
        let mut summary = Vec::new();
        for round in 0..iterations {
            let start = std::time::Instant::now();
            let (status, _) = subject_status_at(
                &connection,
                &subject,
                Some(index),
                None,
                ClaimsReduction::Full,
            )
            .unwrap()
            .expect("the live subject reduces");
            let full_ms = start.elapsed().as_secs_f64() * 1e3;
            let start = std::time::Instant::now();
            let (summarized, _) = subject_status_at(
                &connection,
                &subject,
                Some(index),
                None,
                ClaimsReduction::Summary,
            )
            .unwrap()
            .expect("the live subject reduces");
            let summary_ms = start.elapsed().as_secs_f64() * 1e3;
            if round > 0 {
                full.push(full_ms);
                summary.push(summary_ms);
            }
            // Both answers must agree where internal readers look.
            assert_eq!(summarized.claims_summary.first, status.claims.first().cloned());
            assert_eq!(summarized.claims_summary.latest, status.claims.last().cloned());
            assert_eq!(summarized.claims_summary.count, status.claims.len());
            assert_eq!(
                summarized.claims_summary.session_boundary,
                (status.claims.len() > SESSION_CLAIMS)
                    .then(|| status.claims[SESSION_CLAIMS - 1].clone())
            );
        }
        println!(
            "{}\t{}\t{:.1}\t{:.1}\t{:.1}\t{:.1}",
            subject,
            claims,
            percentile(&mut full, 0.5),
            percentile(&mut full, 0.95),
            percentile(&mut summary, 0.5),
            percentile(&mut summary, 0.95)
        );
    }
}

#[test]
fn admission_failure_fences_ready_work_and_exit_until_a_new_incarnation() {
    let store = Store::open_memory("node").unwrap();
    let subject = "agent/node.fixture";
    let append = |kind: &str, fields: BTreeMap<String, Value>| {
        store
            .append_claim(&ClaimInput {
                subject: subject.into(),
                kind: kind.into(),
                actor: Some(subject.into()),
                fields,
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap()
    };
    append(
        "runtime.observed",
        BTreeMap::from([
            ("status".into(), json!("running")),
            ("runtime_id".into(), json!("fixture")),
            ("incarnation_id".into(), json!("first")),
        ]),
    );
    append(
        "harness.observed",
        BTreeMap::from([
            ("state".into(), json!("ready")),
            ("incarnation_id".into(), json!("first")),
        ]),
    );
    assert!(store.current_harness(subject).unwrap().unwrap().is_ready());
    let refusal = append(
        "harness.diagnostic",
        BTreeMap::from([
            ("code".into(), json!("harness-admission-failed")),
            ("status".into(), json!("degraded")),
            (
                "reason".into(),
                json!("opencode 99.42.7 admission failed at admissionLifecycle"),
            ),
            ("incarnation_id".into(), json!("first")),
        ]),
    );
    append(
        "harness.observed",
        BTreeMap::from([
            ("state".into(), json!("working")),
            ("incarnation_id".into(), json!("first")),
        ]),
    );
    let refused = store.current_harness(subject).unwrap().unwrap();
    assert!(!refused.is_ready());
    assert_eq!(refused.state, "indeterminate");
    assert_eq!(refused.since_unix_ms, refusal.accepted_at_unix_ms);
    assert!(refused.reason.unwrap().contains("admissionLifecycle"));
    append(
        "runtime.observed",
        BTreeMap::from([
            ("status".into(), json!("exited")),
            ("incarnation_id".into(), json!("first")),
        ]),
    );
    let exited = store.current_harness(subject).unwrap().unwrap();
    assert!(exited.reason.unwrap().contains("admissionLifecycle"));
    assert_eq!(exited.since_unix_ms, refusal.accepted_at_unix_ms);
    append(
        "runtime.observed",
        BTreeMap::from([
            ("status".into(), json!("running")),
            ("runtime_id".into(), json!("fixture")),
            ("incarnation_id".into(), json!("second")),
        ]),
    );
    append(
        "harness.observed",
        BTreeMap::from([
            ("state".into(), json!("ready")),
            ("incarnation_id".into(), json!("second")),
        ]),
    );
    assert!(store.current_harness(subject).unwrap().unwrap().is_ready());
}

#[test]
fn expired_claude_login_fences_later_ready_and_work_claims_in_same_incarnation() {
    let store = Store::open_memory("node").unwrap();
    let subject = "agent/node.claude";
    let append = |kind: &str, actor: Option<&str>, fields: BTreeMap<String, Value>| {
        store
            .append_claim(&ClaimInput {
                subject: subject.into(),
                kind: kind.into(),
                actor: actor.map(str::to_owned),
                fields,
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
    };
    append(
        "runtime.observed",
        None,
        BTreeMap::from([
            ("status".into(), Value::String("running".into())),
            ("runtime_id".into(), Value::String("node.claude".into())),
            ("incarnation_id".into(), Value::String("first".into())),
        ]),
    );
    append(
        "harness.observed",
        Some(subject),
        BTreeMap::from([
            ("state".into(), Value::String("ready".into())),
            ("incarnation_id".into(), Value::String("first".into())),
        ]),
    );
    assert!(store.current_harness(subject).unwrap().unwrap().is_ready());
    append(
        "harness.diagnostic",
        Some(subject),
        BTreeMap::from([
            ("status".into(), Value::String("unauthenticated".into())),
            ("code".into(), Value::String("provider-auth-expired".into())),
            ("incarnation_id".into(), Value::String("first".into())),
        ]),
    );
    append(
        "harness.observed",
        Some(subject),
        BTreeMap::from([
            ("state".into(), Value::String("ready".into())),
            ("incarnation_id".into(), Value::String("first".into())),
        ]),
    );
    let blocked = store.current_harness(subject).unwrap().unwrap();
    assert_eq!(blocked.state, "needs-login");
    assert_eq!(blocked.reason.as_deref(), Some("providerAuth"));
    assert!(!blocked.is_ready());
    append(
        "runtime.observed",
        None,
        BTreeMap::from([
            ("status".into(), Value::String("running".into())),
            ("runtime_id".into(), Value::String("node.claude".into())),
            ("incarnation_id".into(), Value::String("second".into())),
        ]),
    );
    append(
        "harness.observed",
        Some(subject),
        BTreeMap::from([
            ("state".into(), Value::String("ready".into())),
            ("incarnation_id".into(), Value::String("second".into())),
        ]),
    );
    assert!(store.current_harness(subject).unwrap().unwrap().is_ready());
}

#[test]
fn a_claude_trust_prompt_fences_its_incarnation_as_not_ready_with_the_reason() {
    let store = Store::open_memory("node").unwrap();
    let subject = "agent/node.claude";
    let append = |kind: &str, actor: Option<&str>, fields: BTreeMap<String, Value>| {
        store
            .append_claim(&ClaimInput {
                subject: subject.into(),
                kind: kind.into(),
                actor: actor.map(str::to_owned),
                fields,
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
    };
    let running = |incarnation: &str| {
        BTreeMap::from([
            ("status".into(), Value::String("running".into())),
            ("runtime_id".into(), Value::String("node.claude".into())),
            ("incarnation_id".into(), Value::String(incarnation.into())),
        ])
    };
    let ready = |incarnation: &str| {
        BTreeMap::from([
            ("state".into(), Value::String("ready".into())),
            ("incarnation_id".into(), Value::String(incarnation.into())),
        ])
    };
    append("runtime.observed", None, running("first"));
    append(
        "harness.diagnostic",
        Some(subject),
        BTreeMap::from([
            ("status".into(), Value::String("blocked".into())),
            ("code".into(), Value::String("provider-trust-prompt".into())),
            ("incarnation_id".into(), Value::String("first".into())),
        ]),
    );
    // A channel that initializes behind the prompt must not make the seat look ready.
    append("harness.observed", Some(subject), ready("first"));
    let blocked = store.current_harness(subject).unwrap().unwrap();
    assert_eq!(blocked.state, "blocked");
    assert_eq!(blocked.reason.as_deref(), Some("providerTrustPrompt"));
    assert!(!blocked.is_ready());

    append("runtime.observed", None, running("second"));
    append("harness.observed", Some(subject), ready("second"));
    assert!(store.current_harness(subject).unwrap().unwrap().is_ready());
}
