#![allow(unused_imports)]
use crate::store::attention_ivm::*;
use crate::store::custom::RegistrationRequest;
use smallclaims::ivm::{Readiness, SourceCut, events};

pub fn install(store: &Store, views: &Views) {
    store.project_replication_backlog().unwrap();
    let mut writer = store.connection.write();
    let tx = writer.transaction().unwrap();
    let index = smallclaims::store::current_index_tx(&tx).unwrap();
    views
        .initialize_empty(
            &tx,
            SourceCut {
                epoch: 1,
                admitted: index,
                projected: index,
                local_generation: 0,
            },
        )
        .unwrap();
    events::install(&tx, 1024).unwrap();
    for view in [PERSON_VIEW, CUSTOM_VIEW] {
        let mut after = String::new();
        loop {
            let keys = seed_page(&tx, view, &after, 2).unwrap();
            if keys.is_empty() {
                break;
            }
            after = keys.last().unwrap().clone();
        }
    }
    while !backfill_page(&tx, 0, 2).unwrap() {}
    tx.execute(/* sqlite-large-value: allow-for-now lv-h277021197eb9 owner=fractal_store::store::attention_ivm::tests_shared_seats TODO=bound-or-reference-values-and-reclamation */ "UPDATE ivm_views SET ready=1", []).unwrap();
    tx.commit().unwrap();
}

pub fn input(subject: &str, kind: &str, actor: &str, fields: Value) -> ClaimInput {
    ClaimInput {
        subject: subject.into(),
        kind: kind.into(),
        actor: Some(actor.into()),
        fields: serde_json::from_value(fields).unwrap(),
        evidence: vec![],
        expected_subject: None,
        idempotency_key: None,
    }
}

pub fn custom(store: &Store, subject: &str, person: &str) {
    let manifest = serde_json::from_str(/* sqlite-large-value: allow-for-now lv-h5650237ff33b owner=fractal_store::store::attention_ivm::tests_shared_seats TODO=trace-statement-and-value-representation */ include_str!(
        "../../../../../examples/fractal/custom-review.json"
    ))
    .unwrap();
    store
        .register_custom_kind(&RegistrationRequest {
            manifest,
            actor: "agent/garden/seed".into(),
        })
        .unwrap();
    store
        .append_claim(&input(
            subject,
            "custom.garden.review.v1.requested",
            "agent/garden/seed",
            json!({"title":"Choose a seed","detail":"Keep or discard","recipient":person}),
        ))
        .unwrap();
}
