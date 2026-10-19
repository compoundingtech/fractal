#![allow(unused_imports)]
use crate::store::*;
use crate::graph::parse_test_intent;
use std::fs;
use std::path::PathBuf;

pub const FLEET: &str = "5b4b1a8e-7c3d-4e2f-9a1b-0c6d5e4f3a2b";

pub const PERSON: &str = "person/avery";

/// SplitMix64: small, seeded, and the same on every platform.
pub struct Rng(pub u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut value = self.0;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^ (value >> 31)
    }

    fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound as u64) as usize
    }

    pub fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
}

pub struct Node {
    pub name: String,
    pub path: PathBuf,
    pub store: Option<Store>,
}

impl Node {
    fn open(directory: &Path, name: &str) -> Self {
        let mut node = Self {
            name: name.to_owned(),
            path: directory.join(format!("{name}.sqlite3")),
            store: None,
        };
        node.start();
        node
    }

    /// A node whose store an older build wrote: before the claim-log diet, with a reopened run
    /// and claims that replicated under the retention rules of that time.
    fn from_old_build(directory: &Path) -> Self {
        use std::io::Read as _;

        let path = directory.join("writer.sqlite3");
        let mut bytes = Vec::new();
        flate2::read::GzDecoder::new(
            &include_bytes!("../../../st3-daemon/tests/fixtures/pre-diet-store.sqlite3.gz")[..],
        )
        .read_to_end(&mut bytes)
        .unwrap();
        fs::write(&path, bytes).unwrap();
        let mut node = Self {
            name: "writer".into(),
            path,
            store: None,
        };
        node.start();
        node
    }

    pub fn store(&self) -> &Store {
        self.store.as_ref().expect("the node runs")
    }

    /// Open the store and start as the daemon does.
    fn start(&mut self) {
        let store = Store::open(&self.path, &self.name).unwrap();
        store.bind_fleet(FLEET).unwrap();
        store.validate_replication_backlog().unwrap();
        store.apply_replication_repairs().unwrap();
        store.settle_runs_for_canonical_replay().unwrap();
        assert!(store.project_replication_backlog().unwrap());
        self.store = Some(store);
    }

    /// Stop at once, whatever was under way, and start again.
    fn restart(&mut self) {
        self.store = None;
        self.start();
    }

    fn graph_digest(&self) -> String {
        self.store()
            .replication_status(true, Some(FLEET), &[])
            .unwrap()
            .graph_digest
    }

    fn authority_digest(&self) -> String {
        self.store()
            .replication_status(true, Some(FLEET), &[])
            .unwrap()
            .authority_digest
    }
}

/// How one exchange reaches its receiver.
#[derive(Clone, Copy, Debug)]
pub enum Delivery {
    /// As one exchange.
    Whole,
    /// The newest envelope first, then the rest, as out-of-order history arrives.
    NewestFirst,
    /// In random pieces, in random order.
    Scattered,
    /// The receiver stores the envelopes and stops before it admits them.
    CrashAfterReceipt,
    /// The receiver admits the claims and stops before it projects them.
    CrashAfterAdmission,
}

pub fn mission_source(revision: usize) -> String {
    format!(
        r#"version 2

mission "sim" state="ready" {{
  goal "Converge revision {revision}."
  step "prepare" {{ assigned-to "agent/sim.worker" }}
  step "check" {{ assigned-to "agent/sim.worker"; depends-on {{ step "prepare" completed }} }}
  step "probe" {{ goal "Probe revision {revision}."; assigned-to "agent/sim.worker"; depends-on {{ step "prepare" completed }} }}
  step "announce" {{ assigned-to "agent/sim.worker"; depends-on {{ step "check" completed }} }}
  finally {{ step "report" {{ agentless }} }}
}}
"#
    )
}

pub struct Simulation {
    pub rng: Rng,
    pub nodes: Vec<Node>,
    /// Unordered pairs of node indexes whose link is down.
    pub partitioned: BTreeSet<(usize, usize)>,
    /// Every run any node started.
    pub runs: Vec<String>,
    pub revision: usize,
    pub key: usize,
    pub faults: Vec<String>,
    pub log: Vec<String>,
}

impl Simulation {
    pub fn new(seed: u64, directory: &Path) -> Self {
        let mut rng = Rng(seed);
        let count = 2 + rng.below(3);
        let mut nodes = Vec::new();
        if rng.chance(35) {
            nodes.push(Node::from_old_build(directory));
        }
        while nodes.len() < count {
            nodes.push(Node::open(directory, &format!("node-{}", nodes.len())));
        }
        Self {
            rng,
            nodes,
            partitioned: BTreeSet::new(),
            runs: Vec::new(),
            revision: 0,
            key: 0,
            faults: Vec::new(),
            log: Vec::new(),
        }
    }

    fn next_key(&mut self, what: &str) -> String {
        self.key += 1;
        format!("sim-{what}-{}", self.key)
    }

    pub fn step(&mut self, faults: bool) {
        let node = self.rng.below(self.nodes.len());
        match self.rng.below(100) {
            0..=39 => self.write(node),
            40..=74 => {
                let to = self.rng.below(self.nodes.len());
                if to != node {
                    let delivery = *self.rng.pick(&[
                        Delivery::Whole,
                        Delivery::Whole,
                        Delivery::NewestFirst,
                        Delivery::Scattered,
                        Delivery::CrashAfterReceipt,
                        Delivery::CrashAfterAdmission,
                    ]);
                    self.deliver(node, to, delivery);
                }
            }
            75..=81 => {
                self.log.push(format!("restart {}", self.nodes[node].name));
                self.nodes[node].restart();
            }
            82..=89 => {
                let other = self.rng.below(self.nodes.len());
                if other != node {
                    let pair = (node.min(other), node.max(other));
                    if !self.partitioned.remove(&pair) {
                        self.partitioned.insert(pair);
                    }
                    self.log.push(format!("toggle link {pair:?}"));
                }
            }
            _ if faults => self.fault(node),
            _ => self.write(node),
        }
    }

    fn write(&mut self, node: usize) {
        let choice = self.rng.below(100);
        let key = self.next_key("write");
        let name = self.nodes[node].name.clone();
        let result: std::result::Result<String, String> = match choice {
            0..=9 => {
                self.revision += 1;
                let source = mission_source(self.revision);
                let intent = parse_test_intent(&source, "node").unwrap();
                let store = self.nodes[node].store();
                store
                    .mission(
                        &intent,
                        IntentInput {
                            kdl: source,
                            source_name: None,
                        },
                    )
                    .map_err(|error| error.to_string())
                    .and_then(|planned| {
                        store
                            .apply(&intent, &planned.subject_tokens, &key)
                            .map_err(|error| error.to_string())
                    })
                    .map(|_| format!("publish revision {}", self.revision))
            }
            10..=21 => match self.nodes[node]
                .store()
                .create_mission_run(&MissionRunRequest {
                    mission: "sim".into(),
                    revision: None,
                    workspace: "/tmp".into(),
                    requester: Some(PERSON.into()),
                    mode: Some("run".into()),
                    inputs: BTreeMap::new(),
                    idempotency_key: key,
                }) {
                Ok(run) => {
                    self.runs.push(run.id.clone());
                    Ok(format!("start run {}", run.id))
                }
                Err(error) => Err(error.to_string()),
            },
            22..=79 => {
                let Some(run) = self.known_run(node) else {
                    return;
                };
                if run.steps.is_empty() {
                    return;
                }
                let step = run.steps[self.rng.below(run.steps.len())].subject.clone();
                let action = self.rng.below(15);
                let status = *self.rng.pick(&[
                    "ready",
                    "working",
                    "completed",
                    "completed",
                    "failed",
                    "cancelled",
                ]);
                let (run_status, run_phase) = *self.rng.pick(&[
                    ("running", "final"),
                    ("failed", "terminal"),
                    ("completed", "terminal"),
                ]);
                let store = self.nodes[node].store();
                match action {
                    0..=5 => store
                        .set_step_state(&step, status, Some("the simulation moved it"))
                        .map(|_| format!("{step} {status}"))
                        .map_err(|error| error.to_string()),
                    6 => store
                        .set_mission_run_state(&run.id, run_status, run_phase, Some("simulated"))
                        .map(|_| format!("run {} {run_status}/{run_phase}", run.id))
                        .map_err(|error| error.to_string()),
                    7 => store
                        .retry_failed_step(&step, PERSON, "retry it", &key)
                        .map(|_| format!("retry {step}"))
                        .map_err(|error| error.to_string()),
                    8 => {
                        // Revising a failed run reopens it, one carried-step batch at a time.
                        let source = mission_source(self.revision.max(1));
                        let intent = parse_test_intent(&source, "node").unwrap();
                        store
                            .adopt_mission_revision(
                                &run.id,
                                &intent.missions["sim"],
                                PERSON,
                                "reopen it",
                                &key,
                            )
                            .map(|_| format!("adopt into {}", run.id))
                            .map_err(|error| error.to_string())
                    }
                    // A seat's lease work: its node applies each action to the step as it writes
                    // it, and a quiet renewal moves only the local lease.
                    10..=13 => {
                        // A seat claims a ready step, then renews, reports on, submits or
                        // releases a step it holds.
                        let ready = run.steps.iter().find(|step| step.status == "ready");
                        let held = run
                            .steps
                            .iter()
                            .filter(|step| matches!(step.status.as_str(), "claimed" | "working"))
                            .collect::<Vec<_>>();
                        let (step, work) = match (ready, held.is_empty()) {
                            (Some(ready), _) if held.is_empty() || self.rng.chance(50) => {
                                (ready.subject.clone(), "claim")
                            }
                            (_, false) => (
                                self.rng.pick(&held).subject.clone(),
                                *self
                                    .rng
                                    .pick(&["renew", "renew", "progress", "complete", "release"]),
                            ),
                            _ => {
                                // Nothing to lease yet: make the chosen step ready first.
                                if let Err(error) =
                                    store.set_step_state(&step, "ready", Some("a seat can take it"))
                                {
                                    self.log.push(format!("{name}: refused: {error}"));
                                }
                                (step, "claim")
                            }
                        };
                        let summary = self
                            .rng
                            .chance(50)
                            .then(|| format!("{work} by the simulation"));
                        store
                            .work_action(
                                &step,
                                work,
                                &WorkRequest {
                                    wait_ms: None,
                                    actor: Some("agent/sim.worker".into()),
                                    incarnation: Some(format!("{name}-incarnation")),
                                    summary,
                                    reason: None,
                                    evidence: Vec::new(),
                                    idempotency_key: key,
                                },
                            )
                            .map(|_| format!("{work} {step}"))
                            .map_err(|error| error.to_string())
                    }
                    // Lane and intake claims: event-only kinds that extend the graph in any order.
                    14 => {
                        let (subject, kind, fields) = if self.rng.chance(50) {
                            (
                                "lane/sim/train".to_owned(),
                                "lane.marked",
                                json!({"entry": format!("mission-run/{}", run.id.trim_start_matches("mission-run/")), "state": "waiting"}),
                            )
                        } else {
                            (
                                "subscription/sim/intake".to_owned(),
                                "subscription.mission-requested",
                                json!({
                                    "mission": "mission/sim", "mission_revision": "revision",
                                    "resource": "resource/sim", "resource_input": "source",
                                    "workspace": "/tmp", "discovery": key
                                }),
                            )
                        };
                        store
                            .append_claim(&ClaimInput {
                                subject: subject.clone(),
                                kind: kind.into(),
                                actor: None,
                                fields: serde_json::from_value(fields).unwrap(),
                                evidence: Vec::new(),
                                expected_subject: None,
                                idempotency_key: Some(key),
                            })
                            .map(|_| format!("{kind} {subject}"))
                            .map_err(|error| error.to_string())
                    }
                    _ => store
                        .request_mission_run_cancellation(&run.id, "the simulation cancels it")
                        .map(|_| format!("cancel {}", run.id))
                        .map_err(|error| error.to_string()),
                }
            }
            // Kinds whose retention the claim-log diet changed: a heartbeat that replicates only
            // when its state changes, and a runtime action that stays local when the system
            // records it and replicates when an agent does.
            80..=91 => {
                let state = *self.rng.pick(&["idle", "working", "idle"]);
                self.nodes[node]
                    .store()
                    .append_claim(&ClaimInput {
                        subject: format!("agent/{name}.worker"),
                        kind: "harness.observed".into(),
                        actor: Some(format!("agent/{name}.worker")),
                        fields: serde_json::from_value(json!({
                            "state": state,
                            "incarnation_id": format!("{name}-incarnation"),
                            "observed_at_ms": self.key,
                        }))
                        .unwrap(),
                        evidence: Vec::new(),
                        expected_subject: None,
                        idempotency_key: Some(key),
                    })
                    .map(|_| format!("heartbeat {state}"))
                    .map_err(|error| error.to_string())
            }
            _ => {
                let actor = self.rng.chance(50).then(|| format!("agent/{name}.worker"));
                self.nodes[node]
                    .store()
                    .append_claim(&ClaimInput {
                        subject: format!("agent/{name}.worker"),
                        kind: "runtime.action.requested".into(),
                        actor: actor.clone(),
                        fields: serde_json::from_value(json!({"action": "terminate"})).unwrap(),
                        evidence: Vec::new(),
                        expected_subject: None,
                        idempotency_key: Some(key),
                    })
                    .map(|_| format!("action by {actor:?}"))
                    .map_err(|error| error.to_string())
            }
        };
        self.log.push(format!(
            "{name}: {}",
            result.unwrap_or_else(|error| format!("refused: {error}"))
        ));
    }

    /// A run this node holds, from all the runs any node started.
    fn known_run(&mut self, node: usize) -> Option<MissionRunView> {
        if self.runs.is_empty() {
            return None;
        }
        let run = self.runs[self.rng.below(self.runs.len())].clone();
        self.nodes[node].store().mission_run(&run).ok().flatten()
    }

    fn linked(&self, left: usize, right: usize) -> bool {
        !self
            .partitioned
            .contains(&(left.min(right), left.max(right)))
    }

    /// Send what `to` lacks from `from`, as the worker and the main daemon do. Returns how many
    /// envelopes moved.
    fn deliver(&mut self, from: usize, to: usize, delivery: Delivery) -> usize {
        if !self.linked(from, to) {
            return 0;
        }
        let exchange = self.nodes[from]
            .store()
            .export_replication_exchange(
                FLEET,
                &self.nodes[to].store().replication_inventory().unwrap(),
            )
            .unwrap();
        if exchange.envelopes.is_empty() {
            return 0;
        }
        let moved = exchange.envelopes.len();
        let relay = self.nodes[from].name.clone();
        self.log.push(format!(
            "{} -> {}: {moved} envelopes {delivery:?}",
            relay, self.nodes[to].name
        ));
        let pieces = match delivery {
            Delivery::NewestFirst => {
                let mut older = exchange.envelopes.clone();
                let newest = older.split_off(older.len() - 1);
                vec![newest, older]
            }
            Delivery::Scattered => {
                let mut pieces = Vec::new();
                let mut rest = exchange.envelopes.clone();
                while !rest.is_empty() {
                    let take = 1 + self.rng.below(rest.len());
                    let tail = rest.split_off(take);
                    pieces.push(rest);
                    rest = tail;
                }
                let mut shuffled = Vec::new();
                while !pieces.is_empty() {
                    shuffled.push(pieces.remove(self.rng.below(pieces.len())));
                }
                shuffled
            }
            _ => vec![exchange.envelopes.clone()],
        };
        let store = self.nodes[to].store();
        for envelopes in pieces {
            store
                .receive_replication_exchange(
                    &relay,
                    FLEET,
                    &ReplicationExchange {
                        envelopes,
                        ..exchange.clone()
                    },
                )
                .unwrap();
            if matches!(delivery, Delivery::CrashAfterReceipt) {
                self.nodes[to].restart();
                return moved;
            }
            store.validate_replication_backlog().unwrap();
            store.apply_replication_repairs().unwrap();
            if matches!(delivery, Delivery::CrashAfterAdmission) {
                self.nodes[to].restart();
                return moved;
            }
            assert!(store.project_replication_backlog().unwrap());
        }
        moved
    }

    /// An older build's fault on one node.
    fn fault(&mut self, node: usize) {
        let name = self.nodes[node].name.clone();
        let kind = self.rng.below(3);
        let fault = match kind {
            // It lost claims it had admitted, and kept their envelopes.
            0 => {
                let lost = self.claim_sample(node, "state='valid'");
                if lost.is_empty() {
                    return;
                }
                let store = self.nodes[node].store();
                {
                    let connection = store.connection.write();
                    connection.execute_batch("PRAGMA foreign_keys=OFF").unwrap();
                    for claim in &lost {
                        connection
                            .execute(/* sqlite-large-value: allow-for-now lv-h7c8aa3193820 owner=st3_store::store::convergence_shared_attention_tests TODO=prepare-body-and-predecessor-values */ "DELETE FROM claims WHERE id=?1", [claim])
                            .unwrap();
                    }
                    connection.execute_batch("PRAGMA foreign_keys=ON").unwrap();
                }
                store.replay_replication_graph().unwrap();
                format!("{name} lost {} claims", lost.len())
            }
            // It could not admit claims of a kind it did not know; this build knows them.
            1 => {
                let unknown = self.claim_sample(
                    node,
                    &format!("state='valid' AND writer<>'{}'", name.replace('\'', "''")),
                );
                if unknown.is_empty() {
                    return;
                }
                let store = self.nodes[node].store();
                {
                    let connection = store.connection.write();
                    connection.execute_batch("PRAGMA foreign_keys=OFF").unwrap();
                    for claim in &unknown {
                        connection
                            .execute(
                                /* sqlite-large-value: allow-for-now lv-h8ddc22937363 owner=st3_store::store::convergence_shared_attention_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_records SET state='unknown',
                                     error_code='unknown-claim-kind',
                                     error_message='this build does not know the claim kind'
                                 WHERE claim_id=?1",
                                [claim],
                            )
                            .unwrap();
                        connection
                            .execute(/* sqlite-large-value: allow-for-now lv-had68b14bbd43 owner=st3_store::store::convergence_shared_attention_tests TODO=prepare-body-and-predecessor-values */ "DELETE FROM claims WHERE id=?1", [claim])
                            .unwrap();
                    }
                    connection.execute_batch("PRAGMA foreign_keys=ON").unwrap();
                }
                store.replay_replication_graph().unwrap();
                format!("{name} could not admit {} claims", unknown.len())
            }
            // It projected by another rule: a step on another attempt, a run over too soon.
            _ => {
                let (step_offset, run_offset) = (self.rng.next() % 1000, self.rng.next() % 1000);
                let connection = self.nodes[node].store().connection.write();
                let changed = connection
                    .execute(
                        /* sqlite-large-value: allow-for-now lv-h0d930bfc58bc owner=st3_store::store::convergence_shared_attention_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE step_runs SET attempt=attempt+1 WHERE subject=(
                             SELECT subject FROM step_runs ORDER BY subject
                             LIMIT 1 OFFSET ?1 % MAX(1, (SELECT COUNT(*) FROM step_runs)))",
                        [step_offset],
                    )
                    .unwrap()
                    + connection
                        .execute(
                            /* sqlite-large-value: allow-for-now lv-h1269c9692630 owner=st3_store::store::convergence_shared_attention_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE mission_runs SET status='failed', phase='terminal' WHERE id=(
                                 SELECT id FROM mission_runs ORDER BY id
                                 LIMIT 1 OFFSET ?1 % MAX(1, (SELECT COUNT(*) FROM mission_runs)))",
                            [run_offset],
                        )
                        .unwrap();
                if changed == 0 {
                    return;
                }
                format!("{name} projected {changed} rows by another rule")
            }
        };
        self.log.push(fault.clone());
        self.faults.push(fault);
    }

    /// Up to five claims whose records match `filter`, chosen by the seed.
    fn claim_sample(&mut self, node: usize, filter: &str) -> Vec<String> {
        let ids = {
            let connection = self.nodes[node].store().readers.get();
            let mut statement = connection
                .prepare(/* sqlite-large-value: allow-for-now lv-h3fd8370c9f75 owner=st3_store::store::convergence_shared_attention_tests TODO=trace-statement-and-value-representation */ &format!(
                    "SELECT DISTINCT claims.id FROM claims
                     JOIN replica_records ON replica_records.claim_id=claims.id
                     WHERE replica_records.{filter}
                     ORDER BY claims.id"
                ))
                .unwrap();
            statement
                .query_map([], |row| row.get::<_, String>(0))
                .unwrap()
                .collect::<std::result::Result<Vec<_>, _>>()
                .unwrap()
        };
        let mut sample = BTreeSet::new();
        for _ in 0..ids.len().min(1 + self.rng.below(5)) {
            sample.insert(self.rng.pick(&ids).clone());
        }
        sample.into_iter().collect()
    }

    /// Every link up, every node restarted on this build, exchanges until nothing moves.
    pub fn settle(&mut self) {
        self.partitioned.clear();
        for node in &mut self.nodes {
            node.restart();
        }
        for _ in 0..32 {
            let mut moved = 0;
            for from in 0..self.nodes.len() {
                for to in 0..self.nodes.len() {
                    if from != to {
                        moved += self.deliver(from, to, Delivery::Whole);
                    }
                }
            }
            if moved == 0 {
                return;
            }
        }
        panic!("envelopes still move after 32 rounds of exchanges");
    }

    pub fn digests(&self) -> Vec<(String, String, String)> {
        self.nodes
            .iter()
            .map(|node| {
                (
                    node.name.clone(),
                    node.authority_digest(),
                    node.graph_digest(),
                )
            })
            .collect()
    }
}

pub fn env_number(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}
