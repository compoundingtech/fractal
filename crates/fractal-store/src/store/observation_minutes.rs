//! Immutable phone minute claims. The graph is the only admission and evaluation authority.
use super::*;
use serde::{Deserialize, Serialize};

pub const KIND: &str = "custom.client.observation-minute";
const REPORT_LOOKUP: &str = "SELECT id,store_index,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms FROM claims WHERE kind='custom.client.observation-minute' AND json_extract(body,'$.fields.source')=?1 AND json_extract(body,'$.fields.report_id')=?2";
const MINUTE_LOOKUP: &str = "SELECT id,store_index,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms FROM claims WHERE subject=?1 AND kind='custom.client.observation-minute'";
const LIVE_MINUTES: &str = "SELECT id,store_index,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms FROM claims INDEXED BY claims_observation_minute_time_index WHERE kind='custom.client.observation-minute' AND CAST(json_extract(body,'$.fields.start_ms') AS INTEGER)>=?1 AND CAST(json_extract(body,'$.fields.start_ms') AS INTEGER)<=?2";

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

/// Admission and evaluation must recognize exactly the same source-backed authority.
/// Merely replicated bytes, even in a signed batch, cannot claim a phone's replay key.
fn admitted_minute(connection: &Connection, claim: &ClaimRecord) -> Result<Option<ObservationMinute>, FractalError> {
    if !crate::pairing::operation_is_trusted(connection, &claim.id, &claim.origin,
        claim.actor.as_deref(), &claim.body["fields"]).map_err(internal)? { return Ok(None); }
    let Ok(minute) = serde_json::from_value::<ObservationMinute>(claim.body["fields"].clone()) else { return Ok(None); };
    if minute.subject() != claim.subject { return Ok(None); }
    match source_for_grant(connection, &minute.pairing_grant, claim.actor.as_deref().unwrap_or_default(), false) {
        Ok(source) if source == minute.source => Ok(Some(minute)),
        Ok(_) => Ok(None),
        Err(error) if error.code == "forbidden" => Ok(None),
        Err(error) => Err(error),
    }
}

/// Once-only cardinality, like replay detection, counts only admitted phone evidence.
pub(super) fn admitted_minute_exists(connection: &Connection, subject: &str) -> Result<bool, FractalError> {
    let mut statement = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h8d4d7e936cfe owner=fractal_store::store::observation_minutes TODO=trace-statement-and-value-representation */ MINUTE_LOOKUP).map_err(internal)?;
    let rows = statement.query_map([subject], smallclaims::store::claim_from_row).map_err(internal)?;
    for row in rows {
        if admitted_minute(connection, &row.map_err(internal)?)?.is_some() {
            return Ok(true);
        }
    }
    Ok(false)
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
            let rows = reports.query_map(params![first.source, first.report_id], smallclaims::store::claim_from_row).map_err(internal)?;
            for row in rows {
                let Some(old) = admitted_minute(tx, &row.map_err(internal)?)? else { continue; };
                if old.report_digest != first.report_digest { return Err(conflict()); }
            }
            let mut missing = Vec::new();
            for (minute, input) in minutes.iter().zip(&inputs) {
                let mut existing = tx.prepare_cached(/* sqlite-large-value: allow-for-now lv-h0b3749d94a8e owner=fractal_store::store::observation_minutes TODO=trace-statement-and-value-representation */ MINUTE_LOOKUP).map_err(internal)?;
                let rows = existing.query_map([&input.subject], smallclaims::store::claim_from_row).map_err(internal)?;
                let mut found = false;
                for row in rows {
                    let Some(old) = admitted_minute(tx, &row.map_err(internal)?)? else { continue; };
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
                let Some(minute) = admitted_minute(&connection, &claim)? else { continue; };
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
        let fleet = "11111111-1111-4111-8111-111111111112";
        store.bind_fleet(fleet).unwrap();
        let key = Arc::new(smallclaims::fleet::MemberKey::generate().unwrap().0);
        store.pin_fleet_anchor(key.public()).unwrap();
        store.set_member_key(Some(key.clone())).unwrap();
        store.admit_fleet_anchor(fleet, key.public(), "listening").unwrap();
        let enroll = |origin: &str| {
            let member = Arc::new(smallclaims::fleet::MemberKey::generate().unwrap().0);
            let invite = store.create_fleet_invite(Some(origin), std::time::Duration::from_secs(600),
                &["fabric".into()], "person/example", false).unwrap();
            let code = smallclaims::fleet::code::JoinCode {
                fleet_id: uuid::Uuid::parse_str(fleet).unwrap(),
                invite: hex::decode(invite.invite).unwrap().try_into().unwrap(),
                token: invite.token,
                fingerprint: smallclaims::fleet::code::fingerprint(key.public()),
                expires_at: invite.expires_at_unix_ms / 1000,
                name: Some(origin.into()), migrate: false, endpoints: vec![], pairing: None,
            };
            let joiner = smallclaims::fleet::handshake::Joiner::start(
                &code, &member, origin, "dial-out", None, "test").unwrap();
            assert!(matches!(store.redeem_fleet_invite(joiner.request()).unwrap(),
                smallclaims::store::FleetRedemption::Admitted { .. }));
            member
        };
        let equal_key = enroll("minute-fold-equal");
        let conflict_key = enroll("minute-fold-conflict");
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
        let baseline = store.export_replication_exchange(fleet, &Default::default()).unwrap();
        assert!(store.accept_observation_minutes("person/example", std::slice::from_ref(&minute)).unwrap());
        let append_independent = |origin: &str, member: Arc<smallclaims::fleet::MemberKey>, minute: &ObservationMinute| {
            let independent = Store::open_memory(origin).unwrap();
            // Membership alone does not configure the local fleet used to sign exports.
            independent.bind_fleet(fleet).unwrap();
            independent.pin_fleet_anchor(key.public()).unwrap();
            independent.set_member_key(Some(member.clone())).unwrap();
            independent.receive_replication_exchange("minute-fold", fleet, &baseline).unwrap();
            independent.validate_replication_backlog().unwrap();
            independent.project_replication_backlog().unwrap();
            assert!(matches!(independent.fleet_membership().unwrap().window(origin, 1),
                smallclaims::fleet::Window::Keyed(keys) if keys.contains(member.public())));
            assert!(independent.accept_observation_minutes("person/example", std::slice::from_ref(minute)).unwrap());
            assert_eq!(independent.observation_minutes(120_000).unwrap(), vec![minute.clone()]);
            let claim = independent.latest_claim(&minute.subject(), Some(KIND)).unwrap().unwrap();
            let exchange = independent.export_replication_exchange(fleet, &Default::default()).unwrap();
            assert!(exchange.envelopes.iter().any(|envelope| envelope.writer == origin
                && envelope.member_key.as_deref() == Some(member.public()) && envelope.signature.is_some()),
                "{origin}: phone evidence must travel in a member-signed envelope");
            store.receive_replication_exchange(origin, fleet, &exchange).unwrap();
            store.validate_replication_backlog().unwrap();
            store.project_replication_backlog().unwrap();
            assert!(store.claim_by_id(&claim.id).unwrap().is_some(),
                "{origin}: replicated minute must be projected; rejected records: {:?}",
                store.replica_records(true).unwrap());
            assert!(admitted_minute(&store.readers.get(), &claim).unwrap().is_some(),
                "{origin}: replicated admission must retain its signer and phone-source authority");
        };
        // Disconnected receivers admit against the same pre-report graph, then send
        // independently signed batches to the receiver through normal replication.
        append_independent("minute-fold-equal", equal_key, &minute);
        assert_eq!(store.observation_minutes(120_000).unwrap(), vec![minute.clone()]);
        let mut conflict = minute;
        conflict.report_digest = "b".repeat(64);
        conflict.count = 2;
        conflict.buckets = vec![(103, 2)];
        append_independent("minute-fold-conflict", conflict_key, &conflict);
        assert_eq!(store.observation_minutes(120_000).unwrap(), Vec::<ObservationMinute>::new());
    }

    #[test]
    fn unadmitted_replicated_minutes_cannot_occupy_phone_replay_keys() {
        for spoof in ["equal", "conflicting", "wrong-person"] {
            let issuer = Store::open_memory("issuer").unwrap();
            let fleet = "observation-adversarial-fleet";
            issuer.bind_fleet(fleet).unwrap();
            let key = Arc::new(smallclaims::fleet::MemberKey::generate().unwrap().0);
            issuer.pin_fleet_anchor(key.public()).unwrap();
            issuer.set_member_key(Some(key.clone())).unwrap();
            issuer.admit_fleet_anchor(fleet, key.public(), "listening").unwrap();
            let grant = "custom/client/adversarial-phone";
            issuer.append_client_operation_claim(&ClaimInput {
                subject: grant.into(), kind: "custom.client.pairing-completed".into(),
                actor: Some("person/example".into()),
                fields: serde_json::from_value(json!({"device_id":"example-device-key",
                    "person_id":"person/example","session_actor":"client/example-phone",
                    "credential_hash":"example-digest","scopes":[],"expires_at_unix_ms":u64::MAX})).unwrap(),
                evidence: vec![], expected_subject: None, idempotency_key: None,
            }).unwrap();
            let authentic = ObservationMinute {
                source: issuer.observation_source(grant, "person/example").unwrap(), pairing_grant: grant.into(),
                report_id: "11111111-1111-4111-8111-111111111111".into(), report_digest: "a".repeat(64),
                target: "ios-connect".into(), carrier: "fabric".into(), path: None,
                start_ms: 60_000, count: 1, over: 0, max_ms: 100, buckets: vec![(103, 1)],
            };
            let mut forged = authentic.clone();
            if spoof == "conflicting" {
                forged.report_digest = "b".repeat(64);
                forged.count = 2;
                forged.buckets = vec![(103, 2)];
            }
            // A real signed replication envelope is not a phone-admission receipt.
            // Also exercise a native receipt whose actor is not the paired person.
            issuer.with_all_runs_fenced(|tx| -> Result<(), FractalError> {
                let actor = if spoof == "wrong-person" { "person/other" } else { "person/example" };
                let claim = append_claim_tx(tx, issuer.origin(), &forged.subject(), KIND, Some(actor),
                    &json!({"fields":forged,"evidence":[]}), &[], None).map_err(internal)?;
                if spoof == "wrong-person" {
                    crate::pairing::append_operation_receipt_tx(tx, &claim).map_err(internal)?;
                }
                Ok(())
            }).unwrap();
            let exchange = issuer.export_replication_exchange(fleet, &Default::default()).unwrap();
            let receiver = Store::open_memory("receiver").unwrap();
            receiver.pin_fleet_anchor(key.public()).unwrap();
            receiver.receive_replication_exchange("issuer", fleet, &exchange).unwrap();
            receiver.validate_replication_backlog().unwrap();
            receiver.project_replication_backlog().unwrap();
            assert!(receiver.observation_minutes(120_000).unwrap().is_empty(), "{spoof}");
            let before = receiver.index().unwrap();
            assert!(receiver.accept_observation_minutes("person/example", std::slice::from_ref(&authentic)).unwrap(), "{spoof}");
            let accepted = receiver.index().unwrap();
            assert!(accepted > before, "{spoof}: authentic admission must append rather than acknowledge a spoof");
            assert_eq!(receiver.observation_minutes(120_000).unwrap(), vec![authentic.clone()], "{spoof}");
            assert!(!receiver.accept_observation_minutes("person/example", std::slice::from_ref(&authentic)).unwrap(), "{spoof}");
            assert_eq!(receiver.index().unwrap(), accepted, "{spoof}: authentic retry must not append");
            receiver.with_all_runs_fenced(|tx| -> Result<(), FractalError> {
                let error = super::super::validate_claim_cardinality(
                    tx, &authentic.subject(), KIND, Some("person/example"), &BTreeMap::new(),
                    &fractal_schema::Cardinality::Once,
                ).unwrap_err();
                assert_eq!(error.code, "claim-cardinality", "{spoof}: admitted evidence still occupies the once-only key");
                Ok(())
            }).unwrap();
            let mut conflict = authentic;
            conflict.report_digest = "c".repeat(64);
            assert_eq!(receiver.accept_observation_minutes("person/example", &[conflict]).unwrap_err().code,
                "idempotency-conflict", "{spoof}: admitted content still fences conflicting retries");
        }
    }
}
