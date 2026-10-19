#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::custom::tests_shared_delivery::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::custom::tests_shared_delivery::{DECIDER, FLEET, RAW_KINDS, SEAT, TREE, decision_tree, empty_input, example, legacy_sync, registration};
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::custom::tests_shared_delivery::{request, signed_pair, signed_sync, tree_claim, tree_document, tree_status};
use super::*;

#[test]
fn custom_bad_projection_isolated_from_other_sources_and_core_reads() {
    let s = Store::open_memory("alder").unwrap();
    registration(&s);
    request(&s, "custom/garden/review/v1/good");
    let mut m = example();
    m.version = 2;
    m.subject_prefix = "custom/garden/review/v2/".into();
    m.attention.as_mut().unwrap().title = Expr::Constant { value: json!(17) };
    s.register_custom_kind(&RegistrationRequest {
        manifest: m.clone(),
        actor: "agent/garden/seed".into(),
    })
    .unwrap();
    let id = "custom/garden/review/v2/bad";
    request(&s, id);
    assert_eq!(s.custom_subject(id).unwrap().unwrap()["state"], "invalid");
    assert_eq!(s.attention_items(Some("person/lichen")).unwrap().len(), 1);
    s.status(None).unwrap();
    s.append_claim(&ClaimInput {
        subject: "custom/legacy/fact".into(),
        kind: "custom.legacy.recorded".into(),
        fields: BTreeMap::from([("any".into(), json!({"open":true}))]),
        ..empty_input()
    })
    .unwrap();
    assert!(s.custom_subject("custom/legacy/fact").unwrap().is_none());
    m.slots.insert(
        "wrong".into(),
        schema::Slot {
            kind: "unknown".into(),
            select: Select::First,
        },
    );
    assert!(
        s.register_custom_kind(&RegistrationRequest {
            manifest: m,
            actor: "agent/garden/seed".into()
        })
        .is_err()
    );
}
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
#[test]
fn custom_offline_registration_conflicts_and_indexed_reads() {
    let (a, b) = signed_pair();
    registration(&a);
    let mut m = example();
    m.fields.insert(
        "variant".into(),
        Expr::Constant {
            value: json!("second"),
        },
    );
    b.register_custom_kind(&RegistrationRequest {
        manifest: m,
        actor: "agent/garden/seed".into(),
    })
    .unwrap();
    let id = "custom/garden/review/v1/conflicted";
    request(&a, id);
    signed_sync(&a, &b);
    signed_sync(&b, &a);
    assert_eq!(a.custom_subject(id).unwrap().unwrap()["state"], "conflict");
    assert!(a.attention_items(Some("person/lichen")).unwrap().is_empty());
    assert_eq!(
        a.custom_registrations().unwrap(),
        b.custom_registrations().unwrap()
    );
    for sql in [
        "EXPLAIN QUERY PLAN SELECT body FROM custom_sources WHERE kind='garden.review' AND subject>'a' ORDER BY subject LIMIT 51",
        "EXPLAIN QUERY PLAN SELECT body FROM custom_sources WHERE active=1 AND person='person/lichen' ORDER BY subject",
    ] {
        let plan = a
            .readers
            .get()
            .prepare(/* sqlite-large-value: allow-for-now lv-h249a304d552a owner=st3_store::store::custom::tests TODO=trace-statement-and-value-representation */ sql)
            .unwrap()
            .query_map([], |r| r.get::<_, String>(3))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
            .join(" ");
        assert!(plan.contains("SEARCH custom_sources"), "{plan}");
        assert!(!plan.contains("claims"), "{plan}");
    }
}

#[test]
fn custom_projection_row_bounds_and_invalid_replicated_facts_leave_core_healthy() {
    let a = Store::open_memory("alder").unwrap();
    registration(&a);
    request(&a, "custom/garden/review/v1/healthy");
    let bad = "custom/garden/review/v1/malformed";
    request(&a, bad);
    a.connection.batched(|tx| append_claim_tx(tx,&a.origin,bad,"custom.garden.review.v1.answered",Some("person/lichen"),&json!({"fields":{"_registration":registration_hash(tx,bad),"request":a_creation(tx,bad),"selection":17}}),&[],None)).unwrap().unwrap();
    assert_eq!(a.custom_subject(bad).unwrap().unwrap()["state"], "invalid");
    let mut m = example();
    m.version = 2;
    m.subject_prefix = "custom/garden/review/v2/".into();
    m.fields = (0..64)
        .map(|n| {
            (
                format!("detail_{n}"),
                Expr::Field {
                    slot: "request".into(),
                    field: "detail".into(),
                },
            )
        })
        .collect();
    a.register_custom_kind(&RegistrationRequest {
        manifest: m,
        actor: "agent/garden/seed".into(),
    })
    .unwrap();
    let big = "custom/garden/review/v2/large";
    a.append_claim(&ClaimInput {
        subject: big.into(),
        kind: example().creation_kind,
        actor: Some("agent/garden/seed".into()),
        fields: serde_json::from_value(
            json!({"title":"Large read","detail":"x".repeat(8000),"recipient":"person/lichen"}),
        )
        .unwrap(),
        ..empty_input()
    })
    .unwrap();
    assert_eq!(a.custom_subject(big).unwrap().unwrap()["state"], "invalid");
    assert_eq!(a.attention_items(Some("person/lichen")).unwrap().len(), 1);
    a.status(None).unwrap();
    let b = Store::open_memory("birch").unwrap();
    legacy_sync(&a, &b);
    assert_eq!(
        a.custom_subject(bad).unwrap(),
        b.custom_subject(bad).unwrap()
    );
    assert_eq!(
        a.custom_subject(big).unwrap(),
        b.custom_subject(big).unwrap()
    );
    assert_eq!(
        projection_digest::oracle(&a.readers.get()).unwrap(),
        projection_digest::oracle(&b.readers.get()).unwrap()
    );
}
fn registration_hash(tx: &Transaction<'_>, id: &str) -> String {
    super::registration(tx, id).unwrap().unwrap().0
}
fn a_creation(tx: &Transaction<'_>, id: &str) -> String {
    claims(tx, id).unwrap()[0].id.clone()
}

#[test]
fn custom_late_document_recovers_only_its_source_and_cycles_are_invalid() {
    let s = Store::open_memory("alder").unwrap();
    let mut m = example();
    let context: schema::Field =
        serde_json::from_value(json!({"value_type":"string","document":true})).unwrap();
    m.claims
        .get_mut(&m.creation_kind.clone())
        .unwrap()
        .fields
        .insert("context".into(), context);
    let derived = "custom.garden.review.v1.derived";
    m.claims.insert(
        derived.into(),
        schema::ClaimSchema {
            authority: Authority::Owner,
            fields: BTreeMap::new(),
            additional_fields: false,
        },
    );
    m.slots.insert(
        "derived".into(),
        schema::Slot {
            kind: derived.into(),
            select: Select::Last,
        },
    );
    s.register_custom_kind(&RegistrationRequest {
        manifest: m,
        actor: "agent/garden/seed".into(),
    })
    .unwrap();
    let id = "custom/garden/review/v1/context";
    let content = b"Immutable garden context";
    let hash = hex::encode(Sha256::digest(content));
    let registration = s.custom_registrations().unwrap()[0]["registration"]
        .as_str()
        .unwrap()
        .to_owned();
    s.connection.batched(|tx|append_claim_tx(tx,&s.origin,id,&example().creation_kind,Some("agent/garden/seed"),&json!({"fields":{"_registration":registration,"title":"Review context","detail":"Read the immutable evidence.","recipient":"person/lichen","context":format!("doc/garden/context@{hash}")}}),&[],None)).unwrap().unwrap();
    assert_eq!(
        s.custom_subject(id).unwrap().unwrap()["state"],
        "pending-dependencies"
    );
    assert!(s.attention_items(Some("person/lichen")).unwrap().is_empty());
    s.put_document_as(
        "doc/garden/context",
        content,
        &None,
        "garden-context-001",
        Some("agent/garden/seed"),
    )
    .unwrap();
    assert_eq!(s.custom_subject(id).unwrap().unwrap()["state"], "ready");
    assert_eq!(s.attention_items(Some("person/lichen")).unwrap().len(), 1);
    let a = "custom/garden/review/v1/cycle-a";
    let b = "custom/garden/review/v1/cycle-b";
    request(&s, a);
    request(&s, b);
    let kinds = vec![example().creation_kind];
    for (source, dependency) in [(a, b), (b, a)] {
        let basis = s.custom_basis_revision(dependency, &kinds).unwrap();
        s.append_claim(&ClaimInput {
            subject: source.into(),
            kind: derived.into(),
            actor: Some("agent/garden/seed".into()),
            fields: serde_json::from_value(
                json!({"_basis":[{"subject":dependency,"kinds":kinds,"revision":basis}]}),
            )
            .unwrap(),
            ..empty_input()
        })
        .unwrap();
    }
    assert_eq!(s.custom_subject(a).unwrap().unwrap()["state"], "invalid");
    assert_eq!(s.custom_subject(b).unwrap().unwrap()["state"], "invalid");
    let before = projection_digest::oracle(&s.readers.get()).unwrap();
    s.connection
        .batched(replay_graph_from_nothing_tx)
        .unwrap()
        .unwrap();
    assert_eq!(before, projection_digest::oracle(&s.readers.get()).unwrap());
}

