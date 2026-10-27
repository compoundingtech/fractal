#![allow(unused_imports)]
use crate::store::owned_sets::{Options, Source};
use crate::store::*;
use crate::parse_intent;
use crate::store::shared::*;

pub fn bundle(command: &str, meadow: bool) -> NormalizedIntent {
    parse_intent(
        &format!(
            "version 2\nagent \"garden/orchard\" {{ command {command:?} }}\n{}",
            if meadow {
                "agent \"garden/meadow\" { command \"true\" }"
            } else {
                ""
            }
        ),
        "amber",
    )
    .unwrap()
}

pub fn direct(store: &Store, input: &NormalizedIntent, key: &str) -> Result<ApplyResponse, St3Error> {
    let preview = store
        .mission(
            input,
            IntentInput {
                kdl: "".into(),
                source_name: None,
            },
        )
        .unwrap();
    store.apply_as(input, &preview.subject_tokens, key, Some("person/operator"))
}

pub fn share(from: &Store, to: &Store) {
    to.import_replication(&from.origin, &from.export_replication(0).unwrap())
        .unwrap();
}

pub fn signed_fleet() -> Vec<Store> {
    let fleet = "7c9e6679-7425-40de-944b-e07fc1f90ae7";
    let keys = (0..3)
        .map(|_| Arc::new(crate::fleet::MemberKey::generate().unwrap().0))
        .collect::<Vec<_>>();
    let stores = ["amber", "cobalt", "ivory"]
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let store = Store::open_memory(*name).unwrap();
            store.bind_fleet(fleet).unwrap();
            store.pin_fleet_anchor(keys[0].public()).unwrap();
            store.set_member_key(Some(keys[i].clone())).unwrap();
            store
        })
        .collect::<Vec<_>>();
    for (i, store) in stores.iter().enumerate() {
        let mut fields = json!({"fleet_id":fleet,"member_key":keys[i].public(),"via":if i==0{"anchor"}else{"invite"},"mode":"listening"});
        if i != 0 {
            fields["sponsor"] = json!("host/amber");
        }
        stores[0]
            .append_claim(&ClaimInput {
                subject: format!("host/{}", store.origin),
                kind: "fleet.member-admitted".into(),
                actor: None,
                fields: serde_json::from_value(fields).unwrap(),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
        store
            .append_claim(&ClaimInput {
                subject: format!("daemon/{}", store.origin),
                kind: "daemon.started".into(),
                actor: None,
                fields: serde_json::from_value(
                    json!({"status":"running","features":{"owned_sets":1}}),
                )
                .unwrap(),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
    }
    for from in &stores {
        for to in &stores {
            if from.origin != to.origin {
                signed_share(from, to);
            }
        }
    }
    stores
}

pub fn signed_share(from: &Store, to: &Store) {
    let fleet = "7c9e6679-7425-40de-944b-e07fc1f90ae7";
    let exchange = from
        .export_replication_exchange_answering(
            fleet,
            &to.replication_inventory().unwrap(),
            &to.replication_signature_requests().unwrap(),
        )
        .unwrap();
    to.receive_replication_exchange(&from.origin, fleet, &exchange)
        .unwrap();
    let admission = to.validate_replication_backlog().unwrap();
    assert_eq!(admission.invalid, 0);
    assert_eq!(admission.unknown, 0);
    to.project_replication_backlog().unwrap();
}
