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
fn batched_work_metadata_reduces_whole_read_statements_on_4200_steps() {
    let store = fixture(4200);
    let at = now_ms();
    let (first, first_work, first_time) = measured(|| read(&store, ACTOR, at).unwrap());
    assert_eq!(first.len(), 4200);
    parity(&store, ACTOR, at); // Warm both paths outside the cost scopes.
    let (old, old_work, old_time) = measured(|| oracle(&store, ACTOR, at).unwrap());
    let (new, new_work, new_time) = measured(|| read(&store, ACTOR, at).unwrap());
    assert_eq!(
        serde_json::to_value(new).unwrap(),
        serde_json::to_value(old).unwrap()
    );
    let batched_statements = 4200_usize.div_ceil(CHUNK) as u64 + 1;
    println!(
        "4200-step first candidate: {first_work:?} {first_time:?}; warm old: {old_work:?} {old_time:?}; warm new: {new_work:?} {new_time:?}"
    );
    MAX_SNAPSHOT_CALL_NS.with(|maximum| {
        println!(
            "maximum candidate snapshot-call upper bound: {} ns",
            maximum.get()
        )
    });
    assert!(new_work.statements > 0);
    assert_eq!(
        old_work.statements,
        new_work.statements + 8400 - batched_statements
    );
    assert!(
        new_work.vm_steps < old_work.vm_steps,
        "old={old_work:?} new={new_work:?}"
    );
}

#[test]
fn batched_work_metadata_plan_seeks_run_generation_root_and_definition() {
    let store = fixture(129);
    let connection = store.readers.get();
    for query in [OWNERS, DEFINITIONS] {
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
        } else {
            vec!["generations", "runs", "revisions"]
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
