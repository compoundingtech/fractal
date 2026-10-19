#![allow(unused_imports)]
use crate::store::*;
use smallclaims::ivm::{Definition, LocalChange, Readiness, View, Views, source_cut};
use crate::store::mission_ivm::*;
use smallclaims::ivm::{SourceCut, events};
use crate::store::attention_ivm::tests_shared_seats::*;
use crate::store::terminal_gate_evidence::native_source_transaction_hooks_tests_shared_seats::*;

pub fn start(store: &Store, id: &str, key: &str) -> MissionRunView {
        store
            .create_mission_run(&MissionRunRequest {
                mission: id.into(),
                revision: None,
                workspace: "/example/project".into(),
                requester: Some("person/avery".into()),
                mode: None,
                inputs: BTreeMap::new(),
                idempotency_key: key.into(),
            })
            .unwrap()
    }

