//! Fixtures of `api::tests` that the tests of the crates above (fractal-api-mailbox) use too.
use super::*;
use std::path::PathBuf;

pub fn state(root: &Path) -> AppState {
    AppState {
        seat_admission: Default::default(),
        owning_person: None,
        store: Arc::new(Store::open_memory("node").unwrap()),
        notify: Arc::new(Notify::new()),
        event_notify: watch::channel(0_u64).0,
        node: "node".into(),
        state_dir: root.to_path_buf(),
        pty_root: root.join("pty"),
        pty_binary: PathBuf::from("pty"),
        fleet_id: None,
        configured_peers: Vec::new(),
        client_relay: None,
        native_session_home: None,
        planner_default: PlannerSpec::default(),
        harness_question_sink: None,
        assistant_receipts: Default::default(),
        private_notes: Default::default(),
    }
}

pub fn probe_claim(key: &str, state: &str) -> ClaimInput {
    ClaimInput {
        subject: "agent/probe".into(),
        kind: "harness.observed".into(),
        actor: Some("agent/probe".into()),
        fields: BTreeMap::from([("state".into(), Value::String(state.into()))]),
        evidence: Vec::new(),
        expected_subject: None,
        idempotency_key: Some(key.into()),
    }
}
