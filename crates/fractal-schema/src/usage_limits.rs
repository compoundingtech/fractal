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

Optional `source_frontier` is a closed object with `producer_incarnation`, `host`,
`sequence` (unsigned integer), nullable `previous_claim` (the actual accepted predecessor
claim ID), `accounts` (sorted distinct `{provider, account}` objects), `observed_at`,
`covered_from`, `covered_through` (RFC3339), and `complete` (boolean). Sequence zero
requires null predecessor; later deltas require one. The host must equal the original
claim origin, and each reading must belong to the fixed account roster. A new journal,
roster or continuity gap requires a new incarnation. Publication time is not coverage.
Each delta contains at most 128 quota and credit facts combined. Intermediate chunks
are incomplete; only a complete, contiguous source/mirror chain certifies its interval.
Collectors retain the accepted claim ID before sealing the next immutable request.
Older observations without a frontier remain readable evidence, never completeness.

Optional `period_readings` contains governing, comparable quota-period closures:
`{period_id, account, provider, window, closed_at, measured_at, unused_fraction,
close_kind, coverage, source, host, last_sample_at, max_gap_seconds}`. Text identities
are nonblank, timestamps RFC3339, source is `omp-usage`, close_kind is
`natural_rollover` or `early_reset`, max_gap_seconds is finite and nonnegative.
Coverage uses the same enum; reported unused_fraction is 0–1, otherwise null.
Unused fraction is an observed UPPER bound from the last pre-reset sample, never
an exact terminal forfeiture. It is reported only for monotone usage in one stable,
governing provider period; incomparable scoped windows cannot be added. Early resets
are not natural loss. Bounds crossing the configured budget remain indeterminate.
All account-bearing objects (quota, credit, period and roster entries) also permit
optional nullable `provider_plan`, populated from the authoritative account registry.
Missing plans remain readable historical evidence, but cannot prove plan-scoped loss
coverage. Changing the plan changes the fixed source roster and requires a new incarnation.
Optional bounded `gaps` records are closed `{provider, account_ref, provider_plan,
source, host, reason, measured_at}` objects, with nullable provider_plan, RFC3339
measured_at from the actual source observation, and nonblank text otherwise.
They expose unsupported or unresolved account identities outside the certified source
roster without inventing account digests. Gaps count toward the 128-fact delta limit;
they never certify empty provider populations.

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
    let optional_plan = keys.contains(&"account") && keys.contains(&"provider") && object.contains_key("provider_plan");
    if object.len() != keys.len() + usize::from(optional_plan)
        || keys.iter().any(|key| !object.contains_key(*key))
        || (optional_plan && !(object["provider_plan"].is_null() || text(&object["provider_plan"])))
    {
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
    if let Some(frontier) = fields.get("source_frontier") {
        let frontier = object(frontier, &[
            "producer_incarnation", "host", "sequence", "previous_claim", "accounts",
            "observed_at", "covered_from", "covered_through", "complete",
        ])?;
        let sequence = frontier["sequence"].as_u64().ok_or_else(invalid)?;
        let accounts = frontier["accounts"].as_array().ok_or_else(invalid)?;
        let mut previous = None;
        for account in accounts {
            let account = object(account, &["provider", "account"])?;
            if !text(&account["provider"]) || !text(&account["account"]) { return Err(invalid()); }
            let key = (account["provider"].as_str().unwrap(), account["account"].as_str().unwrap());
            if previous.is_some_and(|previous| previous >= key) { return Err(invalid()); }
            previous = Some(key);
        }
        if !text(&frontier["producer_incarnation"]) || !text(&frontier["host"])
            || accounts.is_empty() || !frontier["complete"].is_boolean()
            || !["observed_at", "covered_from", "covered_through"].iter().all(|key| timestamp(&frontier[*key]))
            || if sequence == 0 { !frontier["previous_claim"].is_null() } else { !text(&frontier["previous_claim"]) }
        { return Err(invalid()); }
        let parse = |key: &str| chrono::DateTime::parse_from_rfc3339(frontier[key].as_str().unwrap()).unwrap();
        if parse("covered_from") > parse("covered_through") || parse("covered_through") > parse("observed_at") {
            return Err(invalid());
        }
        let readings = fields.get("readings").and_then(Value::as_array).ok_or_else(invalid)?;
        let credits = fields.get("credit_readings").and_then(Value::as_array).ok_or_else(invalid)?;
        let periods = fields.get("period_readings").map(|value| value.as_array().ok_or_else(invalid)).transpose()?;
        let gaps = fields.get("gaps").map(|value| value.as_array().ok_or_else(invalid)).transpose()?;
        if readings.len() + credits.len() + periods.map_or(0, Vec::len) + gaps.map_or(0, Vec::len) > 128 { return Err(invalid()); }
        if gaps.into_iter().flatten().any(|gap| gap["host"] != frontier["host"]
            || gap["measured_at"].as_str().and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok()).is_none_or(|at| at > parse("covered_through")))
        { return Err(invalid()); }
        for reading in readings.iter().chain(credits).chain(periods.into_iter().flatten()) {
            if reading["host"] != frontier["host"]
                || !accounts.iter().any(|account| account["provider"] == reading["provider"] && account["account"] == reading["account"]
                    && account.get("provider_plan").unwrap_or(&Value::Null) == reading.get("provider_plan").unwrap_or(&Value::Null))
                || reading["measured_at"].as_str().and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
                    .is_none_or(|at| at > parse("covered_through"))
            { return Err(invalid()); }
        }
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
    if let Some(periods) = fields.get("period_readings") {
        for period in periods.as_array().ok_or_else(invalid)? {
            let period = object(period, &["period_id", "account", "provider", "window", "closed_at",
                "measured_at", "unused_fraction", "close_kind", "coverage", "source", "host",
                "last_sample_at", "max_gap_seconds"])?;
            let fraction = &period["unused_fraction"];
            if !["period_id", "account", "provider", "window", "host"].iter().all(|key| text(&period[*key]))
                || !["closed_at", "measured_at", "last_sample_at"].iter().all(|key| timestamp(&period[*key]))
                || period["source"] != "omp-usage"
                || !matches!(period["close_kind"].as_str(), Some("natural_rollover" | "early_reset"))
                || !matches!(period["coverage"].as_str(), Some("reported" | "unreported" | "unsupported"))
                || !period["max_gap_seconds"].as_f64().is_some_and(|gap| gap.is_finite() && gap >= 0.0)
                || if period["coverage"] == "reported" {
                    !fraction.as_f64().is_some_and(|fraction| fraction.is_finite() && (0.0..=1.0).contains(&fraction))
                } else { !fraction.is_null() }
            { return Err(invalid()); }
            let parse = |key: &str| chrono::DateTime::parse_from_rfc3339(period[key].as_str().unwrap()).unwrap();
            if parse("last_sample_at") > parse("closed_at") || parse("closed_at") > parse("measured_at") { return Err(invalid()); }
        }
    }
    if let Some(gaps) = fields.get("gaps") {
        let gaps = gaps.as_array().ok_or_else(invalid)?;
        if gaps.len() > 128 { return Err(invalid()); }
        for gap in gaps {
            let gap = object(gap, &["provider", "account_ref", "provider_plan", "source", "host", "reason", "measured_at"])?;
            if !["provider", "account_ref", "host", "reason"].iter().all(|key| text(&gap[*key]))
                || gap["source"] != "omp-usage"
                || !timestamp(&gap["measured_at"])
                || !(gap["provider_plan"].is_null() || text(&gap["provider_plan"]))
            { return Err(invalid()); }
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

    fn frontier_fields() -> BTreeMap<String, Value> {
        let mut fields = fields();
        fields.insert("source_frontier".into(), json!({
            "producer_incarnation":"journal-one","sequence":0,"previous_claim":null,"host":"alder",
            "accounts":[{"provider":"anthropic","account":"digest","provider_plan":null}],
            "observed_at":"2026-11-07T10:00:00Z","covered_from":"2026-11-07T10:00:00Z",
            "covered_through":"2026-11-07T10:00:00Z","complete":true
        }));
        fields
    }

    #[test]
    fn source_frontiers_are_closed_bounded_and_bind_roster_plan_host_and_clock() {
        registry().validate_claim(PRODUCER, KIND, &frontier_fields()).unwrap();
        for (key,value) in [
            ("sequence",json!(1)),("previous_claim",json!("not-null-at-zero")),
            ("host",json!("foreign")),("covered_through",json!("2026-11-08T10:00:00Z")),
            ("complete",json!("true")),("extra",json!(true)),
        ] {
            let mut fields = frontier_fields();
            fields.get_mut("source_frontier").unwrap()[key]=value;
            assert!(registry().validate_claim(PRODUCER,KIND,&fields).is_err(),"{key}");
        }
        let mut fields=frontier_fields();
        fields.get_mut("readings").unwrap()[0]["provider_plan"]=json!("anthropic/pro");
        assert!(registry().validate_claim(PRODUCER,KIND,&fields).is_err());
        let mut fields=frontier_fields();
        let reading=fields["readings"][0].clone();
        fields.insert("readings".into(),json!(vec![reading;128]));
        assert!(registry().validate_claim(PRODUCER,KIND,&fields).is_err(),"credit plus128 quota facts exceeds delta bound");
    }

    #[test]
    fn natural_periods_are_validated_as_bounds_and_explicit_gaps_need_no_fake_digest() {
        let mut fields=frontier_fields();
        let period=json!({"period_id":"period-one","account":"digest","provider":"anthropic","provider_plan":null,
            "window":"weekly","closed_at":"2026-11-07T10:00:00Z","measured_at":"2026-11-07T10:00:00Z",
            "last_sample_at":"2026-11-07T09:59:00Z","max_gap_seconds":60,"unused_fraction":0.5,
            "close_kind":"natural_rollover","coverage":"reported","source":"omp-usage","host":"alder"});
        fields.insert("period_readings".into(),json!([period]));
        fields.insert("gaps".into(),json!([{"provider":"example-unsupported","account_ref":"account/unsupported",
            "provider_plan":null,"source":"omp-usage","host":"alder","reason":"unsupported",
            "measured_at":"2026-11-07T10:00:00Z"}]));
        registry().validate_claim(PRODUCER,KIND,&fields).unwrap();
        for (key,value) in [("unused_fraction",json!(1.01)),("max_gap_seconds",json!(-1)),
            ("last_sample_at",json!("2026-11-08T00:00:00Z")),("coverage",json!("unreported"))] {
            let mut invalid=fields.clone();
            invalid.get_mut("period_readings").unwrap()[0][key]=value;
            assert!(registry().validate_claim(PRODUCER,KIND,&invalid).is_err(),"{key}");
        }
    }
}
