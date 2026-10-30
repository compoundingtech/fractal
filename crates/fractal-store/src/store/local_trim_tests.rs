//! Local trim through Fractal's store: a member with `[trim] local` trims its own due cut with
//! the audited retention rules, and every reader answers the same from the full log and the
//! trimmed one.
use super::*;
use crate::store::checkpoint_agreement_tests_shared_attention_tests::{context, kinds, step};
use crate::store::tests_shared_seats::{TEST_FLEET, graph_digest_of};
use proptest::prelude::*;

const SEATS: &[&str] = &["agent/node.amber", "agent/node.birch"];
const STATES: &[&str] = &["starting", "ready", "idle", "working", "blocked"];

/// A member with both seats declared, so the seat readers have something to answer.
fn member(trims_locally: bool) -> Store {
    let store = Store::open_memory("node").unwrap();
    store.bind_fleet(TEST_FLEET).unwrap();
    let source = "version 2\nagent \"amber\" { command \"true\" }\nagent \"birch\" { command \"true\" }\n";
    let intent = crate::graph::parse_test_intent(source, "node").unwrap();
    let plan = store
        .mission(&intent, crate::model::IntentInput { kdl: source.into(), source_name: None })
        .unwrap();
    store.apply(&intent, &plan.subject_tokens, "local-trim").unwrap();
    store.import_retired_identities_paced(|_| {}).unwrap();
    store.trim_locally(trims_locally).unwrap();
    store
}

/// What `[trim] notify` sets at startup.
fn set_notify(store: &Store, agent: &str) {
    *store.extension::<crate::store::answers_check::TrimSettings>().notify.lock().unwrap() = Some(agent.into());
}

fn local_member(_name: &str) -> Store {
    member(true)
}

/// One write of a random history.
#[derive(Clone, Debug)]
enum Write {
    Observer { observer: u8, status: u8, cursor: Option<u8> },
    Resource { resource: u8, state: u8 },
    Harness { seat: u8, incarnation: u8, state: u8, restamp: bool, provider_auth: Option<bool> },
}

fn write() -> impl Strategy<Value = Write> {
    prop_oneof![
        (0..3u8, 0..3u8, proptest::option::of(0..4u8))
            .prop_map(|(observer, status, cursor)| Write::Observer { observer, status, cursor }),
        (0..3u8, 0..3u8).prop_map(|(resource, state)| Write::Resource { resource, state }),
        (0..2u8, 0..2u8, 0..5u8, any::<bool>(), proptest::option::of(any::<bool>())).prop_map(
            |(seat, incarnation, state, restamp, provider_auth)| Write::Harness {
                seat,
                incarnation,
                state,
                restamp,
                provider_auth,
            }
        ),
    ]
}

fn apply(store: &Store, write: &Write, n: usize) {
    let (subject, kind, fields) = match write {
        Write::Observer { observer, status, cursor } => {
            let mut fields = BTreeMap::from([(
                "status".into(),
                json!(["ok", "changed", "failed"][usize::from(*status)]),
            )]);
            if let Some(cursor) = cursor {
                fields.insert("cursor".into(), json!(format!("cursor-{cursor}")));
            }
            (format!("observer/watch-{observer}"), "observer.observed", fields)
        }
        // As an observer writes it: no actor, and its observer named.
        Write::Resource { resource, state } => {
            let fields = BTreeMap::from([
                ("kind".into(), json!("vcs.pull-request")),
                ("state".into(), json!(["open", "merged", "closed"][usize::from(*state)])),
                ("observer".into(), json!("observer/watch-0")),
            ]);
            (format!("resource/pr/{resource}"), "resource.observed", fields)
        }
        Write::Harness { seat, incarnation, state, restamp, provider_auth } => {
            let seat = SEATS[usize::from(*seat)];
            // The incarnation runs, so the seat readers fold its observations. The key makes
            // this one claim per incarnation.
            store
                .append_claim(&ClaimInput {
                    subject: seat.into(),
                    kind: "runtime.observed".into(),
                    actor: Some(seat.into()),
                    fields: BTreeMap::from([
                        ("status".into(), json!("running")),
                        ("runtime_id".into(), json!(seat.trim_start_matches("agent/"))),
                        ("incarnation_id".into(), json!(format!("{seat}-{incarnation}"))),
                    ]),
                    evidence: Vec::new(),
                    expected_subject: None,
                    idempotency_key: Some(format!("runtime:{seat}-{incarnation}")),
                })
                .unwrap();
            let mut fields = BTreeMap::from([
                ("state".into(), json!(STATES[usize::from(*state)])),
                ("driver".into(), json!("codex")),
                ("incarnation_id".into(), json!(format!("{seat}-{incarnation}"))),
                ("observed_at_ms".into(), json!(n as u64 + 1)),
                ("status_transition".into(), json!(!restamp)),
            ]);
            if let Some(provider_auth) = provider_auth {
                fields.insert("provider_auth".into(), json!(provider_auth));
            }
            (seat.to_owned(), "harness.observed", fields)
        }
    };
    store
        .append_claim(&ClaimInput {
            subject: subject.clone(),
            kind: kind.into(),
            actor: (kind == "harness.observed").then(|| subject.clone()),
            fields,
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: Some(format!("history-{n}")),
        })
        .unwrap();
}

/// Every answer the readers of these kinds give: the whole graph, each observer's and resource's
/// newest observation (what resources and reconcile read), each resource's observations newest
/// first (what a GitHub watch walks back to find when a pull request left the merge queue), each
/// seat's current harness, and each seat's status history.
fn answers(store: &Store, now: u128) -> (String, Vec<String>) {
    let connection = store.readers.get();
    let mut seats = Vec::new();
    for observer in 0..3 {
        let newest = store.latest_claim(&format!("observer/watch-{observer}"), Some("observer.observed")).unwrap();
        seats.push(format!("observer/watch-{observer}: {:?}", newest.map(|claim| claim.body)));
        let newest = store.latest_claim(&format!("resource/pr/{observer}"), Some("resource.observed")).unwrap();
        seats.push(format!("resource/pr/{observer}: {:?}", newest.map(|claim| claim.body)));
        let walked = connection
            .prepare(
                "SELECT accepted_at_unix_ms, json_extract(body, '$.fields.state') FROM claims
                 WHERE subject=?1 AND kind='resource.observed'
                 ORDER BY length(accepted_at_unix_ms) DESC, accepted_at_unix_ms DESC, store_index DESC",
            )
            .unwrap()
            .query_map([format!("resource/pr/{observer}")], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        seats.push(format!("resource/pr/{observer} walked back: {walked:?}"));
    }
    for seat in SEATS {
        let harness = store.current_harness(seat).unwrap();
        let history = seat_status::history_at(&connection, seat, now, i64::MAX as u64).unwrap();
        seats.push(format!("{seat}: {harness:?} {history}"));
    }
    (graph_digest_of(store), seats)
}

fn claims_of(store: &Store, kind: &str) -> i64 {
    store
        .readers
        .get()
        .query_row("SELECT COUNT(*) FROM claims WHERE kind=?1", [kind], |row| row.get(0))
        .unwrap()
}

#[test]
fn a_member_that_trims_locally_trims_its_own_due_cut_once() {
    let scratch = tempfile::tempdir().unwrap();
    let context = context(scratch.path(), 0);
    let store = local_member("alder");
    for n in 0..40 {
        apply(&store, &Write::Observer { observer: (n % 2) as u8, status: (n % 3) as u8, cursor: Some(1) }, n);
    }
    for n in 40..50 {
        apply(&store, &Write::Resource { resource: 0, state: (n % 3) as u8 }, n);
    }
    let before = answers(&store, context.now_unix_ms);
    assert_eq!(kinds(&step(&store, &context)), ["trimmed"]);
    assert_eq!(answers(&store, context.now_unix_ms), before, "the readers answer the same");
    assert_eq!(claims_of(&store, "observer.observed"), 2, "the newest of each observer stays");
    assert_eq!(claims_of(&store, "resource.observed"), 10, "resource observations stay whole");
    let retired: i64 = store
        .readers
        .get()
        .query_row("SELECT COUNT(*) FROM retired_claim_ids", [], |row| row.get(0))
        .unwrap();
    assert!(retired > 0, "what it deleted is retired, so it travels and is never re-admitted");
    // The same day again: nothing more to do, and no staging left behind.
    assert!(step(&store, &context).is_empty());
    assert_eq!(std::fs::read_dir(scratch.path()).unwrap().filter(|entry| {
        entry.as_ref().unwrap().file_name().to_string_lossy().starts_with("capture-")
    }).count(), 0);
}

#[test]
fn an_ordinary_member_does_not_trim_locally() {
    let scratch = tempfile::tempdir().unwrap();
    let context = context(scratch.path(), 0);
    let store = member(false);
    for n in 0..10 {
        apply(&store, &Write::Observer { observer: 0, status: 0, cursor: None }, n);
    }
    assert!(!kinds(&step(&store, &context)).contains(&"trimmed"));
    assert_eq!(claims_of(&store, "observer.observed"), 10);
}

#[test]
fn a_plan_that_would_drop_the_newest_claim_of_a_key_stops_before_deleting_anything() {
    let scratch = tempfile::tempdir().unwrap();
    let context = context(scratch.path(), 0);
    let store = local_member("alder");
    for n in 0..6 {
        apply(&store, &Write::Observer { observer: 0, status: 0, cursor: None }, n);
    }
    let claims = claims_of(&store, "observer.observed");
    set_notify(&store, "agent/example/operations");
    store.set_trim_fault(Some(smallclaims::store::TrimFault::DropNewest));
    let actions = step(&store, &context);
    assert_eq!(kinds(&actions), ["graph-changed"]);
    assert_eq!(claims_of(&store, "observer.observed"), claims, "nothing is deleted");
    // The stop alerts the `[trim] notify` agent, once however often it is recorded.
    store.record_local_trim_stops(&actions).unwrap();
    store.record_local_trim_stops(&actions).unwrap();
    let faults = || -> Vec<(String, String)> {
        store
            .readers
            .get()
            .prepare(
                "SELECT kind, json_extract(body,'$.fields.reviewer') FROM claims
                 WHERE subject='daemon/node' AND kind IN ('operational.failure','operational.recovered')
                   AND json_extract(body,'$.fields.episode') LIKE 'local-trim-stopped/%'
                 ORDER BY store_index",
            )
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get::<_, Option<String>>(1)?.unwrap_or_default())))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    };
    assert_eq!(faults(), [("operational.failure".to_string(), "agent/example/operations".to_string())]);
    // Halted until resumed. Only the `[trim] notify` agent or a person may resume, and the
    // resume ends the fault.
    assert!(step(&store, &context).is_empty());
    assert!(store.resume_stopped_trims("agent/example/other", "looked at the plan").is_err());
    assert!(store.resume_stopped_trims("agent/example/operations", " ").is_err());
    assert_eq!(faults().len(), 1, "a refused resume leaves the fault open");
    assert!(step(&store, &context).is_empty(), "a refused resume leaves the member halted");
    store.resume_stopped_trims("agent/example/operations", "looked at the plan").unwrap();
    assert_eq!(faults().last().unwrap().0, "operational.recovered");
    assert!(step(&store, &context).is_empty(), "the stopped cut is set aside, not retried");
}

#[test]
fn only_a_person_or_the_notify_agent_resumes_a_member_that_trims_locally() {
    let store = local_member("alder");
    // With no `[trim] notify` agent, only a person may.
    assert!(store.resume_stopped_trims("agent/example/operations", "looked").is_err());
    store.resume_stopped_trims("person/operator", "looked").unwrap();
    set_notify(&store, "agent/example/operations");
    assert!(store.resume_stopped_trims("agent/example/other", "looked").is_err());
    store.resume_stopped_trims("agent/example/operations", "looked").unwrap();
    store.resume_stopped_trims("person/operator", "looked").unwrap();
    // An ordinary member: a person only, whatever `[trim] notify` names.
    let ordinary = member(false);
    set_notify(&ordinary, "agent/example/operations");
    assert!(ordinary.resume_stopped_trims("agent/example/operations", "looked").is_err());
    ordinary.resume_stopped_trims("person/operator", "looked").unwrap();
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 24, ..ProptestConfig::default() })]

    /// For random histories of observer and harness observations, every reader answers the same
    /// before and after the local trim, and the newest claim of each subject and kind stays.
    #[test]
    fn readers_answer_the_same_from_the_full_and_the_trimmed_log(
        writes in proptest::collection::vec(write(), 1..60)
    ) {
        let scratch = tempfile::tempdir().unwrap();
        let context = context(scratch.path(), 0);
        let store = local_member("alder");
        for (n, write) in writes.iter().enumerate() {
            apply(&store, write, n);
        }
        let newest: BTreeSet<String> = store
            .readers
            .get()
            .prepare(
                "SELECT id FROM claims AS claim WHERE kind IN ('observer.observed', 'resource.observed', 'harness.observed')
                 AND store_index=(SELECT MAX(store_index) FROM claims AS newer
                                  WHERE newer.subject=claim.subject AND newer.kind=claim.kind)",
            )
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        let before = answers(&store, context.now_unix_ms);
        let actions = kinds(&step(&store, &context));
        prop_assert!(!actions.contains(&"graph-changed"), "{actions:?}");
        prop_assert_eq!(answers(&store, context.now_unix_ms), before);
        for id in &newest {
            prop_assert!(store.claim_by_id(id).unwrap().is_some(), "the newest claim {} stays", id);
        }
    }
}

/// The audited `harness.observed` rule drops a credential transition whose loss moves the
/// `since` of a needs-login seat (ready without provider auth, then with, then without again),
/// which no projection digest covers. Until a rule keeps it, local trim leaves the kind whole.
#[test]
fn local_trim_leaves_harness_observations_whole() {
    let scratch = tempfile::tempdir().unwrap();
    let context = context(scratch.path(), 0);
    let store = local_member("alder");
    let history = [(1, false), (0, true), (1, false)];
    for (n, (state, provider_auth)) in history.into_iter().enumerate() {
        let write = Write::Harness { seat: 0, incarnation: 0, state, restamp: false, provider_auth: Some(provider_auth) };
        apply(&store, &write, n);
    }
    for n in 3..20 {
        apply(&store, &Write::Observer { observer: 0, status: 0, cursor: None }, n);
    }
    let harness = claims_of(&store, "harness.observed");
    let before = answers(&store, context.now_unix_ms);
    assert_eq!(kinds(&step(&store, &context)), ["trimmed"]);
    assert_eq!(claims_of(&store, "harness.observed"), harness);
    assert_eq!(claims_of(&store, "observer.observed"), 1);
    assert_eq!(answers(&store, context.now_unix_ms), before);
}

/// A GitHub watch finds when a pull request left the merge queue by walking the request's
/// observations back to the watch's last wake, which can be before the cut. The audited
/// `resource.observed` rule keeps only the newest, so local trim leaves the kind whole until a
/// rule keeps that history.
#[test]
fn local_trim_leaves_resource_observations_whole() {
    let scratch = tempfile::tempdir().unwrap();
    let context = context(scratch.path(), 0);
    let store = local_member("alder");
    for (n, state) in [0, 0, 2, 0].into_iter().enumerate() {
        apply(&store, &Write::Resource { resource: 0, state }, n);
    }
    for n in 4..20 {
        apply(&store, &Write::Observer { observer: 0, status: 0, cursor: None }, n);
    }
    let before = answers(&store, context.now_unix_ms);
    assert_eq!(kinds(&step(&store, &context)), ["trimmed"]);
    assert_eq!(claims_of(&store, "resource.observed"), 4);
    assert_eq!(claims_of(&store, "observer.observed"), 1);
    assert_eq!(answers(&store, context.now_unix_ms), before);
}

/// One step of a history for the capture equivalence test: a write, or something that makes a
/// guard of the local cut answer for an observer claim.
#[derive(Clone, Debug)]
enum Step {
    Write(Write),
    /// A claim of another kind whose evidence cites the `nth` observer claim so far.
    Cite(u8),
    /// A mission definition naming the `nth` observer claim, which protects it.
    Protect(u8),
    /// A repair naming the `nth` observer claim its replacement, by claim or by record.
    Repair(u8, bool),
    /// An observer claim and a claim of another kind in one operation, in two envelopes.
    Shared(u8),
}

fn step_strategy() -> impl Strategy<Value = Step> {
    prop_oneof![
        6 => write().prop_map(Step::Write),
        1 => any::<u8>().prop_map(Step::Cite),
        1 => any::<u8>().prop_map(Step::Protect),
        1 => (any::<u8>(), any::<bool>()).prop_map(|(nth, by_claim)| Step::Repair(nth, by_claim)),
        1 => (0..3u8).prop_map(Step::Shared),
    ]
}

fn take_step(store: &Store, step: &Step, n: usize) {
    let observed = || -> Vec<String> {
        store
            .readers
            .get()
            .prepare("SELECT id FROM claims WHERE kind='observer.observed' ORDER BY store_index")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    };
    let nth = |nth: u8| {
        let ids = observed();
        (!ids.is_empty()).then(|| ids[usize::from(nth) % ids.len()].clone())
    };
    match step {
        Step::Write(write) => apply(store, write, n),
        Step::Cite(k) => {
            let Some(id) = nth(*k) else { return };
            store
                .append_claim(&ClaimInput {
                    subject: "resource/pr/9".into(),
                    kind: "resource.observed".into(),
                    actor: None,
                    fields: BTreeMap::from([
                        ("kind".into(), json!("vcs.pull-request")),
                        ("state".into(), json!("open")),
                        ("observer".into(), json!("observer/watch-0")),
                    ]),
                    evidence: vec![id],
                    expected_subject: None,
                    idempotency_key: Some(format!("history-{n}")),
                })
                .unwrap();
        }
        Step::Protect(k) => {
            let Some(id) = nth(*k) else { return };
            store
                .connection
                .write()
                .execute(
                    "INSERT OR REPLACE INTO mission_definitions(mission_id, revision, state, claim_id)
                     VALUES (?1, 'r', 'active', ?2)",
                    params![format!("mission/protects-{n}"), id],
                )
                .unwrap();
        }
        Step::Repair(k, true) => {
            let Some(id) = nth(*k) else { return };
            store
                .append_claim(&ClaimInput {
                    subject: format!("repair/test-{n}"),
                    kind: "record.repaired".into(),
                    actor: None,
                    fields: BTreeMap::from([
                        ("record".into(), json!(format!("record/original-{n}"))),
                        ("replacement".into(), json!(id)),
                        ("reason".into(), json!("a test repair")),
                    ]),
                    evidence: Vec::new(),
                    expected_subject: None,
                    idempotency_key: Some(format!("history-{n}")),
                })
                .unwrap();
        }
        Step::Repair(k, false) => {
            let Some(id) = nth(*k) else { return };
            store
                .connection
                .write()
                .execute(
                    "INSERT INTO replica_records(record_ref, writer, sequence, envelope_hash, position, raw,
                         state, claim_id, replacement_claim_id, updated_at_unix_ms)
                     VALUES (?1, 'node.elsewhere', ?2, 'repaired', 0, X'', 'repaired', ?3, ?4, '1')",
                    params![format!("record/repaired-{n}"), n as i64, format!("claim/original-{n}"), id],
                )
                .unwrap();
        }
        Step::Shared(observer) => {
            let operation = json!({"id": format!("operation/shared-{n}"), "request_digest": format!("digest-{n}")});
            for (subject, kind, fields) in [
                (format!("observer/watch-{observer}"), "observer.observed", json!({"status": "ok"})),
                (
                    format!("resource/pr/{observer}"),
                    "resource.observed",
                    json!({"kind": "vcs.pull-request", "state": "open", "observer": format!("observer/watch-{observer}")}),
                ),
            ] {
                // One transaction each: two batches, so two envelopes.
                let mut connection = store.connection.write();
                let transaction = connection.transaction().unwrap();
                append_claim_tx(
                    &transaction,
                    &store.origin,
                    &subject,
                    kind,
                    None,
                    &json!({"fields": fields, "_operation": operation}),
                    &[],
                    None,
                )
                .unwrap();
                transaction.commit().unwrap();
            }
        }
    }
}

/// Stage `store` at `cut` as a local cut does, fully or scoped, and plan it.
fn local_plan(store: &Store, cut: u128, dir: &std::path::Path, scoped: bool) -> (Vec<String>, Vec<String>, Vec<String>) {
    let kinds = smallclaims::store::LOCAL_TRIM_KINDS;
    let staged = store.checkpoint_staged_set_scoped(cut, None, dir, scoped.then_some(kinds)).unwrap();
    staged.protect_all_but(kinds).unwrap();
    smallclaims::store::checkpoint_staging::without_reference_plan(|| store.plan_staged_checkpoint(&staged)).unwrap();
    let claims = staged.dropped_claims(0, usize::MAX >> 1).unwrap().into_iter().map(|claim| claim.id).collect();
    let envelopes = staged
        .dropped_envelopes(0, usize::MAX >> 1)
        .unwrap()
        .into_iter()
        .map(|envelope| format!("{}/{}/{}", envelope.writer, envelope.sequence, envelope.envelope_hash))
        .collect();
    (claims, envelopes, staged.guard_answers(kinds).unwrap())
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 32, ..ProptestConfig::default() })]

    /// For random histories, a local cut's scoped capture plans the same drop as a full capture
    /// restricted to the kinds it drops, and every guard answers the same for each claim it
    /// could drop: newest of slot, writer's newest envelope, operations, envelopes, protections,
    /// repairs and citations.
    #[test]
    fn a_scoped_capture_plans_what_a_full_one_plans(
        steps in proptest::collection::vec(step_strategy(), 1..60)
    ) {
        let scratch = tempfile::tempdir().unwrap();
        let context = context(scratch.path(), 0);
        let store = local_member("alder");
        for (n, step) in steps.iter().enumerate() {
            take_step(&store, step, n);
        }
        store.seal_local_batches().unwrap();
        let cut = smallclaims::store::checkpoint::newest_due_cut(context.now_unix_ms);
        let full = local_plan(&store, cut, &scratch.path().join("full"), false);
        let scoped = local_plan(&store, cut, &scratch.path().join("scoped"), true);
        prop_assert_eq!(&scoped.0, &full.0, "dropped claims");
        prop_assert_eq!(&scoped.1, &full.1, "dropped envelopes");
        prop_assert_eq!(&scoped.2, &full.2, "guard answers");
    }
}
