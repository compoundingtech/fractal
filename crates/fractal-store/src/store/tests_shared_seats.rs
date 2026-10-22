#![allow(unused_imports)]
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::tests_shared::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::tests_shared::{TEST_FLEET, exchange_from, receive_and_project};
use crate::store::*;
use crate::graph::parse_test_intent as parse_intent;
use crate::store::shared_seats::*;
use crate::model::{ReplicaEnvelope, ReplicaEnvelopeId, ReplicationInventory};

pub fn roster_cache_store() -> Store {
    let store = Store::open_memory("node").unwrap();
    let source = "version 2\nagent \"amber\" { command \"true\" }\n";
    let intent = parse_intent(source, "node").unwrap();
    let plan = store
        .mission(
            &intent,
            crate::model::IntentInput {
                kdl: source.into(),
                source_name: None,
            },
        )
        .unwrap();
    store.apply(&intent, &plan.subject_tokens, "roster-cache").unwrap();
    store.append_claim(&local_observation_for_test("harness.observed", json!({
        "state":"idle", "driver":"codex", "incarnation_id":"amber-1", "observed_at_ms":1,
    }))).unwrap();
    store
}

pub fn local_observation_for_test(kind: &str, fields: Value) -> ClaimInput {
    ClaimInput {
        subject: "agent/node.amber".into(),
        kind: kind.into(),
        actor: Some("agent/node.amber".into()),
        fields: serde_json::from_value(fields).unwrap(),
        evidence: Vec::new(),
        expected_subject: None,
        idempotency_key: None,
    }
}

pub fn local_timeline_observation_for_test() -> ClaimInput {
    local_observation_for_test("harness.timeline", json!({
        "operation":"append", "entry_id":"local-activity", "source_id":"fixture/local-activity",
        "sequence":1, "revision":1, "role":"assistant", "entry_type":"content", "final":true,
        "driver":"codex", "incarnation_id":"amber-1",
        "observed_at_unix_ms":1_900_000_000_000_u64, "body":{"text":"local activity"},
    }))
}

pub fn simple(command: &str) -> NormalizedIntent {
    parse_intent(
        &format!("version 2\n exec \"work\" {{ command {command:?}; restart \"never\" }} "),
        "node",
    )
    .expect("intent")
}

pub fn publish_mission(store: &Store, source: &str, key: &str) -> MissionSpec {
    let intent = crate::graph::parse_intent(source, "node").expect("mission intent");
    let planned = store
        .mission(
            &intent,
            IntentInput {
                kdl: source.into(),
                source_name: None,
            },
        )
        .expect("mission preview");
    store
        .apply(&intent, &planned.subject_tokens, key)
        .expect("mission publish");
    intent
        .missions
        .values()
        .next()
        .expect("published mission")
        .clone()
}

pub fn observe_runtime(store: &Store, subject: &str, status: &str, incarnation: &str) {
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
                ("incarnation_id".into(), Value::String(incarnation.into())),
            ]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
}

pub fn query_plan(store: &Store, sql: &str, params: impl rusqlite::Params) -> Vec<String> {
    store
        .readers
        .get()
        .prepare(/* sqlite-large-value: allow-for-now lv-h63b64c5b049a owner=fractal_store::store::tests_shared_seats TODO=trace-statement-and-value-representation */ &format!("EXPLAIN QUERY PLAN {sql}"))
        .unwrap()
        .query_map(params, |row| row.get::<_, String>(3))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

/// Claims written as bare rows, about subjects no claim spec admits, and the store index of
/// the last one.
pub fn insert_raw_claims(store: &Store, claims: &[(&str, &str)]) -> u64 {
    let connection = store.connection.lock().unwrap();
    connection
        .execute_batch("PRAGMA foreign_keys = OFF")
        .unwrap();
    for (subject, kind) in claims {
        connection
            .execute(
                /* sqlite-large-value: allow-for-now lv-h54cdeb4d9314 owner=fractal_store::store::tests_shared_seats TODO=prepare-body-and-predecessor-values */ "INSERT INTO claims(id, batch_id, subject, kind, origin, actor, body,
                                        predecessors, accepted_at_unix_ms)
                     VALUES (?1, 'batch/raw', ?2, ?3, 'node', NULL, '{}', '[]', '1')",
                params![Uuid::now_v7().to_string(), subject, kind],
            )
            .unwrap();
    }
    connection
        .execute_batch("PRAGMA foreign_keys = ON")
        .unwrap();
    connection
        .query_row("SELECT MAX(store_index) FROM claims", [], |row| row.get(0))
        .unwrap()
}

/// Receive `source`'s new envelopes and project them, and say whether that replayed the
/// graph from nothing.
pub fn projection_replayed(target: &Store, relay: &str, source: &Store) -> bool {
    let exchange = exchange_from(source, &target.replication_inventory().unwrap());
    target
        .receive_replication_exchange(relay, TEST_FLEET, &exchange)
        .unwrap();
    target.validate_replication_backlog().unwrap();
    target.apply_replication_repairs().unwrap();
    FULL_REPLAYS.with(|replays| replays.set(0));
    assert!(target.project_replication_backlog().unwrap());
    FULL_REPLAYS.with(std::cell::Cell::get) > 0
}

/// Exchange from `controller` to `worker` until the worker holds every claim: an export
/// sends one bounded page, so a large tree takes several.
pub fn replicate_fully(controller: &Store, worker: &Store) {
    for _ in 0..64 {
        let exchange = exchange_from(controller, &worker.replication_inventory().unwrap());
        if exchange.envelopes.is_empty() {
            break;
        }
        receive_and_project(worker, "controller", &exchange);
    }
    assert_eq!(
        worker.replication_inventory().unwrap().digest,
        controller.replication_inventory().unwrap().digest,
        "the worker caught up"
    );
}

/// The shapes a standing tree holds besides children started from the root's steps, made on
/// the controller and replicated to the worker: a grandchild started from a child's step, a
/// run a subscription delivers straight under the root, and a person ask owned by the root
/// and one owned by a child.
pub struct TreeShapes {
    pub controller: Store,
    pub worker: Store,
    pub root: MissionRunView,
    pub children: Vec<MissionRunView>,
    pub grandchild: MissionRunView,
    pub delivered: MissionRunView,
    pub root_ask: StepRunView,
    pub child_ask: StepRunView,
}

/// The tree's runs: the root, its children, the grandchild, the delivered run and two asks.
pub const TREE_SHAPE_RUNS: usize = 1 + 4 + 1 + 1 + 2;

pub fn graph_digest_of(store: &Store) -> String {
    store
        .replication_status(true, Some(TEST_FLEET), &[])
        .unwrap()
        .graph_digest
}

pub fn timeline_observation(subject: &str, incarnation: &str, entry: &str) -> ClaimInput {
    ClaimInput {
        subject: subject.into(),
        kind: "harness.timeline".into(),
        actor: Some(subject.into()),
        fields: BTreeMap::from([
            ("operation".into(), Value::String("append".into())),
            ("entry_id".into(), Value::String(entry.into())),
            ("revision".into(), Value::from(1)),
            ("role".into(), Value::String("assistant".into())),
            ("entry_type".into(), Value::String("content".into())),
            ("final".into(), Value::Bool(true)),
            (
                "body".into(),
                json!({"media_type": "text/plain", "text": entry}),
            ),
            ("driver".into(), Value::String("codex".into())),
            ("incarnation_id".into(), Value::String(incarnation.into())),
            ("sequence".into(), Value::from(entry.len() as u64)),
        ]),
        evidence: Vec::new(),
        expected_subject: None,
        idempotency_key: Some(format!("timeline:{subject}:{incarnation}:{entry}")),
    }
}

pub const TAKEOVER_SOURCE: &str = r#"version 2

mission "takeover" state="ready" {
  goal "Take over one service."
  step "prepare" { }
  step "deploy-check" { depends-on { step "prepare" completed } }
  step "smoke-check" { depends-on { step "prepare" completed } }
  step "announce" { depends-on { step "deploy-check" completed } }
  finally { step "report" { agentless } }
}
"#;

pub fn publish_takeover(store: &Store, source: &str, key: &str) -> MissionSpec {
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
    store.apply(&intent, &planned.subject_tokens, key).unwrap();
    intent.missions["takeover"].clone()
}

pub fn takeover_step<'a>(run: &'a MissionRunView, path: &str) -> &'a StepRunView {
    run.steps.iter().find(|step| step.step == path).unwrap()
}

pub fn full_canonical_claim_id_oracle(
    connection: &Connection,
    subject: &str,
    at: Option<u64>,
) -> Vec<String> {
    connection
        .prepare(/* sqlite-large-value: allow-for-now lv-h2f4809cfd539 owner=fractal_store::store::tests_shared_seats TODO=trace-statement-and-value-representation */ &canonical_sql(
            "SELECT id FROM claims WHERE subject=?1 AND store_index<=?2
             ORDER BY CANONICAL_ASC(claims)",
        ))
        .unwrap()
        .query_map(params![subject, at.unwrap_or(i64::MAX as u64)], |row| {
            row.get::<_, String>(0)
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
}

/// One exchange carrying exactly `envelopes`, as a peer that holds only those would send it.
pub fn exchange_of(peer: &str, envelopes: Vec<ReplicaEnvelope>) -> ReplicationExchange {
    ReplicationExchange {
        build: None,
        claim_ranges: None,
        projection_digests: Default::default(),
        peer: peer.into(),
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
    }
}

pub fn harness_state(subject: &str, state: &str, observed_at_ms: u64) -> ClaimInput {
    ClaimInput {
        subject: subject.into(),
        kind: "harness.observed".into(),
        actor: Some(subject.into()),
        fields: BTreeMap::from([
            ("state".into(), Value::String(state.into())),
            ("incarnation_id".into(), Value::String("inc-1".into())),
            ("observed_at_ms".into(), Value::from(observed_at_ms)),
        ]),
        evidence: Vec::new(),
        expected_subject: None,
        idempotency_key: Some(format!("state:{subject}:{state}:{observed_at_ms}")),
    }
}
