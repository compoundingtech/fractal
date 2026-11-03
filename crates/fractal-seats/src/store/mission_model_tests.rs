#![allow(unused_imports)]
use crate::mission::*;
use crate::graph::*;
use crate::store::Store;
use std::collections::{BTreeMap, BTreeSet};
use kdl::{KdlDocument, KdlNode, KdlValue};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use crate::declaration_syntax::*;
use crate::model::{BaselineSpec, CompletionSpec, DependencySpec, GateSpec, LoopAttentionSpec, LoopExhaustionSpec, LoopSpec, LoopStopSpec, MetricSource, MetricSpec, MissionInputKind, MissionInputSpec, MissionSpec, MissionState, ProductSpec, RetrySpec, RevisionCutover, St3Error, StepSpec, UsedMissionSpec, WorkSelector};


#[test]
fn the_parent_index_gives_every_step_the_parent_the_search_finds() {
    let source = r#"version 2
mission "outer" state="ready" {
  goal "Nest steps."
  step "top" { agentless }
  step "group" {
agentless
mission "inner" state="ready" {
  goal "Inside."
  step "leaf" { agentless }
  step "deeper" { agentless; mission "innermost" state="ready" { goal "Deepest."; step "bottom" { agentless } } }
}
  }
  step "last" { agentless }
}"#;
    let intent = crate::graph::parse_intent(source, "node").unwrap();
    let mission = &intent.missions["outer"];
    let index = crate::mission::parent_step_index(mission);
    assert!(index.len() >= 6, "{index:?}");
    for (path, parent) in &index {
        assert_eq!(*parent, crate::mission::parent_step_path(mission, path), "{path}");
    }
    assert!(index.values().any(Option::is_none) && index.values().any(Option::is_some));
    assert_eq!(crate::mission::parent_step_path(mission, "no-such-step"), None);
    assert!(!index.contains_key("no-such-step"));
}

#[test]
fn keep_open_and_system_are_bare_flags() {
    let source = r#"version 2
mission "observer" state="ready" {
  system
  goal "Watch for work."
  step "hold" { agentless; keep-open }
  step "work" { agentless }
}"#;
    let intent = crate::graph::parse_intent(source, "node").unwrap();
    let mission = &intent.missions["observer"];
    assert!(mission.system);
    assert!(mission.steps["hold"].keep_open);
    assert!(!mission.steps["work"].keep_open);
    assert!(crate::mission::step_keeps_open(Some(mission), "hold"));
    assert!(
        crate::mission::step_keeps_open(None, "seat-retirement"),
        "a fleet name, before the flag"
    );
    assert!(!crate::mission::step_keeps_open(Some(mission), "work"));
    assert!(crate::mission::is_system("mission/observer", Some(mission)));
    assert!(crate::mission::is_system("mission/fleet/example/ci/run", None));
    assert!(!crate::mission::is_system("mission/fleet/example/build", None));
    // An unflagged mission serializes as before, so its published revision is unchanged.
    let plain = source.replace("  system\n", "").replace("; keep-open", "");
    let plain = &crate::graph::parse_intent(&plain, "node").unwrap().missions["observer"];
    let body = serde_json::to_value(plain).unwrap();
    assert!(body.get("system").is_none() && body["steps"]["hold"].get("keep_open").is_none());
    for invalid in [
        source.replace("  system\n", "  system #true\n"),
        source.replace("  system\n", "  system\n  system\n"),
        source.replace("keep-open }", "keep-open \"yes\" }"),
        source.replace("keep-open }", "keep-open; keep-open }"),
    ] {
        assert!(
            crate::graph::parse_intent(&invalid, "node").is_err(),
            "{invalid}"
        );
    }
}

#[test]
fn a_doubled_dollar_keeps_literal_variable_text() {
    let intent = crate::graph::parse_intent(
        r#"version 2
 mission "quote" state="ready" {
   goal "Explain what a shell variable expands to."
   step "explain" {
 goal "Print `$${HOME}` for ${ST_MISSION_RUN}."
 constraint "Keep `$${name}` as written."
   }
 }"#,
        "node",
    )
    .unwrap();
    let step = &intent.missions["quote"].steps["explain"];
    let variables =
        std::collections::BTreeMap::from([("ST_MISSION_RUN".to_owned(), "quote/1".to_owned())]);
    assert_eq!(
        crate::mission::interpolate(&step.goals[0], &variables).unwrap(),
        "Print `${HOME}` for quote/1."
    );
    assert_eq!(
        crate::mission::interpolate(&step.constraints[0], &variables).unwrap(),
        "Keep `${name}` as written."
    );

    let error = crate::graph::parse_intent(
        r#"version 2
 mission "quote" state="ready" {
   goal "Explain a shell variable."
   step "explain" { goal "Print `${HOME}`." }
 }"#,
        "node",
    )
    .unwrap_err();
    assert_eq!(error.code, "unknown-variable");
    assert_eq!(
        error.message,
        "variable `HOME` is not registered; write `$${HOME}` for the literal text"
    );
}

#[test]
fn after_run_steps_are_agentless_and_start_before_the_rest_of_a_run() {
    use crate::mission::{AFTER_RUN_STEP, after_run_subject, mission_after_run, run_mission};
    use crate::model::{DependencySpec, WorkSelector};
    use std::collections::BTreeMap;

    let intent = crate::graph::parse_intent(
        r#"version 2
 mission "ship" state="ready" {
   goal "Ship after the build run."
   assigned-to "agent/example/shipper"
   input "build" kind="text"
   step "wait" { after-run "${input.build}" }
   step "ship" { depends-on { step "wait" completed } }
   finally { step "report" { agentless } }
 }"#,
        "node",
    )
    .unwrap();
    let mission = intent.missions["ship"].clone();
    let wait = &mission.steps["wait"];
    assert_eq!(wait.after_run.as_deref(), Some("${input.build}"));
    assert_eq!(wait.work_selector, Some(WorkSelector::Agentless));
    assert_eq!(after_run_subject("build/1"), "mission-run/build/1");
    assert_eq!(
        after_run_subject("mission-run/build/1"),
        "mission-run/build/1"
    );

    let selected = crate::graph::parse_intent(
        r#"version 2
 mission "bad" state="ready" {
   goal "Wait with an agent."
   step "wait" { assigned-to "agent/example/shipper"; after-run "build/1" }
 }"#,
        "node",
    )
    .unwrap_err();
    assert_eq!(selected.code, "conflicting-work-selector");

    let hashes = mission
        .steps
        .values()
        .map(|step| (step.id.clone(), step.definition_hash.clone()))
        .collect::<BTreeMap<_, _>>();
    let after = mission_after_run(mission.clone(), "build/1").unwrap();
    assert_eq!(after.display_order[0], AFTER_RUN_STEP);
    let step = &after.steps[AFTER_RUN_STEP];
    assert_eq!(step.after_run.as_deref(), Some("mission-run/build/1"));
    assert_eq!(step.work_selector, Some(WorkSelector::Agentless));
    let waits = |id: &str| {
        after.steps[id]
            .dependencies
            .contains(&DependencySpec::Step {
                step: AFTER_RUN_STEP.into(),
                state: "completed".into(),
            })
    };
    assert!(waits("wait") && waits("ship") && !waits("report"));
    for (id, hash) in hashes {
        assert_eq!(after.steps[&id].definition_hash, hash);
    }
    assert_eq!(run_mission(mission.clone(), None).unwrap(), mission);
    assert_eq!(
        mission_after_run(after, "build/2").unwrap_err().code,
        "after-run-step-exists"
    );

    let itself = crate::graph::parse_intent(
        r#"version 2
 mission-run "ship/1" {
   mission "mission/ship@0000000000000000000000000000000000000000000000000000000000000000"
   workspace "/work"
   requester "person/operator"
   after "ship/1"
 }"#,
        "node",
    )
    .unwrap_err();
    assert_eq!(itself.code, "invalid-mission-run-after");
}

#[test]
fn queue_authority_still_parses_and_rejects_invalid_rules() {
    let intent = crate::graph::parse_intent(
        r#"version 2
mission "standing" state="ready" {
  goal "Keep the chief of staff available."
  agent "chief" {
workspace "."
command "true"
queue-authority {
  move "example/fabric/builder"
  move "example/review/*"
  move "${ST_MISSION_RUN}/worker"
}
  }
}"#,
        "node",
    )
    .unwrap();
    // A run interpolates its declarations before it materializes them.
    let declarations = crate::mission::interpolate_kdl(
        intent.missions["standing"]
            .declarations_kdl
            .as_deref()
            .unwrap(),
        &std::collections::BTreeMap::from([("ST_MISSION_RUN".into(), "standing".into())]),
    )
    .unwrap();
    let runtime =
        crate::graph::parse_execution_intent(&declarations, "node", "standing").unwrap();
    let desired = &runtime.subjects["agent/standing/chief"].desired;
    assert_eq!(
        crate::graph::declared_authority_blocks(desired),
        ["queue-authority"]
    );

    for source in [
        r#"version 2
mission "bad" state="ready" { goal "Reject empty authority."; agent "bad" { workspace "."; command "true"; queue-authority { } } }"#,
        r#"version 2
mission "bad" state="ready" { goal "Reject a prefixed seat."; agent "bad" { workspace "."; command "true"; queue-authority { move "agent/example/worker" } } }"#,
        r#"version 2
mission "bad" state="ready" { goal "Reject an internal wildcard."; agent "bad" { workspace "."; command "true"; queue-authority { move "example/*/worker" } } }"#,
        r#"version 2
mission "bad" state="ready" { goal "Reject a bare wildcard."; agent "bad" { workspace "."; command "true"; queue-authority { move "*" } } }"#,
        r#"version 2
mission "bad" state="ready" { goal "Reject a duplicate rule."; agent "bad" { workspace "."; command "true"; queue-authority { move "example/*"; move "example/*" } } }"#,
        r#"version 2
mission "bad" state="ready" { goal "Reject an unknown verb."; agent "bad" { workspace "."; command "true"; queue-authority { reorder "example/worker" } } }"#,
        r#"version 2
mission "bad" state="ready" { goal "Reject a second block."; agent "bad" { workspace "."; command "true"; queue-authority { move "a" }; queue-authority { move "b" } } }"#,
    ] {
        assert!(
            crate::graph::parse_intent(source, "node").is_err(),
            "{source}"
        );
    }
}
