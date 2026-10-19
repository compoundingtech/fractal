#![allow(unused_imports)]
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::tests_shared::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::tests_shared::{TEST_FLEET, exchange_from, receive_and_project};
use crate::SeatStatusStore;
use super::*;
use crate::graph::parse_test_intent as parse_intent;
use proptest::prelude::*;
use st3_store::store::shared_seats::*;
use st3_store::store::shared_seats::{BASE_REBUILDS, DEFINITIONS_PARSED, RUN_PART_RUNS, STATUS_AFTER_DESIRED_READ, STEPS_ENRICHED, SUBJECT_REDUCTIONS};
use st3_store::store::tests_shared_seats::*;
use st3_store::store::tests_shared_seats::{TAKEOVER_SOURCE, TREE_SHAPE_RUNS, TreeShapes, expire_work_lease_by_claim, failed_takeover_run, full_canonical_claim_id_oracle, graph_digest_of, insert_raw_claims, late_claim_matches_a_replay, local_observation_for_test};
use st3_store::store::tests_shared_seats::{local_timeline_observation_for_test, observe_runtime, projection_replayed, publish_mission, publish_takeover, query_plan, replicate_fully, replicated_run_tree, roster_cache_store, simple};
use st3_store::store::tests_shared_seats::{takeover_step, timeline_observation};


#[test]
fn an_agent_card_build_does_not_hold_other_read_snapshots_at_the_cache_lock() {
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(&directory.path().join("graph.db"), "node").unwrap());
    store
        .cached_agent_resources(0, true, |_| Ok(vec![json!({"id": "agent/cached"})]))
        .unwrap();
    let (entered, building) = std::sync::mpsc::channel();
    let (release, released) = std::sync::mpsc::channel();
    let slow = store.clone();
    let builder = std::thread::spawn(move || {
        slow.read_snapshot(|_| {
            slow.cached_agent_resources(1, false, |_| {
                entered.send(()).unwrap();
                released.recv().unwrap();
                Ok(vec![json!({"id": "agent/slow"})])
            })
        })
        .unwrap()
    });
    building.recv().unwrap();
    let (finished, result) = std::sync::mpsc::channel();
    let reader = store.clone();
    let other = std::thread::spawn(move || {
        finished
            .send(reader.read_snapshot(|_| {
                reader.cached_agent_resources(0, true, |_| panic!("cached snapshot was lost"))
            }))
            .unwrap();
    });
    let completed = result.recv_timeout(std::time::Duration::from_secs(1));
    let published = completed.as_ref().ok().map(|_| {
        store
            .cached_agent_resources(1, false, |_| Ok(vec![json!({"id": "agent/published"})]))
            .unwrap()
    });
    release.send(()).unwrap();
    let resumed = builder.join().unwrap();
    other.join().unwrap();
    assert_eq!(completed.unwrap().unwrap()[0]["id"], "agent/cached");
    assert_eq!(resumed, published.unwrap());
}

/// A heartbeat moves no card, so heartbeat-only local deltas keep pinning the same roster
/// rows; an agent timeline row is the local delta a card reads.
#[test]
fn heartbeat_local_deltas_pin_the_same_roster_rows() {
    let store = roster_cache_store();
    let index = store.index().unwrap();
    let card = json!({"id": "agent/node.amber", "name": "amber"});
    store
        .cached_agent_resources(index, false, |_| Ok(vec![card.clone()]))
        .unwrap();
    let pinned = store
        .agent_resources_cached_at(index, false, None)
        .unwrap()
        .unwrap();
    for n in 0..8 {
        let response = store.append_claim(&local_observation_for_test(
            "harness.observed",
            json!({"state":"idle", "driver":"codex", "incarnation_id":"amber-1",
                "observed_at_ms":300_001 + n}),
        )).unwrap();
        assert!(local_observation_position(&response).is_some());
        assert_eq!(
            store.index().unwrap(),
            index,
            "a local observation keeps the graph cut"
        );
        let hit = store
            .agent_resources_cached_at(index, false, None)
            .unwrap()
            .expect("a heartbeat moves no card, so the pinned rows still answer");
        assert!(Arc::ptr_eq(&hit, &pinned), "heartbeat deltas must reuse the roster rows");
    }
    assert_eq!(store.agent_resources_builds_for_test(), 1);
    store.append_local_observations_for_test(&[local_timeline_observation_for_test()]);
    assert_eq!(store.index().unwrap(), index);
    assert!(
        store
            .agent_resources_cached_at(index, false, None)
            .unwrap()
            .is_none(),
        "an agent timeline row is the local delta a card reads",
    );
    store
        .cached_agent_resources(index, false, |_| Ok(vec![card]))
        .unwrap();
    assert!(
        store
            .agent_resources_cached_at(index, false, None)
            .unwrap()
            .is_some()
    );
    assert_eq!(store.agent_resources_builds_for_test(), 2);
}

/// A page that already covered every agent at the cut upgrades to the complete window by
/// reusing the pinned rows: no clone, no sort, no build.
#[test]
fn a_covered_page_upgrade_reuses_the_pinned_roster_rows() {
    let store = roster_cache_store();
    let index = store.index().unwrap();
    let selected = BTreeSet::from(["agent/node.amber".to_string()]);
    store
        .cached_agent_resources_for(index, false, Some(&selected), |_| {
            Ok(vec![json!({"id": "agent/node.amber", "name": "amber"})])
        })
        .unwrap();
    let partial = store
        .agent_resources_cached_at(index, false, Some(&selected))
        .unwrap()
        .unwrap();
    store
        .cached_agent_resources_for(index, false, None, |changed| {
            panic!("nothing moved and nothing is uncovered, so no card is built: {changed:?}")
        })
        .unwrap();
    let complete = store
        .agent_resources_cached_at(index, false, None)
        .unwrap()
        .unwrap();
    assert!(Arc::ptr_eq(&complete, &partial));
    assert_eq!(store.agent_resources_builds_for_test(), 1);
}

/// An exact agent selector folds its own card: the complete roster at the same cut neither
/// answers it nor loses its place in the roster cache to it.
#[test]
fn exact_agent_cards_neither_borrow_nor_evict_the_fleet_roster() {
    let store = roster_cache_store();
    let index = store.index().unwrap();
    store
        .cached_agent_resources(index, false, |_| Ok(vec![
            json!({"id": "agent/node.amber", "name": "amber", "_fleet": true}),
            json!({"id": "agent/node.cobalt", "name": "cobalt", "_fleet": true}),
        ]))
        .unwrap();
    let fleet = store.agent_resources_cached_at(index, false, None).unwrap().unwrap();
    assert!(store.exact_agent_resources_cached_at(index, false, "agent/node.amber").unwrap().is_none(),
        "a fleet publication must not answer an exact agent read");
    let card = |agent: &str| json!({"id": agent, "name": agent});
    let amber = store
        .cached_exact_agent_resources(index, false, "agent/node.amber", |changed| {
            let (subjects, previous) = changed.expect("an exact read names its agent");
            assert_eq!(subjects, &BTreeSet::from(["agent/node.amber".to_owned()]));
            assert!(previous.is_empty(), "no fleet rows are borrowed as the delta source");
            Ok(vec![card("agent/node.amber")])
        })
        .unwrap();
    assert_eq!(amber, [card("agent/node.amber")]);
    store
        .cached_exact_agent_resources(index, false, "agent/node.amber", |changed| {
            panic!("the exact card at this cut is already folded: {changed:?}")
        })
        .unwrap();
    for n in 0..super::seats::EXACT_AGENT_RESOURCES_CACHE + 1 {
        let agent = format!("agent/node.followed-{n}");
        store.cached_exact_agent_resources(index, false, &agent, |_| Ok(vec![card(&agent)])).unwrap();
    }
    assert!(store.exact_agent_resources_cached_at(index, false, "agent/node.amber").unwrap().is_none(),
        "the least recently folded agent leaves the exact cache first");
    let still = store.agent_resources_cached_at(index, false, None).unwrap()
        .expect("exact reads never evict the complete roster");
    assert!(Arc::ptr_eq(&still, &fleet));
    assert_eq!(store.agent_roster_valid_until(index, Some("agent/node.amber")), None);
}

/// The frontier answers an old cut exactly — its newest agent timeline row — from the
/// partial frontier index, with VM work that newer heartbeats never grow; the pre-followup
/// lookup read every newer local row of every kind. Focused fixture: run it explicitly and
/// alone, with --ignored --nocapture, because its counters are process-wide.
#[test]
#[ignore = "focused roster frontier fixture; run explicitly with --ignored --nocapture"]
fn the_roster_frontier_old_cut_seek_skips_newer_heartbeats() {
    let store = roster_cache_store();
    let cut = store.index().unwrap();
    store
        .append_local_observations_for_test(&[local_timeline_observation_for_test()]);
    let relevant_id = store
        .readers
        .get()
        .query_row(
            "SELECT id FROM local_observations ORDER BY id DESC LIMIT 1",
            [],
            |row| row.get::<_, u64>(0),
        )
        .unwrap();

    let later = insert_raw_claims(&store, &[("host/one", "transport.observed")]);
    let insert_newer = |heartbeats: usize, irrelevant: usize, usage: usize| {
        let connection = store.connection.lock().unwrap();
        let mut statement = connection
            .prepare(
                /* sqlite-large-value: allow-for-now lv-h2b512bbccd21 owner=st3_seats::store::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO local_observations(after_store_index, subject, kind, body,
                                                        observed_at_unix_ms)
                     VALUES (?1, ?2, ?3, '{}', 1)",
            )
            .unwrap();
        for _ in 0..heartbeats {
            statement
                .execute(/* sqlite-large-value: allow-for-now lv-hdcae1914f7a5 owner=st3_seats::store::tests TODO=trace-statement-and-value-representation */ params![later, "agent/node.amber", "harness.observed"])
                .unwrap();
        }
        for _ in 0..irrelevant {
            statement
                .execute(/* sqlite-large-value: allow-for-now lv-h47319cca04cb owner=st3_seats::store::tests TODO=trace-statement-and-value-representation */ params![later, "mission-run/x", "harness.timeline"])
                .unwrap();
        }
        for _ in 0..usage {
            statement
                .execute(/* sqlite-large-value: allow-for-now lv-h26b14a3937fb owner=st3_seats::store::tests TODO=trace-statement-and-value-representation */ params![later, "agent/node.amber", "harness.timeline"])
                .unwrap();
        }
    };
    insert_newer(4_000, 500, 200);

    let frontier = |at: u64| {
        store
            .readers
            .get()
            .query_row(/* sqlite-large-value: allow-for-now lv-h9778b49bdc80 owner=st3_seats::store::tests TODO=trace-statement-and-value-representation */ ROSTER_LOCAL_FRONTIER, [at], |row| row.get::<_, u64>(0))
            .unwrap()
    };
    assert_eq!(
        frontier(cut),
        relevant_id,
        "an old cut sees exactly its newest agent timeline row"
    );
    let usage_frontier = store
        .readers
        .get()
        .query_row(
            "SELECT id FROM local_observations WHERE subject='agent/node.amber'
                 AND kind='harness.timeline' ORDER BY id DESC LIMIT 1",
            [],
            |row| row.get::<_, u64>(0),
        )
        .unwrap();
    assert_eq!(
        frontier(later),
        usage_frontier,
        "the later cut sees the newest agent timeline row, of any entry type"
    );

    let plan = query_plan(&store, ROSTER_LOCAL_FRONTIER, params![cut]);
    assert!(
        plan.iter()
            .any(|step| step.contains("local_observations_roster_frontier_index")),
        "{plan:?}"
    );
    assert!(plan.iter().all(|step| !step.contains("TEMP B-TREE")), "{plan:?}");

    let pre_followup = "SELECT COALESCE((SELECT id FROM local_observations
             WHERE after_store_index<=?1 ORDER BY id DESC LIMIT 1), 0)";
    let vm_steps = |sql: &str, at: u64| {
        let connection = store.connection.lock().unwrap();
        let _: u64 = connection.query_row(/* sqlite-large-value: allow-for-now lv-h97c86c176dd3 owner=st3_seats::store::tests TODO=trace-statement-and-value-representation */ sql, [at], |row| row.get(0)).unwrap();
        let before = smallclaims::sqlite::work::total().vm_steps;
        let _: u64 = connection.query_row(/* sqlite-large-value: allow-for-now lv-hb9c62bf00f59 owner=st3_seats::store::tests TODO=trace-statement-and-value-representation */ sql, [at], |row| row.get(0)).unwrap();
        drop(connection);
        smallclaims::sqlite::work::total().vm_steps - before
    };
    let old_before = vm_steps(pre_followup, cut);
    let new_before = vm_steps(ROSTER_LOCAL_FRONTIER, cut);
    insert_newer(4_000, 0, 0);
    let old_after = vm_steps(pre_followup, cut);
    let new_after = vm_steps(ROSTER_LOCAL_FRONTIER, cut);
    println!(
        "roster frontier fixture: heartbeats=4000..8000 irrelevant_timeline=500 \
             agent_usage_timeline=200 vm_old_cut_before={old_before} \
             vm_new_cut_before={new_before} vm_old_cut_after={old_after} \
             vm_new_cut_after={new_after}"
    );
    assert_eq!(
        new_after, new_before,
        "the frontier seek's work cannot grow with newer heartbeat rows"
    );
    assert!(
        old_after > old_before,
        "the pre-followup lookup read every newer row of every kind"
    );
    assert!(
        new_before < old_before,
        "the frontier index bounds the old-cut lookup"
    );
}

#[test]
fn terminal_capability_lookup_fences_the_subject_head_after_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("graph.db");
    let store = Store::open(&path, "node").unwrap();
    let append = |subject: &str, kind: &str| {
        store.append_claim(&ClaimInput {
            subject: subject.into(),
            kind: kind.into(),
            actor: None,
            fields: BTreeMap::from([("capability_hash".into(), json!("secret-digest"))]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        }).unwrap()
    };
    let attached = append("custom/client/attachment-a", "custom.client.terminal-attached");
    append("custom/client/decoy", "custom.client.other");
    assert_eq!(
        store.terminal_attachment_for_capability_hash("secret-digest").unwrap()
            .map(|(claim, current)| (claim.id, current)),
        Some((attached.id.clone(), true)),
    );
    assert!(store.terminal_attachment_for_capability_hash("unknown").unwrap().is_none());
    append("custom/client/attachment-a", "custom.client.terminal-consumed");
    drop(store);
    let reopened = Store::open(&path, "node").unwrap();
    assert_eq!(
        reopened.terminal_attachment_for_capability_hash("secret-digest").unwrap()
            .map(|(claim, current)| (claim.id, current)),
        Some((attached.id, false)),
    );
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(16))]

    // A store refilled from a peer can admit an attachment after the claim that consumed it.
    // The capability's head is the canonical newest claim on its subject, never the one this
    // store admitted last, so a consumed or detached capability cannot become current again.
    #[test]
    fn terminal_capability_head_is_pure_under_shuffled_split_and_duplicate_delivery(
        ranks in proptest::collection::vec(any::<u64>(), 8),
        width in 1usize..4,
        copies in 1usize..3,
    ) {
        let source = Store::open_memory("alder").unwrap();
        let append = |subject: &str, kind: &str, hash: &str| {
            source.append_claim(&ClaimInput {
                subject: subject.into(),
                kind: kind.into(),
                actor: None,
                fields: BTreeMap::from([("capability_hash".into(), json!(hash))]),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: None,
            }).unwrap()
        };
        let consumed = append("custom/client/attachment-a", "custom.client.terminal-attached", "consumed-digest");
        append("custom/client/attachment-a", "custom.client.terminal-consumed", "consumed-digest");
        let live = append("custom/client/attachment-b", "custom.client.terminal-attached", "live-digest");
        let exchange = exchange_from(&source, &ReplicationInventory::default());
        prop_assert!(exchange.envelopes.len() <= ranks.len());
        let mut order = (0..exchange.envelopes.len()).collect::<Vec<_>>();
        order.sort_by_key(|&index| (ranks[index], index));
        let target = Store::open_memory("birch").unwrap();
        for chunk in order.chunks(width) {
            let envelopes = chunk.iter().flat_map(|&index|
                std::iter::repeat_n(exchange.envelopes[index].clone(), copies)).collect();
            let admission = receive_and_project(&target, "alder", &exchange_of("alder", envelopes));
            prop_assert_eq!((admission.invalid, admission.unknown), (0, 0));
        }
        let head = |hash: &str| target.terminal_attachment_for_capability_hash(hash).unwrap()
            .map(|(claim, current)| (claim.id, current));
        prop_assert_eq!(head("consumed-digest"), Some((consumed.id, false)));
        prop_assert_eq!(head("live-digest"), Some((live.id, true)));
    }
}

#[test]
fn seat_rename_persists_across_a_real_store_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.sqlite3");
    let original = {
        let store = Store::open(&path, "node").unwrap();
        let intent = parse_intent(r#"version 2
agent "test/worker" { workspace "."; command "true"; name "Initial" }
"#, "node").unwrap();
        let preview = store.mission(&intent, IntentInput {
            kdl: String::new(), source_name: None,
        }).unwrap();
        store.apply_as(&intent, &preview.subject_tokens, "initial", Some("person/operator")).unwrap();
        let original = store.desired_subject_with_writer("agent/test/worker").unwrap().unwrap();
        store.rename_agent("agent/test/worker", Some("Renamed"), "rename").unwrap();
        original
    };
    let store = Store::open(&path, "node").unwrap();
    let (mut renamed, writer) = store.desired_subject_with_writer("agent/test/worker").unwrap().unwrap();
    assert_eq!(writer, original.1);
    assert_eq!(renamed.member.as_ref().unwrap().display_name.as_deref(), Some("Renamed"));
    renamed.set_display_name(Some("Initial")).unwrap();
    assert_eq!(renamed, original.0);
    store.rename_agent("agent/test/worker", None, "clear").unwrap();
    drop(store);
    let store = Store::open(&path, "node").unwrap();
    let cleared = store.desired_subject_with_writer("agent/test/worker").unwrap().unwrap().0;
    assert_eq!(crate::model::effective_agent_name("agent/test/worker", Some(&cleared.desired)), "test/worker");
    assert_eq!(cleared.member.unwrap().display_name, None);
}

#[test]
fn a_rename_that_merges_concurrent_renames_keeps_the_launch_revision() {
    let left = Store::open_memory("left").unwrap();
    let right = Store::open_memory("right").unwrap();
    let intent = parse_intent(r#"version 2
agent "test/worker" { command "true"; name "A" }
"#, "left").unwrap();
    let preview = left.mission(&intent, IntentInput {
        kdl: String::new(), source_name: None,
    }).unwrap();
    left.apply(&intent, &preview.subject_tokens, "a").unwrap();
    let launch = left.selected_desired_token("agent/test/worker").unwrap().unwrap();
    receive_and_project(&right, "left", &exchange_from(&left, &ReplicationInventory::default()));
    // Two hosts relabel the same seat concurrently, then one relabels the merged fork.
    left.rename_agent("agent/test/worker", Some("R1"), "r1").unwrap();
    right.rename_agent("agent/test/worker", Some("R2"), "r2").unwrap();
    receive_and_project(&right, "left", &exchange_from(&left, &right.replication_inventory().unwrap()));
    receive_and_project(&left, "right", &exchange_from(&right, &left.replication_inventory().unwrap()));
    left.rename_agent("agent/test/worker", Some("R3"), "r3").unwrap();
    let merged = left.selected_desired_token("agent/test/worker").unwrap().unwrap();
    assert_eq!(left.claim_by_id(&merged).unwrap().unwrap().predecessors.len(), 2);
    let lineage = left.launch_lineage("agent/test/worker").unwrap();
    assert_eq!(lineage.first(), Some(&merged));
    assert_eq!(lineage.last(), Some(&launch), "{lineage:?}");
}

#[test]
fn a_suspension_lineage_snapshot_ignores_a_predecessor_replicated_after_it() {
    let left = Store::open_memory("left").unwrap();
    let right = Store::open_memory("right").unwrap();
    let reader = Store::open_memory("reader").unwrap();
    let intent = parse_intent(r#"version 2
agent "test/worker" { command "true"; name "A" }
"#, "left").unwrap();
    let preview = left.mission(&intent, IntentInput {
        kdl: String::new(), source_name: None,
    }).unwrap();
    left.apply(&intent, &preview.subject_tokens, "a").unwrap();
    let declared = left.selected_desired_token("agent/test/worker").unwrap().unwrap();
    receive_and_project(&right, "left", &exchange_from(&left, &ReplicationInventory::default()));
    // A relabel keeps the declared launch, so the declaration stays in its successor's lineage.
    right.rename_agent("agent/test/worker", Some("B"), "b").unwrap();
    let renamed = right.selected_desired_token("agent/test/worker").unwrap().unwrap();
    assert_eq!(
        right.suspension_lineage_at("agent/test/worker", None).unwrap(),
        vec![renamed.clone(), declared.clone()]
    );
    // The reader receives the relabel before the declaration it succeeds.
    receive_and_project(
        &reader, "right", &exchange_from(&right, &left.replication_inventory().unwrap()),
    );
    assert!(reader.claim_by_id(&declared).unwrap().is_none());
    let at = reader.index().unwrap();
    receive_and_project(&reader, "left", &exchange_from(&left, &reader.replication_inventory().unwrap()));
    assert!(reader.claim_by_id(&declared).unwrap().is_some());
    assert_eq!(
        reader.suspension_lineage_at("agent/test/worker", Some(at)).unwrap(),
        vec![renamed.clone()],
        "a snapshot walks only the ancestry it held"
    );
    assert_eq!(
        reader.suspension_lineage_at("agent/test/worker", None).unwrap(),
        vec![renamed, declared]
    );
}

#[test]
fn seat_rename_refuses_an_unreadable_member_and_an_empty_label() {
    let store = Store::open_memory("node").unwrap();
    let intent = parse_intent(r#"version 2
agent "test/worker" { command "true"; name "Initial" }
"#, "node").unwrap();
    let preview = store.mission(&intent, IntentInput {
        kdl: String::new(), source_name: None,
    }).unwrap();
    store.apply(&intent, &preview.subject_tokens, "initial").unwrap();
    let before = store.selected_desired_token("agent/test/worker").unwrap();
    assert_eq!(
        store.rename_agent("agent/test/worker", Some(""), "empty").unwrap_err().code,
        "invalid-agent-name"
    );
    // A launch published by a newer peer stays until a build that can read it takes it up.
    store.replace_desired_member_for_test("agent/test/worker", r#"{"future":true}"#);
    assert_eq!(
        store.rename_agent("agent/test/worker", Some("Renamed"), "unreadable").unwrap_err().code,
        "unreadable-agent-member"
    );
    assert_eq!(store.selected_desired_token("agent/test/worker").unwrap(), before);
    let connection = store.readers.get();
    let member: String = connection
        .query_row("SELECT member FROM desired WHERE subject='agent/test/worker'", [], |row| row.get(0))
        .unwrap();
    assert_eq!(member, r#"{"future":true}"#);
}

#[test]
fn seat_rename_accepts_an_empty_body_and_rejects_a_non_array_body() {
    let store = Store::open_memory("node").unwrap();
    let mut intent = parse_intent(r#"version 2
agent "test/empty" { command "true" }
"#, "node").unwrap();
    // An empty canonical KDL body has no children key. Exercise that stored shape
    // independently of launch validation, which requires a command for new seats.
    let empty = intent.subjects.get_mut("agent/test/empty").unwrap();
    empty.desired.as_object_mut().unwrap().remove("children");
    empty.member = None;
    let preview = store.mission(&intent, IntentInput {
        kdl: String::new(), source_name: None,
    }).unwrap();
    store.apply(&intent, &preview.subject_tokens, "empty").unwrap();
    store.rename_agent("agent/test/empty", Some("Empty seat"), "rename-empty").unwrap();
    let mut desired = store.desired_subject_with_writer("agent/test/empty").unwrap().unwrap().0;
    assert_eq!(crate::model::effective_agent_name("agent/test/empty", Some(&desired.desired)), "Empty seat");
    desired.desired["children"] = json!({});
    assert_eq!(desired.set_display_name(Some("Invalid")).unwrap_err().code, "invalid-agent-declaration");
}

#[test]
fn eval_cleanup_includes_descendant_mission_runtimes() {
    let store = Store::open_memory("node").unwrap();
    let source = r#"
version 2

mission "eval/root" state="ready" timeout="1m" {
  goal "Own one isolated eval tree."
}

mission "eval/child" state="ready" {
  goal "Run one child agent."
}
"#;
    publish_mission(&store, source, "publish-eval-tree");
    let root = store
        .create_mission_run(&MissionRunRequest {
            mission: "eval/root".into(),
            revision: None,
            workspace: "/eval".into(),
            requester: Some("person/test".into()),
            mode: Some("eval".into()),
            inputs: BTreeMap::new(),
            idempotency_key: "eval-tree-root".into(),
        })
        .unwrap();
    let mut seat = crate::graph::parse_intent(
        r#"version 2
agent "eval/root/seat" { workspace "/eval"; command "true"; restart "never" }
"#,
        "node",
    )
    .unwrap();
    seat.subjects
        .get_mut("agent/eval/root/seat")
        .unwrap()
        .owner_run = Some(root.subject.clone());
    store
        .apply_internal(&seat, "declare-eval-root-seat")
        .unwrap();
    let child = store
        .create_child_mission_run(
            &MissionRunRequest {
                mission: "eval/child".into(),
                revision: None,
                workspace: "/eval/child".into(),
                requester: Some("daemon/test".into()),
                mode: Some("run".into()),
                inputs: BTreeMap::new(),
                idempotency_key: "eval-tree-child".into(),
            },
            &root,
            "step-run/subscription/root/reviews",
            None,
        )
        .unwrap();
    let declaration = crate::graph::parse_execution_intent(
        r#"version 2
agent "worker" { workspace "/eval/child"; command "true"; restart "never" }
"#,
        "node",
        &child.id,
    )
    .unwrap();
    store
        .apply_internal(&declaration, "declare-eval-child-worker")
        .unwrap();

    let runtime_records = store.eval_runtime_records(&root.subject).unwrap();
    assert!(runtime_records.contains(&(format!("{}.worker", child.id), true)));
    assert!(runtime_records.contains(&("eval.root.seat".into(), true)));
    assert!(
        store
            .desired_subjects()
            .unwrap()
            .iter()
            .any(|desired| desired.owner_run.as_deref() == Some(child.subject.as_str()))
    );

    assert!(
        store
            .retire_eval_owned_desired(&root.subject)
            .unwrap()
            .is_empty()
    );
    assert!(store.desired_subjects().unwrap().iter().all(|desired| {
        desired.owner_run.as_deref() != Some(root.subject.as_str())
            && desired.owner_run.as_deref() != Some(child.subject.as_str())
    }));
}

#[test]
fn a_declarative_resource_refresh_emits_schema_valid_observer_state() {
    let store = Store::open_memory("node").unwrap();
    let source = r#"version 2
resource "refresh/file" { kind "filesystem.file" }
observer "refresh/file" {
  resource "resource/refresh/file"
  provider "local.file"
  locator "/tmp/st3-refresh-test"
  field "status"
}
"#;
    let intent = crate::graph::parse_execution_intent(source, "node", "refresh-run").unwrap();
    let observer = intent
        .subjects
        .values()
        .find(|subject| subject.kind == "observer")
        .unwrap()
        .subject
        .clone();
    store
        .apply_internal(&intent, "declare-refresh-observer")
        .unwrap();

    let refresh = r#"version 2
resource "refresh/file" {
  refresh "manual" { timeout "1s" }
}
"#;
    let intent = crate::graph::parse_intent(refresh, "node").unwrap();
    let planned = store
        .mission(
            &intent,
            IntentInput {
                kdl: refresh.into(),
                source_name: None,
            },
        )
        .unwrap();
    store
        .apply_as(
            &intent,
            &planned.subject_tokens,
            "request-resource-refresh",
            Some("person/operator"),
        )
        .unwrap();

    let state = store
        .latest_claim(&observer, Some("observer.refresh-requested"))
        .unwrap()
        .unwrap();
    assert!(state.body["fields"]["attempt"].as_str().is_some());
}

#[test]
fn observer_completion_fences_roll_back_facts_state_and_idempotency() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    for failure in [false, true] {
        for (accepted_checks, stale_revision) in [(0, false), (1, false), (2, true)] {
            let store = Store::open_memory("node").unwrap();
            let intent = crate::graph::parse_execution_intent(r#"version 2
resource "fenced/file" { kind "filesystem.file" }
observer "fenced/file" {
  resource "resource/fenced/file"
  provider "local.file"
  locator "/tmp/st3-fenced-file"
  field "status"
}
"#, "node", "fenced-run").unwrap();
            store.apply_internal(&intent, "fenced-observer").unwrap();
            let observer = intent.subjects.values().find(|subject| subject.kind == "observer")
                .unwrap().subject.as_str();
            let resource = intent.subjects.values().find(|subject| subject.kind == "resource")
                .unwrap().subject.as_str();
            let revision = store.selected_desired_revision(observer).unwrap().unwrap();
            let selected = if stale_revision { "stale-revision" } else { &revision };
            let input = ClaimInput {
                subject: observer.into(), kind: "observer.state".into(), actor: None,
                fields: BTreeMap::from([
                    ("state".into(), json!("unreachable")),
                    ("revision".into(), json!(revision)),
                    ("attempt".into(), json!("fenced-attempt")),
                    ("reason".into(), json!("provider failed")),
                ]),
                evidence: Vec::new(), expected_subject: None,
                idempotency_key: Some("fenced-failure".into()),
            };
            let before = store.index().unwrap();
            let counts = || {
                let connection = store.readers.get();
                ["claims", "batches", "operations", "idempotency"].map(|table| {
                    connection.query_row(/* sqlite-large-value: allow-for-now lv-ha9307f584647 owner=st3_seats::store::tests TODO=trace-statement-and-value-representation */ &format!("SELECT COUNT(*) FROM {table}"), [], |row| row.get::<_, u64>(0)).unwrap()
                })
            };
            let before_counts = counts();
            let checks = AtomicUsize::new(0);
            let current = || checks.fetch_add(1, Ordering::SeqCst) < accepted_checks;
            let result = if failure {
                store.append_observer_state(&input, selected, &current).map(|_| ())
            } else {
                store.record_resource_observation(
                    observer, selected, Some("fenced-attempt"), resource, Some("cursor"),
                    &json!({"status": "ready"}), 10, &[], Some(&current),
                ).map(|_| ())
            };
            assert_eq!(result.unwrap_err().code, if stale_revision {
                "stale-observer-revision"
            } else {
                "observer-operation-retired"
            });
            assert_eq!(checks.load(Ordering::SeqCst), if stale_revision { 1 } else { accepted_checks + 1 });
            assert_eq!(store.index().unwrap(), before);
            assert_eq!(counts(), before_counts);
            assert!(store.latest_actual_value(resource).unwrap().is_none());
            assert!(store.latest_actual_value(observer).unwrap().is_none());

            // Rejection must not poison the request's idempotency or operation cache.
            if failure {
                let receipt = store.append_observer_state(&input, &revision, &|| true).unwrap();
                assert_eq!(store.latest_actual_value(observer).unwrap().unwrap()["state"], "unreachable");
                let retry = store.append_observer_state(&input, &revision, &|| true).unwrap();
                assert_eq!(retry.0.id, receipt.0.id);
                assert!(receipt.1);
                assert!(!retry.1);
            } else {
                let outcome = store.record_resource_observation(
                    observer, &revision, Some("fenced-attempt"), resource, Some("cursor"),
                    &json!({"status": "ready"}), 10, &[], Some(&|| true),
                ).unwrap();
                assert!(outcome.baseline);
                assert_eq!(store.latest_actual_value(resource).unwrap().unwrap()["facts"]["status"], "ready");
                assert_eq!(store.latest_actual_value(observer).unwrap().unwrap()["state"], "healthy");
            }
        }
    }
}

#[test]
fn unchanged_scheduled_observations_do_not_append_claims() {
    let store = Store::open_memory("node").unwrap();
    let source = r#"version 2
resource "quiet/file" { kind "filesystem.file" }
observer "quiet/file" {
  resource "resource/quiet/file"
  provider "local.file"
  locator "/tmp/st3-quiet-file"
  field "status"
}
"#;
    let intent = crate::graph::parse_execution_intent(source, "node", "quiet-run").unwrap();
    let observer = intent
        .subjects
        .values()
        .find(|subject| subject.kind == "observer")
        .unwrap()
        .subject
        .clone();
    let resource = intent
        .subjects
        .values()
        .find(|subject| subject.kind == "resource")
        .unwrap()
        .subject
        .clone();
    store.apply_internal(&intent, "quiet-observer").unwrap();
    let revision = store.selected_desired_revision(&observer).unwrap().unwrap();
    let facts = json!({"status": "ready"});
    let (_, wrote) = store.record_resource_observation_outcome(&observer, &revision, None, &resource, None, &facts, 1, &[], None)
        .unwrap();
    assert!(wrote);
    let (_, wrote) = store.record_resource_observation_outcome(&observer, &revision, None, &resource, None, &facts, 1, &[], None)
        .unwrap();
    assert!(!wrote, "an idempotency hit is not a durable change");
    let before = store.index().unwrap();

    let (outcome, wrote) = store.record_resource_observation_outcome(&observer, &revision, None, &resource, None, &facts, 2, &[], None)
        .unwrap();
    assert!(!wrote);

    assert_eq!(store.index().unwrap(), before);
    assert!(outcome.observation_claim.is_none());
    assert!(outcome.changed_fields.is_empty());
}

#[test]
fn manual_refresh_records_an_unchanged_receipt() {
    let store = Store::open_memory("node").unwrap();
    let source = r#"version 2
resource "manual/file" { kind "filesystem.file" }
observer "manual/file" {
  resource "resource/manual/file"
  provider "local.file"
  locator "/tmp/st3-manual-file"
  field "status"
}
"#;
    let intent = crate::graph::parse_execution_intent(source, "node", "manual-run").unwrap();
    let observer = intent
        .subjects
        .values()
        .find(|subject| subject.kind == "observer")
        .unwrap()
        .subject
        .clone();
    let resource = intent
        .subjects
        .values()
        .find(|subject| subject.kind == "resource")
        .unwrap()
        .subject
        .clone();
    store.apply_internal(&intent, "manual-observer").unwrap();
    let revision = store.selected_desired_revision(&observer).unwrap().unwrap();
    let facts = json!({"status": "ready"});
    store.record_resource_observation(&observer, &revision, None, &resource, None, &facts, 1, &[], None)
        .unwrap();

    store.record_resource_observation(&observer, &revision, Some("attempt-one"), &resource, Some("cursor-one"), &facts, 2, &[], None)
        .unwrap();

    let receipt = store
        .latest_claim(&observer, Some("observer.observed"))
        .unwrap()
        .unwrap();
    assert_eq!(receipt.body["fields"]["attempt"], "attempt-one");
    assert_eq!(receipt.body["fields"]["changed"], false);
}

#[test]
fn observer_refresh_requests_complete_in_order() {
    let store = Store::open_memory("node").unwrap();
    let source = r#"version 2
resource "ordered/file" { kind "filesystem.file" }
observer "ordered/file" {
  resource "resource/ordered/file"
  provider "local.file"
  locator "/tmp/st3-ordered-file"
  field "status"
}
"#;
    let intent = crate::graph::parse_execution_intent(source, "node", "ordered-run").unwrap();
    let observer = intent
        .subjects
        .values()
        .find(|subject| subject.kind == "observer")
        .unwrap()
        .subject
        .clone();
    let resource = intent
        .subjects
        .values()
        .find(|subject| subject.kind == "resource")
        .unwrap()
        .subject
        .clone();
    store.apply_internal(&intent, "ordered-observer").unwrap();
    let revision = store.selected_desired_revision(&observer).unwrap().unwrap();
    for attempt in ["attempt-one", "attempt-two"] {
        store
            .append_claim(&ClaimInput {
                subject: observer.clone(),
                kind: "observer.refresh-requested".into(),
                actor: None,
                fields: BTreeMap::from([
                    ("attempt".into(), Value::String(attempt.into())),
                    ("revision".into(), Value::String(revision.clone())),
                ]),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
    }
    assert_eq!(
        store.pending_observer_refresh_attempt(&observer).unwrap(),
        Some("attempt-one".into())
    );

    store.record_resource_observation(&observer, &revision, Some("attempt-one"), &resource, None, &json!({"status": "ready"}), 1, &[], None)
        .unwrap();
    assert_eq!(
        store.pending_observer_refresh_attempt(&observer).unwrap(),
        Some("attempt-two".into())
    );
    store
        .append_claim(&ClaimInput {
            subject: observer.clone(),
            kind: "observer.state".into(),
            actor: None,
            fields: BTreeMap::from([
                ("attempt".into(), Value::String("attempt-two".into())),
                (
                    "reason".into(),
                    Value::String("provider unavailable".into()),
                ),
                ("revision".into(), Value::String(revision)),
                ("state".into(), Value::String("unreachable".into())),
            ]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
    assert_eq!(
        store.pending_observer_refresh_attempt(&observer).unwrap(),
        None
    );
}

#[test]
fn repeated_agent_snapshot_does_not_wait_for_busy_read_connections() {
    let store = Arc::new(Store::open_memory("node").unwrap());
    let snapshot = store.index().unwrap();
    store
        .status_for_subject_prefix_at(
            "agent/",
            Some(snapshot),
            true,
            ClaimsReduction::Summary,
        )
        .unwrap();

    let (ready_send, ready_recv) = std::sync::mpsc::channel();
    let (release_send, release_recv) = std::sync::mpsc::channel();
    let holder = store.clone();
    let held = std::thread::spawn(move || {
        holder.hold_read_connections_for_test(|| {
            ready_send.send(()).unwrap();
            release_recv.recv().unwrap();
        });
    });
    ready_recv.recv().unwrap();
    let reader = store.clone();
    let (result_send, result_recv) = std::sync::mpsc::channel();
    let read = std::thread::spawn(move || {
        result_send
            .send(reader.status_for_subject_prefix_at(
                "agent/",
                Some(snapshot),
                true,
                ClaimsReduction::Summary,
            ))
            .unwrap();
    });
    let result = result_recv.recv_timeout(std::time::Duration::from_millis(250));
    release_send.send(()).unwrap();
    held.join().unwrap();
    read.join().unwrap();
    assert_eq!(result.unwrap().unwrap().store_index, snapshot);
}

#[test]
fn bounded_status_reductions_select_only_relevant_subjects() {
    let store = Store::open_memory("node").unwrap();
    let observe = |subject: &str, runtime_id: &str| {
        store
            .append_claim(&ClaimInput {
                subject: subject.into(),
                kind: "runtime.observed".into(),
                actor: None,
                fields: BTreeMap::from([
                    ("runtime_id".into(), Value::String(runtime_id.into())),
                    ("status".into(), Value::String("running".into())),
                ]),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap()
            .store_index
    };
    let first_index = observe("agent/selected", "selected");
    observe("exec/also-runtime", "also-runtime");
    store
        .append_claim(&ClaimInput {
            subject: "host/unrelated".into(),
            kind: "transport.observed".into(),
            actor: None,
            fields: BTreeMap::from([
                ("protocol".into(), Value::String("test".into())),
                ("status".into(), Value::String("up".into())),
            ]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();

    let agents = store
        .status_for_subject_prefix_at("agent/", None, true, ClaimsReduction::Summary)
        .unwrap();
    assert_eq!(
        agents
            .subjects
            .iter()
            .map(|subject| subject.subject.as_str())
            .collect::<Vec<_>>(),
        ["agent/selected"]
    );

    let runtimes = store
        .status_for_claim_kind_at("runtime.observed", None, true, ClaimsReduction::Summary)
        .unwrap();
    assert_eq!(
        runtimes
            .subjects
            .iter()
            .map(|subject| subject.subject.as_str())
            .collect::<Vec<_>>(),
        ["agent/selected", "exec/also-runtime"]
    );

    let first_snapshot = store
        .status_for_claim_kind_at(
            "runtime.observed",
            Some(first_index),
            true,
            ClaimsReduction::Summary,
        )
        .unwrap();
    assert_eq!(first_snapshot.store_index, first_index);
    assert_eq!(first_snapshot.subjects.len(), 1);
    assert_eq!(first_snapshot.subjects[0].subject, "agent/selected");

    observe("agent/new", "new");
    let current_agents = store
        .status_for_subject_prefix_at("agent/", None, true, ClaimsReduction::Summary)
        .unwrap();
    assert_eq!(current_agents.subjects.len(), 2);
    let old_agents = store
        .status_for_subject_prefix_at(
            "agent/",
            Some(first_index),
            true,
            ClaimsReduction::Summary,
        )
        .unwrap();
    assert_eq!(old_agents.subjects.len(), 1);

    for number in 0..80 {
        observe(&format!("agent/bulk-{number:02}"), "bulk");
    }
    let bulk = store
        .status_for_subject_prefix_at("agent/", None, true, ClaimsReduction::Summary)
        .unwrap();
    assert_eq!(bulk.subjects.len(), 82);
    assert!(
        bulk.subjects
            .windows(2)
            .all(|pair| pair[0].subject < pair[1].subject)
    );
    assert_eq!(
        store
            .status_for_subject_prefix_at(
                "agent/",
                Some(first_index),
                true,
                ClaimsReduction::Summary,
            )
            .unwrap()
            .subjects
            .len(),
        1
    );
}

#[test]
fn agent_card_and_full_status_caches_never_exchange_harness_or_provenance() {
    let store = Store::open_memory("node").unwrap();
    let subject = "agent/cache/modes";
    observe_runtime(&store, subject, "running", "one");
    store.append_claim(&ClaimInput {
        subject: subject.into(), kind: "harness.observed".into(), actor: Some(subject.into()),
        fields: BTreeMap::from([
            ("state".into(), json!("idle")), ("incarnation_id".into(), json!("one")),
            ("driver".into(), json!("claude")),
        ]), evidence: Vec::new(), expected_subject: None, idempotency_key: None,
    }).unwrap();
    let cut = store.index().unwrap();
    let card = store.agent_card_status_at(None, cut, true).unwrap();
    assert!(card.subjects[0].harness.is_none());
    assert_eq!(card.subjects[0].claims.len(), 1);
    let full = store.status_for_subject_prefix_at("agent/", Some(cut), true, ClaimsReduction::Full).unwrap();
    assert!(full.subjects[0].harness.is_some());
    assert_eq!(full.subjects[0].claims.len(), 2);
    SUBJECT_REDUCTIONS.with(|count| count.set(0));
    let again = store.agent_card_status_at(None, cut, true).unwrap();
    assert_eq!(serde_json::to_value(again).unwrap(), serde_json::to_value(card).unwrap());
    assert_eq!(SUBJECT_REDUCTIONS.with(std::cell::Cell::get), 0);
}

#[test]
fn terminal_resource_status_never_warms_full_harness_or_provenance() {
    let store = Store::open_memory("node").unwrap();
    let subject = "agent/terminal-summary";
    observe_runtime(&store, subject, "running", "one");
    for _ in 0..20 {
        store
            .append_claim(&ClaimInput {
                subject: subject.into(),
                kind: "harness.observed".into(),
                actor: Some(subject.into()),
                fields: BTreeMap::from([
                    ("state".into(), json!("idle")),
                    ("incarnation_id".into(), json!("one")),
                    ("driver".into(), json!("claude")),
                ]),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
    }
    let cut = store.index().unwrap();
    let lean = store.terminal_resource_status_at(None, cut, true).unwrap();
    assert_eq!(lean.subjects.len(), 1);
    assert!(lean.subjects[0].harness.is_none());
    assert_eq!(lean.subjects[0].claims.len(), 1);
    assert!(
        store
            .smalltalk
            .subject_cache
            .lock()
            .unwrap()
            .statuses
            .is_empty()
    );
    SUBJECT_REDUCTIONS.with(|count| count.set(0));
    store.terminal_resource_status_at(None, cut, true).unwrap();
    assert_eq!(SUBJECT_REDUCTIONS.with(std::cell::Cell::get), 0);
    let full = store
        .status_for_claim_kind_at("runtime.observed", Some(cut), true, ClaimsReduction::Full)
        .unwrap();
    assert!(full.subjects[0].harness.is_some());
    assert!(full.subjects[0].claims.len() > 1);
    observe_runtime(&store, subject, "stopped", "two");
    let newer = store
        .terminal_resource_status_at(None, store.index().unwrap(), true)
        .unwrap();
    assert_eq!(
        newer.subjects[0].actual.as_ref().unwrap()["status"],
        "stopped"
    );
    assert_eq!(
        serde_json::to_value(store.terminal_resource_status_at(None, cut, true).unwrap())
            .unwrap(),
        serde_json::to_value(lean).unwrap()
    );
}

#[test]
fn an_internal_apply_replays_after_desired_state_drifts() {
    let store = Store::open_memory("node").unwrap();
    let intent = simple("true");
    let first = store.apply_internal(&intent, "materialize:test").unwrap();
    let replay = store.apply_internal(&intent, "materialize:test").unwrap();

    assert!(first.changed);
    assert!(!replay.changed);
    assert_eq!(replay.store_index, first.store_index);
    assert!(replay.batch_id.is_none());

    let stop =
        crate::graph::parse_test_intent("version 2\nstop \"exec/work\"\n", "node").unwrap();
    assert!(store.apply_internal(&stop, "stop:test").unwrap().changed);

    let restored = store.apply_internal(&intent, "materialize:test").unwrap();
    assert!(restored.changed);
    assert_eq!(
        store.selected_desired_kind("exec/work").unwrap(),
        Some("exec".into())
    );
}

#[test]
fn an_internal_apply_restores_a_repaired_desired_claim() {
    let store = Store::open_memory("node").unwrap();
    let intent = simple("true");
    let first = store.apply_internal(&intent, "materialize:test").unwrap();
    let claim = first.claim_ids.first().expect("desired claim").clone();

    {
        let connection = store.connection.lock().expect("store mutex");
        connection
            .execute(
                /* sqlite-large-value: allow-for-now lv-hf0619f693ab6 owner=st3_seats::store::tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO replica_records(
                         record_ref, writer, sequence, envelope_hash, position, raw, state,
                         claim_id, subject_hint, kind_hint, updated_at_unix_ms
                     ) VALUES (?1, 'node', 1, 'test-envelope', 0, X'', 'repaired',
                               ?2, 'exec/work', 'intent.desired', '0')",
                params![format!("record/{claim}"), claim],
            )
            .unwrap();
        connection
            .execute(/* sqlite-large-value: allow-for-now lv-h3e4dd6ffa61f owner=st3_seats::store::tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM desired WHERE subject='exec/work'", [])
            .unwrap();
    }

    let restored = store.apply_internal(&intent, "materialize:test").unwrap();
    assert!(restored.changed);
    assert_ne!(restored.batch_id, first.batch_id);
    assert_eq!(
        store.selected_desired_kind("exec/work").unwrap(),
        Some("exec".into())
    );
}

fn tree_shapes() -> TreeShapes {
    let (controller, worker, root, children) = replicated_run_tree(4);
    // Only a declared agent's live work can ask a person.
    let agents = crate::graph::parse_internal_intent(
        r#"version 2
agent "desk.one" { workspace "/tmp"; command "true"; restart always; }
agent "worker.one" { workspace "/tmp"; command "true"; restart always; }
"#,
        "controller",
    )
    .unwrap();
    controller.apply_internal(&agents, "tree-shape-agents").unwrap();
    let request = |key: &str| MissionRunRequest {
        mission: "request".into(),
        revision: None,
        workspace: "/tmp".into(),
        requester: Some("person/test".into()),
        mode: Some("run".into()),
        inputs: BTreeMap::new(),
        idempotency_key: key.into(),
    };
    let grandchild = controller
        .create_child_mission_run(
            &request("grandchild"),
            &children[1],
            &children[1].steps[0].subject,
            None,
        )
        .unwrap();
    let delivered = controller
        .create_child_mission_run(
            &request("delivered"),
            &root,
            "subscription/front-desk-intake",
            None,
        )
        .unwrap();
    for run in [&root, &grandchild, &delivered] {
        controller.set_step_state(&run.steps[0].subject, "ready", None).unwrap();
    }
    let ask = |step: &str, actor: &str, key: &str| {
        controller
            .work_action(
                step,
                "claim",
                &WorkRequest {
                    wait_ms: None,
                    actor: Some(actor.into()),
                    incarnation: Some(format!("{key}-incarnation")),
                    summary: None,
                    reason: None,
                    evidence: Vec::new(),
                    idempotency_key: format!("{key}-claim"),
                },
            )
            .unwrap();
        controller
            .ask_person(&crate::model::PersonAskRequest {
                legacy_request: None,
                person: "person/test".into(),
                title: "Choose a date".into(),
                reason: "Reply with a date.".into(),
                actor: actor.into(),
                step: Some(step.into()),
                new_run: None,
                incarnation: Some(format!("{key}-incarnation")),
                request: None,
                idempotency_key: key.into(),
            })
            .unwrap()
    };
    let root_ask = ask(&root.steps[0].subject, "agent/desk.one", "root-ask");
    // The worker's seat queue takes its oldest ready step first.
    let child_ask = ask(&children[0].steps[0].subject, "agent/worker.one", "child-ask");
    replicate_fully(&controller, &worker);
    TreeShapes {
        controller,
        worker,
        root,
        children,
        grandchild,
        delivered,
        root_ask,
        child_ask,
    }
}

#[test]
fn a_late_claim_about_a_grandchild_rebuilds_its_child_part_and_matches_a_replay() {
    let shapes = tree_shapes();
    let step = shapes.grandchild.steps[0].subject.clone();
    let parts = late_claim_matches_a_replay(&shapes, |controller| {
        controller.set_step_state(&step, "blocked", Some("blocked early")).unwrap();
    });
    assert!((1..TREE_SHAPE_RUNS).contains(&parts), "{parts} runs rebuilt as parts");
}

#[test]
fn a_late_claim_about_a_delivered_run_rebuilds_its_part_and_matches_a_replay() {
    let shapes = tree_shapes();
    let step = shapes.delivered.steps[0].subject.clone();
    let parts = late_claim_matches_a_replay(&shapes, |controller| {
        controller.set_step_state(&step, "blocked", Some("blocked early")).unwrap();
    });
    assert!((1..TREE_SHAPE_RUNS).contains(&parts), "{parts} runs rebuilt as parts");
}

#[test]
fn late_claims_about_asks_owned_by_the_root_and_by_a_child_match_a_replay() {
    for owned_by_root in [true, false] {
        let shapes = tree_shapes();
        let ask = if owned_by_root { &shapes.root_ask } else { &shapes.child_ask };
        let ask = ask.subject.clone();
        let parts = late_claim_matches_a_replay(&shapes, |controller| {
            controller.set_step_state(&ask, "blocked", Some("blocked early")).unwrap();
        });
        assert!(
            (1..TREE_SHAPE_RUNS).contains(&parts),
            "{parts} runs rebuilt as parts (owned by the root: {owned_by_root})"
        );
    }
}

#[test]
fn a_late_claim_in_the_batch_that_ends_the_root_matches_a_replay() {
    let shapes = tree_shapes();
    let step = shapes.children[2].steps[0].subject.clone();
    let root = shapes.root.id.clone();
    late_claim_matches_a_replay(&shapes, |controller| {
        controller.set_step_state(&step, "blocked", Some("blocked early")).unwrap();
        controller
            .set_mission_run_state(&root, "cancelled", "terminal", Some("the desk closes"))
            .unwrap();
    });
}

#[test]
fn person_reads_seek_what_they_show() {
    let store = Store::open_memory("node").unwrap();
    let connection = store.connection.lock().unwrap();
    let plan = |sql: &str| {
        let mut statement = connection
            .prepare(/* sqlite-large-value: allow-for-now lv-h5479676c182d owner=st3_seats::store::tests TODO=trace-statement-and-value-representation */ &format!("EXPLAIN QUERY PLAN {sql}"))
            .unwrap();
        let parameters = vec!["agent/example"; statement.parameter_count()];
        statement
            .query_map(rusqlite::params_from_iter(parameters), |row| {
                row.get::<_, String>(3)
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
            .join("\n")
    };
    for order in ["ASC", "DESC"] {
        let plan = plan(&incarnation_claim_time_query(order));
        assert!(
            plan.contains("claims_incarnation_accepted_index (subject=? AND <expr>=?)")
                && !plan.contains("TEMP B-TREE"),
            "a session's {order} date must be one seek into its incarnation's claims:\n{plan}"
        );
    }
    let agents = plan(AGENT_STATUS_INDEX_QUERY);
    assert!(
        agents.contains("USING INTEGER PRIMARY KEY") && !agents.contains("MULTI-INDEX OR"),
        "the newest agent claim must be a walk back from the snapshot:\n{agents}"
    );
    let harness = plan(&newest_claims_of_kind_query(
        "claims.id",
        "harness.observed",
    ));
    assert!(
        harness.contains("claims_subject_kind_accepted_index (subject=? AND kind=?)")
            && !harness.contains("TEMP B-TREE FOR ORDER BY"),
        "a seat's newest observation must not sort every observation it made:\n{harness}"
    );
    let latest = plan(&latest_claim_of_kind_query());
    assert!(
        latest.contains("claims_subject_kind_accepted_index (subject=? AND kind=?)")
            && !latest.contains("TEMP B-TREE FOR ORDER BY"),
        "a subject's newest claim of a kind must not sort every claim of the kind:\n{latest}"
    );
    let open_steps = plan(&work_at_snapshot_query(true));
    assert!(
        open_steps.contains("step_runs_open_index"),
        "a seat queue must read the open steps, not every step:\n{open_steps}"
    );
    let selected_steps = plan(&selected_work_at_snapshot_query(false));
    assert!(
        !selected_steps.contains("SCAN step_runs\n"),
        "the published work list refolds its changed steps by subject, not every step:\n{selected_steps}"
    );
    for open_only in [false, true] {
        let seat_steps = plan(&seat_work_at_snapshot_query(open_only));
        assert!(
            seat_steps.contains("step_runs_assignee_index (assignee=?)")
                && seat_steps.contains("step_runs_lease_index (lease_owner=?)")
                && seat_steps.contains("step_runs_available_index")
                && !seat_steps.contains("SCAN step_runs\n"),
            "a seat's steps must be found by seat, not by reading every step:\n{seat_steps}"
        );
    }
    let joins = plan(&seat_queue_joins_query());
    assert!(
        joins.contains("SCAN mission_runs USING INDEX mission_runs_open_index")
            && joins.contains("step_runs_run_index (run_id=?)"),
        "a seat queue must read the open runs' steps, not every assigned step:\n{joins}"
    );
    let runs = plan(&open_mission_run_headers_query());
    assert!(
        runs.contains("mission_runs_open_index"),
        "the missions tree must read the open runs, not every run:\n{runs}"
    );
}

#[test]
fn owned_subject_queries_look_owners_up_by_primary_key() {
    let store = Store::open_memory("node").unwrap();
    let connection = store.connection.lock().unwrap();
    for (name, query) in [
        ("terminal owned runtimes", TERMINAL_OWNED_RUNTIME_SUBJECTS),
        ("retired owned intake", RETIRED_OWNED_INTAKE_SUBJECTS),
    ] {
        let mut statement = connection
            .prepare(/* sqlite-large-value: allow-for-now lv-h937f05039d2f owner=st3_seats::store::tests TODO=trace-statement-and-value-representation */ &format!("EXPLAIN QUERY PLAN {query}"))
            .unwrap();
        let plan = statement
            .query_map([], |row| row.get::<_, String>(3))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(
            !plan
                .iter()
                .any(|step| step.starts_with("SCAN owner")
                    || step.starts_with("SCAN generation")),
            "{name} must not scan owners for each desired row:\n{}",
            plan.join("\n")
        );
        assert!(
            plan.iter()
                .any(|step| step.starts_with("SEARCH owner USING INDEX")),
            "{name} must look its owner run up by primary key:\n{}",
            plan.join("\n")
        );
    }
}

#[test]
fn work_actions_require_an_active_incarnation_bound_lease() {
    let store = Store::open_memory("node").unwrap();
    let intent = crate::graph::parse_test_intent(
        r#"
version 2

  mission "lease" state="ready" {
    goal "Complete mission lease."
    step "work" { assigned-to "agent/worker" }
    step "other" { assigned-to "agent/worker" }
  }

"#,
        "node",
    )
    .unwrap();
    let planned = store
        .mission(
            &intent,
            IntentInput {
                kdl: "lease".into(),
                source_name: None,
            },
        )
        .unwrap();
    store
        .apply(&intent, &planned.subject_tokens, "lease-mission")
        .unwrap();
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: "lease".into(),
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/test".into()),
            mode: Some("run".into()),
            inputs: BTreeMap::new(),
            idempotency_key: "lease-run".into(),
        })
        .unwrap();
    let subject = &run
        .steps
        .iter()
        .find(|step| step.step == "work")
        .unwrap()
        .subject;
    let other = &run
        .steps
        .iter()
        .find(|step| step.step == "other")
        .unwrap()
        .subject;
    store.set_step_state(subject, "ready", None).unwrap();
    store.set_step_state(other, "ready", None).unwrap();
    let queue = store
        .agent_work_queues()
        .unwrap()
        .remove("agent/node.worker")
        .unwrap();
    let selected = store.agent_work_queues_for(Some("agent/node.worker")).unwrap();
    assert_eq!(selected.len(), 1);
    assert_eq!(selected["agent/node.worker"], queue);
    assert!(store.agent_work_queues_for(Some("agent/absent")).unwrap().is_empty());
    assert_eq!(queue.active_work_count, 0);
    assert_eq!(queue.queued_work_count, 2);
    assert_eq!(
        queue.next_work_id.as_deref(),
        queue.upcoming_work_ids.first().map(String::as_str)
    );
    assert!(queue.upcoming_work_ids.contains(subject));
    assert!(queue.upcoming_work_ids.contains(other));
    let wake_tag = format!("st3-work:{subject}@1@1@incarnation");
    for (name, recipient) in [
        ("correct-recipient", "agent/node.worker"),
        ("wrong-recipient", "agent/someone.else"),
    ] {
        store
            .append_claim(&ClaimInput {
                subject: format!("message/{name}"),
                kind: "message.sent".into(),
                actor: Some("daemon/runtime".into()),
                fields: BTreeMap::from([
                    ("from".into(), Value::String("daemon/runtime".into())),
                    ("to".into(), Value::String(recipient.into())),
                    ("content".into(), Value::String("wake".into())),
                    ("status".into(), Value::String("sent".into())),
                    (
                        "tags".into(),
                        Value::Array(vec![Value::String(wake_tag.clone())]),
                    ),
                ]),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: Some(format!("indexed-wake-{name}")),
            })
            .unwrap();
    }
    let wake = store
        .work_at_snapshot(Some("agent/node.worker"), true, now_ms())
        .unwrap()
        .into_iter()
        .find(|step| step.subject == *subject)
        .unwrap()
        .wake
        .unwrap();
    assert_eq!(
        wake.attempts, 1,
        "a wake for another recipient is not this agent's attempt"
    );
    let mut lean = store
        .work_for_reconcile_all()
        .unwrap()
        .into_iter()
        .find(|step| step.subject == *subject)
        .unwrap();
    assert!(lean.wake.is_none());
    store
        .populate_work_wake_for_reconcile(&mut lean, now_ms())
        .unwrap();
    assert_eq!(lean.wake, Some(wake));
    for (kind, status) in [
        ("message.delivered", "delivered"),
        ("message.read", "read"),
        ("message.closed", "closed"),
    ] {
        store
            .append_claim(&ClaimInput {
                subject: "message/correct-recipient".into(),
                kind: kind.into(),
                actor: Some("agent/node.worker".into()),
                fields: BTreeMap::from([("status".into(), Value::String(status.into()))]),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: Some(format!("{status}-indexed-wake")),
            })
            .unwrap();
    }
    store
        .populate_work_wake_for_reconcile(&mut lean, now_ms())
        .unwrap();
    assert_eq!(
        lean.wake.as_ref().unwrap().acknowledged_by.as_deref(),
        Some("consumed"),
        "a closed wake must not leave an overdue reconcile deadline"
    );
    let request = |incarnation: &str, key: &str| WorkRequest {
        wait_ms: None,
        actor: Some("agent/node.worker".into()),
        incarnation: Some(incarnation.into()),
        summary: None,
        reason: None,
        evidence: Vec::new(),
        idempotency_key: key.into(),
    };

    let missing = WorkRequest {
        wait_ms: None,
        incarnation: None,
        ..request("one", "claim-without-incarnation")
    };
    let error = store.work_action(subject, "claim", &missing).unwrap_err();
    assert_eq!(error.code, "missing-work-incarnation");

    let error = store
        .work_action(
            subject,
            "progress",
            &request("one", "progress-before-claim"),
        )
        .unwrap_err();
    assert_eq!(error.code, "work-not-claimed");
    store
        .work_action(subject, "claim", &request("one", "claim-one"))
        .unwrap();
    let queue = store
        .agent_work_queues()
        .unwrap()
        .remove("agent/node.worker")
        .unwrap();
    assert_eq!(store.agent_work_queues_for(Some("agent/node.worker")).unwrap()["agent/node.worker"], queue);
    assert_eq!(queue.current_work_ids, [subject.clone()]);
    assert_eq!(queue.active_work_count, 1);
    assert_eq!(queue.next_work_id.as_deref(), Some(other.as_str()));
    assert_eq!(queue.queued_work_count, 1);
    let error = store
        .work_action(other, "claim", &request("one", "claim-independent"))
        .unwrap_err();
    assert_eq!(error.code, "agent-capacity");
    let error = store
        .work_action(subject, "progress", &request("two", "progress-two"))
        .unwrap_err();
    assert_eq!(error.code, "wrong-work-incarnation");
    store
        .work_action(subject, "progress", &request("one", "progress-one"))
        .unwrap();
    // Lease expiry changes the read projection before any repair commits.
    store
        .connection
        .lock()
        .unwrap()
        .execute(
            /* sqlite-large-value: allow-for-now lv-h973552e71230 owner=st3_seats::store::tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE step_runs SET lease_expires_at_unix_ms='0' WHERE subject=?1",
            [subject],
        )
        .unwrap();
    let queue = store
        .agent_work_queues()
        .unwrap()
        .remove("agent/node.worker")
        .unwrap();
    assert_eq!(store.agent_work_queues_for(Some("agent/node.worker")).unwrap()["agent/node.worker"], queue);
    assert!(queue.current_work_ids.is_empty());
    assert_eq!(queue.active_work_count, 0);
    assert_eq!(queue.queued_work_count, 2);
    let seat = store.seat_queue("agent/node.worker").unwrap();
    assert_eq!(queue.current_work_ids, seat.current_work_ids);
    assert_eq!(queue.next_work_id, seat.next_work_id);
    assert_eq!(store.step_run(subject).unwrap().unwrap().status, "ready");
    let reclaimed = store
        .work_action(subject, "claim", &request("two", "reclaim-after-expiry"))
        .unwrap();
    assert_eq!(reclaimed.claim_incarnation.as_deref(), Some("two"));
    assert_eq!(
        store
            .work_action(
                subject,
                "progress",
                &request("one", "old-progress-after-reclaim")
            )
            .unwrap_err()
            .code,
        "wrong-work-incarnation"
    );
    store
        .work_action(
            subject,
            "progress",
            &request("two", "new-progress-after-reclaim"),
        )
        .unwrap();
    store
        .set_step_state(subject, "blocked", Some("waiting for a dependency"))
        .unwrap();

    expire_work_lease_by_claim(&store, subject, "agent/node.worker", "two");
    let reclaimable = store.step_run(subject).unwrap().unwrap();
    assert_eq!(reclaimable.status, "ready");
    assert!(reclaimable.claimant.is_none());
    let error = store
        .work_action(subject, "complete", &request("one", "complete-expired"))
        .unwrap_err();
    assert_eq!(error.code, "work-not-claimed");
    store
        .set_step_state(subject, "ready", Some("the worker lease expired"))
        .unwrap();
    let view = store.step_run(subject).unwrap().unwrap();
    assert!(view.claimant.is_none());
    assert!(view.claim_incarnation.is_none());
    assert!(view.claim_expires_at_unix_ms.is_none());
}

#[test]
fn response_usage_stays_local_while_attributed_totals_replicate() {
    let local = Store::open_memory("host-one").unwrap();
    let subject = "agent/example.worker";
    let seed = local
        .append_claim(&ClaimInput {
            subject: subject.into(),
            kind: "harness.observed".into(),
            actor: Some(subject.into()),
            fields: BTreeMap::from([
                ("state".into(), Value::String("ready".into())),
                ("driver".into(), Value::String("claude".into())),
                ("incarnation_id".into(), Value::String("inc-one".into())),
            ]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: Some("usage-seed".into()),
        })
        .unwrap();
    local.connection.lock().unwrap().execute(
        /* sqlite-large-value: allow-for-now lv-h70a700671f2e owner=st3_seats::store::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO desired(subject, kind, revision, claim_id, body, owner_run, owner_generation, owner_step)
             VALUES (?1, 'agent', 'revision', ?2, '{}', 'mission-run/example', 'run-generation/example', 'step-run/example/record')",
        params![subject, seed.id],
    ).unwrap();
    let record = |entry: &str, input: u64, output: u64, reads: u64, writes: u64| {
        let mut claim = timeline_observation(subject, "inc-one", entry);
        claim
            .fields
            .insert("entry_type".into(), Value::String("usage".into()));
        claim
            .fields
            .insert("role".into(), Value::String("system".into()));
        claim
            .fields
            .insert("driver".into(), Value::String("claude".into()));
        claim.fields.insert(
            "body".into(),
            json!({
                "semantics": "response", "model": "claude-example", "turn_id": entry,
                "input_tokens": input, "output_tokens": output,
                "cached_tokens": reads, "cache_write_tokens": writes,
                "total_tokens": input + output + reads + writes,
            }),
        );
        claim
    };
    let first = record("turn-a", 4, 2, 20, 3);
    let (observation, appended) = local.append_claim_outcome(&first).unwrap();
    assert!(appended);
    assert_eq!(
        observation.body["fields"]["attribution"]["mission_run_id"],
        "mission-run/example"
    );
    assert_eq!(
        observation.body["fields"]["attribution"]["step_id"],
        "step-run/example/record"
    );
    let rollup = local
        .usage_rollup_for_timeline(&observation)
        .unwrap()
        .unwrap();
    assert_eq!(rollup.fields["total_tokens"], 29);
    assert_eq!(rollup.fields["host"], "host-one");
    local.append_claim(&rollup).unwrap();
    let baseline = rollup.fields["observed_at_unix_ms"].as_u64().unwrap();
    let (repeated, appended) = local.append_claim_outcome(&first).unwrap();
    assert!(!appended);
    assert_eq!(
        local
            .usage_rollup_for_timeline(&repeated)
            .unwrap()
            .unwrap()
            .fields["total_tokens"],
        29
    );
    std::thread::sleep(std::time::Duration::from_millis(2));
    let mut second = record("turn-b", 5, 1, 30, 0);
    second
        .fields
        .insert("source_id".into(), Value::String("same-response".into()));
    let (observation, appended) = local.append_claim_outcome(&second).unwrap();
    assert!(appended);
    let rollup = local
        .usage_rollup_for_timeline(&observation)
        .unwrap()
        .unwrap();
    assert_eq!(rollup.fields["total_tokens"], 65);
    local.append_claim(&rollup).unwrap();
    let mut replay = second.clone();
    replay
        .fields
        .insert("incarnation_id".into(), Value::String("inc-two".into()));
    replay.idempotency_key = Some("replayed-after-restart".into());
    let (second_replay, _) = local.append_claim_outcome(&replay).unwrap();
    assert!(
        local
            .usage_rollup_for_timeline(&second_replay)
            .unwrap()
            .is_none()
    );
    let period = local
        .usage_period_rows(baseline, baseline + 60_000)
        .unwrap();
    assert_eq!(period.len(), 1);
    assert_eq!(period[0]["total_tokens"], 36);
    assert_eq!(period[0]["cached_tokens"], 30);
    assert_eq!(
        local
            .usage_summary_at(subject, None, None)
            .unwrap()
            .unwrap()
            .total_tokens,
        65
    );

    let peer = Store::open_memory("host-two").unwrap();
    let exchange = exchange_from(&local, &ReplicationInventory::default());
    receive_and_project(&peer, "host-one", &exchange);
    assert!(
        peer.timeline_claims_for_incarnation_at(subject, "inc-one", None, false, 10)
            .unwrap()
            .claims
            .is_empty()
    );
    assert_eq!(
        peer.usage_period_rows(baseline, baseline + 60_000).unwrap(),
        period
    );
    assert_eq!(
        peer.usage_summary_at(subject, None, None)
            .unwrap()
            .unwrap()
            .total_tokens,
        65
    );
}

#[test]
fn usage_provenance_survives_resume_replay_periods_and_reopen() {
    let scratch = tempfile::tempdir().unwrap();
    let path = scratch.path().join("usage.sqlite3");
    let local = Store::open(&path, "host-one").unwrap();
    let subject = "agent/example.provenance";
    let binding = |incarnation: &str, session: &str| {
        local
            .append_claim(&ClaimInput {
                subject: subject.into(),
                kind: "harness.session-file".into(),
                actor: Some(subject.into()),
                fields: BTreeMap::from([
                    ("harness".into(), json!("codex")),
                    ("session_id".into(), json!(session)),
                    ("incarnation_id".into(), json!(incarnation)),
                ]),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: Some(format!("binding-{incarnation}")),
            })
            .unwrap();
    };
    let respond = |incarnation: &str, entry: &str, input: u64, reported: bool| {
        let mut claim = timeline_observation(subject, incarnation, entry);
        claim.fields.insert("driver".into(), json!("codex"));
        claim.fields.insert("entry_type".into(), json!("usage"));
        let mut body = json!({"semantics":"response", "model":"gpt-6.1-sol",
            "input_tokens":input,"output_tokens":100,"total_tokens":input+100});
        if reported {
            body["cost"] = json!(0.001);
        }
        claim.fields.insert("body".into(), body);
        let observation = local.append_claim(&claim).unwrap();
        let rollup = local
            .usage_rollup_for_timeline(&observation)
            .unwrap()
            .unwrap();
        local.append_client_claim(&rollup).unwrap();
        (claim, observation, rollup)
    };
    binding("inc-one", "native-example");
    let (first, observation, baseline) = respond("inc-one", "price-short", 1000, false);
    let provenance = &baseline.fields["pricing_provenance"][0];
    assert_eq!(provenance["price_table_id"], crate::pricing::PRICE_TABLE_ID);
    assert_eq!(
        provenance["price_table_version"],
        crate::pricing::price_table_version()
    );
    assert_eq!(provenance["cost_source"], "computed");
    assert_eq!(provenance["rates_usd_per_million_tokens"]["input"], 2.0);
    assert_eq!(baseline.fields["native_session_id"], "native-example");
    let since = baseline.fields["observed_at_unix_ms"].as_u64().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(2));
    respond("inc-one", "price-long", 300000, false);
    let (_, _, mixed) = respond("inc-one", "price-reported", 10, true);
    assert_eq!(
        mixed.fields["pricing_provenance"].as_array().unwrap().len(),
        3
    );
    let computed = mixed.fields["pricing_provenance"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| p["cost_source"] == "computed")
        .collect::<Vec<_>>();
    assert!(
        computed
            .iter()
            .any(|p| p["rates_usd_per_million_tokens"]["input"] == 4.0
                && p["rates_usd_per_million_tokens"]["output"] == 15.0)
    );
    let reported = mixed.fields["pricing_provenance"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["cost_source"] == "provider_reported")
        .unwrap();
    assert!(reported.get("rates_usd_per_million_tokens").is_none());
    assert!(reported.get("price_table_id").is_none());
    // Replaying the response never adds another provenance contribution.
    local.append_claim(&first).unwrap();
    assert_eq!(
        local
            .usage_rollup_for_timeline(&observation)
            .unwrap()
            .unwrap()
            .fields,
        mixed.fields
    );
    let period = local.usage_period_rows(since, u64::MAX).unwrap();
    assert_eq!(period[0]["pricing_provenance"].as_array().unwrap().len(), 2);
    assert_eq!(period[0]["total_tokens"], 300210);
    assert_eq!(
        period[0]["pricing_provenance"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["total_tokens"].as_u64().unwrap())
            .sum::<u64>(),
        300210
    );
    let typed: st3_client::UsageRow = serde_json::from_value(period[0].clone()).unwrap();
    assert_eq!(typed.native_session_id.as_deref(), Some("native-example"));
    assert_eq!(typed.pricing_provenance.unwrap().len(), 2);
    // Resume binds a new incarnation to the same conversation. A newer conversation on
    // this seat must not steal the old incarnation's spend.
    binding("inc-two", "native-example");
    let (_, _, resumed) = respond("inc-two", "after-resume", 20, false);
    assert_eq!(resumed.fields["native_session_id"], "native-example");
    binding("inc-three", "native-other");
    assert_eq!(
        local
            .usage_rollup_for_timeline(&observation)
            .unwrap()
            .unwrap()
            .fields["native_session_id"],
        "native-example"
    );
    let all = local.usage_period_rows(0, u64::MAX).unwrap();
    let peer = Store::open_memory("host-two").unwrap();
    receive_and_project(
        &peer,
        "host-one",
        &exchange_from(&local, &ReplicationInventory::default()),
    );
    assert_eq!(peer.usage_period_rows(0, u64::MAX).unwrap(), all);
    drop(local);
    let reopened = Store::open(&path, "host-one").unwrap();
    assert_eq!(reopened.usage_period_rows(0, u64::MAX).unwrap(), all);
    assert_eq!(
        reopened
            .usage_rollup_for_timeline(&observation)
            .unwrap()
            .unwrap()
            .fields,
        mixed.fields
    );
}

#[test]
fn usage_period_keeps_price_versions_separate_without_repricing_the_baseline() {
    let local = Store::open_memory("host-one").unwrap();
    let subject = "agent/example.price-change";
    let contribution = |version: &str, tokens: u64, rate: u64| {
        json!({
            "price_table_id":"st.api-list", "price_table_version":version,
            "cost_source":"computed",
            "rates_usd_per_million_tokens":{"input":rate,"output":rate,"cache_read":rate,
                "cache_write_5m":rate,"cache_write_1h":rate},
            "total_tokens":tokens,"input_tokens":tokens,"output_tokens":0,
            "cache_write_tokens":0,"cache_write_1h_tokens":0,"cached_tokens":0,
            "cost_microusd":tokens*rate,"reported_cost_microusd":0,"unpriced_tokens":0,
        })
    };
    let old = contribution("example-v1", 10, 1);
    let new = contribution("example-v2", 20, 2);
    for (at, total, cost, provenance) in [
        (10, 10, 10, json!([old.clone()])),
        (20, 30, 50, json!([old, new])),
    ] {
        local
            .append_client_claim(&ClaimInput {
                subject: subject.into(),
                kind: "harness.usage".into(),
                actor: Some(subject.into()),
                fields: serde_json::from_value(json!({
                    "semantics":"response_rollup","driver":"codex","incarnation_id":"inc-one",
                    "model":"example-model","native_session_id":"native-example",
                    "observed_at_unix_ms":at,"total_tokens":total,"input_tokens":total,
                    "cost_microusd":cost,"pricing_provenance":provenance,
                }))
                .unwrap(),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
    }
    let rows = local.usage_period_rows(10, 20).unwrap();
    let (reported_rows, estimate) = local.usage_period_report(10, 20).unwrap();
    assert_eq!(reported_rows, rows);
    assert_eq!(estimate["days"][0]["usage_cost_microusd"], 40);
    assert_eq!(rows[0]["total_tokens"], 20);
    assert_eq!(rows[0]["cost_microusd"], 40);
    let provenance = rows[0]["pricing_provenance"].as_array().unwrap();
    assert_eq!(provenance.len(), 1);
    assert_eq!(provenance[0]["price_table_version"], "example-v2");
    assert_eq!(provenance[0]["rates_usd_per_million_tokens"]["input"], 2);
    assert_eq!(provenance[0]["total_tokens"], 20);
    assert_eq!(provenance[0]["cost_microusd"], 40);
    let lifetime = local.usage_period_rows(0, 20).unwrap();
    assert_eq!(
        lifetime[0]["pricing_provenance"].as_array().unwrap().len(),
        2
    );
    assert_eq!(lifetime[0]["cost_microusd"], 50);
}

#[test]
fn historical_usage_claims_leave_provenance_unknown() {
    let local = Store::open_memory("host-one").unwrap();
    let subject = "agent/example.legacy";
    local
        .append_client_claim(&ClaimInput {
            subject: subject.into(),
            kind: "harness.usage".into(),
            actor: Some(subject.into()),
            fields: BTreeMap::from([
                ("semantics".into(), json!("response_rollup")),
                ("driver".into(), json!("claude")),
                ("incarnation_id".into(), json!("legacy-inc")),
                ("total_tokens".into(), json!(99)),
                ("observed_at_unix_ms".into(), json!(10)),
                ("pricing".into(), json!("old-label")),
            ]),
            evidence: vec![],
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
    let rows = local.usage_period_rows(0, 100).unwrap();
    assert_eq!(rows[0]["unpriced_tokens"], 99);
    assert_eq!(rows[0]["pricing"], "old-label");
    assert!(rows[0].get("pricing_provenance").is_none());
    assert!(rows[0].get("native_session_id").is_none());
    let typed: st3_client::UsageRow = serde_json::from_value(rows[0].clone()).unwrap();
    assert!(typed.native_session_id.is_none());
    assert!(typed.pricing_provenance.is_none());
}

#[test]
fn response_spend_is_priced_and_kept_apart_by_account_and_step() {
    let local = Store::open_memory("host-one").unwrap();
    let subject = "agent/example.worker";
    let observed = |state: &str, key: &str| {
        local
            .append_claim(&ClaimInput {
                subject: subject.into(),
                kind: "harness.observed".into(),
                actor: Some(subject.into()),
                fields: BTreeMap::from([
                    ("state".into(), Value::String(state.into())),
                    ("driver".into(), Value::String("claude".into())),
                    ("incarnation_id".into(), Value::String("inc-one".into())),
                ]),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: Some(key.into()),
            })
            .unwrap()
    };
    let seed = observed("working", "spend-seed");
    let own_step = |step: &str| {
        local.connection.lock().unwrap().execute(
            /* sqlite-large-value: allow-for-now lv-hdcfb50d4b54b owner=st3_seats::store::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO desired(subject, kind, revision, claim_id, body, owner_run, owner_generation, owner_step)
                 VALUES (?1, 'agent', 'revision', ?2, '{}', 'mission-run/example', 'run-generation/example', ?3)",
            params![subject, seed.id, step],
        ).unwrap();
    };
    let respond = |entry: &str, body: Value| {
        let mut claim = timeline_observation(subject, "inc-one", entry);
        claim
            .fields
            .insert("entry_type".into(), Value::String("usage".into()));
        claim
            .fields
            .insert("role".into(), Value::String("system".into()));
        claim
            .fields
            .insert("driver".into(), Value::String("claude".into()));
        claim.fields.insert("body".into(), body);
        let (observation, appended) = local.append_claim_outcome(&claim).unwrap();
        assert!(appended);
        let rollup = local
            .usage_rollup_for_timeline(&observation)
            .unwrap()
            .unwrap();
        local.append_client_claim(&rollup).unwrap();
        rollup
    };
    let opus = |account: &str| {
        json!({"semantics": "response", "model": "claude-opus-5-5", "account": account,
            "input_tokens": 1000, "output_tokens": 100, "cached_tokens": 10000,
            "cache_write_tokens": 2000, "cache_write_1h_tokens": 1000, "total_tokens": 13100})
    };
    let start = now_ms() as u64;
    own_step("step-run/example/build");
    let build = respond("response-a", opus("claude/aaaaaaaaaaaaaaaa"));
    // $4 input, $20 output, $0.20 cache read, $5 and $8 for five-minute and one-hour writes.
    assert_eq!(
        build.fields["cost_microusd"],
        4_000 + 2_000 + 2_000 + 5_000 + 8_000
    );
    assert_eq!(build.fields["account"], "claude/aaaaaaaaaaaaaaaa");
    assert_eq!(build.fields["pricing"], crate::pricing::PRICING_REVISION);
    own_step("step-run/example/review");
    respond("response-b", opus("claude/bbbbbbbbbbbbbbbb"));
    respond(
        "response-c",
        json!({"semantics": "response", "model": "local-example", "input_tokens": 70, "output_tokens": 7, "total_tokens": 77}),
    );
    respond(
        "response-d",
        json!({"semantics": "response", "model": "local-example", "input_tokens": 9, "output_tokens": 1, "total_tokens": 10, "cost": 0.0025, "currency": "USD"}),
    );

    // Each key replicates even while the harness keeps working: a step change must not
    // leave the previous step's last totals pending behind the next step's.
    let replicated = local.claims_for(subject, Some("harness.usage")).unwrap();
    assert_eq!(replicated.len(), 3, "{replicated:#?}");
    // The local model's second response waits for the next interval, or for the harness
    // to stop working.
    observed("ready", "spend-idle");
    assert_eq!(
        local
            .claims_for(subject, Some("harness.usage"))
            .unwrap()
            .len(),
        4
    );

    let rows = local
        .usage_period_rows(start.saturating_sub(1), now_ms() as u64 + 1)
        .unwrap();
    let row = |step: &str, account: &str, model: &str| {
        rows.iter()
            .find(|row| {
                row["step"] == step && row["account"] == account && row["model"] == model
            })
            .unwrap_or_else(|| panic!("{step} {account} {model}: {rows:#?}"))
    };
    let build = row(
        "step-run/example/build",
        "claude/aaaaaaaaaaaaaaaa",
        "claude-opus-5-5",
    );
    assert_eq!(build["cost_microusd"], 21_000);
    assert_eq!(build["unpriced_tokens"], 0);
    let review = row(
        "step-run/example/review",
        "claude/bbbbbbbbbbbbbbbb",
        "claude-opus-5-5",
    );
    assert_eq!(review["cost_microusd"], 21_000);
    assert_eq!(review["mission_run"], "mission-run/example");
    let local_model = row("step-run/example/review", "", "local-example");
    assert_eq!(local_model["total_tokens"], 87);
    assert_eq!(
        local_model["unpriced_tokens"], 77,
        "an unpriced response is never free"
    );
    assert_eq!(local_model["cost_microusd"], 2_500);
    assert_eq!(local_model["reported_cost_microusd"], 2_500);
}

#[test]
fn counted_responses_are_remembered_for_a_bounded_horizon() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("claims.sqlite3");
    {
        let store = Store::open(&path, "host-one").unwrap();
        // An older build kept every counted response forever, without a date.
        store
            .connection
            .lock()
            .unwrap()
            .execute_batch(
                /* sqlite-large-value: allow-for-now lv-h4275d6997a8d owner=st3_seats::store::tests TODO=chunk-or-reference-variable-columns */ "CREATE TABLE local_usage_seen (subject TEXT NOT NULL, source_id TEXT NOT NULL,
                        PRIMARY KEY(subject, source_id));
                     INSERT INTO local_usage_seen VALUES ('agent/example.worker', 'counted-before');",
            )
            .unwrap();
    }
    let store = Store::open(&path, "host-one").unwrap();
    let subject = "agent/example.worker";
    let respond = |entry: &str, at: u128| {
        let mut claim = timeline_observation(subject, "inc-one", entry);
        claim
            .fields
            .insert("entry_type".into(), Value::String("usage".into()));
        claim
            .fields
            .insert("source_id".into(), Value::String(entry.into()));
        claim
            .fields
            .insert("observed_at_unix_ms".into(), Value::from(at as u64));
        claim.fields.insert(
            "body".into(),
            json!({"semantics": "response", "model": "claude-opus-5-5", "input_tokens": 1, "output_tokens": 1, "total_tokens": 2}),
        );
        store.append_claim_outcome(&claim).unwrap();
    };
    let now = now_ms();
    respond("counted-before", now);
    respond("too-old", now - USAGE_RESPONSE_HORIZON_MS - 60_000);
    respond("recent", now - 60_000);
    let total: u64 = store
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT COALESCE(SUM(total_tokens), 0) FROM local_usage_spend",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(total, 2, "only the recent response counts");
    assert_eq!(store.trim_usage_responses(now - 30_000, 1).unwrap(), 1);
    let remembered = store
        .connection
        .lock()
        .unwrap()
        .prepare("SELECT source_id FROM local_usage_responses ORDER BY source_id")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(remembered, ["counted-before"]);
}

#[test]
fn retention_expired_compound_apply_uses_existing_operation_identities() {
    let store = Store::open_memory("node").unwrap();
    let declaration = crate::graph::parse_execution_intent(r#"version 2
resource "receipt/file" { kind "filesystem.file" }
observer "receipt/file" { resource "resource/receipt/file"; provider "local.file"; locator "/tmp/receipt-file"; field "status" }
"#,"node","receipt-run").unwrap();
    store
        .apply_internal(&declaration, "receipt-declaration")
        .unwrap();
    let source = r#"version 2
resource "receipt/file" {
    refresh "first" { timeout "1s" }
    refresh "second" { timeout "1s" }
}
"#;
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
    let first = store
        .apply_as(
            &intent,
            &planned.subject_tokens,
            "expired-compound",
            Some("person/operator"),
        )
        .unwrap();
    assert!(first.changed);
    let operations = store
        .connection
        .write()
        .query_row("SELECT COUNT(*) FROM operations", [], |row| {
            row.get::<_, u64>(0)
        })
        .unwrap();
    store
        .connection
        .write()
        .execute(
            /* sqlite-large-value: allow-for-now lv-hec4df63434b4 owner=st3_seats::store::tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM idempotency WHERE operation_id<>?1",
            [opaque_cache_key("expired-compound")],
        )
        .unwrap();
    store
        .connection
        .write()
        .execute(/* sqlite-large-value: allow-for-now lv-h7567cbadedc5 owner=st3_seats::store::tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE idempotency SET expires_at_unix_ms=0", [])
        .unwrap();
    assert_eq!(store.cleanup_idempotency(0).unwrap().deleted, 1);
    let before = store.index().unwrap();
    let current = store
        .mission(
            &intent,
            IntentInput {
                kdl: source.into(),
                source_name: None,
            },
        )
        .unwrap();
    let repeat = store
        .apply_as(
            &intent,
            &current.subject_tokens,
            "expired-compound",
            Some("person/operator"),
        )
        .unwrap();
    assert!(!repeat.changed);
    assert_eq!(store.index().unwrap(), before);
    assert_eq!(
        store
            .connection
            .write()
            .query_row("SELECT COUNT(*) FROM operations", [], |row| row
                .get::<_, u64>(0))
            .unwrap(),
        operations
    );
}

/// Every node that holds the same claims shows the same graph, however it projected them:
/// one exchange at a time as the source writes, from envelopes that arrive newest first, or
/// all at once as a node that joins late.
#[test]
fn graph_digests_agree_after_sync_whatever_path_projected_the_graph() {
    let source = Store::open_memory("source").unwrap();
    let stepwise = Store::open_memory("stepwise").unwrap();
    let reversed = Store::open_memory("reversed").unwrap();
    let late = Store::open_memory("late").unwrap();
    let sync = |target: &Store| {
        receive_and_project(
            target,
            "source",
            &exchange_from(&source, &target.replication_inventory().unwrap()),
        );
    };

    let failed = failed_takeover_run(&source, &["deploy-check"]);
    sync(&stepwise);
    sync(&reversed);
    // Revising a failed run reopens it. Each carried step claim is its own batch.
    let revised = publish_takeover(
        &source,
        &TAKEOVER_SOURCE.replace(
            r#"step "smoke-check" { depends-on"#,
            r#"step "smoke-check" { goal "Probe the new health endpoint."; depends-on"#,
        ),
        "takeover-revision",
    );
    let reopened = source
        .adopt_mission_revision(
            &failed.id,
            &revised,
            "person/requester",
            "the smoke check probes the new endpoint",
            "revise-failed-takeover",
        )
        .unwrap();
    sync(&stepwise);
    source
        .set_step_state(
            &takeover_step(&reopened, "deploy-check").subject,
            "ready",
            None,
        )
        .unwrap();
    sync(&stepwise);
    // A node admits the envelopes of one exchange in sequence order, so the newest arrives
    // in an exchange of its own and the older ones follow in the next.
    let newest_first = |target: &Store| {
        let mut older = exchange_from(&source, &target.replication_inventory().unwrap());
        let newest = older.envelopes.split_off(older.envelopes.len() - 1);
        receive_and_project(
            target,
            "source",
            &ReplicationExchange {
                envelopes: newest,
                ..older.clone()
            },
        );
        receive_and_project(target, "source", &older);
    };
    newest_first(&reversed);
    // Later writes that arrive out of order make each node rebuild the run tree it already
    // projected, including the reopened generation, without replaying the graph.
    let smoke = takeover_step(&reopened, "smoke-check").subject.clone();
    source.set_step_state(&smoke, "ready", None).unwrap();
    source
        .set_step_state(&smoke, "working", Some("the smoke check started"))
        .unwrap();
    FULL_REPLAYS.with(|replays| replays.set(0));
    newest_first(&stepwise);
    newest_first(&reversed);
    assert_eq!(FULL_REPLAYS.with(std::cell::Cell::get), 0);
    // A replay from nothing, as a restore runs, projects the same graph.
    stepwise.replay_replication_graph().unwrap();
    reversed.replay_replication_graph().unwrap();
    sync(&late);

    let graph = |store: &Store| {
        let run = store.mission_run(&failed.id).unwrap().unwrap();
        let steps = run
            .steps
            .iter()
            .map(|step| (step.step.clone(), step.status.clone(), step.attempt))
            .collect::<Vec<_>>();
        let digest = store
            .replication_status(true, Some(TEST_FLEET), &[])
            .unwrap()
            .graph_digest;
        (run.status, run.phase, steps, digest)
    };
    let expected = graph(&source);
    assert_eq!(
        (expected.0.as_str(), expected.1.as_str()),
        ("running", "normal")
    );
    for (name, target) in [
        ("stepwise", &stepwise),
        ("reversed", &reversed),
        ("late", &late),
    ] {
        assert_eq!(
            canonical_audit::shared_rows(target),
            canonical_audit::shared_rows(&source),
            "{name}: every shared column"
        );
        assert_eq!(graph(target), expected, "{name}");
    }
}

#[test]
fn resource_observations_establish_a_baseline_and_send_selected_changes_once() {
    let store = Store::open_memory("node").unwrap();
    let source = r#"
version 2

  agent "one" { workspace "."; command "true" }
  agent "two" { workspace "."; command "true" }
  resource "github/acme/demo/pull/1" { kind "vcs.pull-request" }
  observer "github/acme/demo/pull/1" {
    resource "resource/github/acme/demo/pull/1"
    provider "github.pull-request"
    locator "acme/demo#1"
    field "head"
    field "state"
    field "checks"
  }
  subscription "one" {
    observer "observer/github/acme/demo/pull/1"
    to "agent/node.one"
    on "state"
    delivery "message"
  }
  subscription "two" {
    observer "observer/github/acme/demo/pull/1"
    to "agent/node.two"
    on "state"
    delivery "message"
  }
  subscription "missing" {
    observer "observer/github/acme/demo/pull/1"
    to "agent/node.missing"
    on "state"
    delivery "message"
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
        planned
            .warnings
            .iter()
            .any(|warning| warning.contains("agent/node.missing"))
    );
    store
        .apply(&intent, &planned.subject_tokens, "publish-watch")
        .unwrap();
    let subscriptions = store
        .desired_subjects()
        .unwrap()
        .into_iter()
        .filter(|desired| desired.kind == "subscription")
        .map(|desired| {
            let spec = crate::graph::subscription_spec(&desired.desired).unwrap();
            (desired.subject, spec)
        })
        .collect::<Vec<_>>();
    let observer_revision = store
        .selected_desired_revision("observer/github/acme/demo/pull/1")
        .unwrap()
        .unwrap();
    assert_eq!(
        store
            .record_resource_observation(
                "observer/github/acme/demo/pull/1",
                "stale-revision",
                None,
                "resource/github/acme/demo/pull/1",
                Some("stale-cursor"),
                &json!({"head": "stale"}),
                50,
                &subscriptions,
                None,
            )
            .unwrap_err()
            .code,
        "stale-observer-revision"
    );
    let baseline = store.record_resource_observation("observer/github/acme/demo/pull/1", &observer_revision, None, "resource/github/acme/demo/pull/1", Some("cursor-one"), &json!({
        "head": "resource/github/acme/demo/ref/a",
        "state": "open",
        "checks": ["pending"]
    }), 100, &subscriptions, None)
        .unwrap();
    assert!(baseline.baseline);
    assert!(baseline.message_subjects.is_empty());
    assert_eq!(
        store
            .claims_for(
                "resource/github/acme/demo/pull/1",
                Some("resource.observed")
            )
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        store
            .latest_actual_value("subscription/missing")
            .unwrap()
            .unwrap()["state"],
        "pending"
    );
    let unchanged = store.record_resource_observation("observer/github/acme/demo/pull/1", &observer_revision, None, "resource/github/acme/demo/pull/1", Some("cursor-one"), &json!({
        "head": "resource/github/acme/demo/ref/a",
        "state": "open",
        "checks": ["pending"]
    }), 200, &subscriptions, None)
        .unwrap();
    assert!(unchanged.changed_fields.is_empty());
    assert!(unchanged.observation_claim.is_none());
    let changed = store.record_resource_observation("observer/github/acme/demo/pull/1", &observer_revision, None, "resource/github/acme/demo/pull/1", Some("cursor-two"), &json!({
        "head": "resource/github/acme/demo/ref/a",
        "state": "closed",
        "checks": ["pending"]
    }), 300, &subscriptions, None)
        .unwrap();
    assert_eq!(changed.changed_fields, ["state"]);
    assert_eq!(changed.message_subjects.len(), 2);
    let retry = store.record_resource_observation("observer/github/acme/demo/pull/1", &observer_revision, None, "resource/github/acme/demo/pull/1", Some("cursor-two"), &json!({
        "head": "resource/github/acme/demo/ref/a",
        "state": "closed",
        "checks": ["pending"]
    }), 300, &subscriptions, None)
        .unwrap();
    assert_eq!(retry.message_subjects, changed.message_subjects);
    assert_eq!(store.messages(None, true).unwrap().len(), 2);
    let stop_source = "version 2\n subscription \"one\" { stop } \n";
    let stop_intent = parse_intent(stop_source, "node").unwrap();
    let stop_mission = store
        .mission(
            &stop_intent,
            IntentInput {
                kdl: stop_source.into(),
                source_name: None,
            },
        )
        .unwrap();
    store
        .apply(
            &stop_intent,
            &stop_mission.subject_tokens,
            "stop-one-subscription",
        )
        .unwrap();
    let after_stop = store.record_resource_observation("observer/github/acme/demo/pull/1", &observer_revision, None, "resource/github/acme/demo/pull/1", Some("cursor-after-stop"), &json!({
        "head": "resource/github/acme/demo/ref/a",
        "state": "open",
        "checks": ["pending"]
    }), 350, &subscriptions, None)
        .unwrap();
    assert_eq!(after_stop.message_subjects.len(), 1);
    assert_eq!(store.messages(None, true).unwrap().len(), 3);
    let unselected = store.record_resource_observation("observer/github/acme/demo/pull/1", &observer_revision, None, "resource/github/acme/demo/pull/1", Some("cursor-three"), &json!({
        "head": "resource/github/acme/demo/ref/a",
        "state": "open",
        "checks": ["passing"]
    }), 400, &subscriptions, None)
        .unwrap();
    assert_eq!(unselected.changed_fields, ["checks"]);
    assert!(unselected.message_subjects.is_empty());
}

#[test]
fn a_subscription_condition_delivers_only_when_it_becomes_true() {
    let store = Store::open_memory("node").unwrap();
    let source = r#"
version 2

agent "target" { workspace "."; command "true" }
resource "pull" { kind "vcs.pull-request" }
observer "pull" {
  resource "resource/pull"
  provider "github.pull-request"
  locator "acme/demo#1"
  field "checks"
}
subscription "green" {
  observer "observer/pull"
  to "agent/node.target"
  on "checks"
  when {
    every "checks" {
      field "status" "is" "completed"
      field "conclusion" "is" "success"
    }
  }
  delivery "message"
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
    store
        .apply(&intent, &planned.subject_tokens, "publish-green-watch")
        .unwrap();
    let subscriptions = store
        .desired_subjects()
        .unwrap()
        .into_iter()
        .filter(|desired| desired.kind == "subscription")
        .map(|desired| {
            let spec = crate::graph::subscription_spec(&desired.desired).unwrap();
            (desired.subject, spec)
        })
        .collect::<Vec<_>>();
    let observer_revision = store
        .selected_desired_revision("observer/pull")
        .unwrap()
        .unwrap();
    let observe = |cursor: &str, checks: Value| {
        store.record_resource_observation("observer/pull", &observer_revision, None, "resource/pull", Some(cursor), &json!({"checks": checks}), 100, &subscriptions, None)
            .unwrap()
    };

    let baseline = observe(
        "baseline",
        json!([
            {"status": "completed", "conclusion": "success"},
            {"status": "in_progress", "conclusion": null}
        ]),
    );
    assert!(baseline.baseline);
    assert!(baseline.message_subjects.is_empty());

    let still_pending = observe(
        "still-pending",
        json!([
            {"status": "completed", "conclusion": "success", "completed_at": "one"},
            {"status": "in_progress", "conclusion": null}
        ]),
    );
    assert!(still_pending.message_subjects.is_empty());

    let green = observe(
        "green",
        json!([
            {"status": "completed", "conclusion": "success", "completed_at": "one"},
            {"status": "completed", "conclusion": "success", "completed_at": "two"}
        ]),
    );
    assert_eq!(green.message_subjects.len(), 1);

    let still_green = observe(
        "still-green",
        json!([
            {"status": "completed", "conclusion": "success", "completed_at": "updated"},
            {"status": "completed", "conclusion": "success", "completed_at": "two"}
        ]),
    );
    assert!(still_green.message_subjects.is_empty());

    let red = observe(
        "red",
        json!([
            {"status": "completed", "conclusion": "failure"},
            {"status": "completed", "conclusion": "success"}
        ]),
    );
    assert!(red.message_subjects.is_empty());
    let green_again = observe(
        "green-again",
        json!([
            {"status": "completed", "conclusion": "success"},
            {"status": "completed", "conclusion": "success"}
        ]),
    );
    assert_eq!(green_again.message_subjects.len(), 1);
    assert_ne!(green_again.message_subjects, green.message_subjects);
    assert_eq!(store.messages(None, true).unwrap().len(), 2);
}

#[test]
fn issue_openers_survive_publishers_and_direct_and_collection_observers() {
    for attribution in [
        json!({"opened_by_run": "mission-run/author"}),
        json!({"opened_by": "agent/node.author", "opened_by_run": "mission-run/author"}),
    ] {
        let store = Store::open_memory("node").unwrap();
        let source = r#"version 2
resource "github/acme/demo" { kind "vcs.repository" }
resource "github/acme/demo/issue/8" { kind "vcs.issue" }
observer "repo" {
  resource "resource/github/acme/demo"
  provider "github.repository"
  locator "acme/demo"
  field "issues"
}
observer "issue" {
  resource "resource/github/acme/demo/issue/8"
  provider "github.issue"
  locator "acme/demo#8"
  field "state"
}
"#;
        let intent = parse_intent(source, "node").unwrap();
        let planned = store
            .mission(&intent, IntentInput { kdl: source.into(), source_name: None })
            .unwrap();
        store.apply(&intent, &planned.subject_tokens, "publish-watch").unwrap();
        let subject = "resource/github/acme/demo/issue/8";
        let publish = |facts: Value| ClaimInput {
            subject: subject.into(),
            kind: "resource.observed".into(),
            actor: Some("agent/node.author".into()),
            fields: BTreeMap::from([
                ("kind".into(), Value::String("vcs.issue".into())),
                ("facts".into(), facts),
            ]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        };
        store.append_client_claim(&publish(attribution.clone())).unwrap();
        store.append_client_claim(&publish(json!({
            "opened_by_run": "mission-run/fixer", "state": "open"
        }))).unwrap();
        let observe = |observer: &str, resource: &str, facts: Value| {
            let revision = store.selected_desired_revision(observer).unwrap().unwrap();
            store.record_resource_observation(observer, &revision, None, resource, None, &facts, 50, &[], None).unwrap();
        };
        observe("observer/issue", subject, json!({"state": "closed"}));
        let actual = store.latest_actual_value(subject).unwrap().unwrap();
        assert_eq!(actual["facts"]["state"], "closed");
        assert_eq!(actual["facts"]["opened_by_run"], "mission-run/author");
        if let Some(opener) = attribution.get("opened_by") {
            assert_eq!(&actual["facts"]["opened_by"], opener);
        }
        observe("observer/repo", "resource/github/acme/demo", json!({"issues": []}));
        observe("observer/repo", "resource/github/acme/demo", json!({
            "issues": [{"number": 8, "state": "open", "title": "Updated issue"}]
        }));
        let actual = store.latest_actual_value(subject).unwrap().unwrap();
        assert_eq!(actual["facts"]["state"], "open");
        assert_eq!(actual["facts"]["title"], "Updated issue");
        assert_eq!(actual["facts"]["opened_by_run"], "mission-run/author");
        if let Some(opener) = attribution.get("opened_by") {
            assert_eq!(&actual["facts"]["opened_by"], opener);
            let changed = store.append_client_claim(&publish(json!({
                "opened_by": "agent/node.fixer", "opened_by_run": "mission-run/fixer"
            }))).unwrap();
            assert_eq!(&changed.body["fields"]["facts"]["opened_by"], opener);
            assert_eq!(changed.body["fields"]["facts"]["opened_by_run"], "mission-run/author");
        }
    }
}

#[test]
fn pull_request_openers_survive_partial_publishers_and_observers() {
    for attribution in [
        json!({"opened_by_run": "mission-run/author"}),
        json!({"opened_by": "agent/node.author", "opened_by_run": "mission-run/author"}),
    ] {
        let store = Store::open_memory("node").unwrap();
        let source = r#"version 2
resource "github/acme/demo/pull/9" { kind "vcs.pull-request" }
observer "pull" {
  resource "resource/github/acme/demo/pull/9"
  provider "github.pull-request"
  locator "acme/demo#9"
  field "state"
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
        store
            .apply(&intent, &planned.subject_tokens, "publish-watch")
            .unwrap();
        let subject = "resource/github/acme/demo/pull/9";
        let publish = |facts: Value| ClaimInput {
            subject: subject.into(),
            kind: "resource.observed".into(),
            actor: Some("agent/node.author".into()),
            fields: BTreeMap::from([
                ("kind".into(), Value::String("vcs.pull-request".into())),
                ("facts".into(), facts),
            ]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        };
        let mut opened = attribution.clone();
        opened["state"] = json!("open");
        store.append_client_claim(&publish(opened)).unwrap();
        // A publisher's partial snapshot of the merge keeps both fields.
        let merged = store
            .append_client_claim(&publish(json!({"state": "merged", "merged": true})))
            .unwrap();
        assert_eq!(merged.body["fields"]["facts"]["state"], "merged");
        assert_eq!(
            merged.body["fields"]["facts"]["opened_by_run"],
            "mission-run/author"
        );
        assert_eq!(
            merged.body["fields"]["facts"].get("opened_by"),
            attribution.get("opened_by")
        );
        // So does an observer that never reads them.
        let revision = store
            .selected_desired_revision("observer/pull")
            .unwrap()
            .unwrap();
        store.record_resource_observation("observer/pull", &revision, None, subject, None, &json!({"state": "closed"}), 50, &[], None)
            .unwrap();
        let actual = store.latest_actual_value(subject).unwrap().unwrap();
        assert_eq!(actual["facts"]["state"], "closed");
        assert_eq!(actual["facts"]["opened_by_run"], "mission-run/author");
        assert_eq!(
            actual["facts"].get("opened_by"),
            attribution.get("opened_by")
        );
        // A later claim cannot take the opener over.
        let fixer = store
            .append_client_claim(&publish(json!({
                "opened_by": "agent/node.fixer", "opened_by_run": "mission-run/fixer"
            })))
            .unwrap();
        assert_eq!(
            fixer.body["fields"]["facts"]["opened_by_run"],
            "mission-run/author"
        );
        if let Some(opener) = attribution.get("opened_by") {
            assert_eq!(&fixer.body["fields"]["facts"]["opened_by"], opener);
        }
    }
}

/// A Claude prompt is answered from a client only while its permission hook still waits: one
/// that has waited longer is answered in the terminal, so no answer is recorded that nothing
/// would deliver. The hook still waiting on it is told it is gone, and exits.
#[test]
fn a_claude_prompt_older_than_its_hook_is_answered_in_the_terminal() {
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
        .apply_as(&intent, &preview.subject_tokens, "late-seat", Some("person/avery"))
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
    let now = now_ms() as u64;
    let window = CLAUDE_PROMPT_ANSWER_WAIT_MS as u64;
    let asked = |observed_at: u64, transition: u64| {
        append(
            "harness.observed",
            json!({"state":"working", "driver":"claude", "incarnation_id":"one",
                "blocked_on":"human", "ask":"permission", "reason":"Read",
                "observed_at_ms": observed_at,
                "ownership_sequence":1, "transition_sequence":transition}),
        )
    };
    // Within the hook's wait, the prompt is answered here.
    asked(now - window + 60_000, 3);
    assert_eq!(prompt().unwrap().answers, vec!["allow".to_owned(), "deny".to_owned()]);
    // Expiring the hook only disables answering through st. The native dialog
    // stays open, and each restatement keeps its original opening time.
    append(
        "harness.observed",
        json!({"state":"working", "driver":"claude", "incarnation_id":"one",
            "blocked_on":null, "ask":null, "ownership_sequence":1, "transition_sequence":4}),
    );
    asked(now - window - 1_000, 5);
    let late = asked(now, 5);
    let shown = prompt().expect("a late prompt is still an alert");
    assert!(shown.answers.is_empty(), "{:?}", shown.answers);
    assert!(shown.detail.contains("waited longer than st can answer it"), "{}", shown.detail);
    assert_eq!(
        shown.requested_at_unix_ms,
        u128::from(now - window - 1_000),
        "it has waited since it opened, not since its latest restatement"
    );
    assert_eq!(
        store
            .answer_native_prompt(seat, &late.id, "allow", "person/avery")
            .unwrap_err()
            .code,
        "stale-fence"
    );
    assert_eq!(store.native_prompt_state(seat, 1, 5).unwrap(), NativePromptState::Open);
    let index = store.index().unwrap();
    let held = store.observed_harness_at(seat, index).unwrap().unwrap();
    assert_eq!(held.blocked_on.as_deref(), Some("human"),
        "hook expiry must retain the native human mail hold");
    assert_eq!(store.seat_observation_at(seat, Some(&held), index, now_ms()).unwrap(), "current");
    append(
        "harness.observed",
        json!({"state":"idle", "driver":"claude", "incarnation_id":"one",
            "blocked_on":null, "ask":null, "ownership_sequence":1, "transition_sequence":6}),
    );
    assert_eq!(store.native_prompt_state(seat, 1, 5).unwrap(), NativePromptState::Gone);
    assert!(store.observed_harness_at(seat, store.index().unwrap()).unwrap().unwrap().blocked_on.is_none(),
        "authoritative native closure releases the mail hold");
}

#[test]
fn harness_projection_is_bound_to_the_current_runtime_epoch() {
    let store = Store::open_memory("node").unwrap();
    let subject = "agent/node.worker";
    let runtime = |incarnation: &str, key: &str| {
        store
            .append_claim(&ClaimInput {
                subject: subject.into(),
                kind: "runtime.observed".into(),
                actor: None,
                fields: BTreeMap::from([
                    ("status".into(), Value::String("running".into())),
                    ("runtime_id".into(), Value::String("node.worker".into())),
                    ("terminal".into(), Value::Bool(true)),
                    ("incarnation_id".into(), Value::String(incarnation.into())),
                ]),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: Some(key.into()),
            })
            .unwrap();
    };
    let harness = |incarnation: Option<&str>, key: &str| {
        let mut fields = BTreeMap::from([
            ("state".into(), Value::String("ready".into())),
            ("driver".into(), Value::String("codex".into())),
            ("transport".into(), Value::String("app-server".into())),
        ]);
        if let Some(incarnation) = incarnation {
            fields.insert("incarnation_id".into(), Value::String(incarnation.into()));
        }
        store
            .append_claim(&ClaimInput {
                subject: subject.into(),
                kind: "harness.observed".into(),
                actor: Some(subject.into()),
                fields,
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: Some(key.into()),
            })
            .unwrap();
    };

    runtime("old", "runtime-old");
    harness(Some("old"), "harness-old");
    assert_eq!(
        store
            .current_harness(subject)
            .unwrap()
            .unwrap()
            .incarnation_id,
        "old"
    );

    runtime("new", "runtime-new");
    assert!(store.current_harness(subject).unwrap().is_none());
    assert!(
        store
            .latest_actual_value(subject)
            .unwrap()
            .unwrap()
            .get("state")
            .is_none()
    );

    harness(Some("wrong"), "harness-wrong");
    assert!(store.current_harness(subject).unwrap().is_none());
    harness(None, "harness-legacy-current-epoch");
    assert_eq!(
        store
            .current_harness(subject)
            .unwrap()
            .unwrap()
            .incarnation_id,
        "new"
    );
    harness(Some("new"), "harness-new");
    // The harness-only status a driver renews its leases from answers what the full one does,
    // without the seat's claim ids; the fence's newest-claim seek names the last claim.
    let only = store.status_harness_only(subject).unwrap();
    assert_eq!(only.subjects.len(), 1);
    assert!(only.subjects[0].claims.is_empty());
    assert_eq!(
        only.subjects[0].harness.as_ref().map(|harness| harness.incarnation_id.clone()),
        store.current_harness(subject).unwrap().map(|harness| harness.incarnation_id)
    );
    assert_eq!(
        store.latest_claim_id(subject).unwrap(),
        store.claims_for(subject, None).unwrap().last().map(|claim| claim.id.clone())
    );
    store
        .append_claim(&ClaimInput {
            subject: subject.into(),
            kind: "harness.observed".into(),
            actor: Some(subject.into()),
            fields: BTreeMap::from([
                ("state".into(), Value::String("working".into())),
                ("driver".into(), Value::String("codex".into())),
                ("incarnation_id".into(), Value::String("new".into())),
                ("reason".into(), Value::Null),
            ]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: Some("harness-new-activity".into()),
        })
        .unwrap();
    let current = store.current_harness(subject).unwrap().unwrap();
    assert_eq!(current.incarnation_id, "new");
    assert_eq!(current.state, "working");
    assert!(current.is_ready());
    assert_eq!(current.driver.as_deref(), Some("codex"));
    assert_eq!(current.transport.as_deref(), Some("app-server"));
    assert_eq!(current.reason, None);

    let mut complete = BTreeMap::from([
        ("state".into(), Value::String("ready".into())),
        ("driver".into(), Value::String("codex".into())),
        ("transport".into(), Value::String("app-server".into())),
        ("incarnation_id".into(), Value::String("new".into())),
    ]);
    for field in ["reason", "blocked_on", "ask", "input_buffer", "exit"] {
        complete.insert(field.into(), Value::Null);
    }
    store
        .append_claim(&ClaimInput {
            subject: subject.into(),
            kind: "harness.observed".into(),
            actor: Some(subject.into()),
            fields: complete,
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: Some("harness-new-complete-snapshot".into()),
        })
        .unwrap();
    let complete = store.current_harness(subject).unwrap().unwrap();
    assert_eq!(complete.state, "ready");
    assert_eq!(complete.transport.as_deref(), Some("app-server"));
    assert_eq!(complete.reason, None);

    store
        .append_claim(&ClaimInput {
            subject: subject.into(),
            kind: "runtime.readiness-deadline-reached".into(),
            actor: None,
            fields: BTreeMap::from([
                ("runtime_id".into(), Value::String("node.worker".into())),
                ("driver".into(), Value::String("codex".into())),
                ("incarnation_id".into(), Value::String("new".into())),
                (
                    "deadline_unix_ms".into(),
                    Value::String("1700000000000".into()),
                ),
                (
                    "reason".into(),
                    Value::String("the harness did not become ready".into()),
                ),
            ]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: Some("readiness-deadline".into()),
        })
        .unwrap();
    let actual = store.latest_actual_value(subject).unwrap().unwrap();
    assert!(actual.get("deadline_unix_ms").is_none());
    assert!(actual.get("reason").is_none());
}

#[test]
fn full_status_stays_full_after_summary_reductions_share_the_caches() {
    let store = Store::open_memory("node").unwrap();
    let subject = "agent/node.caches";
    store
        .append_claim(&ClaimInput {
            subject: subject.into(),
            kind: "runtime.observed".into(),
            actor: Some("person/avery".into()),
            fields: BTreeMap::from([("status".into(), json!("running"))]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
    let index = store.index().unwrap();
    let summarized = store
        .status_for_subject_prefix_at(
            "agent/",
            Some(index),
            true,
            ClaimsReduction::Summary,
        )
        .unwrap()
        .subjects
        .remove(0);
    assert!(summarized.claims.is_empty());
    // The public reduction still carries the whole vector at the same index, through the
    // caches the summary reduction just shared.
    let public = store.status_at(Some(subject), None, Some(index)).unwrap();
    let public = public.subjects.iter().find(|row| row.subject == subject).unwrap();
    assert_eq!(
        public.claims,
        full_canonical_claim_id_oracle(&store.readers.get(), subject, Some(index))
    );
    SUBJECT_REDUCTIONS.with(|count| count.set(0));
    let back = store
        .status_for_subject_prefix_at(
            "agent/",
            Some(index),
            true,
            ClaimsReduction::Summary,
        )
        .unwrap()
        .subjects
        .remove(0);
    assert!(back.claims.is_empty());
    assert_eq!(back.claims_summary.latest, public.claims.last().cloned());
    assert_eq!(SUBJECT_REDUCTIONS.with(std::cell::Cell::get), 0);
    store.status_at(Some(subject), None, Some(index)).unwrap();
    assert_eq!(SUBJECT_REDUCTIONS.with(std::cell::Cell::get), 0);
}

#[test]
fn forgetting_views_invalidates_every_status_reduction_mode() {
    let store = Store::open_memory("node").unwrap();
    observe_runtime(&store, "agent/cache/forget", "running", "one");
    let cut = store.index().unwrap();
    let modes = [
        ClaimsReduction::Full,
        ClaimsReduction::Summary,
        ClaimsReduction::Runtime,
        ClaimsReduction::AgentCard,
    ];
    let read = |mode| {
        store
            .status_for_subject_prefix_at("agent/", Some(cut), true, mode)
            .unwrap()
    };
    for mode in modes {
        read(mode);
    }
    SUBJECT_REDUCTIONS.with(|count| count.set(0));
    for mode in modes {
        read(mode);
    }
    assert_eq!(SUBJECT_REDUCTIONS.with(std::cell::Cell::get), 0);
    store.forget_current_views();
    for mode in modes {
        SUBJECT_REDUCTIONS.with(|count| count.set(0));
        read(mode);
        assert_eq!(SUBJECT_REDUCTIONS.with(std::cell::Cell::get), 1, "{mode:?}");
    }
}

mod canonical_audit {
    use super::*;

    /* sqlite-large-value: allow-for-now lv-hd283d49e24ac owner=st3_seats::store::tests TODO=trace-statement-and-value-representation */ include!("canonical_audit.rs");
}

mod resource_pending_receive_tests {
    use super::*;

    /* sqlite-large-value: allow-for-now lv-h932d39bf3e97 owner=st3_seats::store::tests TODO=trace-statement-and-value-representation */ include!("resource_pending_receive_tests.rs");
}
