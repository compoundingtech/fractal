//! Publication compatibility for agent charters longer than earlier releases accept. Those
//! releases bound an agent's description at 1,000 bytes whenever they parse its declaration
//! again, as a mission run does with the agents its mission declares, so a longer charter
//! publishes only once every active member advertises `features.agent_charter=1`.
use super::*;
use declared_admission::{FleetFeature, validate_fleet};

/// The description limit, in UTF-8 bytes, of releases that do not advertise `agent_charter`.
const EARLIER_LIMIT_BYTES: usize = 1_000;

const AGENT_CHARTER: FleetFeature = FleetFeature {
    keys: &["agent_charter"],
    code: "agent-charter-feature-barrier",
    carries: "an agent charter over 1,000 bytes",
};

fn too_long_for_earlier_releases(description: &str) -> bool {
    description.len() > EARLIER_LIMIT_BYTES
}

/// Whether declaration KDL declares, at any depth, an agent whose charter an earlier release
/// refuses.
fn kdl_declares(source: &str) -> bool {
    fn document_declares(document: &kdl::KdlDocument) -> bool {
        document.nodes().iter().any(|node| {
            node.children().is_some_and(|children| {
                (node.name().value() == "agent"
                    && children.nodes().iter().any(|child| {
                        child.name().value() == "description"
                            && child.entries().iter().any(|entry| {
                                entry.name().is_none()
                                    && entry
                                        .value()
                                        .as_string()
                                        .is_some_and(too_long_for_earlier_releases)
                            })
                    }))
                    || document_declares(children)
            })
        })
    }
    source
        .parse::<kdl::KdlDocument>()
        .is_ok_and(|document| document_declares(&document))
}

/// Whether a declaration value carries such a charter anywhere: a canonical agent node, or the
/// declaration KDL of a mission, its steps, nested missions and loop rounds.
fn json_declares(value: &Value) -> bool {
    match value {
        Value::Object(fields) => {
            (fields.get("name").and_then(Value::as_str) == Some("agent")
                && canonical_child_string(value, "description")
                    .is_some_and(|description| too_long_for_earlier_releases(&description)))
                || fields.iter().any(|(key, value)| match value {
                    Value::String(source) => key == "declarations_kdl" && kdl_declares(source),
                    value => json_declares(value),
                })
        }
        Value::Array(items) => items.iter().any(json_declares),
        _ => false,
    }
}

pub(super) fn validate_publication(
    connection: &Connection,
    origin: &str,
    intent: &NormalizedIntent,
) -> Result<(), FractalError> {
    let mut declared = intent
        .subjects
        .values()
        .any(|desired| json_declares(&desired.desired));
    for mission in intent.missions.values() {
        if declared {
            break;
        }
        declared = json_declares(&serde_json::to_value(mission).map_err(internal)?);
    }
    if declared {
        validate_fleet(connection, origin, &AGENT_CHARTER)?;
    }
    Ok(())
}

/// Cover direct claim publication as well as the declaration APIs. Receipt and replay do not
/// call this local-publication barrier.
pub(super) fn validate_claim(
    connection: &Connection,
    origin: &str,
    kind: &str,
    body: &Value,
) -> Result<(), FractalError> {
    if matches!(kind, "intent.desired" | "mission.published") && json_declares(body) {
        validate_fleet(connection, origin, &AGENT_CHARTER)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::directive_notes::tests_shared_delivery::{claim, enable};

    const ERROR: &str = "agent-charter-feature-barrier";

    /// Exactly 8,192 bytes over several lines, three-byte characters included.
    fn charter() -> String {
        format!("{}ab", "界 line\n".repeat(910))
    }

    fn agent(description: &str) -> String {
        format!(
            "version 2\nagent \"chartered\" {{ workspace \"/tmp\"; command \"true\"; description {description:?}; }}\n"
        )
    }

    fn mission(description: &str) -> String {
        format!(
            "version 2\nmission \"chartered\" state=\"ready\" {{\n  goal \"Work.\"\n  agent \"worker\" {{\n    workspace \"/tmp\"\n    description {description:?}\n    command \"true\"\n  }}\n  step \"work\" {{\n    assigned-to \"agent/${{ST_MISSION_RUN}}/worker\"\n    goal \"Work.\"\n  }}\n}}\n"
        )
    }

    fn publish(store: &Store, source: &str, key: &str) -> Result<ApplyResponse, FractalError> {
        let intent = crate::graph::parse_intent(source, &store.origin)?;
        let planned = store.mission(
            &intent,
            IntentInput {
                kdl: source.into(),
                source_name: None,
            },
        )?;
        store.apply_as(&intent, &planned.subject_tokens, key, Some("person/avery"))
    }

    fn started(store: &Store, features: Value) {
        claim(
            store,
            &format!("daemon/{}", store.origin),
            "daemon.started",
            None,
            json!({"status":"running", "features":features}),
        );
        store.replication_snapshot().unwrap();
    }

    #[test]
    fn a_charter_within_the_earlier_limit_needs_no_fleet_evidence() {
        let within = "x".repeat(EARLIER_LIMIT_BYTES);
        for source in [agent(&within), mission(&within)] {
            publish(&Store::open_memory("alder").unwrap(), &source, "within").unwrap();
        }
    }

    #[test]
    fn a_longer_charter_publishes_once_every_member_advertises_it() {
        assert_eq!(charter().len(), agent_spec::spec::AGENT_DESCRIPTION_MAX_BYTES);
        for source in [agent(&charter()), mission(&charter())] {
            let store = Store::open_memory("alder").unwrap();
            let before = store.index().unwrap();
            assert_eq!(publish(&store, &source, "unanchored").unwrap_err().code, ERROR);
            assert_eq!(store.index().unwrap(), before, "a refusal writes nothing");
            enable(&store);
            // Support for an earlier feature is not support for longer charters.
            started(&store, json!({"declared_admission":1}));
            let refusal = publish(&store, &source, "unadvertised").unwrap_err();
            assert_eq!(refusal.code, ERROR);
            assert!(
                refusal.message.contains("features.agent_charter=1"),
                "{}",
                refusal.message
            );
            started(&store, json!({"declared_admission":1, "agent_charter":1}));
            publish(&store, &source, "advertised").unwrap();
        }
    }

    #[test]
    fn direct_claims_meet_the_same_barrier() {
        let store = Store::open_memory("alder").unwrap();
        let connection = store.readers.get();
        let desired = |description: &str| {
            let intent = crate::graph::parse_intent(&agent(description), "alder").unwrap();
            let desired = intent.subjects.into_values().next().unwrap().desired;
            json!({"fields": {"kind": "agent", "desired": desired}})
        };
        let published = |description: &str| {
            json!({"fields": {"declarations_kdl": format!(
                "agent \"worker\" {{ description {description:?}; command \"true\"; }}"
            )}})
        };
        for (kind, body) in [
            ("intent.desired", desired(&charter())),
            ("mission.published", published(&charter())),
        ] {
            assert_eq!(
                validate_claim(&connection, "alder", kind, &body).unwrap_err().code,
                ERROR,
                "{kind}"
            );
        }
        let within = "x".repeat(EARLIER_LIMIT_BYTES);
        for (kind, body) in [
            ("intent.desired", desired(&within)),
            ("mission.published", published(&within)),
            ("message.sent", desired(&charter())),
        ] {
            validate_claim(&connection, "alder", kind, &body).unwrap();
        }
    }
}
