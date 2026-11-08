//! Observational provider quota and durable credit evidence; never admission authority.
use std::collections::BTreeMap;

use serde_json::{Map, Value};

use crate::{ValidationError, error};

pub const KIND: &str = "usage.limits.observed";
pub const DOCUMENTATION: &str = r#"
## Observed usage limits and credits

`usage.limits.observed` is observational only. Daemon configuration controls local append
admission and read-time folds: `[usage_limits] writers = ["agent/sample/usage-producer"]`.
Each configured agent writes as itself on its own subject. Absent or empty writers deny
local producer appends and exclude all usage observations from folds, not storage.
Replication and backup restore retain every schema-valid observation regardless of local
writers, checking hashes, signatures and same-agent authorship through ordinary admission.
Folds select only currently configured writers before merging quotas or credits. Changing
configuration, including after restart, folds already stored claims without re-replication;
it never changes stored evidence or graph and claim-source digests.
Both `readings` and `credit_readings` arrays are required; empty arrays assert no readings,
not zero usage or successful credit disposition. Every nested object is closed.

Each reading requires `account` (the st.account digest), `account_ref`, `provider`,
`window`, nullable `used_percent` (0–100), nullable `resets_at`, `measured_at`,
`coverage` (`reported`, `unreported`, `unsupported`), `source` (`omp-usage`) and `host`.
Reported readings require a percentage; other coverage requires null, never invented zero.

Each credit reading requires `account`, `account_ref`, `provider`, durable `credit_id`,
nullable `expires_at`, `measured_at`, `state` (`available`, `redeemed`, `forfeited`, `unknown`),
`disposition`, `coverage`, `source` and `host` with the same provenance semantics.
Each credit represents one window-equivalent. Disposition is exactly
`{kind, at, evidence_ref}`: kind is `redeemed`, `declined`, `planned` or `unknown`;
`at` is nullable and `evidence_ref` is nullable nonblank text.
Every nonnull timestamp is RFC3339; `measured_at` is required provider time.
Unknown, missing and stale evidence never implies successful use or disposition.

The complete claim is durable: checkpoints must not prune credit history by keeping
only the newest quota snapshot. Older members retain this unknown kind as retryable
`unknown-claim-kind` evidence, without projecting it or granting authority. Known
claims continue replicating, and an upgraded member replays the retained observations.

"#;

pub fn validate_actor(subject: &str, kind: &str, actor: Option<&str>) -> Result<(), ValidationError> {
    if kind == KIND && (!subject.starts_with("agent/") || actor != Some(subject)) {
        return Err(error("claim-write-forbidden", "usage observations require an agent writing as itself"));
    }
    Ok(())
}

fn invalid() -> ValidationError {
    error("invalid-usage-limits-observation", "invalid quota or credit observation shape")
}

fn object<'a>(value: &'a Value, keys: &[&str]) -> Result<&'a Map<String, Value>, ValidationError> {
    let object = value.as_object().ok_or_else(invalid)?;
    if object.len() != keys.len() || keys.iter().any(|key| !object.contains_key(*key)) {
        return Err(invalid());
    }
    Ok(object)
}

fn text(value: &Value) -> bool {
    value.as_str().is_some_and(|text| !text.trim().is_empty())
}

fn timestamp(value: &Value) -> bool {
    value.as_str().is_some_and(|text| chrono::DateTime::parse_from_rfc3339(text).is_ok())
}

fn nullable_timestamp(value: &Value) -> bool {
    value.is_null() || timestamp(value)
}

fn common(fields: &Map<String, Value>) -> bool {
    ["account", "account_ref", "provider", "host"].iter().all(|key| text(&fields[*key]))
        && timestamp(&fields["measured_at"])
        && fields["source"] == "omp-usage"
        && matches!(fields["coverage"].as_str(), Some("reported" | "unreported" | "unsupported"))
}

pub(crate) fn validate(subject: &str, kind: &str, fields: &BTreeMap<String, Value>) -> Result<(), ValidationError> {
    if kind != KIND {
        return Ok(());
    }
    if !subject.starts_with("agent/") {
        return Err(error("claim-write-forbidden", "usage observations must target an agent"));
    }
    for reading in fields.get("readings").and_then(Value::as_array).ok_or_else(invalid)? {
        let reading = object(reading, &["account", "account_ref", "provider", "window", "used_percent", "resets_at", "measured_at", "coverage", "source", "host"])?;
        let percent = &reading["used_percent"];
        if !common(reading) || !text(&reading["window"])
            || !nullable_timestamp(&reading["resets_at"])
            || !(percent.is_null() || percent.as_f64().is_some_and(|value| value.is_finite() && (0.0..=100.0).contains(&value)))
            || (reading["coverage"] == "reported" && percent.is_null())
            || (reading["coverage"] != "reported" && !percent.is_null()) {
            return Err(invalid());
        }
    }
    for credit in fields.get("credit_readings").and_then(Value::as_array).ok_or_else(invalid)? {
        let credit = object(credit, &["account", "account_ref", "provider", "credit_id", "expires_at", "measured_at", "state", "disposition", "coverage", "source", "host"])?;
        let disposition = object(&credit["disposition"], &["kind", "at", "evidence_ref"])?;
        if !common(credit) || !text(&credit["credit_id"])
            || !nullable_timestamp(&credit["expires_at"])
            || !matches!(credit["state"].as_str(), Some("available" | "redeemed" | "forfeited" | "unknown"))
            || !matches!(disposition["kind"].as_str(), Some("redeemed" | "declined" | "planned" | "unknown"))
            || !nullable_timestamp(&disposition["at"])
            || !(disposition["evidence_ref"].is_null() || text(&disposition["evidence_ref"])) {
            return Err(invalid());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Cardinality, Retention, registry, retention};
    use serde_json::json;
    const PRODUCER: &str = "agent/sample/usage-producer";

    fn fields() -> BTreeMap<String, Value> {
        BTreeMap::from([
            ("readings".into(), json!([{"account":"digest", "account_ref":"ada/claude", "provider":"anthropic", "window":"5h", "used_percent":45, "resets_at":null, "measured_at":"2026-11-07T10:00:00Z", "coverage":"reported", "source":"omp-usage", "host":"alder"}])),
            ("credit_readings".into(), json!([{"account":"digest", "account_ref":"ada/claude", "provider":"anthropic", "credit_id":"credit-1", "expires_at":"2026-11-08T10:00:00Z", "measured_at":"2026-11-07T10:00:00Z", "state":"available", "disposition":{"kind":"planned", "at":null, "evidence_ref":null}, "coverage":"reported", "source":"omp-usage", "host":"alder"}])),
        ])
    }

    #[test]
    fn observations_require_an_agent_writing_as_itself() {
        let fields = fields();
        let spec = registry().validate_public_claim(PRODUCER, KIND, &fields, Some(PRODUCER)).unwrap();
        assert_eq!(spec.cardinality, Cardinality::Append);
        for actor in [None, Some("person/ada"), Some("harness/omp"), Some("agent/run/worker"), Some("agent/sample/other-producer")] {
            assert_eq!(registry().validate_public_claim(PRODUCER, KIND, &fields, actor).unwrap_err().code, "claim-write-forbidden");
        }
        assert!(registry().validate_public_claim("agent/other", KIND, &fields, Some(PRODUCER)).is_err());
    }

    #[test]
    fn credit_evidence_is_durable_not_a_newest_snapshot() {
        assert_eq!(registry().claim(KIND).unwrap().retention, Retention::Durable);
        assert_eq!(retention::policy().kind(KIND).class, retention::Class::Durable);
        assert!(retention::policy().rules_for(KIND).next().is_none());
    }

    #[test]
    fn readings_require_provider_time_and_preserve_missing_coverage() {
        for field in ["measured_at", "resets_at"] {
            let mut fields = fields();
            fields.get_mut("readings").unwrap()[0][field] = json!("not-a-time");
            assert!(registry().validate_claim(PRODUCER, KIND, &fields).is_err());
        }
        for coverage in ["unreported", "unsupported"] {
            let mut fields = fields();
            fields.get_mut("readings").unwrap()[0]["coverage"] = json!(coverage);
            assert!(registry().validate_claim(PRODUCER, KIND, &fields).is_err());
            fields.get_mut("readings").unwrap()[0]["used_percent"] = Value::Null;
            registry().validate_claim(PRODUCER, KIND, &fields).unwrap();
        }
        let mut fields = fields();
        fields.get_mut("credit_readings").unwrap()[0]["disposition"]["kind"] = json!("success");
        assert!(registry().validate_claim(PRODUCER, KIND, &fields).is_err());
    }
}
