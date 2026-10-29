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
fn mission_authority_still_parses_and_free_mode_names_it_as_ignored() {
    let intent = crate::graph::parse_intent(
        r#"version 2
mission "standing" state="ready" {
  goal "Keep the planner available."
  agent "planner" {
workspace "."
command "true"
mission-authority {
  publish "project/generated"
  publish "example/fabric/*"
  start "example/fabric/*"
  revise "example/fabric/queue"
}
  }
}"#,
        "node",
    )
    .unwrap();
    let runtime = crate::graph::parse_execution_intent(
        intent.missions["standing"]
            .declarations_kdl
            .as_deref()
            .unwrap(),
        "node",
        "standing",
    )
    .unwrap();
    let desired = &runtime.subjects["agent/standing/planner"].desired;
    assert_eq!(
        crate::graph::declared_authority_blocks(desired),
        ["mission-authority"]
    );
    assert_eq!(
        crate::graph::mission_declared_authority_blocks(&intent.missions["standing"], "node")
            .unwrap(),
        std::collections::BTreeMap::from([(
            "agent/migration-proof/planner".to_owned(),
            vec!["mission-authority"]
        )])
    );

    for source in [
        r#"version 2
mission "bad" state="ready" { goal "Reject empty authority."; agent "bad" { workspace "."; command "true"; mission-authority { } } }"#,
        r#"version 2
mission "bad" state="ready" { goal "Reject a prefixed mission."; agent "bad" { workspace "."; command "true"; mission-authority { publish "mission/work" } } }"#,
        r#"version 2
mission "bad" state="ready" { goal "Reject an internal wildcard."; agent "bad" { workspace "."; command "true"; mission-authority { publish "work/*/bad" } } }"#,
        r#"version 2
mission "bad" state="ready" { goal "Reject a duplicate rule."; agent "bad" { workspace "."; command "true"; mission-authority { publish "work/*"; publish "work/*" } } }"#,
    ] {
        assert!(crate::graph::parse_intent(source, "node").is_err());
    }
}

#[test]
fn mission_authority_none_still_parses_and_other_values_are_refused() {
    let intent = crate::graph::parse_intent(
        r#"version 2
agent "fleet/fixture-quiet/standing/quiet" { workspace "."; command "true"; mission-authority "none"; }
agent "fleet/fixture-website/standing/website" { workspace "."; command "true"; }"#,
        "node",
    )
    .unwrap();
    assert_eq!(
        crate::graph::declared_authority_blocks(
            &intent.subjects["agent/fleet/fixture-quiet/standing/quiet"].desired
        ),
        ["mission-authority"]
    );
    assert!(
        crate::graph::declared_authority_blocks(
            &intent.subjects["agent/fleet/fixture-website/standing/website"].desired
        )
        .is_empty()
    );

    for source in [
        r#"version 2
agent "fleet/fixture-bad/seat" { workspace "."; command "true"; mission-authority "all"; }"#,
        r#"version 2
agent "fleet/fixture-bad/seat" { workspace "."; command "true"; mission-authority "none" { publish "fleet/fixture-bad/*" } }"#,
        r#"version 2
agent "fleet/fixture-bad/seat" { workspace "."; command "true"; mission-authority "none" "none"; }"#,
        r#"version 2
agent "fleet/fixture-bad/seat" { workspace "."; command "true"; mission-authority none=#true; }"#,
    ] {
        assert!(
            crate::graph::parse_intent(source, "node").is_err(),
            "{source}"
        );
    }
}
