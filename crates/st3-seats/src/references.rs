//! The part of `references` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use crate::SeatsStoreCore;
use std::collections::{BTreeMap, BTreeSet};
use crate::model::{
    DesiredSubject, MissionInputKind, MissionSpec, NormalizedIntent, St3Error, UsedMissionSpec,
    WorkSelector,
};
pub use st3_missions::references::*;
pub use st3_missions::references::{Graph, Scope, graph, publication};

#[cfg(test)]
mod tests {
    use crate::graph::parse_intent;
    use crate::SeatsStoreCore;
    use crate::store::Store;
    use st3_missions::references::seats_shared_tests::*;
    use st3_missions::references::seats_shared_tests::{CYCLE, OBSERVER, SUBSCRIPTION, UNSTORED, WATCHER, cycle_revision, refusals, schedule, watch};

    /// Publishes without the reference checks, as a publication before them did.
    fn publish(store: &Store, source: &str) -> crate::model::ApplyResponse {
        store
            .apply_internal(&parse_intent(source, "node").unwrap(), source)
            .unwrap()
    }

    #[test]
    fn a_pin_to_a_revision_that_is_not_stored_is_refused() {
        let store = Store::open_memory("node").unwrap();
        assert_eq!(
            refusals(&store, &schedule(UNSTORED)),
            [format!(
                "mission `mission/watch` declares schedule `schedule/${{ST_MISSION_RUN}}/cycle` runs a mission that is not stored: mission `mission/cycle@{UNSTORED}` is not stored on this host"
            )]
        );
        let uses = |revision: &str| {
            format!(
                r#"version 2
mission "parent" state="ready" {{
  goal "Run one cycle."
  step "cycle" {{ uses-mission "cycle@{revision}" }}
}}"#
            )
        };
        assert_eq!(
            refusals(&store, &uses(UNSTORED)),
            [format!(
                "step `cycle` of mission `mission/parent` uses a mission that is not stored: mission `mission/cycle@{UNSTORED}` is not stored on this host"
            )]
        );

        // Published by the same document, or stored, the pin resolves.
        let revision = cycle_revision();
        assert_eq!(
            refusals(&store, &format!("{}\n{CYCLE}", schedule(&revision))),
            Vec::<String>::new()
        );
        publish(&store, &format!("version 2\n{CYCLE}"));
        assert_eq!(refusals(&store, &schedule(&revision)), Vec::<String>::new());
        assert_eq!(refusals(&store, &uses(&revision)), Vec::<String>::new());
    }

    #[test]
    fn a_pin_to_the_claim_that_published_a_revision_names_the_revision() {
        let store = Store::open_memory("node").unwrap();
        let published = publish(&store, &format!("version 2\n{CYCLE}"));
        let revision = cycle_revision();
        let [claim] = published.claim_ids.as_slice() else {
            panic!("one claim publishes the mission: {:?}", published.claim_ids);
        };
        assert_eq!(
            refusals(&store, &schedule(claim)),
            [format!(
                "mission `mission/watch` declares schedule `schedule/${{ST_MISSION_RUN}}/cycle` runs a mission that is not stored: `mission/cycle@{claim}` pins the ID of the claim that published `mission/cycle@{revision}`, not a mission revision; pin `mission/cycle@{revision}`"
            )]
        );
    }

    #[test]
    fn st_doctor_lists_references_in_the_graph_that_no_longer_resolve() {
        let store = Store::open_memory("node").unwrap();
        assert_eq!(
            store.unresolved_graph_references().unwrap(),
            Vec::<String>::new()
        );
        publish(
            &store,
            &format!(
                r#"{}
mission "parent" state="ready" {{
  goal "Run one cycle."
  step "cycle" {{ uses-mission "cycle@{UNSTORED}" }}
}}"#,
                watch(&format!("{SUBSCRIPTION}\n{OBSERVER}"))
            ),
        );
        assert_eq!(
            store.unresolved_graph_references().unwrap(),
            [
                "mission `mission/watch` declares subscription `subscription/${ST_MISSION_RUN}/changed` has missing delivery target `agent/example/watcher`".to_owned(),
                format!(
                    "step `cycle` of mission `mission/parent` uses a mission that is not stored: mission `mission/cycle@{UNSTORED}` is not stored on this host"
                ),
            ]
        );
        publish(&store, &format!("version 2\n{WATCHER}"));
        assert_eq!(store.unresolved_graph_references().unwrap().len(), 1);
    }
}
