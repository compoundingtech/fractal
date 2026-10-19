#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::custom::tests_shared_attention_tests::{DECIDER, FLEET, RAW_KINDS, SEAT, TREE, a_creation, decision_tree, empty_input, example, legacy_sync};
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::custom::tests_shared_attention_tests::{registration, registration_hash, reply, request, signed_pair, signed_sync, tree_claim, tree_document, tree_status};
use super::*;

#[test]
fn custom_registration_immutable_and_cannot_capture_legacy_prefix() {
    let s = Store::open_memory("alder").unwrap();
    let first = registration(&s);
    assert_eq!(registration(&s), first);
    let mut m = example();
    m.fields.insert(
        "extra".into(),
        Expr::Constant {
            value: json!("new"),
        },
    );
    assert!(
        s.register_custom_kind(&RegistrationRequest {
            manifest: m,
            actor: "agent/garden/seed".into()
        })
        .is_err()
    );
    let s = Store::open_memory("birch").unwrap();
    s.append_claim(&ClaimInput {
        subject: "custom/garden/review/v1/legacy".into(),
        kind: "custom.garden.legacy".into(),
        ..empty_input()
    })
    .unwrap();
    assert!(
        s.register_custom_kind(&RegistrationRequest {
            manifest: example(),
            actor: "agent/garden/seed".into()
        })
        .is_err()
    );
}
