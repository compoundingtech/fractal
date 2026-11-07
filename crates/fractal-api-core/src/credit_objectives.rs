//! Observational credit objectives over durable omp-usage evidence; never admission policy.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::{Value, json};

use crate::model::ClaimRecord;
use crate::store::Store;

const DAY_MS: u64 = 24 * 60 * 60 * 1_000;
const PRODUCER: &str = "agent/dotfiles/omp-usage-fleet-dev3";

#[derive(Deserialize)]
struct Disposition {
    kind: String,
    at: Option<String>,
    evidence_ref: Option<String>,
}

#[derive(Deserialize)]
struct Credit {
    account: String,
    account_ref: String,
    provider: String,
    credit_id: String,
    expires_at: Option<String>,
    measured_at: String,
    state: String,
    disposition: Disposition,
    coverage: String,
    source: String,
    host: String,
}

struct Evidence {
    credit: Credit,
    claim_id: String,
    measured_at: u64,
}

#[derive(Default)]
struct Credits {
    // Provider, account digest and durable credit identity; host and friendly ref are not identity.
    latest: BTreeMap<(String, String, String), Evidence>,
}

fn timestamp(text: &str) -> Option<u64> {
    chrono::DateTime::parse_from_rfc3339(text)
        .ok()
        .and_then(|time| u64::try_from(time.timestamp_millis()).ok())
}

impl Credits {
    fn observe(&mut self, claim: &ClaimRecord) {
        if claim.kind != "usage.limits.observed" || claim.subject != PRODUCER {
            return;
        }
        for value in claim.body["fields"]["credit_readings"].as_array().into_iter().flatten() {
            let Ok(credit) = Credit::deserialize(value) else { continue };
            let Some(measured_at) = timestamp(&credit.measured_at) else { continue };
            let key = (credit.provider.clone(), credit.account.clone(), credit.credit_id.clone());
            // Provider measurement time wins over delivery order. Claim ID breaks equal-time ties
            // identically on every member, independent of local replication/store order.
            if self.latest.get(&key).is_some_and(|previous| {
                (previous.measured_at, previous.claim_id.as_str()) >= (measured_at, claim.id.as_str())
            }) {
                continue;
            }
            self.latest.insert(key, Evidence { credit, claim_id: claim.id.clone(), measured_at });
        }
    }

    fn rows(&self, now: u64, fresh_ms: u64) -> Vec<Value> {
        let mut providers: BTreeMap<&str, Vec<&Evidence>> = BTreeMap::new();
        for evidence in self.latest.values() {
            providers.entry(&evidence.credit.provider).or_default().push(evidence);
        }
        if providers.is_empty() {
            providers.insert("unknown", Vec::new());
        }
        let mut rows = Vec::new();
        for (provider, evidence) in providers {
            let mut unknown = evidence.is_empty();
            let mut stale = false;
            let mut forfeited = 0_u64;
            let mut expiring = 0_u64;
            let mut at_risk = 0_u64;
            for observation in &evidence {
                let credit = &observation.credit;
                stale |= now.saturating_sub(observation.measured_at) > fresh_ms;
                unknown |= observation.measured_at > now
                    || credit.coverage != "reported"
                    || credit.source != "omp-usage"
                    || !matches!(credit.state.as_str(), "available" | "redeemed" | "forfeited");
                let expiry = credit.expires_at.as_deref().and_then(timestamp);
                let Some(expiry) = expiry else {
                    unknown = true;
                    continue;
                };
                if credit.state == "forfeited" && expiry > now.saturating_sub(7 * DAY_MS) && expiry <= now {
                    forfeited += 1;
                }
                // An expired but still available credit has no confirmed terminal outcome.
                if credit.state == "available" && expiry <= now {
                    unknown = true;
                }
                if expiry > now && expiry <= now.saturating_add(DAY_MS) {
                    expiring += 1;
                    let disposition = &credit.disposition;
                    let documented = matches!(disposition.kind.as_str(), "planned" | "redeemed" | "declined")
                        && disposition.at.as_deref().and_then(timestamp)
                            .is_some_and(|at| at <= observation.measured_at && at <= now)
                        && disposition.evidence_ref.as_deref().is_some_and(|reference| !reference.trim().is_empty());
                    if !documented {
                        at_risk += 1;
                    }
                }
            }
            let sources: Vec<Value> = evidence.iter().map(|observation| {
                let credit = &observation.credit;
                json!({
                    "account": credit.account, "account_ref": credit.account_ref,
                    "credit_id": credit.credit_id, "claim_id": observation.claim_id,
                    "measured_at": credit.measured_at, "expires_at": credit.expires_at,
                    "state": credit.state, "coverage": credit.coverage, "host": credit.host,
                    "disposition": {"kind": credit.disposition.kind, "at": credit.disposition.at,
                        "evidence_ref": credit.disposition.evidence_ref},
                })
            }).collect();
            for (objective, window, target, count, current) in [
                ("CAM-R01", "7d", "0 forfeited window-equivalents", forfeited,
                    format!("{forfeited} forfeited window-equivalents")),
                ("CAM-R02", "24h", "every expiring credit has a documented disposition", at_risk,
                    format!("{at_risk} at-risk of {expiring} expiring credits")),
            ] {
                let status = if stale { "stale" } else if unknown { "no-data" } else if count > 0 { "missed" } else { "met" };
                let known = !stale && !unknown;
                rows.push(json!({
                    "id": format!("{objective}/{provider}"), "kind": "objective", "provider": provider,
                    "about": format!("{objective}: {provider} observed credits"), "target": target,
                    "current": if known { current.clone() } else { format!("unknown; last evidence: {current}") },
                    "window": window, "status": status,
                    "remaining_percent": if known { Some(if count == 0 { 100.0 } else { 0.0 }) } else { None },
                    "explain": format!(
                        "{objective} for {provider}: {target}. {current}. One durable credit is one window-equivalent; identities are provider/account/credit_id. CAM-R01 counts confirmed forfeitures by expiry in the rolling seven days. CAM-R02 checks expiry in the next 24 hours and requires planned, redeemed or declined with a timestamp and evidence reference. Only observed credits are covered; missing, unsupported, unknown or stale evidence never proves success. Observational only: no enforcement."
                    ),
                    "evidence": sources,
                }));
            }
        }
        rows
    }
}

/// Fold all durable pages, rather than a bounded recent page: an older forfeiture or an
/// unrefreshed identity must not disappear and turn an incomplete inventory into success.
pub fn rows(store: &Store, now: u64, fresh_ms: u64) -> anyhow::Result<Vec<Value>> {
    let mut credits = Credits::default();
    let mut cursor = None;
    loop {
        let page = store.claims_for_kind_at("usage.limits.observed", cursor, true, 256)?;
        for claim in &page.claims {
            credits.observe(claim);
        }
        let Some(next) = page.next_cursor else { break };
        cursor = Some(next);
    }
    Ok(credits.rows(now, fresh_ms))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_799_452_800_000;

    fn time(at: u64) -> String {
        chrono::DateTime::from_timestamp_millis(at as i64).unwrap().to_rfc3339()
    }

    fn credit(id: &str, state: &str, expires: u64) -> Value {
        json!({
            "account": "digest", "account_ref": "account/one", "provider": "anthropic",
            "credit_id": id, "expires_at": time(expires), "measured_at": time(NOW),
            "state": state, "coverage": "reported", "source": "omp-usage", "host": "dev3",
            "disposition": {"kind": "unknown", "at": null, "evidence_ref": null},
        })
    }

    fn claim(id: &str, readings: Value) -> ClaimRecord {
        serde_json::from_value(json!({
            "id": id, "store_index": 1, "batch_id": "batch", "subject": PRODUCER,
            "kind": "usage.limits.observed", "origin": "dev3", "actor": PRODUCER,
            "body": {"fields": {"readings": [], "credit_readings": readings}},
            "predecessors": [], "accepted_at_unix_ms": NOW,
        })).unwrap()
    }

    #[test]
    fn deduplicates_identity_across_hosts_and_keeps_accounts_distinct() {
        let mut credits = Credits::default();
        let original = credit("one", "forfeited", NOW - DAY_MS);
        let mut duplicate = original.clone();
        duplicate["host"] = json!("other");
        let mut other_account = original.clone();
        other_account["account"] = json!("other-digest");
        credits.observe(&claim("a", json!([original, other_account])));
        credits.observe(&claim("b", json!([duplicate])));
        let rows = credits.rows(NOW, DAY_MS);
        assert_eq!(rows[0]["current"], "2 forfeited window-equivalents");
        assert_eq!(rows[0]["evidence"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn rolls_forfeiture_by_expiry_not_observation_time() {
        let mut credits = Credits::default();
        credits.observe(&claim("a", json!([
            credit("inside", "forfeited", NOW - 6 * DAY_MS),
            credit("boundary", "forfeited", NOW - 7 * DAY_MS),
            credit("outside", "forfeited", NOW - 8 * DAY_MS),
        ])));
        let rows = credits.rows(NOW, DAY_MS);
        assert_eq!(rows[0]["current"], "1 forfeited window-equivalents");
        assert_eq!(rows[0]["status"], "missed");
    }

    #[test]
    fn documented_dispositions_cover_expiring_credits() {
        for kind in ["planned", "redeemed", "declined"] {
            let mut reading = credit("one", "available", NOW + DAY_MS);
            reading["disposition"] = json!({"kind": kind, "at": time(NOW), "evidence_ref": "document/decision"});
            let mut credits = Credits::default();
            credits.observe(&claim("a", json!([reading])));
            assert_eq!(credits.rows(NOW, DAY_MS)[1]["status"], "met");
        }
    }

    #[test]
    fn unknown_or_undocumented_disposition_is_at_risk() {
        for disposition in [
            json!({"kind": "unknown", "at": null, "evidence_ref": null}),
            json!({"kind": "planned", "at": time(NOW), "evidence_ref": null}),
            json!({"kind": "redeemed", "at": time(NOW + 1), "evidence_ref": "future"}),
        ] {
            let mut reading = credit("one", "available", NOW + 1);
            reading["disposition"] = disposition;
            let mut credits = Credits::default();
            credits.observe(&claim("a", json!([reading])));
            let rows = credits.rows(NOW, DAY_MS);
            assert_eq!(rows[1]["status"], "missed");
            assert_eq!(rows[1]["current"], "1 at-risk of 1 expiring credits");
        }
    }

    #[test]
    fn missing_unknown_unsupported_and_stale_evidence_never_pass() {
        assert_eq!(Credits::default().rows(NOW, DAY_MS)[0]["status"], "no-data");
        for (field, value) in [
            ("state", json!("unknown")), ("coverage", json!("unreported")),
            ("coverage", json!("unsupported")), ("expires_at", Value::Null),
            ("measured_at", json!(time(NOW + 1))),
        ] {
            let mut reading = credit("one", "available", NOW + 2 * DAY_MS);
            reading[field] = value;
            let mut credits = Credits::default();
            credits.observe(&claim("a", json!([reading])));
            for row in credits.rows(NOW, DAY_MS) {
                assert_eq!(row["status"], "no-data");
                assert!(row["remaining_percent"].is_null());
            }
        }
        let mut credits = Credits::default();
        credits.observe(&claim("a", json!([credit("one", "available", NOW + 2 * DAY_MS)])));
        for row in credits.rows(NOW + DAY_MS + 1, DAY_MS) {
            assert_eq!(row["status"], "stale");
            assert!(row["remaining_percent"].is_null());
        }
    }

    #[test]
    fn older_delivery_does_not_replace_newer_provider_measurement() {
        let mut credits = Credits::default();
        let mut old = credit("one", "unknown", NOW + DAY_MS);
        old["measured_at"] = json!(time(NOW - 1));
        credits.observe(&claim("a", json!([credit("one", "available", NOW + 2 * DAY_MS)])));
        credits.observe(&claim("z", json!([old])));
        assert_eq!(credits.rows(NOW, DAY_MS)[0]["status"], "met");
    }

    #[test]
    fn provider_identity_and_equal_time_ties_are_delivery_order_independent() {
        let mut other = credit("one", "available", NOW + 2 * DAY_MS);
        other["provider"] = json!("openai");
        let a = claim("a", json!([credit("one", "unknown", NOW + DAY_MS), other]));
        let z = claim("z", json!([credit("one", "available", NOW + 2 * DAY_MS)]));
        let mut forward = Credits::default();
        forward.observe(&a);
        forward.observe(&z);
        let mut reverse = Credits::default();
        reverse.observe(&z);
        reverse.observe(&a);
        let rows = forward.rows(NOW, DAY_MS);
        assert_eq!(rows, reverse.rows(NOW, DAY_MS));
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0]["status"], "met");
        assert_eq!(rows[2]["id"], "CAM-R01/openai");
    }

    #[test]
    fn durable_observation_reaches_the_shared_slo_endpoint_rows() {
        let store = Store::open_memory("credit-objectives").unwrap();
        let now = chrono::Utc::now().timestamp_millis() as u64;
        let mut reading = credit("one", "available", now + DAY_MS / 2);
        reading["measured_at"] = json!(time(now));
        reading["disposition"] = json!({
            "kind": "planned", "at": time(now), "evidence_ref": "document/credit-plan",
        });
        let observed = store.append_claim(&crate::model::ClaimInput {
            subject: PRODUCER.into(),
            kind: "usage.limits.observed".into(),
            actor: Some(PRODUCER.into()),
            fields: BTreeMap::from([
                ("readings".into(), json!([])),
                ("credit_readings".into(), json!([reading])),
            ]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        }).unwrap();
        let rows = crate::api::slo_rows(&store).unwrap();
        for objective in ["CAM-R01/anthropic", "CAM-R02/anthropic"] {
            let row = rows.iter().find(|row| row["id"] == objective).unwrap();
            assert_eq!(row["status"], "met");
            assert_eq!(row["evidence"][0]["claim_id"], observed.id);
            assert_eq!(row["evidence"][0]["disposition"]["evidence_ref"], "document/credit-plan");
        }
    }
}
