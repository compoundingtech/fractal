//! Observational credit objectives over durable omp-usage evidence; never admission policy.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::{Value, json};

#[cfg(test)]
use crate::model::ClaimRecord;
use crate::store::Store;

const DAY_MS: u64 = 24 * 60 * 60 * 1_000;

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
    measured_at: chrono::DateTime<chrono::FixedOffset>,
}

#[derive(Default)]
struct Credits {
    // Provider, account digest and durable credit identity; host and friendly ref are not identity.
    latest: BTreeMap<(String, String, String), Evidence>,
}

fn state_class(state: &str) -> u8 {
    match state {
        "redeemed" => 2,
        "forfeited" => 1,
        _ => 0,
    }
}

fn timestamp(text: &str) -> Option<u64> {
    chrono::DateTime::parse_from_rfc3339(text)
        .ok()
        .and_then(|time| u64::try_from(time.timestamp_millis()).ok())
}

impl Credits {
    fn observe_reading(&mut self, claim_id: &str, value: &Value) {
        let Ok(credit) = Credit::deserialize(value) else { return };
        let Ok(measured_at) = chrono::DateTime::parse_from_rfc3339(&credit.measured_at) else { return };
        let key = (credit.provider.clone(), credit.account.clone(), credit.credit_id.clone());
        // Terminal states are absorbing across writers and hosts, regardless of measurement time.
        // Within a state class, full provider time then claim ID determine the winner.
        if self.latest.get(&key).is_some_and(|previous| {
            (state_class(&previous.credit.state), previous.measured_at, previous.claim_id.as_str())
                >= (state_class(&credit.state), measured_at, claim_id)
        }) {
            return;
        }
        self.latest.insert(key, Evidence { credit, claim_id: claim_id.to_owned(), measured_at });
    }

    #[cfg(test)]
    fn observe(&mut self, claim: &ClaimRecord) {
        for value in claim.body["fields"]["credit_readings"].as_array().into_iter().flatten() {
            self.observe_reading(&claim.id, value);
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
                let Ok(measured_at) = u64::try_from(observation.measured_at.timestamp_millis()) else {
                    unknown = true;
                    continue;
                };
                stale |= now.saturating_sub(measured_at) > fresh_ms;
                unknown |= measured_at > now
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
                            .is_some_and(|at| at <= measured_at && at <= now)
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
                        "{objective} for {provider}: {target}. {current}. One durable credit is one window-equivalent; identities are provider/account/credit_id. Across writers and hosts, redeemed wins over forfeited, and forfeited wins over available or unknown regardless of timestamp. Within the same state class (available and unknown share a class), newest full-precision measured_at then claim ID wins. CAM-R01 counts confirmed forfeitures by expiry in the rolling seven days. CAM-R02 checks expiry in the next 24 hours and requires planned, redeemed or declined with a timestamp and evidence reference. Only observed credits are covered; missing, unsupported, unknown or stale evidence never proves success. Observational only: no enforcement."
                    ),
                    "evidence": sources,
                }));
            }
        }
        rows
    }
}

/// Read the incremental latest-per-writer inventory, not the durable observation history.
/// Retaining every writer's head lets configuration changes activate older authorized evidence.
/// Only currently configured writers contribute; changing that set never changes stored evidence.
pub fn rows(store: &Store, now: u64, fresh_ms: u64) -> anyhow::Result<Vec<Value>> {
    let credits = Credits::default();
    let writers = store.usage_limits_writers();
    // The unconfigured SLO/history tick must do no credit SQL, even on populated stores.
    if writers.is_empty() {
        return Ok(credits.rows(now, fresh_ms));
    }
    let snapshot = store.usage_credit_readings()?;
    Ok(snapshot_rows(&snapshot, &writers, now, fresh_ms))
}

fn snapshot_rows(
    snapshot: &crate::store::UsageCreditSnapshot,
    writers: &std::collections::BTreeSet<String>,
    now: u64,
    fresh_ms: u64,
) -> Vec<Value> {
    let mut credits = Credits::default();
    for reading in &snapshot.readings {
        if writers.contains(&reading.actor) {
            credits.observe_reading(&reading.claim_id, &reading.credit);
        }
    }
    let mut rows = credits.rows(now, fresh_ms);
    if snapshot.catching_up {
        for row in &mut rows {
            row["status"] = json!("no-data");
            row["remaining_percent"] = Value::Null;
            row["current"] = json!(format!(
                "unknown; catching-up through store index {} of {}; partial evidence: {}",
                snapshot.through_index, snapshot.target_index, row["current"].as_str().unwrap_or("unknown"),
            ));
            row["explain"] = json!(format!(
                "{} Inventory catching-up through store index {} of {} using resumable bounded batches; partial evidence cannot prove either objective.",
                row["explain"].as_str().unwrap_or(""),
                snapshot.through_index, snapshot.target_index,
            ));
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_799_452_800_000;
    const PRODUCER: &str = "agent/sample/usage-producer";

    fn time(at: u64) -> String {
        chrono::DateTime::from_timestamp_millis(at as i64).unwrap().to_rfc3339()
    }

    fn credit(id: &str, state: &str, expires: u64) -> Value {
        json!({
            "account": "digest", "account_ref": "account/one", "provider": "anthropic",
            "credit_id": id, "expires_at": time(expires), "measured_at": time(NOW),
            "state": state, "coverage": "reported", "source": "omp-usage", "host": "alder",
            "disposition": {"kind": "unknown", "at": null, "evidence_ref": null},
        })
    }

    fn claim(id: &str, readings: Value) -> ClaimRecord {
        serde_json::from_value(json!({
            "id": id, "store_index": 1, "batch_id": "batch", "subject": PRODUCER,
            "kind": "usage.limits.observed", "origin": "alder", "actor": PRODUCER,
            "body": {"fields": {"readings": [], "credit_readings": readings}},
            "predecessors": [], "accepted_at_unix_ms": NOW,
        })).unwrap()
    }

    #[test]
    fn catching_up_never_proves_either_objective_even_with_successful_partial_evidence() {
        let store = Store::open_memory("alder").unwrap();
        store.set_usage_limits_writers(vec![PRODUCER.into()]).unwrap();
        let reading = credit("one", "redeemed", NOW + 2 * DAY_MS);
        store.append_claim(&crate::model::ClaimInput {
            subject: PRODUCER.into(), kind: "usage.limits.observed".into(), actor: Some(PRODUCER.into()),
            fields: BTreeMap::from([
                ("readings".into(), json!([])), ("credit_readings".into(), json!([reading])),
            ]),
            evidence: Vec::new(), expected_subject: None, idempotency_key: None,
        }).unwrap();
        store.wait_for_usage_credit_backfill().unwrap();
        let complete = store.usage_credit_readings().unwrap();
        for row in snapshot_rows(&complete, &store.usage_limits_writers(), NOW, DAY_MS) {
            assert_eq!(row["status"], "met");
        }
        for through_index in [0, 3, 9] {
            for include_readings in [false, true] {
                let partial = crate::store::UsageCreditSnapshot {
                    readings: if include_readings { complete.readings.clone() } else { Vec::new() },
                    catching_up: true, through_index, target_index: 10,
                };
                let rows = snapshot_rows(&partial, &store.usage_limits_writers(), NOW, DAY_MS);
                assert_eq!(rows.len(), 2);
                for row in rows {
                    assert_eq!(row["status"], "no-data");
                    assert!(row["remaining_percent"].is_null());
                    let progress = format!("catching-up through store index {through_index} of 10");
                    assert!(row["current"].as_str().unwrap().contains(&progress));
                    assert!(row["explain"].as_str().unwrap().contains(&progress));
                }
            }
        }
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
    fn state_class_precedes_provider_time_in_both_arrival_orders() {
        for (older_state, newer_state, winner, status) in [
            ("redeemed", "unknown", "z", "met"),
            ("redeemed", "forfeited", "z", "met"),
            ("unknown", "available", "a", "met"),
        ] {
            let mut older = credit("one", older_state, NOW - DAY_MS);
            older["measured_at"] = json!(time(NOW - 1));
            let mut newer = credit("one", newer_state, NOW - DAY_MS);
            newer["host"] = json!("other");
            if newer_state == "available" {
                newer["expires_at"] = json!(time(NOW + 2 * DAY_MS));
            }
            let older = claim("z", json!([older]));
            let newer = claim("a", json!([newer]));
            for observations in [[&older, &newer], [&newer, &older]] {
                let mut credits = Credits::default();
                for observation in observations {
                    credits.observe(observation);
                }
                let selected = &credits.latest.values().next().unwrap().credit;
                assert_eq!(selected.state, if winner == "z" { older_state } else { newer_state });
                for row in credits.rows(NOW, DAY_MS) {
                    assert_eq!(row["evidence"][0]["claim_id"], winner);
                    assert_eq!(row["status"], status);
                }
            }
        }
    }

    #[test]
    fn provider_submillisecond_measurements_override_claim_id_and_arrival_order() {
        let older_at = "2026-11-08T10:00:00.000100Z";
        let newer_at = "2026-11-08T10:00:00.000900Z";
        let now = timestamp("2026-11-08T10:00:01Z").unwrap();
        for objective in [0, 1] {
            let expiry = if objective == 0 { now - DAY_MS } else { now + DAY_MS / 2 };
            let mut older = credit("one", if objective == 0 { "forfeited" } else { "available" }, expiry);
            older["measured_at"] = json!(older_at);
            older["disposition"] = json!({
                "kind": "planned", "at": older_at, "evidence_ref": "document/credit-plan",
            });
            let mut newer = credit("one", if objective == 0 { "forfeited" } else { "available" }, expiry);
            newer["measured_at"] = json!(newer_at);
            // The newer unknown disposition must replace the older documented plan.
            let older = claim("z", json!([older]));
            let newer = claim("a", json!([newer]));
            for observations in [[&older, &newer], [&newer, &older]] {
                let mut credits = Credits::default();
                for observation in observations {
                    credits.observe(observation);
                }
                let rows = credits.rows(now, DAY_MS);
                assert_eq!(rows[objective]["status"], "missed");
                assert_eq!(rows[objective]["evidence"][0]["claim_id"], "a");
                assert_eq!(rows[objective]["evidence"][0]["measured_at"], newer_at);
                assert_eq!(
                    rows[objective]["current"],
                    if objective == 0 { "1 forfeited window-equivalents" } else { "1 at-risk of 1 expiring credits" },
                );
            }
        }
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
    fn durable_credit_fold_uses_current_writers_across_revocation_and_restart() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("credits.db");
        let store = Store::open(&path, "alder").unwrap();
        store.set_usage_limits_writers(vec![PRODUCER.into()]).unwrap();
        store.wait_for_usage_credit_backfill().unwrap();
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
        let digest = store.replication_inventory().unwrap().digest;
        // Revocation hides retained evidence immediately, without mutating the durable graph.
        store.set_usage_limits_writers(Vec::new()).unwrap();
        let scope = smallclaims::sqlite::work::SqliteWorkScope::start();
        let unknown = rows(&store, now, DAY_MS).unwrap();
        let spent = scope.finish();
        assert_eq!(spent.statements, 0, "empty writer configuration must do no credit SQL: {spent:?}");
        for row in unknown {
            assert_eq!(row["status"], "no-data");
            assert!(row["evidence"].as_array().unwrap().is_empty());
        }
        let retained = store.claims_for_kind_at("usage.limits.observed", None, true, 256).unwrap();
        assert_eq!(retained.claims.len(), 1);
        assert_eq!(retained.claims[0].id, observed.id);
        assert_eq!(store.replication_inventory().unwrap().digest, digest);
        drop(store);
        let store = Store::open(&path, "alder").unwrap();
        let scope = smallclaims::sqlite::work::SqliteWorkScope::start();
        let unknown = rows(&store, now, DAY_MS).unwrap();
        let spent = scope.finish();
        assert_eq!(spent.statements, 0, "restart with no writers must do no credit SQL: {spent:?}");
        for row in unknown {
            assert_eq!(row["status"], "no-data");
            assert!(row["evidence"].as_array().unwrap().is_empty());
        }
        // Reconfiguring the local writer set activates the original claim, with no reappend.
        store.set_usage_limits_writers(vec![PRODUCER.into()]).unwrap();
        store.wait_for_usage_credit_backfill().unwrap();
        let rows = crate::api::slo_rows(&store).unwrap();
        for objective in ["CAM-R01/anthropic", "CAM-R02/anthropic"] {
            let row = rows.iter().find(|row| row["id"] == objective).unwrap();
            assert_eq!(row["status"], "met");
            assert_eq!(row["evidence"][0]["claim_id"], observed.id);
            assert_eq!(row["evidence"][0]["disposition"]["evidence_ref"], "document/credit-plan");
        }
        assert_eq!(store.replication_inventory().unwrap().digest, digest);
    }

    #[test]
    fn projected_cross_writer_state_precedence_is_arrival_order_independent() {
        const OTHER: &str = "agent/sample/other-producer";
        for (older_state, newer_state, older_wins, status) in [
            ("redeemed", "unknown", true, "met"),
            ("redeemed", "forfeited", true, "met"),
            ("unknown", "available", false, "met"),
        ] {
            for older_actor in [PRODUCER, OTHER] {
                let newer_actor = if older_actor == PRODUCER { OTHER } else { PRODUCER };
                for reverse in [false, true] {
                    let root = tempfile::tempdir().unwrap();
                    let path = root.path().join("credits.db");
                    let store = Store::open(&path, "alder").unwrap();
                    store.set_usage_limits_writers(vec![PRODUCER.into(), OTHER.into()]).unwrap();
                    store.wait_for_usage_credit_backfill().unwrap();
                    let mut older = credit("one", older_state, NOW - DAY_MS);
                    older["measured_at"] = json!(time(NOW - 1));
                    let mut newer = credit("one", newer_state, NOW - DAY_MS);
                    newer["host"] = json!("other");
                    if newer_state == "available" {
                        newer["expires_at"] = json!(time(NOW + 2 * DAY_MS));
                    }
                    let append = |actor: &str, reading: Value| {
                        store.append_claim(&crate::model::ClaimInput {
                            subject: actor.into(), kind: "usage.limits.observed".into(), actor: Some(actor.into()),
                            fields: BTreeMap::from([
                                ("readings".into(), json!([])), ("credit_readings".into(), json!([reading])),
                            ]),
                            evidence: Vec::new(), expected_subject: None, idempotency_key: None,
                        }).unwrap()
                    };
                    let (older_claim, newer_claim) = if reverse {
                        let newer_claim = append(newer_actor, newer);
                        (append(older_actor, older), newer_claim)
                    } else {
                        (append(older_actor, older), append(newer_actor, newer))
                    };
                    let expected = if older_wins { &older_claim.id } else { &newer_claim.id };
                    for row in rows(&store, NOW, DAY_MS).unwrap() {
                        assert_eq!(row["evidence"][0]["claim_id"], expected.as_str());
                        assert_eq!(row["evidence"][0]["state"], if older_wins { older_state } else { newer_state });
                        assert_eq!(row["status"], status);
                    }
                    drop(store);
                    let store = Store::open(&path, "alder").unwrap();
                    store.set_usage_limits_writers(vec![PRODUCER.into(), OTHER.into()]).unwrap();
                    store.wait_for_usage_credit_backfill().unwrap();
                    for row in rows(&store, NOW, DAY_MS).unwrap() {
                        assert_eq!(row["evidence"][0]["claim_id"], expected.as_str());
                        assert_eq!(row["status"], status);
                    }
                }
            }
        }
    }

    #[test]
    fn projected_credit_fold_preserves_precision_ties_and_each_writers_head() {
        const OTHER: &str = "agent/sample/other-producer";
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("credits.db");
        let store = Store::open(&path, "alder").unwrap();
        store.set_usage_limits_writers(vec![PRODUCER.into(), OTHER.into()]).unwrap();
        store.wait_for_usage_credit_backfill().unwrap();
        let now = timestamp("2026-11-08T10:00:01Z").unwrap();
        let append = |actor: &str, at: &str, state: &str| {
            let mut reading = credit("one", state, now - DAY_MS);
            reading["measured_at"] = json!(at);
            store.append_claim(&crate::model::ClaimInput {
                subject: actor.into(), kind: "usage.limits.observed".into(), actor: Some(actor.into()),
                fields: BTreeMap::from([
                    ("readings".into(), json!([])), ("credit_readings".into(), json!([reading])),
                ]),
                evidence: Vec::new(), expected_subject: None, idempotency_key: None,
            }).unwrap()
        };
        let older_at = "2026-11-08T10:00:00.000100Z";
        let newer_at = "2026-11-08T10:00:00.000900Z";
        let newer = append(PRODUCER, newer_at, "forfeited");
        append(PRODUCER, older_at, "unknown");
        let other = append(OTHER, "2026-11-08T10:00:00.000950Z", "forfeited");
        assert_eq!(rows(&store, now, DAY_MS).unwrap()[0]["evidence"][0]["claim_id"], other.id);
        store.set_usage_limits_writers(vec![PRODUCER.into()]).unwrap();
        let selected = rows(&store, now, DAY_MS).unwrap();
        assert_eq!(selected[0]["status"], "missed");
        assert_eq!(selected[0]["evidence"][0]["claim_id"], newer.id);
        assert_eq!(selected[0]["evidence"][0]["measured_at"], newer_at);
        let tied = append(PRODUCER, "2026-11-08T11:00:00.000900+01:00", "forfeited");
        let expected = if tied.id > newer.id { tied.id } else { newer.id };
        assert_eq!(rows(&store, now, DAY_MS).unwrap()[0]["evidence"][0]["claim_id"], expected);
        drop(store);
        let store = Store::open(&path, "alder").unwrap();
        store.set_usage_limits_writers(vec![PRODUCER.into()]).unwrap();
        store.wait_for_usage_credit_backfill().unwrap();
        assert_eq!(rows(&store, now, DAY_MS).unwrap()[0]["evidence"][0]["claim_id"], expected);
        store.set_usage_limits_writers(vec![OTHER.into()]).unwrap();
        assert_eq!(rows(&store, now, DAY_MS).unwrap()[0]["evidence"][0]["claim_id"], other.id);
    }
}
