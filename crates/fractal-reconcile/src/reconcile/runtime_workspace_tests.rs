use super::*;
use super::tests::{FakeRuntime, apply_source};
use crate::fleet::MemberKey;
use serde_json::json;

const AGENT: &str = "agent/example/worker";
const FLEET: &str = "7c9e6679-7425-40de-944b-e07fc1f90ae7";

struct Fixture {
    _root: tempfile::TempDir,
    store: Arc<Store>,
    runtime: Arc<FakeRuntime>,
    reconciler: Reconciler<FakeRuntime>,
    cwd: String,
}

impl Fixture {
    fn new(feature: Option<u8>) -> Self {
        let root = tempfile::tempdir().unwrap();
        let cwd = root.path().join("nested");
        fs::create_dir(&cwd).unwrap();
        let store = Arc::new(Store::open(&root.path().join("graph.db"), "node").unwrap());
        if let Some(feature) = feature {
            let key = Arc::new(MemberKey::generate().unwrap().0);
            store.bind_fleet(FLEET).unwrap();
            store.pin_fleet_anchor(key.public()).unwrap();
            store.set_member_key(Some(key.clone())).unwrap();
            store.admit_fleet_anchor(FLEET, key.public(), "listening").unwrap();
            store.append_claim(&ClaimInput {
                subject: "daemon/node".into(), kind: "daemon.started".into(), actor: None,
                fields: BTreeMap::from([
                    ("status".into(), json!("running")),
                    ("features".into(), json!({"runtime_workspace": feature})),
                ]),
                evidence: vec![], expected_subject: None, idempotency_key: None,
            }).unwrap();
            store.replication_snapshot().unwrap();
        }
        apply_source(&store, &format!(
            "version 2\nagent \"example/worker\" {{ workspace {:?}; cwd {:?}; command \"true\"; restart \"never\" }}",
            root.path(), cwd,
        ), "workspace-producer-source");
        let runtime = Arc::new(FakeRuntime::default());
        let reconciler = Reconciler::new(store.clone(), runtime.clone(), "node".into(), Arc::new(Notify::new()));
        Self { _root: root, store, runtime, reconciler, cwd: cwd.to_string_lossy().into_owned() }
    }

    fn launch(&self) -> RuntimeObservation {
        self.reconciler.reconcile_once().unwrap();
        let (live, changed) = &*self.reconciler.start_jobs.live;
        let mut live = live.lock().unwrap();
        while !live.is_empty() {
            live = changed.wait(live).unwrap();
        }
        drop(live);
        let operation = self.store.runtime_start_for(AGENT).unwrap().unwrap();
        let incarnation = operation.result.as_ref().unwrap()["incarnation"].as_str().unwrap().to_owned();
        let member = self.store.desired_subject_with_writer(AGENT).unwrap().unwrap().0.member.unwrap();
        let observation = RuntimeObservation {
            runtime_id: member.runtime_id, terminal: member.terminal, status: "running".into(),
            exit_code: None, incarnation_id: Some(incarnation),
        };
        *self.runtime.ptys.lock().unwrap() = vec![observation.clone()];
        self.reconciler.reconcile_once().unwrap();
        observation
    }

    fn latest(&self) -> crate::model::ClaimRecord {
        self.store.latest_observation(AGENT, "runtime.observed").unwrap().unwrap()
    }
}

#[test]
fn signed_reader_barrier_publishes_actual_launch_cwd_and_retains_it_across_desired_drift() {
    let fixture = Fixture::new(Some(1));
    assert!(fixture.store.runtime_workspace_publication_ready().unwrap());
    let observation = fixture.launch();
    let launched = fixture.runtime.started_members.lock().unwrap().last().unwrap().clone();
    assert_eq!(launched.cwd, fixture.cwd);
    assert_ne!(launched.workspace, fixture.cwd);
    let claim = fixture.latest();
    assert_eq!(claim.body["fields"]["workspace"], fixture.cwd);
    assert_eq!(claim.body["fields"]["incarnation_id"], observation.incarnation_id.as_deref().unwrap());

    let subject = fixture.store.desired_subject_with_writer(AGENT).unwrap().unwrap().0;
    let mut changed = subject.member.clone().unwrap();
    changed.workspace = "/changed/desired/workspace".into();
    changed.cwd = "/changed/desired/cwd".into();
    fixture.reconciler.record_member_as(&subject, &changed, &observation, true).unwrap();
    assert_eq!(fixture.latest().body["fields"]["workspace"], fixture.cwd);

    let replacement = RuntimeObservation { incarnation_id: Some("unproven-adopted-incarnation".into()), ..observation };
    fixture.reconciler.record_member_as(&subject, &changed, &replacement, true).unwrap();
    assert_eq!(fixture.latest().body["fields"]["workspace"], Value::Null);
}

#[test]
fn unsupported_or_unanchored_readers_omit_workspace_even_after_successful_launch() {
    for feature in [Some(0), None] {
        let fixture = Fixture::new(feature);
        assert!(!fixture.store.runtime_workspace_publication_ready().unwrap());
        fixture.launch();
        assert!(fixture.latest().body["fields"].get("workspace").is_none());
    }
}

#[test]
fn barrier_regression_omits_previously_known_workspace() {
    let fixture = Fixture::new(Some(1));
    let observation = fixture.launch();
    fixture.store.append_claim(&ClaimInput {
        subject: "daemon/node".into(), kind: "daemon.started".into(), actor: None,
        fields: BTreeMap::from([
            ("status".into(), json!("running")), ("features".into(), json!({})),
        ]),
        evidence: vec![], expected_subject: None, idempotency_key: None,
    }).unwrap();
    fixture.store.replication_snapshot().unwrap();
    assert!(!fixture.store.runtime_workspace_publication_ready().unwrap());
    let subject = fixture.store.desired_subject_with_writer(AGENT).unwrap().unwrap().0;
    fixture.reconciler.record_member_as(&subject, subject.member.as_ref().unwrap(), &observation, true).unwrap();
    assert!(fixture.latest().body["fields"].get("workspace").is_none());
}
