use super::*;

fn publish(store: &Store, source: &str) -> Result<ApplyResponse, St3Error> {
    let intent = crate::graph::parse_intent(source, &store.origin)?;
    let planned = store.mission(
        &intent,
        IntentInput {
            kdl: source.into(),
            source_name: None,
        },
    )?;
    store.apply_as(
        &intent,
        &planned.subject_tokens,
        source,
        Some("person/avery"),
    )
}

#[test]
fn declared_admission_publication_and_direct_claims_fail_closed_without_evidence() {
    for child in ["declared-priority \"normal\"", "declared-budgets {}"] {
        for declaration in [
            format!("agent \"new\" {{ command \"true\"; {child}; }}"),
            format!("mission \"new\" state=\"ready\" {{ goal \"Work.\"; {child}; }}"),
        ] {
            let store = Store::open_memory("alder").unwrap();
            let source =
                format!("version 2\nagent \"plain\" {{ command \"true\"; }}\n{declaration}\n");
            let before = store.index().unwrap();
            let error = publish(&store, &source).unwrap_err();
            assert_eq!(error.code, "declared-admission-feature-barrier");
            assert_eq!(
                store.index().unwrap(),
                before,
                "refuse the whole publication"
            );
            assert!(
                store
                    .selected_desired_token("agent/alder.plain")
                    .unwrap()
                    .is_none()
            );

            let intent = crate::graph::parse_intent(&source, "alder").unwrap();
            let (subject, kind, mut body) =
                if let Some(desired) = intent.subjects.get("agent/alder.new") {
                    (
                        "agent/alder.new",
                        "intent.desired",
                        serde_json::to_value(desired).unwrap(),
                    )
                } else {
                    (
                        "mission/new",
                        "mission.published",
                        serde_json::to_value(&intent.missions["new"]).unwrap(),
                    )
                };
            // Canonical mission serialization omits empty budgets; direct authors can
            // still submit that field explicitly, and it must count as authored.
            if kind == "mission.published" && child == "declared-budgets {}" {
                body["declared_budgets"] = json!({});
            }
            let fields = if kind == "intent.desired" {
                json!({"kind": "agent", "desired": body["desired"]})
            } else {
                json!({"body": body})
            };
            let error = store
                .append_claim(&ClaimInput {
                    subject: subject.into(),
                    kind: kind.into(),
                    actor: Some("person/avery".into()),
                    fields: serde_json::from_value(fields).unwrap(),
                    evidence: Vec::new(),
                    expected_subject: None,
                    idempotency_key: None,
                })
                .unwrap_err();
            assert_eq!(error.code, "declared-admission-feature-barrier");
            assert_eq!(
                store.index().unwrap(),
                before,
                "direct claim must not write"
            );
        }
    }
}

#[test]
fn publications_without_declared_admission_children_are_unchanged() {
    let store = Store::open_memory("alder").unwrap();
    publish(
        &store,
        "version 2\nagent \"plain\" { command \"true\"; }\nmission \"plain\" state=\"ready\" { goal \"Work.\"; }\n",
    ).unwrap();
    assert!(
        store
            .selected_desired_token("agent/alder.plain")
            .unwrap()
            .is_some()
    );
    assert!(store.mission_spec("plain", None).unwrap().is_some());
}
