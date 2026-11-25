//! Reader-first durable residency evidence. No producer or policy activation lives here.
use super::*;

pub const KINDS: &[&str] = &[
    "residency.wake-demand", "residency.resumed", "residency.first-turn",
    "residency.outcome", "residency.sleep-ended", "residency.eligibility-outcome",
    "residency.unrouted", "residency.host-sample",
    "residency.wake-audit", "residency.resume-audit", "residency.audit-coverage",
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
            "residency.wake-audit" => &[
                ("message_id", required_string()), ("accepted_at", required_integer()),
                ("suspension_id", required_string()),
                ("suspension_cause", required_enum(&["idle", "manual", "unknown"])),
                ("owner_received_at", required_integer()),
            ],
            "residency.resume-audit" => &[
                ("resume_operation_id", required_string()), ("suspension_id", required_string()),
                ("suspension_cause_at_admission", required_enum(&["idle", "manual", "unknown"])),
                ("suspension_cause_at_completion", enumeration(&["idle", "manual", "unknown"])),
                ("admitted_at", required_integer()), ("resumed_at", integer()),
                ("failure", string()), ("phase", required_enum(&["admitted", "completed"])),
            ],
            "residency.audit-coverage" => &[
                ("observed_at", required_integer()), ("covered_since", required_integer()),
                ("through_at", required_integer()),
                ("previous_coverage", string()), ("audit_ids", required_array()),
                ("audit_count", required_integer()), ("audit_digest", required_string()),
                ("audit_complete", required_boolean()),
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
            cardinality: if matches!(kind, "residency.host-sample" | "residency.audit-coverage") { Cardinality::Append } else { Cardinality::Once },
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
        "residency.wake-audit" => "wake-audit",
        "residency.resume-audit" => "resume-audit",
        "residency.audit-coverage" => "audit-coverage",
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
    if kind == "residency.resume-audit" {
        let completed = fields.get("phase").and_then(Value::as_str) == Some("completed");
        let cause = fields.contains_key("suspension_cause_at_completion");
        let resumed = fields.contains_key("resumed_at");
        let failed = fields.contains_key("failure");
        if (completed && (!cause || resumed == failed)) || (!completed && (cause || resumed || failed)) {
            return Err(error("invalid-claim-field", "resume audit completion needs a cause and exactly one result; admission has no completion fields"));
        }
    }
    if kind == "residency.audit-coverage" {
        let at = |name: &str| fields.get(name).and_then(Value::as_u64).unwrap_or_default();
        if at("covered_since") > at("through_at") || at("through_at") > at("observed_at") {
            return Err(error("invalid-claim-field", "audit coverage needs covered_since <= through_at <= observed_at"));
        }
        let ids = fields.get("audit_ids").and_then(Value::as_array).expect("validated array");
        if ids.len() > 128 || ids.iter().any(|id| id.as_str().is_none_or(str::is_empty))
            || at("audit_count") != ids.len() as u64
            || fields.get("audit_digest").and_then(Value::as_str).is_none_or(|digest|
                digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()))
        {
            return Err(error("invalid-claim-field", "audit coverage needs at most 128 claim ids, their count and SHA-256 digest"));
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
            ("residency.wake-audit", "wake-audit/m", json!({"message_id":"message/m","accepted_at":1,"suspension_id":"claim/s","suspension_cause":"idle","owner_received_at":2})),
            ("residency.resume-audit", "resume-audit/r", json!({"resume_operation_id":"claim/r","suspension_id":"claim/s","suspension_cause_at_admission":"idle","admitted_at":2,"phase":"admitted"})),
            ("residency.audit-coverage", "audit-coverage/c", json!({"observed_at":3,"covered_since":1,"through_at":2,"audit_ids":[],"audit_count":0,"audit_digest":"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855","audit_complete":true})),
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

    #[test]
    fn audit_resume_phases_require_historical_cause_and_exactly_one_terminal_result() {
        let subject = "residency/example/resume-audit/r";
        let mut fields: BTreeMap<String, Value> = serde_json::from_value(json!({
            "host":"example","resume_operation_id":"claim/r","suspension_id":"claim/s",
            "suspension_cause_at_admission":"idle","admitted_at":2,"phase":"admitted"
        })).unwrap();
        registry().validate_replicated_claim(subject, "residency.resume-audit", &fields).unwrap();
        fields.insert("phase".into(), json!("completed"));
        assert!(registry().validate_claim(subject, "residency.resume-audit", &fields).is_err());
        fields.insert("suspension_cause_at_completion".into(), json!("manual"));
        fields.insert("resumed_at".into(), json!(3));
        registry().validate_replicated_claim(subject, "residency.resume-audit", &fields).unwrap();
        fields.insert("failure".into(), json!("manual-suspension"));
        assert!(registry().validate_claim(subject, "residency.resume-audit", &fields).is_err());
        fields.remove("resumed_at");
        registry().validate_claim(subject, "residency.resume-audit", &fields).unwrap();
    }

    #[test]
    fn audit_coverage_cannot_invert_or_extend_its_observation() {
        let subject = "residency/example/audit-coverage/c";
        for (since, through, valid) in [(1, 2, true), (3, 2, false), (1, 4, false)] {
            let fields = serde_json::from_value(json!({
                "host":"example","observed_at":3,"covered_since":since,"through_at":through,
                "audit_ids":[],"audit_count":0,"audit_digest":"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855","audit_complete":true
            })).unwrap();
            assert_eq!(registry().validate_claim(subject, "residency.audit-coverage", &fields).is_ok(), valid);
        }
    }
}
