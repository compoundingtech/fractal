//! Publication compatibility only: declarations do not enforce admission or resource limits.
use super::*;

/// A publication every active fleet member advertises support for before any of it is written.
pub(super) struct FleetFeature {
    /// The `daemon.started` `features` keys a supporting member sets to `1`.
    pub keys: &'static [&'static str],
    /// The refusal code.
    pub code: &'static str,
    /// What the refused publication carries, as refusal messages name it.
    pub carries: &'static str,
}

impl FleetFeature {
    fn refused(&self, reason: impl Into<String>) -> FractalError {
        FractalError::new(self.code, reason)
    }
}

const DECLARED_ADMISSION: FleetFeature = FleetFeature {
    keys: &["declared_admission"],
    code: "declared-admission-feature-barrier",
    carries: "declared admission",
};

/// Attention asks additionally need every member to read them.
const DECLARED_ATTENTION: FleetFeature = FleetFeature {
    keys: &["declared_admission", "declared_attention"],
    ..DECLARED_ADMISSION
};

fn declares_fields(value: &Value, attention_only: bool) -> bool {
    if attention_only {
        return value.get("declared_budgets").and_then(|b| b.get("attention"))
            .and_then(Value::as_array).is_some_and(|entries| !entries.is_empty());
    }
    ["declared_priority", "declared_lane", "declared_budgets"]
        .iter()
        .any(|field| value.get(field).is_some_and(|value| !value.is_null()))
}

fn declares_children(value: &Value, attention_only: bool) -> bool {
    value
        .get("children")
        .and_then(Value::as_array)
        .is_some_and(|children| {
            children.iter().any(|child| {
                child
                    .get("name")
                    .and_then(Value::as_str)
                    .is_some_and(|name| {
                        if attention_only {
                            return name == "declared-budgets"
                                && child.get("children").and_then(Value::as_array).is_some_and(|entries| {
                                    entries.iter().any(|entry| entry.get("name").and_then(Value::as_str) == Some("attention"))
                                });
                        }
                        matches!(
                            name,
                            "declared-priority" | "declared-lane" | "declared-budgets"
                        )
                    })
            })
        })
}

fn declares_mission_json(mission: &Value, attention_only: bool) -> bool {
    declares_fields(mission, attention_only)
        || mission
            .get("declarations_kdl")
            .and_then(Value::as_str)
            .is_some_and(|source| declares_kdl(source, attention_only))
        || mission
            .get("steps")
            .and_then(Value::as_object)
            .is_some_and(|steps| {
                steps.values().any(|step| {
                    step.get("declarations_kdl")
                        .and_then(Value::as_str)
                        .is_some_and(|source| declares_kdl(source, attention_only))
                        || step
                            .get("nested_mission")
                            .is_some_and(|nested| declares_mission_json(nested, attention_only))
                        || step.get("loop_spec").is_some_and(|loop_spec| {
                            ["round", "on_keep", "on_discard"].iter().any(|field| {
                                loop_spec.get(field).is_some_and(|nested| declares_mission_json(nested, attention_only))
                            })
                        })
                })
            })
}

fn declares_kdl(source: &str, attention_only: bool) -> bool {
    fn document_declares(document: &kdl::KdlDocument, attention_only: bool) -> bool {
        document.nodes().iter().any(|node| {
            node.children().as_ref().is_some_and(|children| {
                (matches!(node.name().value(), "agent" | "mission")
                    && children.nodes().iter().any(|child| {
                        if attention_only {
                            return child.name().value() == "declared-budgets"
                                && child.children().as_ref().is_some_and(|entries| {
                                    entries.nodes().iter().any(|entry| entry.name().value() == "attention")
                                });
                        }
                        matches!(
                            child.name().value(),
                            "declared-priority" | "declared-lane" | "declared-budgets"
                        )
                    }))
                    || document_declares(children, attention_only)
            })
        })
    }
    source
        .parse::<kdl::KdlDocument>()
        .is_ok_and(|document| document_declares(&document, attention_only))
}

fn declares_mission(mission: &MissionSpec, attention_only: bool) -> bool {
    (!attention_only && (mission.declared_priority.is_some() || mission.declared_lane.is_some()))
        || mission.declared_budgets.as_ref().is_some_and(|budgets| !attention_only || !budgets.attention.is_empty())
        || mission
            .declarations_kdl
            .as_deref()
            .is_some_and(|source| declares_kdl(source, attention_only))
        || mission.steps.values().any(|step| {
            step.declarations_kdl.as_deref().is_some_and(|source| declares_kdl(source, attention_only))
                || step.nested_mission.as_deref().is_some_and(|nested| declares_mission(nested, attention_only))
                || step.loop_spec.as_ref().is_some_and(|loop_spec| {
                    declares_mission(&loop_spec.round, attention_only)
                        || loop_spec.on_keep.as_deref().is_some_and(|nested| declares_mission(nested, attention_only))
                        || loop_spec
                            .on_discard
                            .as_deref()
                            .is_some_and(|nested| declares_mission(nested, attention_only))
                })
        })
}

pub(super) fn validate_publication(
    connection: &Connection,
    origin: &str,
    intent: &NormalizedIntent,
) -> Result<(), FractalError> {
    let declared = intent.subjects.values().any(|desired| {
        (desired.kind == "agent" && declares_children(&desired.desired, false))
            || desired.member.as_ref().is_some_and(|member| {
                member.declared_priority.is_some()
                    || member.declared_lane.is_some()
                    || member.declared_budgets.is_some()
            })
    }) || intent.missions.values().any(|mission| declares_mission(mission, false));
    if declared {
        let attention = intent.missions.values().any(|mission| declares_mission(mission, true));
        validate_fleet(connection, origin, if attention { &DECLARED_ATTENTION } else { &DECLARED_ADMISSION })?;
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
    let declares = |attention_only| fractal_missions_base::declared_admission::claim_payload_candidates(body)
    .any(|payload| match kind {
        "intent.desired" => {
            declares_fields(payload, attention_only)
                || payload.get("member").is_some_and(|member| declares_fields(member, attention_only))
                || (payload.get("kind").and_then(Value::as_str) == Some("agent")
                    && (declares_children(payload, attention_only)
                        || payload.get("desired").is_some_and(|desired| declares_children(desired, attention_only))))
        }
        "mission.published" => declares_mission_json(payload, attention_only),
        _ => false,
    });
    if declares(false) {
        validate_fleet(connection, origin, if declares(true) { &DECLARED_ATTENTION } else { &DECLARED_ADMISSION })?;
    }
    Ok(())
}

/// The directive-notes barrier's signed membership policy, with no local-member exemption.
pub(super) fn validate_fleet(
    connection: &Connection,
    origin: &str,
    feature: &FleetFeature,
) -> Result<(), FractalError> {
    let pending: bool = connection.query_row(/* sqlite-large-value: allow-for-now lv-h281752a36224 owner=fractal_store::store::declared_admission TODO=trace-statement-and-value-representation */ &format!(
        "SELECT EXISTS(SELECT 1 FROM claims c JOIN batches b ON b.id=c.batch_id
         LEFT JOIN replica_envelopes e ON e.batch_id=b.id
         WHERE c.origin=?1 AND c.kind IN ({})
           AND NOT EXISTS(SELECT 1 FROM replica_envelope_signatures s
             WHERE s.writer=b.origin AND s.sequence=b.replica_sequence
               AND s.envelope_hash=e.envelope_hash))",
        smallclaims::store::FLEET_CLAIM_KINDS,
    ), [origin], |row| row.get(0)).map_err(internal)?;
    if pending {
        return Err(feature.refused(format!(
            "pending local fleet membership must be signed before publishing {}",
            feature.carries
        )));
    }
    let membership = fleet_membership_tx(connection).map_err(internal)?;
    if membership.anchor().is_none()
        || !matches!(
            membership.state(origin),
            crate::fleet::MemberState::Current(_)
        )
    {
        return Err(feature.refused(format!(
            "{} requires anchored membership and an active local member; upgrade all members or fence legacy peers first",
            feature.carries
        )));
    }
    for member in membership
        .incarnations()
        .filter(|member| member.end.is_none())
    {
        if !matches!(
            membership.state(&member.name),
            crate::fleet::MemberState::Current(_)
        ) {
            return Err(feature.refused(format!(
                "conflicted fleet membership cannot enable {}",
                feature.carries
            )));
        }
        // Select the latest advertisement first; an unsigned or false replacement must not
        // fall back to an older positive advertisement, nor one from a previous incarnation.
        let supported: Option<bool> = connection.query_row(/* sqlite-large-value: allow-for-now lv-h1975c08b9306 owner=fractal_store::store::declared_admission TODO=trace-statement-and-value-representation */ &canonical_sql(
            "SELECT NOT EXISTS(SELECT 1 FROM json_each(?5) k
                     WHERE json_extract(claims.body,'$.fields.features.'||k.value) IS NOT 1)
                 AND EXISTS(SELECT 1 FROM replica_envelopes e JOIN replica_envelope_signatures s
                     ON s.writer=e.writer AND s.sequence=e.sequence AND s.envelope_hash=e.envelope_hash
                     WHERE e.batch_id=batches.id AND s.member_key=?4)
             FROM claims JOIN batches ON batches.id=claims.batch_id
             WHERE claims.subject=?1 AND claims.kind='daemon.started' AND claims.origin=?2
                 AND batches.replica_sequence>=?3 ORDER BY CANONICAL_DESC(claims) LIMIT 1"),
            params![format!("daemon/{}", member.name), member.name, member.start, member.member_key,
                serde_json::to_string(feature.keys).map_err(internal)?], |row| row.get(0))
            .optional().map_err(internal)?.flatten();
        if supported != Some(true) {
            let advertised = feature.keys.iter().map(|key| format!("features.{key}=1")).collect::<Vec<_>>();
            return Err(feature.refused(format!(
                "host/{} has not advertised anchored {} for its active incarnation; upgrade or fence it before publishing {}",
                member.name, advertised.join(" and "), feature.carries
            )));
        }
    }
    let fenced = membership.legacy_removed_names().collect::<BTreeSet<_>>();
    for (table, column) in [
        ("replica_envelopes", "writer"),
        ("batches", "origin"),
        ("checkpoint_envelopes", "writer"),
    ] {
        let mut statement = connection.prepare(/* sqlite-large-value: allow-for-now lv-h246466708f80 owner=fractal_store::store::declared_admission TODO=trace-statement-and-value-representation */ &format!(
            "SELECT {column} FROM {table} WHERE {column}>?1 ORDER BY {column} LIMIT 1"
        )).map_err(internal)?;
        let mut previous = String::new();
        while let Some(writer) = statement.query_row(/* sqlite-large-value: allow-for-now lv-heae45914ca84 owner=fractal_store::store::declared_admission TODO=trace-statement-and-value-representation */ [&previous], |row| row.get::<_, String>(0)).optional().map_err(internal)? {
            if !membership.is_keyed_writer(&writer) && !fenced.contains(&writer) {
                return Err(feature.refused(format!("legacy writer `{writer}` remains unfenced; feature advertisements do not establish complete legacy discovery")));
            }
            previous = writer;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod fail_closed_tests;
