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
