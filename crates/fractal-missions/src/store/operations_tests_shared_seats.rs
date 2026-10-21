#![allow(unused_imports)]
use crate::store::*;
use crate::store::operations::*;


pub fn clock_run(store: &Store, key: &str) -> MissionRunView {
        store
            .create_mission_run(&MissionRunRequest {
                mission: "example/overview".into(),
                revision: None,
                workspace: "/example".into(),
                requester: Some("person/operator".into()),
                mode: None,
                inputs: BTreeMap::new(),
                idempotency_key: key.into(),
            })
            .unwrap()
    }
