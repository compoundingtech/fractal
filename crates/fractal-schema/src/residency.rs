//! Reader-first durable residency evidence. No producer or policy activation lives here.
use super::*;

pub const KINDS: &[&str] = &[
    "residency.wake-demand", "residency.resumed", "residency.first-turn",
    "residency.outcome", "residency.sleep-ended", "residency.eligibility-outcome",
    "residency.unrouted", "residency.host-sample",
];

pub(super) fn register(claims: &mut BTreeMap<String, ClaimSpec>) {
    for &kind in KINDS {
        let names: &[(&str, FieldSpec)] = match kind {
            "residency.wake-demand" => &[
                ("demand_at", required_integer()), ("origin_accepted_at", required_integer()),
                ("source_message", required_string()), ("suspended_incarnation", required_string()),
            ],
            "residency.resumed" => &[
                ("resumed_at", required_integer()), ("native_session", required_string()),
                ("incarnation", required_string()),
            ],
            "residency.first-turn" => &[
                ("first_turn_at", required_integer()), ("turn_id", required_string()),
            ],
            "residency.outcome" => &[
                ("observed_at", required_integer()), ("resume_failed", required_integer()),
                ("useless", required_integer()), ("reason", required_string()),
            ],
            "residency.sleep-ended" => &[
                ("suspended_at", required_integer()), ("ended_at", required_integer()),
                ("short_sleep", required_integer()),
                ("end_class", required_enum(&["wake", "manual", "stop", "retire"])),
            ],
            "residency.eligibility-outcome" => &[
                ("eligible_since", required_integer()), ("decided_at", required_integer()),
                ("refused", required_integer()), ("reason", required_string()),
            ],
            "residency.unrouted" => &[
                ("observed_at", required_integer()),
                ("kind", required_enum(&["attach", "explicit-resume"])),
            ],
            "residency.host-sample" => &[
                ("observed_at", required_integer()), ("held_silent_age_seconds", number()),
                ("asleep_seats", required_integer()), ("host_available_delta_bytes", integer()),
                ("private_reclaimed_bytes", integer()),
                ("memory_coverage", required_enum(&["complete", "partial", "unavailable"])),
            ],
            _ => unreachable!("registered residency kind"),
        };
        let mut fields: BTreeMap<String, FieldSpec> = names.iter().cloned()
            .map(|(name, spec)| (name.into(), spec)).collect();
        fields.insert("host".into(), required_string());
        fields.insert("seat".into(), reference_to(&["agent"]));
        claims.insert(kind.into(), ClaimSpec {
            kind: kind.into(), subjects: vec!["residency".into()], fields,
            additional_fields: false, write_policy: WritePolicy::SystemOnly,
            cardinality: if kind == "residency.host-sample" { Cardinality::Append } else { Cardinality::Once },
            projection: None, wakes_reconciler: false, source_kdl: Vec::new(),
            retention: claim_retention(kind),
        });
    }
}

pub(super) fn validate(subject: &str, kind: &str, fields: &BTreeMap<String, Value>) -> Result<(), ValidationError> {
    if !KINDS.contains(&kind) { return Ok(()); }
    let mut path = subject.split('/');
    let expected = match kind {
        "residency.wake-demand" | "residency.resumed" | "residency.first-turn" => "wake",
        "residency.outcome" => "outcome",
        "residency.sleep-ended" => "sleep",
        "residency.eligibility-outcome" => "eligibility",
        "residency.unrouted" => "unrouted",
        _ => "host",
    };
    let host = fields.get("host").and_then(Value::as_str).unwrap_or_default();
    if path.next() != Some("residency") || path.next() != Some(host)
        || path.next() != Some(expected) || path.next().is_none_or(str::is_empty)
        || path.next().is_some() || host.is_empty()
        || (expected == "host" && subject.rsplit('/').next() != Some(host))
    {
        return Err(error("invalid-claim-subject", "residency evidence needs residency/HOST/EPISODE/ID with the matching host"));
    }
    for (name, value) in fields {
        let valid = match name.as_str() {
            "resume_failed" | "useless" | "short_sleep" | "refused" => matches!(value.as_i64(), Some(0 | 1)),
            "held_silent_age_seconds" => value.as_f64().is_some_and(|n| n.is_finite() && n >= 0.0),
            "host_available_delta_bytes" | "private_reclaimed_bytes" => value.as_i64().is_some(),
            _ if value.is_number() => value.as_i64().is_some_and(|n| n >= 0),
            "reason" => value.as_str().is_some_and(|s| !s.is_empty() && s.len() <= 128 && !s.chars().any(char::is_control)),
            _ if value.is_string() => value.as_str().is_some_and(|s| !s.is_empty()),
            _ => true,
        };
        if !valid { return Err(error("invalid-claim-field", format!("invalid residency field `{name}`"))); }
    }
    if kind == "residency.host-sample" {
        let deltas = ["host_available_delta_bytes", "private_reclaimed_bytes"];
        let present = deltas.iter().filter(|name| fields.contains_key(**name)).count();
        match fields.get("memory_coverage").and_then(Value::as_str) {
            Some("complete") if present != 2 => return Err(error("missing-claim-field", "complete memory coverage needs both deltas")),
            Some("unavailable") if present != 0 => return Err(error("invalid-claim-field", "unavailable memory coverage has no deltas")),
            Some("partial") if present != 1 => return Err(error("invalid-claim-field", "partial memory coverage needs one delta")),
            _ => {},
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cases() -> Vec<(&'static str, &'static str, Value)> {
        vec![
            ("residency.wake-demand", "wake/d", json!({"demand_at":1,"origin_accepted_at":0,"source_message":"m","suspended_incarnation":"i"})),
            ("residency.resumed", "wake/d", json!({"resumed_at":2,"native_session":"s","incarnation":"i"})),
            ("residency.first-turn", "wake/d", json!({"first_turn_at":3,"turn_id":"t"})),
            ("residency.outcome", "outcome/o", json!({"observed_at":4,"resume_failed":0,"useless":1,"reason":"no-turn"})),
            ("residency.sleep-ended", "sleep/s", json!({"suspended_at":1,"ended_at":2,"short_sleep":1,"end_class":"wake"})),
            ("residency.eligibility-outcome", "eligibility/e", json!({"eligible_since":0,"decided_at":1,"refused":0,"reason":"suspended"})),
            ("residency.unrouted", "unrouted/u", json!({"observed_at":1,"kind":"attach"})),
            ("residency.host-sample", "host/example", json!({"observed_at":1,"asleep_seats":0,"memory_coverage":"unavailable"})),
        ]
    }

    #[test]
    fn all_residency_evidence_kinds_are_durable_closed_and_system_only() {
        for (kind, suffix, mut value) in cases() {
            value["host"] = json!("example");
            let mut fields = serde_json::from_value::<BTreeMap<String, Value>>(value).unwrap();
            let subject = format!("residency/example/{suffix}");
            let spec = registry().validate_replicated_claim(&subject, kind, &fields).unwrap();
            assert_eq!(spec.retention, Retention::Durable);
            assert_eq!(spec.write_policy, WritePolicy::SystemOnly);
            assert!(!spec.wakes_reconciler);
            assert!(!spec.additional_fields);
            assert!(registry().validate_public_claim(&subject, kind, &fields, Some("person/example")).is_err());
            fields.insert("unknown".into(), json!(0));
            assert_eq!(registry().validate_claim(&subject, kind, &fields).unwrap_err().code, "unknown-claim-field");
            fields.remove("unknown");
            for (name, field) in &spec.fields {
                if !field.required { continue; }
                let original = fields.remove(name).unwrap();
                assert_eq!(registry().validate_claim(&subject, kind, &fields).unwrap_err().code, "missing-claim-field");
                fields.insert(name.clone(), original);
            }
        }
    }

    #[test]
    fn residency_evidence_checks_flags_reasons_hosts_and_coverage_without_inventing_age() {
        for (kind, suffix, mut value) in cases() {
            value["host"] = json!("example");
            let subject = format!("residency/example/{suffix}");
            let mut fields: BTreeMap<String, Value> = serde_json::from_value(value).unwrap();
            assert!(registry().validate_claim(&subject.replace("example", "other"), kind, &fields).is_err());
            for name in ["resume_failed", "useless", "short_sleep", "refused", "reason", "kind", "end_class"] {
                if let Some(original) = fields.get(name).cloned() {
                    fields.insert(name.into(), if original.is_number() { json!(2) } else { json!("x".repeat(129)) });
                    assert!(registry().validate_claim(&subject, kind, &fields).is_err());
                    fields.insert(name.into(), original);
                }
            }
            if kind == "residency.host-sample" {
                assert!(!fields.contains_key("held_silent_age_seconds"));
                fields.insert("memory_coverage".into(), json!("complete"));
                assert!(registry().validate_claim(&subject, kind, &fields).is_err());
                fields.insert("host_available_delta_bytes".into(), json!(-1));
                fields.insert("private_reclaimed_bytes".into(), json!(42));
                registry().validate_claim(&subject, kind, &fields).unwrap();
                fields.insert("memory_coverage".into(), json!("unavailable"));
                assert!(registry().validate_claim(&subject, kind, &fields).is_err());
                fields.insert("memory_coverage".into(), json!("partial"));
                assert!(registry().validate_claim(&subject, kind, &fields).is_err());
                fields.remove("private_reclaimed_bytes");
                registry().validate_claim(&subject, kind, &fields).unwrap();
                fields.insert("held_silent_age_seconds".into(), json!(-1));
                assert!(registry().validate_claim(&subject, kind, &fields).is_err());
            }
        }
    }
}
