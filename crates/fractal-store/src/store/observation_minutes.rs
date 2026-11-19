//! Immutable phone minute claims. The graph is the only admission and evaluation authority.
use super::*;
use serde::{Deserialize, Serialize};

pub const KIND: &str = "custom.client.observation-minute";
const REPORT_LOOKUP: &str = "SELECT body FROM claims WHERE kind='custom.client.observation-minute' AND json_extract(body,'$.fields.source')=?1 AND json_extract(body,'$.fields.report_id')=?2";
const MINUTE_LOOKUP: &str = "SELECT body FROM claims WHERE subject=?1 AND kind='custom.client.observation-minute'";
const LIVE_MINUTES: &str = "SELECT id,store_index,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms FROM claims WHERE kind='custom.client.observation-minute' AND CAST(json_extract(body,'$.fields.start_ms') AS INTEGER)>=?1 AND CAST(json_extract(body,'$.fields.start_ms') AS INTEGER)<=?2 ORDER BY subject";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationMinute {
    pub source: String,
    pub pairing_grant: String,
    pub report_id: String,
    pub report_digest: String,
    pub target: String,
    pub carrier: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub start_ms: u64,
    pub count: u64,
    pub over: u64,
    pub max_ms: u64,
    pub buckets: Vec<(u64, u64)>,
}

impl ObservationMinute {
    pub fn subject(&self) -> String {
        let key = smallclaims::hash::canonical_hash(&(&self.source, &self.target, &self.carrier, &self.path, self.start_ms))
            .expect("minute key serializes");
        format!("custom/client/observation-minute-{key}")
    }

    fn same_content(&self, other: &Self) -> bool {
        self.source == other.source && self.report_id == other.report_id
            && self.report_digest == other.report_digest && self.target == other.target
            && self.carrier == other.carrier && self.path == other.path
            && self.start_ms == other.start_ms && self.count == other.count && self.over == other.over
            && self.max_ms == other.max_ms && self.buckets == other.buckets
    }
}

fn conflict() -> FractalError {
    FractalError::new("idempotency-conflict", "the source already reported this report ID or minute with different content")
}

fn source_for_grant(connection: &Connection, grant: &str, actor: &str, current: bool) -> Result<String, FractalError> {
    let completion = crate::pairing::latest_completion(connection, grant, None, smallclaims::store::now_ms())
        .map_err(internal)?.ok_or_else(|| FractalError::new("forbidden", "an admitted pairing grant is required"))?;
    if current && (!crate::pairing::completion_is_trusted(connection, &completion.id, &completion.origin,
        completion.actor.as_deref(), &completion.body["fields"], smallclaims::store::now_ms()).map_err(internal)?
        || crate::pairing::completion_is_revoked(connection, &completion.subject, &completion.id, &completion.origin).map_err(internal)?) {
        return Err(FractalError::new("forbidden", "the pairing grant expired or was revoked"));
    }
    let device = completion.body["fields"]["device_id"].as_str()
        .filter(|device| !device.is_empty())
        .ok_or_else(|| FractalError::new("forbidden", "pair again with a stable device identity before reporting observations"))?;
    if completion.body["fields"]["person_id"].as_str() != Some(actor) {
        return Err(FractalError::new("forbidden", "observations belong to the paired person"));
    }
    let digest = smallclaims::hash::canonical_hash(&(actor, device)).map_err(internal)?;
    Ok(format!("custom/client/observation-source-{digest}"))
}

impl Store {
    pub fn observation_source(&self, grant: &str, actor: &str) -> Result<String, FractalError> {
        source_for_grant(&self.readers.get(), grant, actor, true)
    }

    /// All report samples commit together. No local report cache, file, path or receiving
    /// member participates in identity. Equal retries return without appending claims.
    pub fn accept_observation_minutes(&self, actor: &str, minutes: &[ObservationMinute]) -> Result<bool, FractalError> {
        if minutes.is_empty() { return Err(FractalError::new("validation-failed", "empty observation report")); }
        let inputs = minutes.iter().map(|minute| {
            let fields = serde_json::from_value(serde_json::to_value(minute).map_err(internal)?).map_err(internal)?;
            let input = ClaimInput { subject: minute.subject(), kind: KIND.into(), actor: Some(actor.into()),
                fields, evidence: vec![], expected_subject: None, idempotency_key: None };
            validate_claim_input(&input)?;
            Ok(input)
        }).collect::<Result<Vec<_>, FractalError>>()?;
        run_fence::execute(&self.graph, None, Some(run_fence::API_TURN_BUDGET), |tx| {
            let first = &minutes[0];
            let source = source_for_grant(tx, &first.pairing_grant, actor, true)?;
            if minutes.iter().any(|minute| minute.source != source
                || minute.pairing_grant != first.pairing_grant
                || minute.report_id != first.report_id || minute.report_digest != first.report_digest) {
                return Err(FractalError::new("forbidden", "observation source must match the authenticated device"));
            }
            let mut reports = tx.prepare_cached(/* sqlite-large-value: allow-for-now lv-hfcc0104f3a41 owner=fractal_store::store::observation_minutes TODO=trace-statement-and-value-representation */ REPORT_LOOKUP).map_err(internal)?;
            let bodies = reports.query_map(params![first.source, first.report_id], |row| row.get::<_, String>(0)).map_err(internal)?;
            for body in bodies {
                let body: Value = serde_json::from_str(&body.map_err(internal)?).map_err(internal)?;
                if body["fields"]["report_digest"] != first.report_digest { return Err(conflict()); }
            }
            let mut missing = Vec::new();
            for (minute, input) in minutes.iter().zip(&inputs) {
                let mut existing = tx.prepare_cached(/* sqlite-large-value: allow-for-now lv-h8d4d7e936cfe owner=fractal_store::store::observation_minutes TODO=trace-statement-and-value-representation */ MINUTE_LOOKUP).map_err(internal)?;
                let rows = existing.query_map([&input.subject], |row| row.get::<_, String>(0)).map_err(internal)?;
                let mut found = false;
                for body in rows {
                    let body: Value = serde_json::from_str(&body.map_err(internal)?).map_err(internal)?;
                    let old: ObservationMinute = serde_json::from_value(body["fields"].clone()).map_err(internal)?;
                    if !old.same_content(minute) { return Err(conflict()); }
                    found = true;
                }
                if !found { missing.push(input); }
            }
            for input in &missing {
                let claim = append_claim_tx(tx, self.origin(), &input.subject, KIND, input.actor.as_deref(),
                    &json!({"fields":input.fields,"evidence":[]}), &[], None).map_err(internal)?;
                crate::pairing::append_operation_receipt_tx(tx, &claim).map_err(internal)?;
            }
            Ok(!missing.is_empty())
        })
    }

    /// Equal claims admitted independently by disconnected members collapse by minute key.
    /// A conflicting replicated key supplies no evidence: choosing a winner would invent a
    /// population. Reopening or replicating needs no local admission reconstruction.
    pub fn observation_minutes(&self, now_ms: u64) -> Result<Vec<ObservationMinute>> {
        self.read_snapshot_unindexed(|| {
            let connection = self.readers.get();
            let mut statement = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h9b74750ad85e owner=fractal_store::store::observation_minutes TODO=trace-statement-and-value-representation */ LIVE_MINUTES)?;
            let rows = statement.query_map(params![now_ms.saturating_sub(3_600_000), now_ms.saturating_sub(60_000)],
                smallclaims::store::claim_from_row)?;
            let mut unique = BTreeMap::<String, Option<ObservationMinute>>::new();
            for row in rows {
                let claim = row?;
                if !crate::pairing::operation_is_trusted(&connection, &claim.id, &claim.origin,
                    claim.actor.as_deref(), &claim.body["fields"])? { continue; }
                let Ok(minute) = serde_json::from_value::<ObservationMinute>(claim.body["fields"].clone()) else { continue; };
                if minute.subject() != claim.subject { continue; }
                match source_for_grant(&connection, &minute.pairing_grant, claim.actor.as_deref().unwrap_or_default(), false) {
                    Ok(source) if source == minute.source => {},
                    Ok(_) => continue,
                    Err(error) if error.code == "forbidden" => continue,
                    Err(error) => return Err(error.into()),
                }
                unique.entry(claim.subject).and_modify(|old| {
                    if old.as_ref().is_some_and(|old| !old.same_content(&minute)) { *old = None; }
                }).or_insert(Some(minute));
            }
            Ok(unique.into_values().flatten().collect())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observation_admission_and_live_window_seek_partial_indexes() {
        let store = Store::open_memory("index-proof").unwrap();
        let connection = store.readers.get();
        for (query, binds, expected) in [
            (REPORT_LOOKUP, vec!["source", "report"], "claims_observation_report_index"),
            (LIVE_MINUTES, vec!["0", "3600000"], "claims_observation_minute_time_index"),
            (MINUTE_LOOKUP, vec!["custom/client/observation-minute-example"], "INDEX"),
        ] {
            let plan = connection.prepare(&format!("EXPLAIN QUERY PLAN {query}")).unwrap()
                .query_map(rusqlite::params_from_iter(binds), |row| row.get::<_, String>(3)).unwrap()
                .collect::<rusqlite::Result<Vec<_>>>().unwrap().join("\n");
            assert!(plan.contains("SEARCH claims"), "{query}: {plan}");
            assert!(plan.contains(expected), "{query}: {plan}");
            assert!(!plan.contains("SCAN claims"), "{query}: {plan}");
        }
    }

    #[test]
    fn equal_claims_collapse_and_conflicting_claims_supply_no_evidence() {
        let store = Store::open_memory("minute-fold").unwrap();
        let grant = "custom/client/fold-phone";
        store.append_client_operation_claim(&ClaimInput {
            subject: grant.into(), kind: "custom.client.pairing-completed".into(),
            actor: Some("person/example".into()),
            fields: serde_json::from_value(json!({"device_id":"example-device-key",
                "person_id":"person/example","session_actor":"client/example-phone",
                "credential_hash":"example-digest","scopes":[],"expires_at_unix_ms":u64::MAX})).unwrap(),
            evidence: vec![], expected_subject: None, idempotency_key: None,
        }).unwrap();
        let minute = ObservationMinute {
            source: store.observation_source(grant, "person/example").unwrap(), pairing_grant: grant.into(),
            report_id: "11111111-1111-4111-8111-111111111111".into(), report_digest: "a".repeat(64),
            target: "ios-connect".into(), carrier: "fabric".into(), path: None,
            start_ms: 60_000, count: 1, over: 0, max_ms: 100, buckets: vec![(103, 1)],
        };
        assert!(store.accept_observation_minutes("person/example", std::slice::from_ref(&minute)).unwrap());
        let append_independent = |minute: &ObservationMinute| {
            store.seal_local_batches().unwrap();
            store.with_all_runs_fenced(|tx| -> Result<(), FractalError> {
                let claim = append_claim_tx(tx, store.origin(), &minute.subject(), KIND, Some("person/example"),
                    &json!({"fields":minute,"evidence":[]}), &[], None).map_err(internal)?;
                crate::pairing::append_operation_receipt_tx(tx, &claim).map_err(internal)?;
                Ok(())
            }).unwrap();
        };
        // Simulate separate immutable batches, as when disconnected receivers admit
        // the same phone report before either has received the other's claims.
        append_independent(&minute);
        assert_eq!(store.observation_minutes(120_000).unwrap().len(), 1);
        let mut conflict = minute;
        conflict.over = 1;
        append_independent(&conflict);
        assert!(store.observation_minutes(120_000).unwrap().is_empty());
    }
}
