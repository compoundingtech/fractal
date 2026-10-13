use super::*;
use smallclaims::sqlite::work::{SqliteWork, SqliteWorkScope};
use std::time::Instant;

const ACTOR: &str = "agent/alder.worker";
const SEED: u64 = 0x2051_2481_2511;

fn fixture(count: usize) -> Store {
    let store = Store::open_memory("alder").unwrap();
    if count > 0 {
        add_run(&store, count, "batch-work");
    }
    store
}

fn add_run(store: &Store, count: usize, name: &str) {
    let steps = (0..count)
        .map(|n| {
            format!("step \"task-{n:05}\" {{ assigned-to \"{ACTOR}\"; goal \"Check item {n}.\"; }}")
        })
        .collect::<Vec<_>>()
        .join("\n");
    let text = format!(
        "version 2\nmission \"{name}\" state=\"ready\" {{ goal \"Process work.\"; {steps} }}"
    );
    let intent = crate::graph::parse_internal_intent(&text, "alder").unwrap();
    store
        .apply_internal(&intent, &format!("{name}-definition"))
        .unwrap();
    store
        .create_mission_run(&MissionRunRequest {
            mission: name.into(),
            revision: None,
            workspace: "/tmp".into(),
            requester: Some(ACTOR.into()),
            mode: Some("run".into()),
            inputs: BTreeMap::new(),
            idempotency_key: format!("{name}-run"),
        })
        .unwrap();
}

fn oracle(store: &Store, actor: &str, at: u128) -> Result<Vec<StepRunView>> {
    // Exact existing per-step path, including owner/definition queries and all enrichments.
    store.read_snapshot(|_| store.work_at_snapshot_internal(Some(actor), true, at, false))
}

fn parity(store: &Store, actor: &str, at: u128) -> Vec<StepRunView> {
    let old = oracle(store, actor, at).unwrap();
    let new = read(store, actor, at).unwrap();
    assert_eq!(
        serde_json::to_value(&new).unwrap(),
        serde_json::to_value(&old).unwrap()
    );
    new
}

fn measured<T>(read: impl FnOnce() -> T) -> (T, SqliteWork, std::time::Duration) {
    let started = Instant::now();
    let scope = SqliteWorkScope::start();
    let value = read();
    (value, scope.finish(), started.elapsed())
}

#[test]
fn batched_work_metadata_matches_every_boundary_and_seeded_effective_state() {
    let at = now_ms();
    for count in [0, 1, 127, 128, 129, 256] {
        let store = fixture(count);
        let baseline = parity(&store, ACTOR, at);
        assert_eq!(baseline.len(), count);
        let mut seed = SEED;
        {
            let connection = store.connection.write();
            for view in &baseline {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                let status = [
                    "pending",
                    "ready",
                    "claimed",
                    "working",
                    "verifying",
                    "blocked",
                    "waiting-person",
                    "completed",
                    "failed",
                    "cancelled",
                ][seed as usize % 10];
                let expiry = if seed & 1 == 0 {
                    at.saturating_sub(1)
                } else {
                    at + 60_000
                };
                connection
                    .execute(
                        "UPDATE step_runs SET status=?2,lease_owner=?3,
                    lease_incarnation='worker-one',lease_expires_at_unix_ms=?4,
                    blocked_reason=?5 WHERE subject=?1",
                        params![
                            view.subject,
                            status,
                            ACTOR,
                            expiry.to_string(),
                            (seed & 2 == 0).then_some("check blockers")
                        ],
                    )
                    .unwrap();
            }
        }
        parity(&store, ACTOR, at);
        parity(&store, ACTOR, at + 60_001);
        for (column, value) in [
            ("phase", "revision-draining"),
            ("phase", "terminal"),
            ("status", "completed"),
        ] {
            store
                .connection
                .write()
                .execute(&format!("UPDATE mission_runs SET {column}=?1"), [value])
                .unwrap();
            parity(&store, ACTOR, at);
        }
    }
    println!("seeded full-vector parity seed={SEED:#x}");
}

#[test]
fn batched_work_metadata_keeps_missing_rows_duplicates_and_errors() {
    let store = fixture(2);
    add_run(&store, 1, "other-work");
    let at = now_ms();
    let original = parity(&store, ACTOR, at);
    let mut requests = vec![original[0].clone(); 129];
    requests[128] = original.last().unwrap().clone();
    requests[126].run = "mission-run/missing".into();
    requests[127].generation = "run-generation/missing".into();
    let connection = store.readers.get();
    let actual = owners(&connection, &requests).unwrap();
    for (view, owner) in requests.iter().zip(actual) {
        let mut old = view.clone();
        let mut new = view.clone();
        apply_effective_step_state(&connection, &mut old, at).unwrap();
        apply_effective_step_state_with_owner(&connection, &mut new, at, owner).unwrap();
        assert_eq!(
            serde_json::to_value(old).unwrap(),
            serde_json::to_value(new).unwrap()
        );
    }
    for count in [0, 1, 127, 128, 129, 256] {
        let boundary = (0..count)
            .map(|n| {
                let mut view = original[0].clone();
                view.run = format!("mission-run/missing-{n}");
                view
            })
            .collect::<Vec<_>>();
        let selected = owners(&connection, &boundary).unwrap();
        assert_eq!(selected.len(), count);
        assert!(selected.into_iter().all(|owner| owner.is_none()));
    }
    drop(connection);
    let old_body: String = store
        .readers
        .get()
        .query_row("SELECT body FROM mission_revisions", [], |r| r.get(0))
        .unwrap();
    store
        .connection
        .write()
        .execute("UPDATE mission_revisions SET body='not a mission'", [])
        .unwrap();
    for view in &original {
        let error = enrich_step_definition(&store.readers.get(), &mut view.clone()).unwrap_err();
        let result = read(&store, ACTOR, at);
        assert!(
            result.is_err(),
            "a malformed pinned definition cannot return a partial vector"
        );
        assert_eq!(result.unwrap_err().to_string(), error.to_string());
    }
    store
        .connection
        .write()
        .execute("UPDATE mission_revisions SET body=?1", [old_body])
        .unwrap();
    parity(&store, ACTOR, at);
    store
        .connection
        .write()
        .execute("DELETE FROM mission_revisions", [])
        .unwrap();
    parity(&store, ACTOR, at);
}

#[test]
fn batched_work_metadata_reads_new_owner_and_definition_on_each_call() {
    let store = fixture(2);
    let at = now_ms();
    let before = parity(&store, ACTOR, at);
    let connection = store.connection.write();
    let body: String = connection
        .query_row("SELECT body FROM mission_revisions", [], |r| r.get(0))
        .unwrap();
    let mut mission: MissionSpec = serde_json::from_str(&body).unwrap();
    for step in mission.steps.values_mut() {
        step.fresh_context = true;
        step.timeout_ms = Some(12345);
    }
    connection
        .execute(
            "UPDATE mission_revisions SET body=?1",
            [serde_json::to_string(&mission).unwrap()],
        )
        .unwrap();
    drop(connection);
    let changed = parity(&store, ACTOR, at);
    assert_ne!(
        serde_json::to_value(before).unwrap(),
        serde_json::to_value(&changed).unwrap()
    );
    assert!(
        changed
            .iter()
            .all(|view| view.fresh_context && view.timeout_ms == Some(12345))
    );
    store
        .connection
        .write()
        .execute("UPDATE run_generations SET status='completed'", [])
        .unwrap();
    assert!(
        parity(&store, ACTOR, at)
            .iter()
            .all(|view| view.status == "cancelled")
    );
}

#[test]
fn batched_work_metadata_keeps_real_person_blockers_and_responses() {
    let (store, _origin, input) = person_work::tests::fixture();
    let before = parity(&store, &input.actor, now_ms());
    let ask = store.ask_person(&input).unwrap();
    let blocked = parity(&store, &input.actor, now_ms());
    assert_ne!(
        serde_json::to_value(before).unwrap(),
        serde_json::to_value(blocked).unwrap()
    );
    store
        .finish_person_step(
            &crate::model::PersonStepResponse {
                delegation: None,
                subject: ask.subject,
                actor: "person/avery".into(),
                summary: "Friday".into(),
                evidence: Vec::new(),
                episode: None,
                answer: None,
                idempotency_key: "batch-person-done".into(),
            },
            false,
        )
        .unwrap();
    let answered = parity(&store, &input.actor, now_ms());
    assert!(answered.iter().any(|view| !view.person_answers.is_empty()));
}

#[test]
fn batched_work_metadata_reduces_whole_read_statements_on_128_1000_4200_steps() {
    for count in [128_usize, 1000, 4200] {
        let store = fixture(count);
        let at = now_ms();
        MAX_SNAPSHOT_CALL_NS.with(|maximum| maximum.set(0));
        let (first_old, first_old_work, first_old_time) =
            measured(|| oracle(&store, ACTOR, at).unwrap());
        let (first_new, first_new_work, first_new_time) =
            measured(|| read(&store, ACTOR, at).unwrap());
        assert_eq!(
            serde_json::to_value(first_new).unwrap(),
            serde_json::to_value(first_old).unwrap()
        );
        parity(&store, ACTOR, at);
        let (old, old_work, old_time) = measured(|| oracle(&store, ACTOR, at).unwrap());
        let (new, new_work, new_time) = measured(|| read(&store, ACTOR, at).unwrap());
        assert_eq!(
            serde_json::to_value(new).unwrap(),
            serde_json::to_value(old).unwrap()
        );
        // This fixture has one exact run/generation pair and one pinned definition.
        let batched_statements = 2;
        println!(
            "{count}-step first calls (after fixture setup, not cold database): old {first_old_work:?} {first_old_time:?}; new {first_new_work:?} {first_new_time:?}; warm old: {old_work:?} {old_time:?}; warm new: {new_work:?} {new_time:?}"
        );
        MAX_SNAPSHOT_CALL_NS.with(|maximum| {
            println!(
                "{count}-step maximum candidate snapshot-call upper bound: {} ns",
                maximum.get()
            )
        });
        assert!(new_work.statements > 0);
        assert_eq!(
            old_work.statements,
            new_work.statements + 2 * count as u64 - batched_statements
        );
        assert!(
            new_work.vm_steps < old_work.vm_steps,
            "old={old_work:?} new={new_work:?}"
        );
    }
}

#[test]
fn batched_work_metadata_plan_seeks_run_generation_root_and_definition() {
    let store = fixture(129);
    let connection = store.readers.get();
    for query in [OWNERS, DEFINITIONS, PHASES] {
        let plan = connection
            .prepare(&format!("EXPLAIN QUERY PLAN {query}"))
            .unwrap()
            .query_map(["[]"], |row| row.get::<_, String>(3))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        println!("{query}\n{plan:#?}");
        assert!(
            plan.iter()
                .any(|line| line.starts_with("SCAN requested VIRTUAL TABLE")),
            "{plan:?}"
        );
        for alias in if query == OWNERS {
            vec!["runs", "generations", "roots"]
        } else if query == DEFINITIONS {
            vec!["generations", "runs", "revisions"]
        } else {
            vec!["mission_runs"]
        } {
            assert!(
                plan.iter()
                    .any(|line| line.starts_with(&format!("SEARCH {alias} "))),
                "{plan:?}"
            );
        }
        assert!(
            plan.iter().all(|line| !line.starts_with("SCAN ")
                || line.starts_with("SCAN requested VIRTUAL TABLE")),
            "{plan:?}"
        );
        assert!(
            plan.iter().all(|line| !line.contains("TEMP B-TREE")),
            "{plan:?}"
        );
    }
}

#[test]
fn batched_work_metadata_keeps_seeded_real_handoff_carry_and_supersession() {
    use crate::model::{WorkAcknowledgeRequest, WorkHandoffRequest};
    const PEER: &str = "agent/alder.peer";
    let store = fixture(16);
    let agents = crate::graph::parse_internal_intent(
        "version 2\nagent \"alder.worker\" { workspace \"/tmp\"; command \"true\" }\nagent \"alder.peer\" { workspace \"/tmp\"; command \"true\" }", "alder").unwrap();
    store.apply_internal(&agents, "batch-actors").unwrap();
    let initial = parity(&store, ACTOR, now_ms());
    for view in &initial {
        store.set_step_state(&view.subject, "ready", None).unwrap();
    }
    let pick = SEED as usize % initial.len();
    let carried = &initial[pick];
    let handed = &initial[(pick + 1) % initial.len()];
    let claim = |actor: &str, key: &str| WorkRequest {
        actor: Some(actor.into()),
        incarnation: Some(format!("{actor}-one")),
        summary: None,
        reason: None,
        evidence: Vec::new(),
        idempotency_key: key.into(),
    };
    store
        .work_action(&carried.subject, "claim", &claim(ACTOR, "carry-claim"))
        .unwrap();
    store
        .work_action(&handed.subject, "claim", &claim(ACTOR, "handoff-claim"))
        .unwrap();
    for (from, to, key) in [(ACTOR, PEER, "to-peer"), (PEER, ACTOR, "to-worker")] {
        store
            .handoff_work(
                &handed.subject,
                &WorkHandoffRequest {
                    actor: from.into(),
                    incarnation: Some(format!("{from}-one")),
                    to: to.into(),
                    note: "Review the same work next.".into(),
                    evidence: Vec::new(),
                    idempotency_key: key.into(),
                },
            )
            .unwrap();
        let source = store
            .latest_claim(&handed.subject, Some("work.released"))
            .unwrap()
            .unwrap();
        parity(&store, ACTOR, now_ms());
        parity(&store, PEER, now_ms());
        store
            .acknowledge_work(
                &handed.subject,
                &WorkAcknowledgeRequest {
                    actor: to.into(),
                    message: source.body["fields"]["handoff_message"]
                        .as_str()
                        .unwrap()
                        .into(),
                },
            )
            .unwrap();
        if to == PEER {
            store
                .work_action(&handed.subject, "claim", &claim(PEER, "peer-claim"))
                .unwrap();
        }
    }
    let handoff_view = parity(&store, ACTOR, now_ms());
    assert!(
        handoff_view
            .iter()
            .find(|view| view.subject == handed.subject)
            .unwrap()
            .constraints
            .iter()
            .any(|line| line.contains("Handoff from"))
    );
    // Explicit draining control: a claimed step remains while its adjacent ready step leaves.
    let original_phase: String = store
        .readers
        .get()
        .query_row("SELECT phase FROM mission_runs", [], |row| row.get(0))
        .unwrap();
    store
        .connection
        .write()
        .execute("UPDATE mission_runs SET phase='revision-draining'", [])
        .unwrap();
    let draining = parity(&store, ACTOR, now_ms());
    assert!(draining.iter().any(|view| view.subject == carried.subject));
    assert!(draining.iter().all(|view| view.status != "ready"));
    store
        .connection
        .write()
        .execute("UPDATE mission_runs SET phase=?1", [&original_phase])
        .unwrap();
    // Admission of an unreported verifying state retains the original claim/lease;
    // ordinary revision adoption emits its carried-claimant evidence.
    store
        .set_step_state(&carried.subject, "verifying", None)
        .unwrap();
    let steps = (0..16)
        .map(|n| {
            format!("step \"task-{n:05}\" {{ assigned-to \"{ACTOR}\"; goal \"Check item {n}.\"; }}")
        })
        .collect::<Vec<_>>()
        .join("\n");
    let changed = crate::graph::parse_internal_intent(&format!("version 2\nmission \"batch-work\" state=\"ready\" {{ goal \"Process work.\"; constraint \"Retain completed work.\"; {steps} }}"), "alder").unwrap();
    store
        .apply_internal(&changed, "batch-new-revision")
        .unwrap();
    let revised = store
        .adopt_mission_revision(
            &carried.run,
            &changed.missions["batch-work"],
            ACTOR,
            "Update guidance",
            "batch-adoption",
        )
        .unwrap();
    assert_ne!(revised.generation, carried.generation);
    let views = parity(&store, ACTOR, now_ms());
    assert!(
        views
            .iter()
            .all(|view| view.generation == revised.generation)
    );
    assert!(
        views
            .iter()
            .any(|view| view.carried_claimant.as_deref() == Some(ACTOR))
    );
    println!("real handoff and revision/carry parity seed={SEED:#x}");
}

#[test]
fn batched_work_metadata_snapshot_survives_second_writer_and_next_call_sees_claim() {
    let root = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(&root.path().join("claims.sqlite3"), "alder").unwrap());
    add_run(&store, 2, "batch-work");
    let before = parity(&store, ACTOR, now_ms());
    let original = store
        .readers
        .get()
        .query_row("SELECT body FROM mission_revisions", [], |r| {
            r.get::<_, String>(0)
        })
        .unwrap();
    let mut mission: MissionSpec = serde_json::from_str(&original).unwrap();
    for step in mission.steps.values_mut() {
        step.fresh_context = true;
    }
    let updated = serde_json::to_string(&mission).unwrap();
    let subject = before[0].subject.clone();
    let (enter, entered) = std::sync::mpsc::sync_channel(1);
    let (done, completed) = std::sync::mpsc::sync_channel(1);
    let writer_store = Arc::clone(&store);
    let writer = std::thread::spawn(move || {
        entered.recv().unwrap();
        {
            let connection = writer_store.connection.write();
            connection
                .execute("UPDATE mission_revisions SET body=?1", [updated])
                .unwrap();
            connection
                .execute("UPDATE mission_runs SET phase='revision-draining'", [])
                .unwrap();
        }
        // An actual admitted state claim also lands while the older read snapshot is pinned.
        writer_store
            .set_step_state(&subject, "completed", Some("Completed on a second thread"))
            .unwrap();
        done.send(()).unwrap();
    });
    AFTER_OWNERS.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move || {
            enter.send(()).unwrap();
            completed
                .recv_timeout(std::time::Duration::from_secs(5))
                .expect("writer waited on the read snapshot");
        }))
    });
    let captured = read(&store, ACTOR, now_ms());
    writer.join().unwrap();
    assert_eq!(
        serde_json::to_value(captured.unwrap()).unwrap(),
        serde_json::to_value(&before).unwrap()
    );
    let after = parity(&store, ACTOR, now_ms());
    assert!(
        after.is_empty(),
        "new draining state drops every non-claimed step"
    );
    let state = store
        .latest_claim(&before[0].subject, Some("step-run.state"))
        .unwrap()
        .unwrap();
    assert_eq!(state.body["fields"]["status"], "completed");
}
