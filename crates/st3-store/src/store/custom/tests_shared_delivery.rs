#![allow(unused_imports)]
use crate::store::custom::*;

pub fn example() -> Manifest {
    serde_json::from_str(/* sqlite-large-value: allow-for-now lv-h908629c8b2f9 owner=st3_store::store::custom::tests TODO=trace-statement-and-value-representation */ include_str!(
        "../../../../../examples/st3/custom-review.json"
    ))
    .unwrap()
}

pub fn registration(store: &Store) -> Value {
    store
        .register_custom_kind(&RegistrationRequest {
            manifest: example(),
            actor: "agent/garden/seed".into(),
        })
        .unwrap()
}

pub fn request(store: &Store, id: &str) -> ClaimRecord {
    store.append_claim(&ClaimInput{subject:id.into(),kind:example().creation_kind,actor:Some("agent/garden/seed".into()),fields:serde_json::from_value(json!({"title":"Retain the seed history?","detail":"Choose Keep or Discard.","recipient":"person/lichen"})).unwrap(),..empty_input()}).unwrap()
}

pub fn legacy_sync(from: &Store, to: &Store) {
    to.import_replication(&from.origin, &from.export_replication(0).unwrap())
        .unwrap();
}

pub fn empty_input() -> ClaimInput {
    ClaimInput {
        subject: String::new(),
        kind: String::new(),
        actor: None,
        fields: BTreeMap::new(),
        evidence: vec![],
        expected_subject: None,
        idempotency_key: None,
    }
}

pub const FLEET: &str = "7c9e6679-7425-40de-944b-e07fc1f90ae7";

pub fn signed_pair() -> (Store, Store) {
    let keys = (0..2)
        .map(|_| Arc::new(crate::fleet::MemberKey::generate().unwrap().0))
        .collect::<Vec<_>>();
    let stores = ["alder", "birch"]
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let s = Store::open_memory(*name).unwrap();
            s.bind_fleet(FLEET).unwrap();
            s.pin_fleet_anchor(keys[0].public()).unwrap();
            s.set_member_key(Some(keys[i].clone())).unwrap();
            s
        })
        .collect::<Vec<_>>();
    for (i, s) in stores.iter().enumerate() {
        let mut f = json!({"fleet_id":FLEET,"member_key":keys[i].public(),"via":if i==0{"anchor"}else{"invite"},"mode":"listening"});
        if i != 0 {
            f["sponsor"] = json!("host/alder");
        }
        stores[0]
            .append_claim(&ClaimInput {
                subject: format!("host/{}", s.origin),
                kind: "fleet.member-admitted".into(),
                fields: serde_json::from_value(f).unwrap(),
                ..empty_input()
            })
            .unwrap();
    }
    signed_sync(&stores[0], &stores[1]);
    let mut iter = stores.into_iter();
    (iter.next().unwrap(), iter.next().unwrap())
}

pub fn signed_sync(from: &Store, to: &Store) {
    let e = from
        .export_replication_exchange_answering(
            FLEET,
            &to.replication_inventory().unwrap(),
            &to.replication_signature_requests().unwrap(),
        )
        .unwrap();
    to.receive_replication_exchange(&from.origin, FLEET, &e)
        .unwrap();
    let admitted = to.validate_replication_backlog().unwrap();
    assert_eq!(admitted.invalid, 0);
    assert_eq!(admitted.unknown, 0);
    assert!(to.project_replication_backlog().unwrap());
}

pub const TREE: &str = "custom/decision/tree/v1/example/decisions";

pub const SEAT: &str = "agent/example/decisions";

pub const DECIDER: &str = "person/lichen";

pub const RAW_KINDS: [&str; 6] = ["opened", "requested", "answered", "assumed", "promoted", "damaged"];

pub fn decision_tree() -> Manifest {
    serde_json::from_str(/* sqlite-large-value: allow-for-now lv-hf193d5d798aa owner=st3_store::store::custom::tests TODO=trace-statement-and-value-representation */ include_str!(
        "../../../../../examples/st3/decision-tree.json"
    ))
    .unwrap()
}

pub fn tree_claim(
    s: &Store,
    tree: &str,
    kind: &str,
    actor: &str,
    fields: Value,
) -> Result<ClaimRecord, St3Error> {
    s.append_claim(&ClaimInput {
        subject: tree.into(),
        kind: format!("custom.decision.tree.v1.{kind}"),
        actor: Some(actor.into()),
        fields: serde_json::from_value(fields).unwrap(),
        ..empty_input()
    })
}

pub fn tree_document(s: &Store, name: &str, bytes: &[u8]) -> String {
    s.put_document_as(name, bytes, &None, name, Some(SEAT)).unwrap();
    format!("{name}@{}", hex::encode(Sha256::digest(bytes)))
}

/// The external tool's derived status, fenced on every raw kind of the same tree subject.
pub fn tree_status(s: &Store, tree: &str, owner: &str, fields: Value) -> ClaimRecord {
    let kinds = RAW_KINDS.map(|k| format!("custom.decision.tree.v1.{k}")).to_vec();
    let mut fields = fields;
    fields["_basis"] = json!([{"subject":tree,"kinds":kinds,"revision":s.custom_basis_revision(tree,&kinds).unwrap()}]);
    tree_claim(s, tree, "status", owner, fields).unwrap()
}
