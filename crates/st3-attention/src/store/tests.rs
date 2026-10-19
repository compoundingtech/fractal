#![allow(unused_imports)]
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::tests_shared_seats::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::tests_shared_seats::{TAKEOVER_SOURCE, TREE_SHAPE_RUNS, TreeShapes, expire_work_lease_by_claim, failed_takeover_run, full_canonical_claim_id_oracle, graph_digest_of, insert_raw_claims, late_claim_matches_a_replay, local_observation_for_test};
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::tests_shared_seats::{local_timeline_observation_for_test, observe_runtime, projection_replayed, publish_mission, publish_takeover, query_plan, replicate_fully, replicated_run_tree, roster_cache_store, simple};
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::tests_shared_seats::{exchange_of, harness_state, takeover_step, timeline_observation};
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::tests_shared::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::tests_shared::{TEST_FLEET, exchange_from, receive_and_project};
use crate::SeatStatusAttentionStore;
use crate::SeatsAttentionStore;
use super::*;
use crate::graph::parse_test_intent as parse_intent;
use proptest::prelude::*;
use st3_store::store::shared_seats_shared_attention_tests::{DEFINITIONS_PARSED, STATUS_AFTER_DESIRED_READ, STEPS_ENRICHED, SUBJECT_REDUCTIONS};
use st3_store::store::tests_shared_attention_tests::{listed, replicated_step_pair, rewrite_envelope, seeded_claim_summary_fixture, start_agentless_run, synced_takeover_pair, worker_extend, worker_work};


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

/// A start rebuilds the tables derived from claims once, then keeps them: every write keeps
/// them current, and rebuilding them read every claim before the API could answer.
#[test]
fn a_start_keeps_the_derived_tables_it_built_before() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("claims.sqlite3");
    drop(Store::open(&path, "node").unwrap());
    {
        let connection = Connection::open(&path).unwrap();
        projection_digest::register(&connection).unwrap();
        connection
            .execute_batch("PRAGMA foreign_keys = OFF")
            .unwrap();
        connection
            .execute(
                /* sqlite-large-value: allow-for-now lv-h9e36cfc904a4 owner=st3_attention::store::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO operations(id, request_digest, canonical_claim_id, state)
                     VALUES ('op/kept', 'digest', 'claim/kept', 'active')",
                [],
            )
            .unwrap();
    }
    let reopened = Store::open(&path, "node").unwrap();
    let kept: bool = reopened
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM operations WHERE id='op/kept')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        kept,
        "a start with current derived tables must not rebuild them"
    );
    drop(reopened);
    {
        let connection = Connection::open(&path).unwrap();
        connection
            .execute(/* sqlite-large-value: allow-for-now lv-h501e0bb530ed owner=st3_attention::store::tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM meta WHERE key='derived_tables_version'", [])
            .unwrap();
    }
    let rebuilt = Store::open(&path, "node").unwrap();
    let kept: bool = rebuilt
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM operations WHERE id='op/kept')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!kept, "a store no build with this rule opened is rebuilt");
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
                /* sqlite-large-value: allow-for-now lv-hf105f9698936 owner=st3_attention::store::tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET body='{\"from\":\"another build\"}' WHERE id=?1",
                [&bad_claim],
            )
            .unwrap();
        connection.execute(/* sqlite-large-value: allow-for-now lv-h9f1b23ae9e8f owner=st3_attention::store::tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM desired", []).unwrap();
        connection
            .execute(
                /* sqlite-large-value: allow-for-now lv-ha01108f164a1 owner=st3_attention::store::tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE projection_health SET status='stale' WHERE aggregate='graph'",
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
            .execute(/* sqlite-large-value: allow-for-now lv-h7a954c50ab0c owner=st3_attention::store::tests TODO=prepare-body-and-predecessor-values */ "DELETE FROM claims WHERE id=?1", [&bad_claim])
            .unwrap();
        connection
            .execute(
                /* sqlite-large-value: allow-for-now lv-h35ccfb76de5b owner=st3_attention::store::tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE projection_health SET status='stale' WHERE aggregate='graph'",
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
fn agent_card_status_cache_follows_owning_generation_changes() {
    let store = Store::open_memory("node").unwrap();
    publish_mission(&store, r#"version 2
mission "card-owner" state="ready" {
  goal "Fence card status to its owner."; step "work" { agentless }
}"#, "card-owner-mission");
    let run = store.create_mission_run(&MissionRunRequest {
        mission: "card-owner".into(), revision: None, workspace: "/tmp".into(),
        requester: Some("person/test".into()), mode: Some("run".into()),
        inputs: BTreeMap::new(), idempotency_key: "card-owner-run".into(),
    }).unwrap();
    let mut intent = parse_intent("version 2\nagent \"worker\" { command \"true\" }\n", "node").unwrap();
    let desired = intent.subjects.values_mut().next().unwrap();
    desired.owner_run = Some(run.subject.clone());
    desired.owner_generation = Some(run.generation.clone());
    let preview = store.mission(&intent, IntentInput { kdl: String::new(), source_name: None }).unwrap();
    store.apply(&intent, &preview.subject_tokens, "card-owned-runtime").unwrap();
    let first = store.index().unwrap();
    let before = store.agent_card_status_at(None, first, true).unwrap();
    assert_eq!(before.subjects[0].projection.layer, "current");
    store.append_claim(&ClaimInput {
        subject: run.generation, kind: "run-generation.superseded".into(),
        actor: Some("person/test".into()), fields: BTreeMap::from([
            ("status".into(), json!("superseded")),
            ("successor".into(), json!("run-generation/replacement")),
            ("reason".into(), json!("test revision")),
        ]), evidence: Vec::new(), expected_subject: None, idempotency_key: None,
    }).unwrap();
    let index = store.index().unwrap();
    let after = store.agent_card_status_at(None, index, true).unwrap();
    assert_eq!(after.subjects[0].projection.layer, "history");
    assert!(store.agent_card_status_at(None, index, false).unwrap().subjects.is_empty());
    assert_eq!(serde_json::to_value(store.agent_card_status_at(None, first, true).unwrap()).unwrap(),
        serde_json::to_value(before).unwrap());
    store.forget_current_views();
    assert_eq!(serde_json::to_value(store.agent_card_status_at(None, index, true).unwrap()).unwrap(),
        serde_json::to_value(after).unwrap());
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
        .prepare(/* sqlite-large-value: allow-for-now lv-hdb70d253dc7a owner=st3_attention::store::tests TODO=trace-statement-and-value-representation */ &format!(
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
fn a_declared_planning_session_creates_its_session_and_planner_atomically() {
    let store = Store::open_memory("node").unwrap();
    let request = store
        .put_document(
            "doc/planning/release/request",
            b"Mission the release.",
            &None,
            "planning-request",
        )
        .unwrap();
    let source = format!(
        r#"version 2
planning-session "planning/release/one" {{
  mission "release"
  request "{}@{}"
  workspace "/work/release"
  requester "person/operator"
  planner "codex" {{ model "gpt-5.6-sol"; effort "medium" }}
}}
"#,
        request.name, request.hash
    );
    let intent = crate::graph::parse_intent(&source, "node").unwrap();
    let preview = store
        .mission(
            &intent,
            IntentInput {
                kdl: source.clone(),
                source_name: None,
            },
        )
        .unwrap();
    assert!(preview.blockers.is_empty(), "{:?}", preview.blockers);
    let applied = store
        .apply_as(
            &intent,
            &preview.subject_tokens,
            "planning-session-publication",
            Some("person/operator"),
        )
        .unwrap();
    let session = store
        .planning_session("planning/release/one")
        .unwrap()
        .unwrap();
    let planner =
        crate::graph::planning_planner_subject("planning-session/planning/release/one");
    assert_eq!(session.status, "planning");
    assert_eq!(session.planner, planner);
    assert!(
        applied
            .operations
            .iter()
            .any(|operation| operation.action == "start-launch")
    );
    assert!(store.selected_desired_token(&planner).unwrap().is_some());
    // The planner seat starts idle; its instructions arrive as one launch request message.
    let requests = store.messages(Some(&planner), false).unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].from, "person/operator");
    assert_eq!(requests[0].title.as_deref(), Some("Launch request"));
    assert!(
        requests[0]
            .content
            .contains("st launch submit planning/release/one"),
        "{}",
        requests[0].content
    );

    let replay_preview = store
        .mission(
            &intent,
            IntentInput {
                kdl: source,
                source_name: None,
            },
        )
        .unwrap();
    let replay = store
        .apply_as(
            &intent,
            &replay_preview.subject_tokens,
            "planning-session-replay",
            Some("person/operator"),
        )
        .unwrap();
    assert!(!replay.changed);
}

#[test]
fn a_declared_human_revision_creates_and_approves_one_proposal() {
    let store = Store::open_memory("node").unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let initial = publish_mission(
        &store,
        r#"version 2
mission "reviewed" state="ready" revisions="human-only" {
  goal "Use the initial definition."
  step "work" { agentless }
}
"#,
        "publish-reviewed-initial",
    );
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: initial.id.clone(),
            revision: Some(initial.revision.clone()),
            workspace: workspace.path().display().to_string(),
            requester: Some("person/operator".into()),
            mode: None,
            inputs: BTreeMap::new(),
            idempotency_key: "run-reviewed".into(),
        })
        .unwrap();
    let candidate = publish_mission(
        &store,
        r#"version 2
mission "reviewed" state="ready" revisions="human-only" {
  goal "Use the reviewed definition."
  step "work" { agentless }
}
"#,
        "publish-reviewed-candidate",
    );
    let revision = format!(
        "version 2\nmission-run {:?} {{\n  revision \"review-one\" {{\n    mission {:?}\n    from {:?}\n    reason \"the definition needs review\"\n  }}\n}}\n",
        run.subject,
        format!("mission/{}@{}", candidate.id, candidate.revision),
        run.generation,
    );
    let intent = crate::graph::parse_intent(&revision, "node").unwrap();
    let preview = store
        .mission(
            &intent,
            IntentInput {
                kdl: revision,
                source_name: None,
            },
        )
        .unwrap();
    let applied = store
        .apply_as(
            &intent,
            &preview.subject_tokens,
            "declare-reviewed-revision",
            Some("person/operator"),
        )
        .unwrap();
    assert!(
        applied
            .operations
            .iter()
            .any(|operation| { operation.action == "propose-revision:review-one" })
    );
    let proposal = store.revision_proposal_for_run(&run.id).unwrap().unwrap();
    assert_eq!(proposal.status, "pending-approval");
    assert_eq!(proposal.reviewers, ["person/operator"]);
    let attention = store.attention_items(Some("person/operator")).unwrap();
    assert_eq!(attention.len(), 1);
    assert_eq!(attention[0].kind, "revision-approval");
    assert_eq!(attention[0].subject, proposal.subject);

    let approved = store
        .approve_revision_proposal(
            &proposal.id,
            "person/operator",
            proposal.preview_hash.as_deref().unwrap(),
            "approve-reviewed-revision",
        )
        .unwrap();
    assert_eq!(approved.status, "applied");
    assert_eq!(approved.mission_run.revision, candidate.revision);
    assert_ne!(approved.mission_run.generation, run.generation);
    assert!(
        store
            .attention_items(Some("person/operator"))
            .unwrap()
            .is_empty()
    );
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
fn snapshot_operational_annotations_follow_owner_claims_at_the_same_index() {
    let store = Store::open_memory("node").expect("store");
    publish_mission(
        &store,
        r#"version 2
mission "snapshot-owner" state="ready" {
  goal "Fence operational history to the selected graph index."
  step "work" { agentless }
}"#,
        "snapshot-owner-mission",
    );
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: "snapshot-owner".into(),
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/test".into()),
            mode: Some("run".into()),
            inputs: BTreeMap::new(),
            idempotency_key: "snapshot-owner-run".into(),
        })
        .unwrap();

    let mut owned = simple("true");
    let desired = owned.subjects.get_mut("exec/work").unwrap();
    desired.owner_run = Some(run.subject.clone());
    desired.owner_generation = Some(run.generation.clone());
    let planned = store
        .mission(
            &owned,
            IntentInput {
                kdl: "owned runtime".into(),
                source_name: None,
            },
        )
        .unwrap();
    let applied = store
        .apply(&owned, &planned.subject_tokens, "snapshot-owned-runtime")
        .unwrap();
    let current_index = applied.store_index;
    let current = store
        .status_at(Some("exec/work"), None, Some(current_index))
        .unwrap();
    assert_eq!(current.subjects[0].projection.layer, "current");

    store
        .append_claim(&ClaimInput {
            subject: run.generation.clone(),
            kind: "run-generation.superseded".into(),
            actor: Some("person/test".into()),
            fields: BTreeMap::from([
                ("status".into(), Value::String("superseded".into())),
                (
                    "successor".into(),
                    Value::String("run-generation/replacement".into()),
                ),
                ("reason".into(), Value::String("test revision".into())),
            ]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
    let superseded_index = store.index().unwrap();
    let superseded = store
        .status_at(Some("exec/work"), None, Some(superseded_index))
        .unwrap();
    assert_eq!(superseded.subjects[0].projection.layer, "history");
    assert!(!superseded.subjects[0].projection.actionable);
    assert!(
        superseded.subjects[0]
            .projection
            .reasons
            .contains(&"superseded".to_owned())
    );

    let replacement = "run-generation/replacement";
    store
        .append_claim(&ClaimInput {
            subject: replacement.into(),
            kind: "run-generation.created".into(),
            actor: Some("person/test".into()),
            fields: BTreeMap::from([
                ("run".into(), Value::String(run.subject.clone())),
                (
                    "revision".into(),
                    Value::String("replacement-revision".into()),
                ),
                ("status".into(), Value::String("running".into())),
                ("reason".into(), Value::String("test revision".into())),
            ]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
    let mut successor_owned = simple("false");
    let desired = successor_owned.subjects.get_mut("exec/work").unwrap();
    desired.owner_run = Some(run.subject.clone());
    desired.owner_generation = Some(replacement.into());
    let planned = store
        .mission(
            &successor_owned,
            IntentInput {
                kdl: "successor-owned runtime".into(),
                source_name: None,
            },
        )
        .unwrap();
    let successor = store
        .apply(
            &successor_owned,
            &planned.subject_tokens,
            "snapshot-successor-owned-runtime",
        )
        .unwrap();
    let successor_status = store
        .status_at(Some("exec/work"), None, Some(successor.store_index))
        .unwrap();
    assert_eq!(successor_status.subjects[0].projection.layer, "current");
    assert!(
        !successor_status.subjects[0]
            .projection
            .reasons
            .contains(&"superseded".to_owned())
    );

    assert!(
        store
            .set_mission_run_state(
                &run.id,
                "cancelled",
                "terminal",
                Some("the owner completed its lifecycle"),
            )
            .unwrap()
    );
    let terminal_index = store.index().unwrap();
    let before_terminal = store
        .status_at(Some("exec/work"), None, Some(current_index))
        .unwrap();
    assert_eq!(before_terminal.subjects[0].projection.layer, "current");
    let terminal = store
        .status_at(Some("exec/work"), None, Some(terminal_index))
        .unwrap();
    assert_eq!(terminal.subjects[0].projection.layer, "history");
    assert!(!terminal.subjects[0].projection.actionable);
    assert!(
        terminal.subjects[0]
            .projection
            .reasons
            .contains(&"terminal-owner".to_owned())
    );
    assert!(
        store
            .status_at(None, None, Some(terminal_index))
            .unwrap()
            .subjects
            .iter()
            .all(|subject| subject.subject != "exec/work")
    );
    let historical = store
        .status_history(None, None, Some(terminal_index))
        .unwrap();
    let owned = historical
        .subjects
        .iter()
        .find(|subject| subject.subject == "exec/work")
        .unwrap();
    assert_eq!(owned.projection.layer, "history");
    assert!(!owned.projection.actionable);
}

#[test]
fn work_extensions_project_locally_and_remotely_without_full_replay() {
    let (controller, worker, step) = replicated_step_pair();
    worker_work(&worker, &step, "claim", None, "extension-claim");
    assert!(!projection_replayed(&controller, "worker", &worker));

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("observer.sqlite3");
    let observer = Store::open(&path, "observer").unwrap();
    receive_and_project(
        &observer,
        "controller",
        &exchange_from(&controller, &ReplicationInventory::default()),
    );
    let extended = worker_extend(&worker, &step);
    assert_eq!(extended.timeout_extension_ms, 300_000);
    worker_work(&worker, &step, "renew", None, "extension-quiet-renewal");
    let quiet_lease = worker
        .step_run(&step)
        .unwrap()
        .unwrap()
        .claim_expires_at_unix_ms;

    // This local extension was already applied. The next peer exchange must not turn it
    // into a full-store replay, and must preserve the local lease and execution budget.
    assert!(!projection_replayed(&worker, "controller", &controller));
    let projected = worker.step_run(&step).unwrap().unwrap();
    assert_eq!(projected.status, "claimed");
    assert_eq!(projected.claim_expires_at_unix_ms, quiet_lease);
    assert_eq!(projected.blocked_reason, extended.blocked_reason);
    let incremental = graph_digest_of(&worker);
    worker.replay_replication_graph().unwrap();
    assert_eq!(incremental, graph_digest_of(&worker));
    assert_eq!(
        worker
            .step_run(&step)
            .unwrap()
            .unwrap()
            .timeout_extension_ms,
        300_000
    );
    assert_eq!(
        worker
            .step_run(&step)
            .unwrap()
            .unwrap()
            .claim_expires_at_unix_ms,
        quiet_lease
    );

    // The remote extension arrives after a newer local state update. Rebuild only this run
    // tree in canonical order rather than overwriting that update or replaying the store.
    controller
        .set_step_state(&step, "blocked", Some("a newer update"))
        .unwrap();
    assert!(!projection_replayed(&controller, "worker", &worker));
    assert!(!projection_replayed(&observer, "controller", &controller));
    for store in [&controller, &observer] {
        let projected = store.step_run(&step).unwrap().unwrap();
        assert_eq!(projected.status, "blocked");
        assert_eq!(projected.timeout_extension_ms, 300_000);
        let incremental = graph_digest_of(store);
        store.replay_replication_graph().unwrap();
        assert_eq!(incremental, graph_digest_of(store));
        let replayed = store.step_run(&step).unwrap().unwrap();
        assert_eq!(projected.status, replayed.status);
        assert_eq!(
            projected.timeout_extension_ms,
            replayed.timeout_extension_ms
        );
        assert_eq!(projected.attempt, replayed.attempt);
    }
    let digest = graph_digest_of(&observer);
    let copy = directory.path().join("checkpoint-replay.sqlite3");
    observer.copy_store_to(&copy).unwrap();
    let mut connection = Connection::open(&copy).unwrap();
    projection_digest::register(&connection).unwrap();
    let transaction = connection.transaction().unwrap();
    checkpoint_rules::replay_from_nothing(&transaction).unwrap();
    transaction.commit().unwrap();
    assert_eq!(
        digest,
        projection_digest::root(&projection_digest::tables(&connection).unwrap())
    );
    let mut restored = step_run_row_tx(&connection, &step).unwrap().unwrap();
    enrich_step_queue(&connection, &mut restored).unwrap();
    let projected = observer.step_run(&step).unwrap().unwrap();
    assert_eq!(restored.status, projected.status);
    assert_eq!(
        restored.claim_expires_at_unix_ms,
        projected.claim_expires_at_unix_ms
    );
    assert_eq!(restored.blocked_reason, projected.blocked_reason);
    assert_eq!(
        restored.timeout_extension_ms,
        projected.timeout_extension_ms
    );
    drop(observer);
    let reopened = Store::open(&path, "observer").unwrap();
    assert_eq!(digest, graph_digest_of(&reopened));
    assert_eq!(
        reopened
            .step_run(&step)
            .unwrap()
            .unwrap()
            .timeout_extension_ms,
        300_000
    );
}

#[test]
fn work_extensions_upgrade_repairs_affected_trees_once_and_keeps_local_renewals() {
    let (controller, worker, step) = replicated_step_pair();
    worker_work(&worker, &step, "claim", None, "upgrade-extension-claim");
    let previous = worker.step_run(&step).unwrap().unwrap();
    let extended = worker_extend(&worker, &step);
    worker_work(&worker, &step, "renew", None, "upgrade-extension-quiet");
    let quiet_lease = worker
        .step_run(&step)
        .unwrap()
        .unwrap()
        .claim_expires_at_unix_ms;
    assert!(!projection_replayed(&worker, "controller", &controller));
    let corrected = graph_digest_of(&worker);

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("upgrade.sqlite3");
    worker.copy_store_to(&path).unwrap();
    {
        let connection = Connection::open(&path).unwrap();
        projection_digest::register(&connection).unwrap();
        // A pre-fix full replay left the last claim's state rather than the extension's.
        connection.execute_batch(
            /* sqlite-large-value: allow-for-now lv-h6147a4dd7237 owner=st3_attention::store::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO meta(key,value) VALUES('canonical_shared_projection_rules','2');
                 INSERT OR REPLACE INTO meta(key,value) VALUES('derived_tables_version','2');",
        ).unwrap();
        connection
            .execute(
                /* sqlite-large-value: allow-for-now lv-h5c4e6798e46d owner=st3_attention::store::tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE step_runs SET blocked_reason=?2, lease_expires_at_unix_ms=?3,
                    updated_at_unix_ms=?4 WHERE subject=?1",
                params![
                    step,
                    previous.blocked_reason,
                    previous
                        .claim_expires_at_unix_ms
                        .map(|value| value.to_string()),
                    previous.updated_at_unix_ms.to_string()
                ],
            )
            .unwrap();
        connection
            .execute(
                /* sqlite-large-value: allow-for-now lv-h5688acdbc080 owner=st3_attention::store::tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM meta WHERE key='work_extended_projection_rules'",
                [],
            )
            .unwrap();
        // A full-store/operations rebuild would remove this unrelated derived row.
        connection
            .execute(
                /* sqlite-large-value: allow-for-now lv-h7ddc2f792b05 owner=st3_attention::store::tests TODO=prepare-body-and-predecessor-values */ "INSERT INTO operations(id,request_digest,canonical_claim_id,state)
                 SELECT 'op/unrelated-upgrade','digest',id,'active' FROM claims
                 WHERE kind='work.extended' LIMIT 1",
                [],
            )
            .unwrap();
    }
    let upgraded = Store::open(&path, "worker").unwrap();
    assert_eq!(
        upgraded
            .smalltalk
            .work_extension_roots_rebuilt
            .load(Ordering::Relaxed),
        1
    );
    let projected = upgraded.step_run(&step).unwrap().unwrap();
    assert_eq!(projected.status, "claimed");
    assert_eq!(projected.timeout_extension_ms, 300_000);
    assert_eq!(projected.blocked_reason, extended.blocked_reason);
    assert_eq!(projected.claim_expires_at_unix_ms, quiet_lease);
    {
        let connection = upgraded.readers.get();
        assert!(
            connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM operations WHERE id='op/unrelated-upgrade')",
                    [],
                    |row| row.get::<_, bool>(0),
                )
                .unwrap()
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT value FROM meta WHERE key='work_extended_projection_rules'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "1"
        );
    }
    // The operations sentinel is outside the corrected graph's real authority.
    upgraded
        .connection
        .write()
        .execute(/* sqlite-large-value: allow-for-now lv-hc75dbe01286b owner=st3_attention::store::tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM operations WHERE id='op/unrelated-upgrade'", [])
        .unwrap();
    assert_eq!(corrected, graph_digest_of(&upgraded));
    drop(upgraded);
    let reopened = Store::open(&path, "worker").unwrap();
    assert_eq!(
        reopened
            .smalltalk
            .work_extension_roots_rebuilt
            .load(Ordering::Relaxed),
        0
    );
    assert_eq!(corrected, graph_digest_of(&reopened));
    assert_eq!(
        reopened
            .step_run(&step)
            .unwrap()
            .unwrap()
            .claim_expires_at_unix_ms,
        quiet_lease
    );
}

#[test]
fn work_extensions_upgrade_and_adopt_a_v5_certificate_without_changing_its_terms() {
    use smallclaims::store::checkpoint::checkpoint_name;
    use smallclaims::store::checkpoint_agreement::{SealTerms, certificates};
    let (controller, worker, step) = replicated_step_pair();
    worker_work(&worker, &step, "claim", None, "historic-extension-claim");
    for index in 0..5 {
        controller
            .append_claim(&ClaimInput {
                subject: "daemon/controller".into(),
                kind: "daemon.diagnostic".into(),
                actor: None,
                fields: BTreeMap::from([
                    ("severity".into(), json!("warning")),
                    ("code".into(), json!("invented")),
                    ("reason".into(), json!(format!("observation {index}"))),
                ]),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
    }
    assert!(!projection_replayed(&controller, "worker", &worker));
    assert!(!projection_replayed(&worker, "controller", &controller));
    let scratch = tempfile::tempdir().unwrap();
    let cut = now_ms() + 1;
    let checkpoint = checkpoint_name(cut);
    let mut legacy_digest = Sha256::new();
    legacy_digest.update(b"st3-checkpoint-rules-v1\0");
    legacy_digest.update(5_u32.to_be_bytes());
    legacy_digest.update(checkpoint_rules::rules_description().as_bytes());
    let legacy_digest = hex::encode(legacy_digest.finalize());
    assert_ne!(legacy_digest, rules_digest());
    let sealed = controller.checkpoint_sealed_identities(cut, None).unwrap();
    let terms = SealTerms {
        cut_unix_ms: cut,
        participants: BTreeSet::from(["controller".into(), "worker".into()]),
        sealed_digest: sealed.digest.clone(),
        rules_digest: legacy_digest.clone(),
    };
    for node in [&controller, &worker] {
        node.publish_seal(&checkpoint, &terms, &sealed, None)
            .unwrap();
    }
    let (_, mut plan, proof) = controller
        .plan_checkpoint_through(cut, None, scratch.path())
        .unwrap();
    assert!(proof.passed, "{proof:?}");
    assert!(!plan.claims.is_empty());
    // The sealed set contains no extensions: v5 and v6 have identical graph/timing proof
    // here. Retain the real v5 rule identity when constructing its historical certificate.
    plan.rules_digest = legacy_digest;
    for node in [&controller, &worker] {
        node.publish_verification(&checkpoint, &terms, &smallclaims::store::checkpoint_staging::StagedPlan::of(&plan), &proof)
            .unwrap();
    }
    assert!(!projection_replayed(&controller, "worker", &worker));
    assert!(!projection_replayed(&worker, "controller", &controller));
    let certificate = certificates(&worker.checkpoint_claims().unwrap(), &checkpoint);
    assert_eq!(certificate.len(), 1);

    // An extension after the historic cut survives its v5 drops and changes the live graph.
    let previous = worker.step_run(&step).unwrap().unwrap();
    let extended = worker_extend(&worker, &step);
    worker_work(&worker, &step, "renew", None, "historic-extension-quiet");
    assert!(!projection_replayed(&worker, "controller", &controller));
    let path = scratch.path().join("historic-upgrade.sqlite3");
    worker.copy_store_to(&path).unwrap();
    {
        let connection = Connection::open(&path).unwrap();
        projection_digest::register(&connection).unwrap();
        connection.execute_batch(
            /* sqlite-large-value: allow-for-now lv-hc3070e5519f6 owner=st3_attention::store::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO meta(key,value) VALUES('canonical_shared_projection_rules','2');
                 INSERT OR REPLACE INTO meta(key,value) VALUES('derived_tables_version','2');",
        ).unwrap();
        connection.execute(/* sqlite-large-value: allow-for-now lv-h0d4aee1ea2ad owner=st3_attention::store::tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE step_runs SET blocked_reason=?2, updated_at_unix_ms=?3 WHERE subject=?1",
            params![step, previous.blocked_reason, previous.updated_at_unix_ms.to_string()]).unwrap();
        connection
            .execute(
                /* sqlite-large-value: allow-for-now lv-haab20c8fdbd1 owner=st3_attention::store::tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM meta WHERE key='work_extended_projection_rules'",
                [],
            )
            .unwrap();
    }
    let upgraded = Store::open(&path, "worker").unwrap();
    let before = upgraded.step_run(&step).unwrap().unwrap();
    assert_eq!(before.blocked_reason, extended.blocked_reason);
    assert_eq!(before.timeout_extension_ms, 300_000);
    let graph = graph_digest_of(&upgraded);
    let reader_cut = now_ms();
    let reader =
        checkpoint_rules::subject_answers(&upgraded.readers.get(), &step, reader_cut).unwrap();
    let mut actions = Vec::new();
    controller
        .trim_checkpoint(
            &checkpoint,
            cut,
            &plan.drop_digest,
            &plan.envelopes,
            &plan.claims,
            false,
            &mut actions,
        )
        .unwrap();
    let manifest = controller.checkpoint_manifest(&checkpoint, cut).unwrap();
    assert!(!upgraded.adopt_checkpoint(&manifest).unwrap().is_empty());
    assert_eq!(graph, graph_digest_of(&upgraded));
    let after = upgraded.step_run(&step).unwrap().unwrap();
    assert_eq!(before.status, after.status);
    assert_eq!(before.blocked_reason, after.blocked_reason);
    assert_eq!(
        before.claim_expires_at_unix_ms,
        after.claim_expires_at_unix_ms
    );
    assert_eq!(before.timeout_extension_ms, after.timeout_extension_ms);
    // Use the same cut for time-dependent reader answers.
    let reader_after =
        checkpoint_rules::subject_answers(&upgraded.readers.get(), &step, reader_cut).unwrap();
    assert_eq!(reader, reader_after);
    assert_eq!(
        certificates(&upgraded.checkpoint_claims().unwrap(), &checkpoint),
        certificate
    );
    assert_eq!(
        upgraded.checkpoint_manifest(&checkpoint, cut).unwrap(),
        manifest
    );
    assert!(upgraded.adopt_checkpoint(&manifest).unwrap().is_empty());
    drop(upgraded);
    let reopened = Store::open(&path, "worker").unwrap();
    assert_eq!(
        certificates(&reopened.checkpoint_claims().unwrap(), &checkpoint),
        certificate
    );
    assert_eq!(
        reopened.checkpoint_manifest(&checkpoint, cut).unwrap(),
        manifest
    );
    assert_eq!(graph, graph_digest_of(&reopened));
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
fn planning_state_and_attention_agree_when_claims_arrive_in_different_orders() {
    let source = Store::open_memory("source").unwrap();
    let kdl = "version 2\nmission \"release\" state=\"ready\" { goal \"Ship a release\" }\n";
    let intent = crate::graph::parse_test_intent(kdl, "source").unwrap();
    let mission = source
        .mission(
            &intent,
            IntentInput {
                kdl: kdl.into(),
                source_name: None,
            },
        )
        .unwrap();
    let doc = source
        .put_document(
            "doc/planning/release/request",
            b"Ship a release",
            &None,
            "request",
        )
        .unwrap();
    let reference = format!("{}@{}", doc.name, doc.hash);
    let subject = "planning-session/planning/release/one";
    let events = [
        (
            "started",
            json!({"mission": "mission/release", "request": reference, "workspace": "/work/release", "requester": "person/avery", "planner": "agent/source.planner"}),
        ),
        (
            "candidate-submitted",
            json!({"candidate_revision": 1, "markdown": reference, "kdl": reference, "mission_revision": "revision-one"}),
        ),
        (
            "previewed",
            json!({"candidate_revision": 1, "preview_hash": "preview-one", "store_index": mission.store_index, "graph": "release", "diff": "new", "mission": mission}),
        ),
        (
            "approved",
            json!({"mission_revision": "revision-one", "requester": "person/avery"}),
        ),
    ];
    for (kind, fields) in events {
        // Distinct accepted times also prove this is independent of the receiver's indexes.
        std::thread::sleep(std::time::Duration::from_millis(2));
        source
            .append_claim(&ClaimInput {
                subject: subject.into(),
                kind: format!("planning-session.{kind}"),
                actor: Some("person/avery".into()),
                fields: serde_json::from_value(fields).unwrap(),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: Some(format!("planning-{kind}")),
            })
            .unwrap();
        if kind == "previewed" {
            source.replay_replication_graph().unwrap();
            let attention = source.attention_items(Some("person/avery")).unwrap();
            assert_eq!(attention.len(), 1);
            assert_eq!(attention[0].kind, "launch-approval");
        }
    }
    let envelopes = exchange_from(&source, &ReplicationInventory::default()).envelopes;
    let in_order = Store::open_memory("in-order").unwrap();
    receive_and_project(
        &in_order,
        "source",
        &exchange_of("source", envelopes.clone()),
    );
    let reversed = Store::open_memory("reversed").unwrap();
    for envelope in envelopes.iter().rev() {
        receive_and_project(
            &reversed,
            "source",
            &exchange_of("source", vec![envelope.clone()]),
        );
    }
    assert_eq!(graph_digest_of(&in_order), graph_digest_of(&reversed));
    for store in [&in_order, &reversed] {
        let launch = store.planning_session(subject).unwrap().unwrap();
        assert_eq!(launch.status, "approved");
        assert_eq!(launch.published_revision.as_deref(), Some("revision-one"));
        assert!(
            store
                .attention_items(Some("person/avery"))
                .unwrap()
                .is_empty()
        );
        // Startup replay must retain the same result, too.
        store.replay_replication_graph().unwrap();
        assert_eq!(
            store.planning_session(subject).unwrap().unwrap().status,
            "approved"
        );
        assert!(
            store
                .attention_items(Some("person/avery"))
                .unwrap()
                .is_empty()
        );
    }
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
                /* sqlite-large-value: allow-for-now lv-hf6680c57adbe owner=st3_attention::store::tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_records SET state='invalid' WHERE record_ref=?1",
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
        .execute(/* sqlite-large-value: allow-for-now lv-ha856f42ff9b4 owner=st3_attention::store::tests TODO=prepare-body-and-predecessor-values */ "DELETE FROM claims WHERE id=?1", [&repaired_claim])
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
                .prepare(/* sqlite-large-value: allow-for-now lv-h8584a5e35ae0 owner=st3_attention::store::tests TODO=trace-statement-and-value-representation */ sql)
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
            /* sqlite-large-value: allow-for-now lv-h335074fbac64 owner=st3_attention::store::tests TODO=prepare-body-and-predecessor-values */ "PRAGMA foreign_keys = OFF;
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
                /* sqlite-large-value: allow-for-now lv-h76fc801f35a0 owner=st3_attention::store::tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM message_index; DELETE FROM meta WHERE key='message_index';",
            )
            .unwrap();
        backfill_message_index(&connection).unwrap();
    }
    let (index, claims) = indexed(&store);
    assert_eq!(index, claims);
}

#[test]
fn legacy_attention_does_not_block_released_work_across_restart_and_replication() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("external-blocker.sqlite3");
    let store = Store::open(&path, "source").unwrap();
    let source = r#"
version 2

mission "external-blocker" state="ready" {
  goal "Wait truthfully for the external host repair."
  step "automated-proof" { assigned-to "agent/ios-owner" }
}
"#;
    let intent = crate::graph::parse_test_intent(source, "source").unwrap();
    let planned = store
        .mission(
            &intent,
            IntentInput {
                kdl: source.into(),
                source_name: None,
            },
        )
        .unwrap();
    store
        .apply(&intent, &planned.subject_tokens, "external-blocker-mission")
        .unwrap();
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: "external-blocker".into(),
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/alex".into()),
            mode: Some("run".into()),
            inputs: BTreeMap::new(),
            idempotency_key: "external-blocker-run".into(),
        })
        .unwrap();
    let subject = run.steps[0].subject.clone();
    store.set_step_state(&subject, "ready", None).unwrap();
    let request = |key: &str, reason: Option<&str>| WorkRequest {
        wait_ms: None,
        actor: Some("agent/source.ios-owner".into()),
        incarnation: Some("ios-owner-one".into()),
        summary: None,
        reason: reason.map(str::to_owned),
        evidence: Vec::new(),
        idempotency_key: key.into(),
    };
    store
        .work_action(&subject, "claim", &request("blocker-claim", None))
        .unwrap();
    let attention = store
        .request_attention(
            "attention/example-mac-xcode",
            &AttentionRequest {
                reviewer: "person/alex".into(),
                title: "ExampleMac needs its Xcode simulator components updated".into(),
                reason: "CoreSimulator must be repaired before automated proof can run.".into(),
                severity: "error".into(),
                targets: vec!["host/example-mac".into(), subject.clone()],
                actor: "agent/source.ios-owner".into(),
                idempotency_key: "example-mac-xcode-attention".into(),
            },
        )
        .unwrap();
    let reason = "ExampleMac has an exact CoreSimulator/CoreDevice mismatch; renewing this claim would be idle and misleading.";
    store
        .work_action(
            &subject,
            "release",
            &request("blocker-release", Some(reason)),
        )
        .unwrap();

    let ready = store.step_run(&subject).unwrap().unwrap();
    assert_eq!(ready.status, "ready");
    assert!(ready.blockers.is_empty());
    drop(store);

    let store = Store::open(&path, "source").unwrap();
    assert_eq!(store.step_run(&subject).unwrap().unwrap().status, "ready");
    let replica = Store::open_memory("replica").unwrap();
    let exchange = exchange_from(&store, &ReplicationInventory::default());
    assert_eq!(
        receive_and_project(&replica, "source", &exchange).invalid,
        0
    );
    let replicated = replica.step_run(&subject).unwrap().unwrap();
    assert_eq!(replicated.status, "ready");
    assert!(replicated.blockers.is_empty());

    store
        .resolve_attention(
            &attention.subject,
            &AttentionResolveRequest {
                delegation: None,
                outcome: "resolved".into(),
                reason: Some("Xcode first-launch setup now succeeds.".into()),
                actor: "person/alex".into(),
                idempotency_key: "resolve-example-mac-xcode".into(),
            },
        )
        .unwrap();
    let reopened = store.step_run(&subject).unwrap().unwrap();
    assert_eq!(reopened.status, "ready");
    assert_eq!(reopened.blocked_reason, None);
    assert!(reopened.blockers.is_empty());
    store
        .work_action(&subject, "claim", &request("unblocked-reclaim", None))
        .unwrap();
}

#[test]
fn operational_repair_closes_expired_claims_and_wake_contradictions() {
    let store = Store::open_memory("node").unwrap();
    let source = r#"version 2
 agent "worker" { workspace "/tmp"; command "true" }
 mission "repairable" state="ready" {
   goal "Exercise bounded operational repair."
   step "work" { assigned-to "agent/node.worker" }
 }"#;
    let intent = crate::graph::parse_test_intent(source, "node").unwrap();
    let planned = store
        .mission(
            &intent,
            IntentInput {
                kdl: source.into(),
                source_name: None,
            },
        )
        .unwrap();
    store
        .apply(&intent, &planned.subject_tokens, "repairable-mission")
        .unwrap();
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: "repairable".into(),
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/operator".into()),
            mode: Some("run".into()),
            inputs: BTreeMap::new(),
            idempotency_key: "repairable-run".into(),
        })
        .unwrap();
    let step = run.steps[0].subject.clone();
    store.set_step_state(&step, "ready", None).unwrap();
    store
        .work_action(
            &step,
            "claim",
            &WorkRequest {
                wait_ms: None,
                actor: Some("agent/node.worker".into()),
                incarnation: Some("worker-one".into()),
                summary: None,
                reason: None,
                evidence: Vec::new(),
                idempotency_key: "repair-expired-claim".into(),
            },
        )
        .unwrap();
    expire_work_lease_by_claim(&store, &step, "agent/node.worker", "worker-one");

    store
        .request_attention(
            "attention/stale-repair",
            &AttentionRequest {
                reviewer: "person/operator".into(),
                title: "Stale repair attention".into(),
                reason: "a superseded target still appears pending".into(),
                severity: "warning".into(),
                targets: vec!["step-run/missing-generation/missing-step".into()],
                actor: "person/operator".into(),
                idempotency_key: "stale-repair-attention".into(),
            },
        )
        .unwrap();
    // The Typecase shape: the mission it names was cancelled after the request.
    let typecase = start_agentless_run(&store, "typecase");
    request_fault(&store, "attention/typecase", &["mission/typecase"]);
    store
        .set_mission_run_state(
            &typecase.id,
            "cancelled",
            "terminal",
            Some("moved to a seat"),
        )
        .unwrap();

    store
        .append_claim(&ClaimInput {
            subject: "agent/node.worker".into(),
            kind: "runtime.observed".into(),
            actor: None,
            fields: BTreeMap::from([
                ("status".into(), Value::String("running".into())),
                ("runtime_id".into(), Value::String("node.worker".into())),
                ("terminal".into(), Value::Bool(true)),
                ("incarnation_id".into(), Value::String("worker-one".into())),
            ]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: Some("repair-runtime".into()),
        })
        .unwrap();
    let diagnostic = store
        .append_claim(&ClaimInput {
            subject: "agent/node.worker".into(),
            kind: "harness.diagnostic".into(),
            actor: Some("agent/node.worker".into()),
            fields: BTreeMap::from([
                ("severity".into(), Value::String("error".into())),
                ("status".into(), Value::String("failed".into())),
                ("code".into(), Value::String("work-wake-exhausted".into())),
                (
                    "reason".into(),
                    Value::String("the legacy wake was not acknowledged".into()),
                ),
                ("incarnation_id".into(), Value::String("worker-one".into())),
                ("step_run".into(), Value::String(step.clone())),
                ("wake_attempts".into(), Value::from(3)),
            ]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: Some("repair-wake-diagnostic".into()),
        })
        .unwrap();
    store
        .append_claim(&ClaimInput {
            subject: "agent/node.worker".into(),
            kind: "harness.observed".into(),
            actor: Some("agent/node.worker".into()),
            fields: BTreeMap::from([
                ("state".into(), Value::String("working".into())),
                ("driver".into(), Value::String("codex".into())),
                ("incarnation_id".into(), Value::String("worker-one".into())),
            ]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: Some("repair-wake-ack".into()),
        })
        .unwrap();

    let plan = store.operational_repair_plan().unwrap();
    let classes = plan
        .items
        .iter()
        .map(|item| item.class.as_str())
        .collect::<BTreeSet<_>>();
    assert!(classes.contains("expired-claim"));
    assert!(classes.contains("wake-contradiction"));
    assert!(!classes.contains("superseded-attention"));
    let wake = plan
        .items
        .iter()
        .find(|item| item.class == "wake-contradiction")
        .unwrap();
    assert_eq!(
        wake.details.get("diagnostic_claim").and_then(Value::as_str),
        Some(diagnostic.id.as_str())
    );
    assert!(
        wake.affected_subjects
            .contains(&"agent/node.worker".to_owned())
    );
    assert!(wake.affected_subjects.contains(&step));

    let stale = store
        .apply_operational_repair(&format!("orpv0:{}", "0".repeat(64)))
        .unwrap_err();
    assert_eq!(stale.code, "stale-repair-plan");

    let result = store.apply_operational_repair(&plan.token).unwrap();
    assert_eq!(result.applied, plan.items.len());
    assert_eq!(store.step_run(&step).unwrap().unwrap().status, "ready");
    assert!(
        store
            .attention_items(Some("person/operator"))
            .unwrap()
            .is_empty()
    );
    let recovered = store
        .latest_claim("agent/node.worker", Some("harness.diagnostic"))
        .unwrap()
        .unwrap();
    assert_eq!(
        recovered
            .body
            .pointer("/fields/status")
            .and_then(Value::as_str),
        Some("recovered")
    );
    assert_eq!(store.operational_repair_plan().unwrap().status, "clean");
    let duplicate = store.apply_operational_repair(&plan.token).unwrap();
    assert_eq!(duplicate.applied, 0);
    assert!(duplicate.already_applied);
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

#[test]
fn all_affected_human_reviewers_approve_the_exact_revision_preview() {
    let store = Store::open_memory("node").unwrap();
    let publish = |goal: &str, key: &str| {
        let source = format!(
            r#"
version 2

  mission "protected" state="ready" revisions="human-only" revision-reviewer="person/mission-reviewer" {{
    goal "Test protected revision."
     agent "worker" {{ workspace "."; command "true" }}
    step "work" revisions="human-only" revision-reviewer="person/step-reviewer" {{
      assigned-to "agent/worker"
      goal {goal:?}
    }}
  }}

"#
        );
        let intent = crate::graph::parse_test_intent(&source, "node").unwrap();
        let planned = store
            .mission(
                &intent,
                IntentInput {
                    kdl: source,
                    source_name: None,
                },
            )
            .unwrap();
        store.apply(&intent, &planned.subject_tokens, key).unwrap();
        intent.missions["protected"].clone()
    };
    let first = publish("Use the first goal.", "protected-one");
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: first.id,
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/requester".into()),
            mode: Some("run".into()),
            inputs: BTreeMap::new(),
            idempotency_key: "protected-run".into(),
        })
        .unwrap();
    let worker = format!("agent/{}/worker", run.id);
    let second = publish("Use the second goal.", "protected-two");
    let proposal = store
        .create_revision_proposal(
            &run.id,
            &second,
            &worker,
            "the first goal is incomplete",
            "protected-proposal",
        )
        .unwrap();
    assert_eq!(proposal.status, "pending-approval");
    assert_eq!(
        proposal.reviewers,
        vec!["person/mission-reviewer", "person/step-reviewer"]
    );
    assert_eq!(store.attention_items(None).unwrap().len(), 2);
    // A mission view reads only the attention that names a run, and finds the same runs.
    assert_eq!(
        store.human_attention_runs().unwrap(),
        store
            .attention_items(None)
            .unwrap()
            .into_iter()
            .filter_map(|item| item.mission_run)
            .collect::<BTreeSet<_>>()
    );
    assert_eq!(
        store.human_attention_runs().unwrap(),
        BTreeSet::from([run.subject.clone()])
    );
    assert_eq!(
        store
            .attention_items(Some("person/mission-reviewer"))
            .unwrap()
            .len(),
        1
    );
    let error = store
        .approve_revision_proposal(
            &proposal.id,
            "person/mission-reviewer",
            "wrong-preview",
            "protected-wrong-preview",
        )
        .unwrap_err();
    assert_eq!(error.code, "stale-revision-preview");
    let first_approval = store
        .approve_revision_proposal(
            &proposal.id,
            "person/mission-reviewer",
            proposal.preview_hash.as_deref().unwrap(),
            "protected-first-approval",
        )
        .unwrap();
    assert_eq!(first_approval.status, "pending-approval");
    assert_eq!(first_approval.mission_run.generation, run.generation);
    assert!(
        store
            .attention_items(Some("person/mission-reviewer"))
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        store
            .attention_items(Some("person/step-reviewer"))
            .unwrap()
            .len(),
        1
    );
    let applied = store
        .approve_revision_proposal(
            &proposal.id,
            "person/step-reviewer",
            proposal.preview_hash.as_deref().unwrap(),
            "protected-final-approval",
        )
        .unwrap();
    assert_eq!(applied.status, "applied");
    assert_ne!(applied.mission_run.generation, run.generation);
    assert_eq!(
        applied
            .proposal
            .as_ref()
            .unwrap()
            .successor_generation
            .as_deref(),
        Some(applied.mission_run.generation.as_str())
    );
    let retried = store
        .approve_revision_proposal(
            &proposal.id,
            "person/step-reviewer",
            proposal.preview_hash.as_deref().unwrap(),
            "protected-final-approval-retry",
        )
        .unwrap();
    assert_eq!(
        retried.mission_run.generation,
        applied.mission_run.generation
    );
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
fn pending_human_reviews_exclude_stale_and_terminal_owners() {
    let store = Store::open_memory("node").unwrap();
    let publish = |goal: &str, key: &str| {
        publish_mission(
            &store,
            &format!(
                r#"version 2
mission "review-current" state="ready" revision-cutover="restart-active" {{
  goal "Review current work."
  step "approval" {{ goal {goal:?}; agentless }}
}}"#
            ),
            key,
        )
    };
    let first = publish("Review the first revision.", "review-current-first");
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: first.id.clone(),
            revision: None,
            workspace: ".".into(),
            requester: Some("person/test".into()),
            mode: Some("run".into()),
            inputs: BTreeMap::new(),
            idempotency_key: "review-current-run".into(),
        })
        .unwrap();
    let first_step = run.steps[0].clone();
    let add_request = |subject: &str,
                       owner: &str,
                       revision: &str,
                       definition: &str,
                       attempt: u32,
                       key: &str| {
        store
            .append_claim(&ClaimInput {
                subject: subject.into(),
                kind: "gate.requested".into(),
                actor: None,
                fields: BTreeMap::from([
                    ("owner".into(), Value::String(owner.into())),
                    ("reviewer".into(), Value::String("person/alex".into())),
                    ("question".into(), Value::String("Approve it?".into())),
                    ("review_targets".into(), Value::Array(Vec::new())),
                    (
                        "decisions".into(),
                        Value::Array(vec![
                            Value::String("approved".into()),
                            Value::String("rejected".into()),
                        ]),
                    ),
                    ("operation".into(), Value::String(subject.into())),
                    ("mission_revision".into(), Value::String(revision.into())),
                    ("step_definition".into(), Value::String(definition.into())),
                    ("attempt".into(), Value::from(attempt)),
                ]),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: Some(key.into()),
            })
            .unwrap()
    };
    add_request(
        "gate-operation/review-current/old-generation",
        &first_step.subject,
        &run.revision,
        &first_step.definition_hash,
        first_step.attempt,
        "old-generation-request",
    );
    assert_eq!(store.pending_human_reviews(None).unwrap().len(), 1);

    let second = publish("Review the second revision.", "review-current-second");
    let revised = store
        .adopt_mission_revision(
            &run.id,
            &second,
            "person/test",
            "use the new review step",
            "review-current-cutover",
        )
        .unwrap();
    assert!(store.pending_human_reviews(None).unwrap().is_empty());
    let current_step = revised.steps[0].clone();

    add_request(
        "gate-operation/review-current/bad-definition",
        &current_step.subject,
        &revised.revision,
        "old-definition",
        current_step.attempt,
        "bad-definition-request",
    );
    add_request(
        "gate-operation/review-current/bad-attempt",
        &current_step.subject,
        &revised.revision,
        &current_step.definition_hash,
        current_step.attempt + 1,
        "bad-attempt-request",
    );
    assert!(store.pending_human_reviews(None).unwrap().is_empty());

    add_request(
        "gate-operation/review-current/terminal-step",
        &current_step.subject,
        &revised.revision,
        &current_step.definition_hash,
        current_step.attempt,
        "terminal-step-request",
    );
    assert_eq!(store.pending_human_reviews(None).unwrap().len(), 1);
    store
        .set_step_state(&current_step.subject, "completed", None)
        .unwrap();
    assert!(store.pending_human_reviews(None).unwrap().is_empty());

    add_request(
        "gate-operation/review-current/terminal",
        &revised.subject,
        &revised.revision,
        &revised.revision,
        1,
        "terminal-request",
    );
    assert_eq!(store.pending_human_reviews(None).unwrap().len(), 1);
    store
        .set_mission_run_state(&revised.id, "cancelled", "normal", None)
        .unwrap();
    assert!(store.pending_human_reviews(None).unwrap().is_empty());
}

#[test]
fn a_gate_asked_again_is_one_review_that_has_waited_since_its_first_request() {
    let store = Store::open_memory("node").unwrap();
    publish_mission(
        &store,
        r#"version 2
mission "asked-again" state="ready" {
  goal "Review one change."
  step "approval" { goal "Approve the change."; agentless }
}"#,
        "asked-again-mission",
    );
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: "asked-again".into(),
            revision: None,
            workspace: ".".into(),
            requester: Some("person/test".into()),
            mode: Some("run".into()),
            inputs: BTreeMap::new(),
            idempotency_key: "asked-again-run".into(),
        })
        .unwrap();
    let step = run.steps[0].clone();
    // An older build asked without a mode; a newer build words the same gate with one,
    // so its request is a new operation.
    let ask = |operation: &str, mode: Option<&str>| {
        let mut fields = BTreeMap::from([
            ("owner".into(), Value::String(step.subject.clone())),
            ("reviewer".into(), Value::String("person/alex".into())),
            ("question".into(), Value::String("Approve it?".into())),
            ("review_targets".into(), Value::Array(Vec::new())),
            (
                "decisions".into(),
                Value::Array(vec![
                    Value::String("approved".into()),
                    Value::String("rejected".into()),
                ]),
            ),
            ("operation".into(), Value::String(operation.into())),
            (
                "mission_revision".into(),
                Value::String(run.revision.clone()),
            ),
            (
                "step_definition".into(),
                Value::String(step.definition_hash.clone()),
            ),
            ("attempt".into(), Value::from(step.attempt)),
        ]);
        if let Some(mode) = mode {
            fields.insert("mode".into(), Value::String(mode.into()));
        }
        store
            .append_claim(&ClaimInput {
                subject: operation.into(),
                kind: "gate.requested".into(),
                actor: None,
                fields,
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: Some(operation.into()),
            })
            .unwrap()
    };
    let first = ask("gate-operation/asked-again/first", None);
    std::thread::sleep(std::time::Duration::from_millis(2));
    let again = ask("gate-operation/asked-again/again", Some("approve"));
    assert!(again.accepted_at_unix_ms > first.accepted_at_unix_ms);

    let reviews = store.pending_human_reviews(None).unwrap();
    assert_eq!(reviews.len(), 1, "{reviews:?}");
    assert_eq!(
        reviews[0].request, again.id,
        "the reviewer answers the request the gate waits on"
    );
    assert_eq!(reviews[0].requested_at_unix_ms, first.accepted_at_unix_ms);

    let items = store.attention_items(Some("person/alex")).unwrap();
    let gates = items
        .iter()
        .filter(|item| item.kind == "human-gate")
        .collect::<Vec<_>>();
    assert_eq!(gates.len(), 1);
    assert_eq!(gates[0].requested_at_unix_ms, first.accepted_at_unix_ms);
}

fn request_fault(store: &Store, subject: &str, targets: &[&str]) {
    store
        .request_attention(
            subject,
            &AttentionRequest {
                reviewer: "person/alex".into(),
                title: format!("Fault {subject}"),
                reason: "a person must decide".into(),
                severity: "warning".into(),
                targets: targets.iter().map(|target| (*target).to_owned()).collect(),
                actor: "daemon/runtime".into(),
                idempotency_key: format!("{subject}:requested"),
            },
        )
        .unwrap();
}

#[test]
fn attention_target_states_describe_each_target_with_a_lifecycle() {
    let store = Store::open_memory("node").unwrap();
    publish_mission(
        &store,
        r#"version 2
mission "typecase" state="ready" {
  goal "Publish a revision."
  step "publish" { goal "Publish it."; agentless }
}"#,
        "typecase-publish",
    );
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: "typecase".into(),
            revision: None,
            workspace: ".".into(),
            requester: Some("person/alex".into()),
            mode: Some("run".into()),
            inputs: BTreeMap::new(),
            idempotency_key: "typecase-run".into(),
        })
        .unwrap();
    let request = AttentionRequest {
        reviewer: "person/alex".into(),
        title: "Another fault".into(),
        reason: "a person must decide".into(),
        severity: "warning".into(),
        targets: Vec::new(),
        actor: "agent/node.requester".into(),
        idempotency_key: "other-fault".into(),
    };
    store
        .request_attention("attention/other-fault", &request)
        .unwrap();
    let targets = [
        "mission/typecase",
        run.subject.as_str(),
        "attention/other-fault",
        "resource/typecase/kdl",
        "agent/node.stopped",
    ]
    .map(str::to_owned);
    let states = |store: &Store| {
        store
            .attention_target_states(&targets)
            .unwrap()
            .into_iter()
            .map(|state| (state.id, state.state))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        states(&store),
        [
            ("mission/typecase".into(), "running".into()),
            (run.subject.clone(), "running".into()),
            ("attention/other-fault".into(), "open".into()),
        ]
    );

    store
        .set_mission_run_state(&run.id, "cancelled", "terminal", Some("moved to a seat"))
        .unwrap();
    store
        .withdraw_attention(
            "attention/other-fault",
            &AttentionWithdrawRequest {
                reason: "cleared".into(),
                actor: "agent/node.requester".into(),
                idempotency_key: "other-fault-withdrawn".into(),
            },
        )
        .unwrap();
    let source = "version 2\nstop \"agent/node.stopped\"\n";
    let intent = crate::graph::parse_intent(source, "node").unwrap();
    let planned = store
        .mission(
            &intent,
            IntentInput {
                kdl: source.into(),
                source_name: None,
            },
        )
        .unwrap();
    store
        .apply(&intent, &planned.subject_tokens, "stopped-agent")
        .unwrap();

    assert_eq!(
        states(&store),
        [
            ("mission/typecase".into(), "cancelled".into()),
            (run.subject.clone(), "cancelled".into()),
            ("attention/other-fault".into(), "withdrawn".into()),
            ("agent/node.stopped".into(), "stopped".into()),
        ]
    );
    assert!(
        store
            .attention_target_states(&targets)
            .unwrap()
            .iter()
            .all(|state| state.since_unix_ms.is_some())
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

/// A call Claude's auto mode classifier denied waits on the seat's person: an alert until they
/// answer it, and the answer (allow once, allow its kind, or dismiss) is what the seat's
/// `PreToolUse` hook reads; the seat is told, so it runs the call again or leaves it.
#[test]
fn a_classifier_denial_is_an_alert_whose_answer_the_seat_hears() {
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
                "reason":"Irreversible Local Destruction", "driver":"claude",
                "session_id":"session-1", "matched_line": json!({"call_id": call}).to_string()}),
            Some(&format!("claude-permission-denied:{seat}:{call}")),
        )
    };
    let alerts = || {
        store
            .attention_items(Some("person/avery"))
            .unwrap()
            .into_iter()
            .filter(|item| item.kind == "harness-prompt")
            .collect::<Vec<_>>()
    };
    append("runtime.observed", json!({"status":"running", "incarnation_id":"one"}), None);
    let first = denied("toolu_1");
    let shown = alerts();
    assert_eq!(shown.len(), 1, "{shown:?}");
    assert_eq!(shown[0].episode, first.id);
    assert_eq!(shown[0].answers, vec!["approve-once", "approve-kind", "dismiss"]);
    assert!(shown[0].title.ends_with("needs you to allow a Bash call"));
    assert!(shown[0].detail.contains("(Irreversible Local Destruction)"), "{}", shown[0].detail);
    assert_eq!(
        store.answer_native_prompt(seat, &first.id, "allow", "person/avery").unwrap_err().code,
        "invalid-answer"
    );
    assert_eq!(
        store.answer_native_prompt(seat, &first.id, "approve-once", "person/other").unwrap_err().code,
        "forbidden"
    );
    assert_eq!(store.prompt_answer(&first.id).unwrap().answer, None);
    store.answer_native_prompt(seat, &first.id, "approve-once", "person/avery").unwrap();
    assert_eq!(store.prompt_answer(&first.id).unwrap().answer.as_deref(), Some("approve-once"));
    assert!(alerts().is_empty(), "an answered denial is no longer an alert");
    // The seat hears it, from st, and runs the call again.
    let told = store
        .claims_for(&format!(
            "message/{}",
            &hex::encode(sha2::Sha256::digest(format!("st3-denial-answer:{}", first.id).as_bytes()))[..16]
        ), Some("message.sent"))
        .unwrap();
    assert_eq!(told.len(), 1);
    assert_eq!(told[0].body["fields"]["to"], seat);
    assert!(told[0].body["fields"]["content"].as_str().unwrap().contains("Run that same call again"));
    assert_eq!(
        store.answer_native_prompt(seat, &first.id, "dismiss", "person/avery").unwrap_err().code,
        "stale-fence",
        "an answer is final"
    );
    // A denial from before the seat restarted waits on nobody.
    let old = denied("toolu_2");
    assert_eq!(alerts().len(), 1);
    append("runtime.observed", json!({"status":"exited", "incarnation_id":"one"}), None);
    append("runtime.observed", json!({"status":"running", "incarnation_id":"two"}), None);
    assert!(alerts().is_empty());
    assert_eq!(
        store.answer_native_prompt(seat, &old.id, "approve-kind", "person/avery").unwrap_err().code,
        "stale-fence"
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
            /* sqlite-large-value: allow-for-now lv-h2fe4c158856d owner=st3_attention::store::tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET accepted_at_unix_ms=CAST(1000000-(store_index/8) AS TEXT)
                        WHERE kind='harness.usage'",
            [],
        )
        .unwrap();
        tx.execute(
            /* sqlite-large-value: allow-for-now lv-had92f7501d26 owner=st3_attention::store::tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE batches SET replica_sequence=10000-(SELECT MIN(store_index)
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
        /* sqlite-large-value: allow-for-now lv-hfac7d4bbec24 owner=st3_attention::store::tests TODO=chunk-or-reference-variable-columns */ "CREATE TABLE batches(id TEXT PRIMARY KEY, origin TEXT, replica_sequence INTEGER);
             CREATE TABLE claims(id TEXT PRIMARY KEY, subject TEXT, kind TEXT, batch_id TEXT,
                                 store_index INTEGER, accepted_at_unix_ms TEXT);
             CREATE INDEX claims_subject_accepted_index ON claims
                 (subject, length(accepted_at_unix_ms), accepted_at_unix_ms);
             CREATE TABLE replica_records(claim_id TEXT, position INTEGER);
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
            /* sqlite-large-value: allow-for-now lv-hf9d85f3d0181 owner=st3_attention::store::tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO claims VALUES ('claim-{index:06}','agent/target','runtime.observed','{batch}',{},'{time}');
                 INSERT INTO replica_records VALUES ('claim-{index:06}',{});",
            index + 1,
            (index % 5) as u64
        ));
    }
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-hd48cf270c3e3 owner=st3_attention::store::tests TODO=trace-statement-and-value-representation */ &fixture).unwrap();
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
/// `CLAIM_SUMMARY_BENCH=/path/to/copy.sqlite3 cargo test -p st3-daemon --lib claim_summary -- --ignored --nocapture`.
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

mod activity_purity_tests {
    use super::*;

    /* sqlite-large-value: allow-for-now lv-hdf1ccf4245aa owner=st3_attention::store::tests TODO=trace-statement-and-value-representation */ include!("activity_purity_tests.rs");
}

mod actual_purity_tests {
    use super::*;

    /* sqlite-large-value: allow-for-now lv-hdf93049bf710 owner=st3_attention::store::tests TODO=trace-statement-and-value-representation */ include!("actual_purity_tests.rs");
}

mod fault_history_tests {
    use super::*;

    /* sqlite-large-value: allow-for-now lv-h8861ce10ba48 owner=st3_attention::store::tests TODO=trace-statement-and-value-representation */ include!("fault_history_tests.rs");
}

mod mission_deadline_tests {
    use super::*;

    /* sqlite-large-value: allow-for-now lv-hed656ca3663d owner=st3_attention::store::tests TODO=trace-statement-and-value-representation */ include!("mission_deadline_tests.rs");
}

mod projection_divergence_tests {
    use super::*;

    /* sqlite-large-value: allow-for-now lv-h5c3f1237e5d8 owner=st3_attention::store::tests TODO=trace-statement-and-value-representation */ include!("projection_divergence_tests.rs");
}

mod stopped_seat_declaration_controls {
    use super::*;

    /* sqlite-large-value: allow-for-now lv-he7c17e391229 owner=st3_attention::store::tests TODO=trace-statement-and-value-representation */ include!("stopped_seat_declaration_controls.rs");
}
