//! Publication compatibility only: declarations do not enforce admission or resource limits.
use super::*;

fn refused(reason: impl Into<String>) -> FractalError {
    FractalError::new("declared-admission-feature-barrier", reason)
}

fn declares_fields(value: &Value) -> bool {
    ["declared_priority", "declared_lane", "declared_budgets"]
        .iter()
        .any(|field| value.get(field).is_some_and(|value| !value.is_null()))
}

fn declares_children(value: &Value) -> bool {
    value
        .get("children")
        .and_then(Value::as_array)
        .is_some_and(|children| {
            children.iter().any(|child| {
                child
                    .get("name")
                    .and_then(Value::as_str)
                    .is_some_and(|name| {
                        matches!(
                            name,
                            "declared-priority" | "declared-lane" | "declared-budgets"
                        )
                    })
            })
        })
}

fn declares_mission_json(mission: &Value) -> bool {
    declares_fields(mission)
        || mission
            .get("declarations_kdl")
            .and_then(Value::as_str)
            .is_some_and(declares_kdl)
        || mission
            .get("steps")
            .and_then(Value::as_object)
            .is_some_and(|steps| {
                steps.values().any(|step| {
                    step.get("declarations_kdl")
                        .and_then(Value::as_str)
                        .is_some_and(declares_kdl)
                        || step
                            .get("nested_mission")
                            .is_some_and(declares_mission_json)
                        || step.get("loop_spec").is_some_and(|loop_spec| {
                            ["round", "on_keep", "on_discard"].iter().any(|field| {
                                loop_spec.get(field).is_some_and(declares_mission_json)
                            })
                        })
                })
            })
}

fn declares_kdl(source: &str) -> bool {
    fn document_declares(document: &kdl::KdlDocument) -> bool {
        document.nodes().iter().any(|node| {
            node.children().as_ref().is_some_and(|children| {
                (matches!(node.name().value(), "agent" | "mission")
                    && children.nodes().iter().any(|child| {
                        matches!(
                            child.name().value(),
                            "declared-priority" | "declared-lane" | "declared-budgets"
                        )
                    }))
                    || document_declares(children)
            })
        })
    }
    source
        .parse::<kdl::KdlDocument>()
        .is_ok_and(|document| document_declares(&document))
}

fn declares_mission(mission: &MissionSpec) -> bool {
    mission.declared_priority.is_some()
        || mission.declared_lane.is_some()
        || mission.declared_budgets.is_some()
        || mission
            .declarations_kdl
            .as_deref()
            .is_some_and(declares_kdl)
        || mission.steps.values().any(|step| {
            step.declarations_kdl.as_deref().is_some_and(declares_kdl)
                || step.nested_mission.as_deref().is_some_and(declares_mission)
                || step.loop_spec.as_ref().is_some_and(|loop_spec| {
                    declares_mission(&loop_spec.round)
                        || loop_spec.on_keep.as_deref().is_some_and(declares_mission)
                        || loop_spec
                            .on_discard
                            .as_deref()
                            .is_some_and(declares_mission)
                })
        })
}

pub(super) fn validate_publication(
    connection: &Connection,
    origin: &str,
    intent: &NormalizedIntent,
) -> Result<(), FractalError> {
    let declared = intent.subjects.values().any(|desired| {
        (desired.kind == "agent" && declares_children(&desired.desired))
            || desired.member.as_ref().is_some_and(|member| {
                member.declared_priority.is_some()
                    || member.declared_lane.is_some()
                    || member.declared_budgets.is_some()
            })
    }) || intent.missions.values().any(declares_mission);
    if declared {
        validate_fleet(connection, origin)?;
    }
    Ok(())
}

/// Cover direct claim publication as well as the atomic declaration APIs. Receipt/replay does
/// not call this local-publication barrier: historical declarations remain readable.
pub(super) fn validate_claim(
    connection: &Connection,
    origin: &str,
    kind: &str,
    body: &Value,
) -> Result<(), FractalError> {
    // The writer persists the whole body, and desired/mission projection decodes that
    // body even when schema_fields_for_body selects an explicit `fields` object.
    // Mission capacity validation also reads `fields.body`. Inspect a conservative
    // superset of these locations, retaining the existing `body` envelope checks;
    // an empty decoy must never hide declarations in another persisted location.
    let declared = [
        Some(body),
        body.get("fields"),
        body.get("body"),
        body.get("fields").and_then(|fields| fields.get("body")),
    ]
    .into_iter()
    .flatten()
    .any(|payload| match kind {
        "intent.desired" => {
            declares_fields(payload)
                || payload.get("member").is_some_and(declares_fields)
                || (payload.get("kind").and_then(Value::as_str) == Some("agent")
                    && (declares_children(payload)
                        || payload.get("desired").is_some_and(declares_children)))
        }
        "mission.published" => declares_mission_json(payload),
        _ => false,
    });
    if declared {
        validate_fleet(connection, origin)?;
    }
    Ok(())
}

// Until the fleet-evidence gate lands, no local writer may persist these declarations.
fn validate_fleet(_connection: &Connection, _origin: &str) -> Result<(), FractalError> {
    Err(refused(
        "declared admission publication is disabled until the fleet feature gate lands; upgrade all members or fence legacy peers first",
    ))
}

#[cfg(test)]
mod fail_closed_tests;
