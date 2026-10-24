//! The part of `references` that moved up from fractal-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use std::collections::{BTreeMap, BTreeSet};
use crate::model::{
    DesiredSubject, MissionInputKind, MissionSpec, NormalizedIntent, St3Error, UsedMissionSpec,
    WorkSelector,
};
pub use fractal_delivery::references::*;
pub use fractal_delivery::references::{Graph, Scope, graph};
#[cfg(any(test, feature = "test-support"))]
pub mod seats_shared_tests;

/// Every reference in `intent` that resolves neither in the intent nor in `graph`.
pub fn publication(
    intent: &NormalizedIntent,
    graph: &impl Graph,
    host: &str,
) -> Result<Vec<String>, St3Error> {
    let mut refusals = Vec::new();
    for desired in intent.subjects.values() {
        subject_references(
            desired,
            None,
            &Scope::empty(intent),
            graph,
            host,
            &mut refusals,
        )?;
    }
    for run in runs(&intent.missions.values().collect::<Vec<_>>()) {
        let scope = Scope {
            run: run_declarations(&run, host),
            intent: Some(intent),
        };
        for mission in run {
            mission_references(mission, &scope, graph, host, &mut refusals)?;
        }
    }
    refusals.sort();
    refusals.dedup();
    Ok(refusals)
}

#[cfg(test)]
mod tests {
    #[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::references::seats_shared_tests::*;
    #[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::references::seats_shared_tests::{CYCLE, OBSERVER, SUBSCRIPTION, UNSTORED, WATCHER, cycle_revision, refusals, schedule, watch};
    use crate::store::Store;

    #[test]
    fn unpinned_schedule_refuses_unknown_mission_at_publish() {
        let store = Store::open_memory("node").unwrap();
        let source = watch(
            r#"schedule "cycle" {
  every "6h"
  anchor "2026-01-01T00:00:00Z"
  work { mission "unknown"; workspace "/tmp/cycles" }
}"#,
        );
        assert_eq!(
            refusals(&store, &source),
            [
                "mission `mission/watch` declares schedule `schedule/${ST_MISSION_RUN}/cycle` references unpublished mission `mission/unknown`"
            ]
        );
        for state in ["draft", "ready", "retired"] {
            let declared = format!(
                "{source}\nmission \"unknown\" state=\"{state}\" {{ goal \"Cycle.\"; step \"work\" {{ agentless }} }}"
            );
            assert!(refusals(&store, &declared).is_empty());
        }
    }

    #[test]
    fn a_step_selecting_an_agent_nothing_declares_is_refused() {
        let store = Store::open_memory("node").unwrap();
        let missing = r#"version 2
mission "work" state="ready" {
  goal "Complete the work."
  step "do-work" { assigned-to "agent/example/nobody" }
}"#;
        assert_eq!(
            refusals(&store, missing),
            ["mission `mission/work` references missing eligible agent `agent/example/nobody`"]
        );
        let declared =
            format!("{missing}\nagent \"example/nobody\" {{ workspace \"/tmp\"; command \"true\" }}");
        assert_eq!(refusals(&store, &declared), Vec::<String>::new());
    }

    #[test]
    fn an_agent_the_mission_declares_for_its_run_resolves() {
        let store = Store::open_memory("node").unwrap();
        let source = r#"version 2
mission "work" state="ready" {
  goal "Complete the available work."
  agent "worker" {
    identity "fleet.worker"
    workspace "."
    command "true"
  }
  step "do-work" {
    assigned-to "agent/${ST_MISSION_RUN}/fleet.worker"
  }
}"#;
        assert_eq!(refusals(&store, source), Vec::<String>::new());
    }

    #[test]
    fn a_subscription_to_a_missing_observer_or_mission_is_refused() {
        let store = Store::open_memory("node").unwrap();
        assert_eq!(
            refusals(&store, &format!("{}\n{WATCHER}", watch(SUBSCRIPTION))),
            [
                "mission `mission/watch` declares subscription `subscription/${ST_MISSION_RUN}/changed` references missing observer `observer/status`"
            ]
        );
        assert_eq!(
            refusals(&store, &watch(&format!("{SUBSCRIPTION}\n{OBSERVER}"))),
            [
                "mission `mission/watch` declares subscription `subscription/${ST_MISSION_RUN}/changed` has missing delivery target `agent/example/watcher`"
            ]
        );
        assert_eq!(
            refusals(
                &store,
                &format!(
                    "{}\n{WATCHER}",
                    watch(&format!("{SUBSCRIPTION}\n{OBSERVER}"))
                )
            ),
            Vec::<String>::new()
        );

        let mission = watch(&format!(
            r#"{OBSERVER}
subscription "changed" {{
  observer "observer/status"
  on "status"
  delivery "mission" {{
    mission "ghost"
    resource "status"
    workspace "/tmp/ghost"
  }}
}}"#
        ));
        assert_eq!(
            refusals(&store, &mission),
            [
                "mission `mission/watch` declares subscription `subscription/${ST_MISSION_RUN}/changed` references unpublished mission `mission/ghost`"
            ]
        );
    }
}
