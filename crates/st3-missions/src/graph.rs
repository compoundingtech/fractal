//! The part of `graph` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use std::collections::{BTreeMap, BTreeSet, HashSet};
use kdl::{KdlDocument, KdlNode, KdlValue};
use serde_json::{Map, Value, json};
use sha2::{Digest as _, Sha256};
use crate::declaration_syntax::*;
pub use crate::declaration_syntax::parse_duration;
#[cfg(test)]
use crate::model::GateSpec;
use crate::model::{
    DesiredSubject, LaneSpec, LaunchSpec, MemberKind, MemberLifecycle, MemberSpec,
    MissionRevisionOperation, MissionRunCreation, MissionRunDeclaration, NamedCancellation,
    NormalizedIntent, ObserverSpec, PlannerSpec, PlanningFeedbackOperation,
    PlanningSessionCreation, PlanningSessionDeclaration, QuantifiedFieldSpec,
    ReplicaRepairDeclaration, ResourceRefreshOperation, RestartIntensity, RestartType,
    RuntimeResetOperation, ScheduleSpec, St3Error, SubscriptionConditionSpec, SubscriptionSpec,
};
pub use st3_store::graph::*;
pub use st3_store::graph::{agent_under, observer_spec, parse_execution_intent, parse_intent, parse_internal_intent, planning_planner_request, planning_planner_subject, runtime_id, runtime_proof_variables, schedule_spec, subscription_spec, validate_mission_runtimes};

/// Empty membership is meaningful only on the owned-set route; apply still requires explicit intent.
pub fn parse_owned_set_intent(source: &str, default_host: &str) -> Result<NormalizedIntent, St3Error> {
    let intent = parse_intent_with_owner(source, default_host, None, true, true)?;
    validate_mission_runtimes(&intent, default_host)?;
    Ok(intent)
}

/// Each agent a mission declares, in its own declarations, its steps' declarations, or any nested
/// mission, that carries an authority block, keyed by the agent subject of a proof run. Free mode
/// ignores these blocks, so a preview names them.
pub fn mission_declared_authority_blocks(
    mission: &crate::model::MissionSpec,
    default_host: &str,
) -> Result<BTreeMap<String, Vec<&'static str>>, St3Error> {
    fn visit(
        mission: &crate::model::MissionSpec,
        default_host: &str,
        blocks: &mut BTreeMap<String, Vec<&'static str>>,
    ) -> Result<(), St3Error> {
        let variables = runtime_proof_variables(mission);
        let sources = mission.declarations_kdl.iter().chain(
            mission
                .steps
                .values()
                .filter_map(|step| step.declarations_kdl.as_ref()),
        );
        for source in sources {
            let source = crate::mission::interpolate_kdl(source, &variables)?;
            let runtime = parse_execution_intent(&source, default_host, "migration-proof")?;
            for (subject, desired) in &runtime.subjects {
                let declared = declared_authority_blocks(&desired.desired);
                if desired.kind == "agent" && !declared.is_empty() {
                    blocks.insert(subject.clone(), declared);
                }
            }
        }
        for step in mission.steps.values() {
            if let Some(nested) = &step.nested_mission {
                visit(nested, default_host, blocks)?;
            }
        }
        Ok(())
    }
    let mut blocks = BTreeMap::new();
    visit(mission, default_host, &mut blocks)?;
    Ok(blocks)
}

pub fn resolve_document_references(
    source: &str,
    bindings: &BTreeMap<String, String>,
) -> Result<String, St3Error> {
    let mut document = source
        .parse::<KdlDocument>()
        .map_err(|error| St3Error::new("invalid-kdl", error.to_string()))?;
    for node in document.nodes_mut() {
        resolve_node_documents(node, bindings);
    }
    document.autoformat();
    Ok(document.to_string())
}

fn resolve_node_documents(node: &mut KdlNode, bindings: &BTreeMap<String, String>) {
    for entry in node.entries_mut() {
        let replacement = match entry.value() {
            KdlValue::String(value) if value.starts_with("doc/") && !value.contains('@') => {
                bindings.get(value).map(|hash| format!("{value}@{hash}"))
            }
            _ => None,
        };
        if let Some(replacement) = replacement {
            entry.set_value(replacement);
        }
    }
    let gate = node.name().value() == "gate";
    let mission = node.name().value() == "mission";
    if let Some(children) = node.children_mut() {
        for child in children.nodes_mut() {
            if !(gate && is_document_gate(child) || mission && child.name().value() == "provenance") {
                resolve_node_documents(child, bindings);
            }
        }
    }
}

/// The authority blocks a desired agent declaration carries, including `mission-authority
/// "none"`. They still parse, so existing declarations stay valid, but free mode grants every
/// agent in the fleet what its person may do and ignores them until principals and grants land.
pub fn declared_authority_blocks(desired: &Value) -> Vec<&'static str> {
    let children = desired
        .get("children")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    [
        "mission-authority",
        "queue-authority",
        "seat-authority",
        "agent-authority",
    ]
    .into_iter()
    .filter(|block| {
        children
            .iter()
            .any(|child| child.get("name").and_then(Value::as_str) == Some(*block))
    })
    .collect()
}

/// The declared settings of a lane, or `stopped` once cleanup or a revision retired it.
pub fn lane_spec(value: &Value) -> Option<LaneSpec> {
    if value.get("name").and_then(Value::as_str) != Some("lane") {
        return None;
    }
    let children = value
        .get("children")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    if children.len() == 1 && children[0].get("name").and_then(Value::as_str) == Some("stop") {
        return Some(LaneSpec {
            entries: None,
            approver: None,
            stopped: true,
        });
    }
    let text = |name| {
        canonical_child_value(value, name)
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    Some(LaneSpec {
        entries: text("entries"),
        approver: text("approver"),
        stopped: false,
    })
}

/// Render a normalized desired tree rather than the original authored source.
pub fn render_agent_desired_kdl(desired: &Value) -> Result<String, St3Error> {
    if desired.get("name").and_then(Value::as_str) != Some("agent") {
        return Err(St3Error::new(
            "invalid-declaration",
            "expected an agent root",
        ));
    }
    let mut document = KdlDocument::new();
    let mut version = KdlNode::new("version");
    version.entries_mut().push(kdl::KdlEntry::new(2_i128));
    document.nodes_mut().push(version);
    document.nodes_mut().push(render_desired_node(desired)?);
    document.autoformat();
    Ok(document.to_string())
}

/// Preserve environment names while hiding values in a declaration read.
pub fn redact_agent_env_values(tree: &mut Value) {
    let is_env = tree.get("name").and_then(Value::as_str) == Some("env");
    if let Some(children) = tree.get_mut("children").and_then(Value::as_array_mut) {
        for child in children {
            if is_env {
                if let Some(arguments) = child.get_mut("arguments").and_then(Value::as_array_mut) {
                    for value in arguments {
                        *value = Value::String("<redacted>".into());
                    }
                }
                if let Some(properties) = child.get_mut("properties").and_then(Value::as_object_mut)
                {
                    for value in properties.values_mut() {
                        *value = Value::String("<redacted>".into());
                    }
                }
            } else {
                redact_agent_env_values(child);
            }
        }
    }
}

pub fn render_desired_node(tree: &Value) -> Result<KdlNode, St3Error> {
    let invalid = || St3Error::new("invalid-declaration", "malformed desired KDL node tree");
    let object = tree.as_object().ok_or_else(invalid)?;
    let name = object
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(invalid)?;
    let mut node = KdlNode::new(name);
    if let Some(arguments) = object.get("arguments") {
        for value in arguments.as_array().ok_or_else(invalid)? {
            node.entries_mut()
                .push(kdl::KdlEntry::new(render_desired_value(value)?));
        }
    }
    if let Some(properties) = object.get("properties") {
        for (name, value) in properties.as_object().ok_or_else(invalid)? {
            node.entries_mut().push(kdl::KdlEntry::new_prop(
                name.as_str(),
                render_desired_value(value)?,
            ));
        }
    }
    if let Some(children) = object.get("children") {
        let mut document = KdlDocument::new();
        for child in children.as_array().ok_or_else(invalid)? {
            document.nodes_mut().push(render_desired_node(child)?);
        }
        if !document.nodes().is_empty() || name == "harness" {
            node.set_children(document);
        }
    } else if name == "harness" {
        // Older normalized trees omitted empty children. A validated harness
        // always has a body, so recover that required syntax when rendering it.
        node.set_children(KdlDocument::new());
    }
    Ok(node)
}

fn render_desired_value(value: &Value) -> Result<KdlValue, St3Error> {
    match value {
        Value::Null => Ok(KdlValue::Null),
        Value::Bool(value) => Ok(KdlValue::Bool(*value)),
        Value::String(value) => Ok(KdlValue::String(value.clone())),
        Value::Number(value) => {
            if let Some(integer) = value.as_i64() {
                Ok(KdlValue::Integer(integer.into()))
            } else if let Some(float) = value.as_f64() {
                Ok(KdlValue::Float(float))
            } else {
                Err(St3Error::new(
                    "invalid-declaration",
                    "desired KDL number is outside the supported range",
                ))
            }
        }
        _ => Err(St3Error::new(
            "invalid-declaration",
            "desired KDL entries must be scalar",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalized_agent_tree_round_trips_through_canonical_kdl() {
        for source in [
            r#"version 2
agent "dotfiles/steward" {
    host "dev3"
    workspace "/srv/work"
    env { TOKEN "private"; MODE "ready" }
    harness "omp" {
        model "openai-codex/gpt-6-sol"
        args "--foo" "--bar"
    }
}"#,
            "version 2\nagent \"fleet/cos/standing/cos\" { command \"true\" }",
        ] {
            let parsed = parse_intent(source, "node").unwrap();
            let (subject, desired) = parsed
                .subjects
                .iter()
                .find(|(_, desired)| desired.kind == "agent")
                .unwrap();
            let rendered = render_agent_desired_kdl(&desired.desired).unwrap();
            let reparsed = parse_intent(&rendered, "node").unwrap();
            assert_eq!(reparsed.subjects[subject].desired, desired.desired);
            assert_eq!(
                render_agent_desired_kdl(&reparsed.subjects[subject].desired).unwrap(),
                rendered
            );
        }
    }

    #[test]
    fn a_lane_is_a_mission_declaration_with_a_prefix_and_a_person_approver() {
        let source = |lane: &str| {
            format!(
                r#"version 2
mission "example/train" state="ready" {{
  goal "Merge one at a time."
  {lane}
  step "drive" {{ agentless; gate "done" {{ field "done" "resource/example/done" "is" "true" }} }}
}}
"#
            )
        };
        let valid = r#"lane "app" {
    entries "resource/github/acme/app/ci/pull-request/"
    approver "person/ada"
  }"#;
        let intent = parse_intent(&source(valid), "node").unwrap();
        let declarations = intent.missions["example/train"]
            .declarations_kdl
            .clone()
            .unwrap();
        let run = parse_execution_intent(&declarations, "node", "train-run").unwrap();
        let lane = &run.subjects["lane/train-run/app"];
        assert_eq!(lane.kind, "lane");
        assert_eq!(
            lane_spec(&lane.desired),
            Some(LaneSpec {
                entries: Some("resource/github/acme/app/ci/pull-request/".into()),
                approver: Some("person/ada".into()),
                stopped: false,
            })
        );
        parse_intent(&source(r#"lane "bare""#), "node").unwrap();

        for (lane, code) in [
            (
                r#"lane "app" { entries "resource/github/acme/app/ci/pull-request" }"#,
                "invalid-lane-entries",
            ),
            (
                r#"lane "app" { approver "agent/example/driver" }"#,
                "invalid-lane-approver",
            ),
            (r#"lane "app" { order "fifo" }"#, "unknown-child"),
        ] {
            assert_eq!(
                parse_intent(&source(lane), "node").unwrap_err().code,
                code,
                "{lane}"
            );
        }
        let in_step = source("").replace("step \"drive\" {", "step \"drive\" {\n    lane \"app\";");
        assert_eq!(
            parse_intent(&in_step, "node").unwrap_err().code,
            "lane-inside-step"
        );
        assert!(parse_intent("version 2\nlane \"app\"", "node").is_err());
    }

    #[test]
    fn plain_harness_bodies_survive_normalization_and_legacy_rendering() {
        for harness in ["claude", "codex", "pi", "omp", "opencode"] {
            let source = format!(
                "version 2\nagent \"worker\" {{ workspace \"/work\"; harness {harness:?} {{}} }}\n"
            );
            let intent = parse_test_intent(&source, "node").unwrap();
            assert!(parse_test_intent(&source.replace("{}", ""), "node").is_err());
            let desired = &intent.subjects["agent/node.worker"].desired;
            let body = desired["children"].as_array().unwrap()
                .iter().find(|child| child["name"] == "harness").unwrap();
            assert!(body.get("children").is_none());
            for explicit_empty in [false, true] {
                let mut tree = desired.clone();
                if explicit_empty {
                    let body = tree["children"].as_array_mut().unwrap()
                        .iter_mut().find(|child| child["name"] == "harness").unwrap();
                    body.as_object_mut().unwrap().insert("children".into(), json!([]));
                }
                let node = render_desired_node(&tree).unwrap();
                let restored = parse_test_intent(&format!("version 2\n{node}\n"), "node").unwrap();
                assert_eq!(restored.subjects["agent/node.worker"].desired, *desired);
            }
        }
    }

    #[test]
    fn built_in_gates_expand_to_st_gate_commands() {
        let source = r#"
version 2

  mission "proof" state="ready" {
    goal "Land the change."
    step "land" {
      gate "the fix merged" { merged "acme/app#42" }
      gate "CI passed on main" { ci-passed "linux-gate" repo="acme/app" branch="main" }
      gate "CI passed on the release" {
        ci-passed "st/ci" repo="acme/app" commit="${input.commit}"
        host "builder"
        time-limit "5m"
      }
      gate "the suite passes on main" {
        cargo-test "log_diet" package="st3"
        workspace "repo"
      }
      gate "the handoff is published" { document "doc/acme/handoff" }
    }
    input "commit" kind="text"
  }
"#;
        let intent = parse_test_intent(source, "node-a").unwrap();
        let gates = &intent.missions["proof"].steps["land"].gates;
        let mechanical = |index: usize| match &gates[index] {
            GateSpec::Mechanical {
                command,
                host,
                workspace,
                time_limit_ms,
                ..
            } => (
                command.as_str(),
                host.as_str(),
                workspace.as_str(),
                *time_limit_ms,
            ),
            other => panic!("gate {index} is not an exec gate: {other:?}"),
        };
        assert_eq!(
            mechanical(0),
            (
                r#""$ST3_BIN" gate merged acme/app#42"#,
                "node-a",
                "${ST_WORKSPACE}",
                120_000
            )
        );
        assert_eq!(
            mechanical(1).0,
            r#""$ST3_BIN" gate ci-passed linux-gate --repo acme/app --ref main"#
        );
        assert_eq!(
            mechanical(2),
            (
                r#""$ST3_BIN" gate ci-passed st/ci --repo acme/app --ref '${input.commit}'"#,
                "builder",
                "${ST_WORKSPACE}",
                300_000
            )
        );
        assert_eq!(
            mechanical(3),
            (
                r#""$ST3_BIN" gate cargo-test log_diet --package st3 --ref origin/main"#,
                "node-a",
                "repo",
                3_600_000
            )
        );
        assert!(matches!(
            &gates[4],
            GateSpec::Exists { subject, .. } if subject == "doc/acme/handoff"
        ));
        // Publication neither requires nor pins the document a gate waits for.
        assert!(intent.document_refs.is_empty());
        let resolved = resolve_document_references(
            source,
            &BTreeMap::from([("doc/acme/handoff".into(), "a".repeat(64))]),
        )
        .unwrap();
        assert!(
            resolved.contains(r#"document "doc/acme/handoff""#),
            "{resolved}"
        );

        for (gate, code) in [
            (r#"merged "acme/app""#, "invalid-pull-request"),
            (
                r#"ci-passed "linux-gate" repo="acme/app""#,
                "invalid-gate-shape",
            ),
            (r#"cargo-test "log_diet""#, "missing-gate-field"),
            (r#"document "resource/acme""#, "invalid-document-reference"),
            (
                r#"merged "acme/app#1"
        merged "acme/app#2""#,
                "invalid-gate-shape",
            ),
        ] {
            let source = format!(
                "version 2\nmission \"bad\" state=\"ready\" {{\n  goal \"Reject it.\"\n  step \"one\" {{\n    gate \"bad\" {{\n        {gate}\n    }}\n  }}\n}}\n"
            );
            assert_eq!(
                parse_test_intent(&source, "node-a").unwrap_err().code,
                code,
                "{gate}"
            );
        }
    }

    #[test]
    fn manual_rollout_is_a_strict_declaration_field_and_changes_the_digest() {
        let source = "version 2\nagent \"garden/orchard\" { rollout \"manual\"; harness \"claude\" { model \"example-model\"; } }";
        let parsed = parse_owned_set_intent(source, "amber").unwrap();
        assert!(crate::rollout::manual(
            &parsed.subjects["agent/garden/orchard"]
        ));
        let automatic =
            parse_owned_set_intent(&source.replace("rollout \"manual\";", ""), "amber").unwrap();
        assert_ne!(
            smallclaims::hash::canonical_hash(&parsed.subjects).unwrap(),
            smallclaims::hash::canonical_hash(&automatic.subjects).unwrap()
        );
        for field in [
            "rollout \"automatic\";",
            "rollout 1;",
            "rollout \"manual\"; rollout \"manual\";",
            "rollout \"manual\" { ignored; }",
            "rollout \"manual\" ignored=\"value\";",
        ] {
            assert!(
                parse_owned_set_intent(&source.replace("rollout \"manual\";", field), "amber")
                    .is_err(),
                "{field}"
            );
        }
    }

    #[test]
    fn a_mission_can_pin_bare_document_references() {
        let source = r#"
version 2

  message "task" {
    to "worker"
    content "doc/tasks/work"
  }

"#;
        let resolved = resolve_document_references(
            source,
            &BTreeMap::from([("doc/tasks/work".into(), "a".repeat(64))]),
        )
        .unwrap();
        let intent = parse_intent(&resolved, "node").unwrap();
        assert_eq!(
            intent.document_refs.into_iter().next().unwrap(),
            format!("doc/tasks/work@{}", "a".repeat(64))
        );
    }
}
