use super::*;
use crate::fleet::MemberKey;
use crate::store::directive_notes::tests_shared_delivery::{FLEET, claim, enable, sync};

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
fn runtime_workspace_accepts_legacy_string_and_null_locally_and_replicated() {
    // Reader acceptance is phase one, independent of the phase-two publication barrier.
    let (store, anchor) = enabled();
    let (peer, _) = peer(&store, &anchor);
    assert!(!store.runtime_workspace_publication_ready().unwrap());
    for workspace in [None, Some(json!("/workspace/launched")), Some(Value::Null)] {
        let mut fields = json!({"status":"running", "incarnation_id":"launch-one"});
        if let Some(workspace) = workspace {
            fields["workspace"] = workspace;
        }
        let local = claim(&store, "agent/alder.worker", "runtime.observed",
            Some("agent/alder.worker"), fields.clone());
        assert_eq!(local.body["fields"], fields);
        sync(&store, &peer);
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
fn runtime_workspace_requires_every_active_signed_reader_and_latest_advertisement() {
    let (store, anchor) = enabled();
    assert!(store.runtime_workspace_publication_ready().unwrap());
    let (peer, _) = peer(&store, &anchor);
    assert!(!store.runtime_workspace_publication_ready().unwrap());
    for features in [json!({}), json!({"runtime_workspace":0}),
        json!({"runtime_workspace":"1"}), json!({"runtime_workspace":1})] {
        started(&peer, features.clone());
        sync(&peer, &store);
        assert_eq!(store.runtime_workspace_publication_ready().unwrap(),
            features["runtime_workspace"] == json!(1), "{features}");
    }
    started(&peer, json!({"runtime_workspace":0}));
    sync(&peer, &store);
    assert!(!store.runtime_workspace_publication_ready().unwrap(),
        "an older positive advertisement cannot override the latest refusal");
}

#[test]
fn runtime_workspace_unknown_pending_legacy_and_conflicted_membership_fail_closed() {
    let unknown = Store::open_memory("alder").unwrap();
    started(&unknown, json!({"runtime_workspace":1}));
    assert!(!unknown.runtime_workspace_publication_ready().unwrap());

    let pending = Store::open_memory("alder").unwrap();
    enable(&pending);
    started(&pending, json!({"runtime_workspace":1}));
    assert!(!pending.runtime_workspace_publication_ready().unwrap());
    pending.replication_snapshot().unwrap();
    assert!(pending.runtime_workspace_publication_ready().unwrap());

    let (store, _) = enabled();
    let legacy = Store::open_memory("legacy").unwrap();
    legacy.bind_fleet(FLEET).unwrap();
    started(&legacy, json!({"runtime_workspace":1}));
    sync(&legacy, &store);
    assert!(!store.runtime_workspace_publication_ready().unwrap());

    let (store, anchor) = enabled();
    let (peer, _) = peer(&store, &anchor);
    started(&peer, json!({"runtime_workspace":1}));
    sync(&peer, &store);
    assert!(store.runtime_workspace_publication_ready().unwrap());
    let conflicting = MemberKey::generate().unwrap().0;
    claim(&store, "host/birch", "fleet.member-admitted", None,
        json!({"fleet_id":FLEET,"member_key":conflicting.public(),"via":"invite","sponsor":"host/alder","mode":"listening"}));
    assert!(!store.runtime_workspace_publication_ready().unwrap());
    store.replication_snapshot().unwrap();
    assert!(!store.runtime_workspace_publication_ready().unwrap());
}

#[test]
fn runtime_workspace_unsigned_advertisement_cannot_enable_publication() {
    let (store, anchor) = enabled();
    let (peer, _) = peer(&store, &anchor);
    started(&peer, json!({"runtime_workspace":1}));
    let mut exchange = peer.export_replication_exchange(
        FLEET, &store.replication_inventory().unwrap()).unwrap();
    exchange.signatures.clear();
    for envelope in &mut exchange.envelopes {
        envelope.signature = None;
    }
    store.receive_replication_exchange("birch", FLEET, &exchange).unwrap();
    store.validate_replication_backlog().unwrap();
    store.apply_replication_repairs().unwrap();
    assert!(store.project_replication_backlog().unwrap());
    assert!(!store.runtime_workspace_publication_ready().unwrap());
    sync(&peer, &store);
    assert!(store.runtime_workspace_publication_ready().unwrap());
}

#[test]
fn runtime_workspace_previous_incarnation_does_not_enable_replacement() {
    let (store, anchor) = enabled();
    let (peer, key) = peer(&store, &anchor);
    started(&peer, json!({"runtime_workspace":1}));
    sync(&peer, &store);
    assert!(store.runtime_workspace_publication_ready().unwrap());
    let high_water = writer_high_water(&peer.readers.get(), "birch").unwrap().unwrap();
    claim(&store, "host/birch", "fleet.member-removed", None,
        json!({"member_key":key.public(),"high_water":high_water,"reason":"Replace incarnation"}));
    store.replication_snapshot().unwrap();
    let replacement = MemberKey::generate().unwrap().0;
    claim(&store, "host/birch", "fleet.member-admitted", None,
        json!({"fleet_id":FLEET,"member_key":replacement.public(),"via":"invite","sponsor":"host/alder","mode":"listening"}));
    store.replication_snapshot().unwrap();
    assert!(!store.runtime_workspace_publication_ready().unwrap());
}
