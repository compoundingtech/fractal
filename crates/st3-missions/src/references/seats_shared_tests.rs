#![allow(unused_imports)]
pub use st3_store::references::seats_shared_tests::*;
use std::collections::{BTreeMap, BTreeSet};
use crate::model::{DesiredSubject, MissionInputKind, MissionSpec, NormalizedIntent, St3Error, UsedMissionSpec, WorkSelector};
use crate::graph::parse_intent;
use crate::store::Store;
use crate::MissionsStoreCore;


pub fn refusals(store: &Store, source: &str) -> Vec<String> {
        store
            .unresolved_references(&parse_intent(source, "node").unwrap())
            .unwrap()
    }
