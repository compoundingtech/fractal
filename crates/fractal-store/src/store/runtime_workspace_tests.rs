use super::*;
use crate::fleet::MemberKey;
use crate::store::directive_notes::tests_shared_delivery::{FLEET, claim, enable, sync};
use crate::store::fleet_admission_tests_shared_delivery::{fleet, key};

fn started(store: &Store, features: Value) {
    claim(store, &format!("daemon/{}", store.origin), "daemon.started", None,
        json!({"status":"running", "features":features}));
}

fn enabled() -> (Store, Arc<MemberKey>) {
    let store = Store::open_memory("alder").unwrap();
    let key = enable(&store);
    started(&store, json!({"runtime_workspace":1}));
    store.replication_snapshot().unwrap();
    (store, key)
}

fn peer(store: &Store, anchor: &MemberKey) -> (Store, Arc<MemberKey>) {
    let key = Arc::new(MemberKey::generate().unwrap().0);
    claim(store, "host/birch", "fleet.member-admitted", None,
        json!({"fleet_id":FLEET,"member_key":key.public(),"via":"invite","sponsor":"host/alder","mode":"listening"}));
    store.replication_snapshot().unwrap();
    let peer = Store::open_memory("birch").unwrap();
    peer.bind_fleet(FLEET).unwrap();
    peer.pin_fleet_anchor(anchor.public()).unwrap();
    peer.set_member_key(Some(key.clone())).unwrap();
    sync(store, &peer);
    (peer, key)
}

#[test]
fn runtime_workspace_accepts_absent_string_and_null_locally_and_replicated() {
    // Reader acceptance is phase one, independent of the phase-two publication barrier.
    let (store, anchor) = enabled();
    let (peer, _) = peer(&store, &anchor);
    for workspace in [None, Some(json!("/workspace/launched")), Some(Value::Null)] {
        let mut fields = json!({"status":"running", "incarnation_id":"launch-one"});
        if let Some(workspace) = workspace {
            fields["workspace"] = workspace;
        }
        let local = claim(&store, "agent/alder.worker", "runtime.observed",
            Some("agent/alder.worker"), fields.clone());
        assert_eq!(local.body["fields"], fields);
        let admission = sync(&store, &peer);
        assert_eq!((admission.unknown, admission.invalid, admission.held), (0, 0, 0));
        let replicated = claim_by_id_tx(&peer.readers.get(), &local.id).unwrap().unwrap();
        assert_eq!(replicated.body, local.body);
    }
    for workspace in [json!(42), json!({"cwd":"/workspace/launched"}), json!([])] {
        let fields = serde_json::from_value(json!({"status":"running", "workspace":workspace})).unwrap();
        assert!(fractal_schema::registry().validate_claim(
            "agent/alder.worker", "runtime.observed", &fields).is_err());
    }
}

#[test]
fn previous_schema_accepts_legacy_runtime_but_refuses_workspace_and_unknown_kinds() {
    // Reconstruct the real pre-phase-1 schema from the same registry source:
    // workspace is this phase's only claim-registry addition. Keep every other
    // kind and field, rather than building a registry that refuses all writes.
    let mut previous = fractal_schema::registry().clone();
    previous.claims.get_mut("runtime.observed").unwrap().fields.remove("workspace").unwrap();
    let subject = "agent/a.worker";
    let legacy: BTreeMap<String, Value> = serde_json::from_value(
        json!({"status":"running", "incarnation_id":"launch-one"}),
    ).unwrap();
    assert!(previous.validate_claim(subject, "runtime.observed", &legacy).is_ok());
    for workspace in [json!("/workspace/launched"), Value::Null] {
        let mut fields = legacy.clone();
        fields.insert("workspace".into(), workspace);
        assert_eq!(previous.validate_claim(subject, "runtime.observed", &fields)
            .unwrap_err().code, "unknown-claim-field");
        assert!(fractal_schema::registry().validate_claim(
            subject, "runtime.observed", &fields).is_ok());
    }
    assert_eq!(previous.validate_claim(subject, "runtime.future", &legacy)
        .unwrap_err().code, "unknown-claim-kind");
}

#[test]
fn unsupported_runtime_workspace_readers_refuse_signed_but_retain_envelopes() {
    for unknown_kind in [false, true] {
        for workspace in [None, Some(json!("/workspace/launched")), Some(Value::Null)] {
            // A reader lacking only the optional field still accepts its absence.
            if !unknown_kind && workspace.is_none() {
                continue;
            }
            let older_key = key();
            let signed_key = key();
            let (_, source, readers) = fleet(&[("older", &older_key), ("signed", &signed_key)]);
            let older = &readers[0];
            let signed_reader = &readers[1];
            // The first append by an agent also publishes its key.granted delegation
            // in a separate envelope. Establish that supported evidence first, so the
            // exchange below contains only the deliberately unsupported observation.
            source.ensure_principal_key("agent/a.worker").unwrap();
            for reader in [older, signed_reader] {
                sync(&source, reader);
                let baseline = source.export_replication_exchange(
                    FLEET, &reader.replication_inventory().unwrap(),
                ).unwrap();
                assert!(baseline.envelopes.is_empty());
                assert!(baseline.signed_envelopes.is_empty());
            }
            let mut registry = fractal_schema::registry().clone();
            let code = if unknown_kind {
                registry.claims.remove("runtime.observed").unwrap();
                "unknown-claim-kind"
            } else {
                registry.claims.get_mut("runtime.observed").unwrap().fields.remove("workspace").unwrap();
                "unknown-claim-field"
            };
            for reader in [older, signed_reader] {
                reader.set_claim_registry(registry.clone());
            }
            let mut fields = json!({"status":"running", "incarnation_id":"launch-one"});
            if let Some(workspace) = workspace {
                fields["workspace"] = workspace;
            }
            let input = ClaimInput {
                subject: "agent/a.worker".into(),
                kind: "runtime.observed".into(),
                actor: Some("agent/a.worker".into()),
                fields: serde_json::from_value(fields).unwrap(),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: None,
            };
            // set_claim_registry overrides replication, not this build's local
            // validator. Previous-schema local refusal is tested separately above.
            let claim = source.append_claim(&input).unwrap();
            let exchange = source.export_replication_exchange(
                FLEET, &older.replication_inventory().unwrap(),
            ).unwrap();
            assert_eq!(exchange.envelopes.len(), 1);
            let identity = ReplicaEnvelopeId {
                writer: exchange.envelopes[0].writer.clone(),
                sequence: exchange.envelopes[0].sequence,
                hash: exchange.envelopes[0].hash.clone(),
            };

            // Use real released, individually signed claims, not a legacy admission shim.
            while !source.sign_own_claims().unwrap().done {}
            while !source.release_envelopes(0).unwrap().done {}
            let signed = source.signed_envelopes(&[identity]).unwrap();
            assert_eq!(signed.len(), 1);
            assert_eq!(signed[0].claims.len(), 1);
            assert_eq!(signed[0].claims[0].claim.id, claim.id);
            assert_eq!(signed_reader.receive_signed_envelopes(FLEET, None, &signed).unwrap(), (0, 1));
            assert!(signed_reader.claim_by_id(&claim.id).unwrap().is_none());
            assert!(signed_reader.replica_records(true).unwrap().is_empty());

            let before = older.replication_snapshot().unwrap();
            let summary = older.export_replication_summary(FLEET).unwrap();
            assert!(!summary.graph_digest.is_empty());
            assert!(!summary.projection_digests.is_empty());
            older.receive_replication_exchange("a", FLEET, &exchange).unwrap();
            let admission = older.validate_replication_backlog().unwrap();
            assert_eq!((admission.valid, admission.unknown, admission.invalid, admission.held), (0, 1, 0, 0));
            older.project_replication_backlog().unwrap();
            let retained = older.replica_records(true).unwrap();
            assert_eq!(retained.len(), 1);
            assert_eq!(retained[0].claim_id.as_deref(), Some(claim.id.as_str()));
            assert_eq!(retained[0].state, "unknown");
            assert_eq!(retained[0].error_code.as_deref(), Some(code));
            let raw: Vec<u8> = older.readers.get().query_row(
                "SELECT raw FROM replica_records WHERE claim_id=?1",
                [&claim.id], |row| row.get(0),
            ).unwrap();
            assert!(!raw.is_empty());
            assert!(older.claim_by_id(&claim.id).unwrap().is_none());
            assert!(older.claims_for(&input.subject, Some(&input.kind)).unwrap().is_empty());
            let after = older.replication_snapshot().unwrap();
            assert_eq!(after.graph_digest, before.graph_digest);
            assert_eq!(after.projection_digests, before.projection_digests);
            let summary = older.export_replication_summary(FLEET).unwrap();
            assert!(!summary.authority_digest.is_empty());
            assert!(summary.graph_digest.is_empty());
            assert!(summary.projection_digests.is_empty());
        }
    }
}
