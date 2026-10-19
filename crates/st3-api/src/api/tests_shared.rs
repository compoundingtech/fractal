//! Fixtures of `api::tests` that the tests of the crates above (st3-api-mailbox) use too.
use super::*;
use std::path::PathBuf;

pub fn state(root: &Path) -> AppState {
    AppState {
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
    }
}
