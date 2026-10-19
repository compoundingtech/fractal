#![allow(unused_imports)]
use super::*;
use crate::store::custom::RegistrationRequest;
use smallclaims::ivm::{Readiness, SourceCut, events};
use st3_attention::store::attention_ivm::tests_shared_seats::*;
use st3_attention::store::attention_ivm::tests_shared_seats::{custom, input, install, views};


#[test]
fn unproved_retirement_fences_person_family_and_preserves_custom_availability() {
    let (store, _, request) = person_work::tests::fixture();
    let views = views(&store);
    store.ask_person(&request).unwrap();
    custom(
        &store,
        "custom/garden/review/v1/independent",
        "person/lichen",
    );
    install(&store, &views);
    let intent = crate::graph::parse_internal_intent(
        r#"version 2
stop "agent/alder.asker"
"#,
        "alder",
    )
    .unwrap();
    store.apply_internal(&intent, "stop-requester").unwrap();
    let mut writer = store.connection.write();
    let tx = writer.transaction().unwrap();
    let previous = source_cut(&tx).unwrap().unwrap();
    let index = smallclaims::store::current_index_tx(&tx).unwrap();
    views
        .publish_cut(
            &tx,
            SourceCut {
                admitted: index,
                projected: index,
                ..previous
            },
        )
        .unwrap();
    assert!(!flush(&tx, &views, 0, 128).unwrap());
    assert!(matches!(
        views.readiness(&tx, PERSON_VIEW, 1).unwrap(),
        Readiness::Fenced
    ));
    assert!(matches!(
        views.readiness(&tx, CUSTOM_VIEW, 1).unwrap(),
        Readiness::Ready(_)
    ));
    assert!(window(&tx, &views, PERSON_VIEW, "person/avery", u128::MAX, 501).is_err());
    assert_eq!(
        window(&tx, &views, CUSTOM_VIEW, "person/lichen", u128::MAX, 501)
            .unwrap()
            .len(),
        1
    );
    tx.commit().unwrap();
}
