use super::*;

fn publish(store: &Store, source: &str) -> Result<ApplyResponse, FractalError> {
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

fn decoy_bodies(payload: &Value) -> [Value; 5] {
    let mut empty_body = payload.clone();
    empty_body["body"] = json!({});
    let mut empty_fields = payload.clone();
    empty_fields["fields"] = json!({});
    [
        empty_body.clone(),
        empty_fields,
        json!({"fields": {"body": payload}, "body": {}}),
        json!({"body": payload, "fields": {}}),
        json!({"fields": empty_body}),
    ]
}

fn assert_direct_claim_decoys_refused(subject: &str, kind: &str, payload: &Value) {
    for body in decoy_bodies(payload) {
        // Keep the public raw append path and exact envelopes, with schema for the real gate.
        let store = Store::open_memory("alder").unwrap();
        let connection = store.readers.get();
        let transaction = connection.unchecked_transaction().unwrap();
        let error = append_claim_tx(
            &transaction, "alder", subject, kind, Some("person/avery"), &body, &[], None,
        ).unwrap_err();
        let refusal = error.downcast_ref::<FractalError>().unwrap_or_else(|| {
            panic!("expected the declared-admission barrier for {kind} {body}, got {error}")
        });
        assert_eq!(refusal.code, "declared-admission-feature-barrier", "{subject} {kind} {body}");
    }
}

fn assert_barrier_decoys_refused(subject: &str, kind: &str, payload: &Value) {
    for body in decoy_bodies(payload) {
        // Exercise envelope detection directly: typed validation can independently reject
        // member/canonical drift, and the evidence gate requires an initialized store.
        let store = Store::open_memory("alder").unwrap();
        let before = store.index().unwrap();
        let refusal = validate_claim(&store.readers.get(), "alder", kind, &body).unwrap_err();
        assert_eq!(
            refusal.code, "declared-admission-feature-barrier",
            "{subject} {kind} {body}"
        );
        assert_eq!(store.index().unwrap(), before, "decoy refusal must not write");
    }
}

#[test]
fn direct_declaration_claim_decoy_envelopes_fail_closed() {
    for child in [
        "declared-priority \"normal\"",
        "declared-lane \"check\"",
        "declared-budgets {}",
    ] {
        let source = format!(
            "version 2\nagent \"new\" {{ command \"true\"; {child}; }}\n\
             mission \"new\" state=\"ready\" {{ goal \"Work.\"; {child}; }}\n"
        );
        let intent = crate::graph::parse_intent(&source, "alder").unwrap();
        let mut desired = serde_json::to_value(&intent.subjects["agent/alder.new"]).unwrap();
        let mut mission = serde_json::to_value(&intent.missions["new"]).unwrap();
        if child == "declared-budgets {}" {
            desired["member"]["declared_budgets"] = json!({});
            mission["declared_budgets"] = json!({});
        }
        assert_direct_claim_decoys_refused("agent/alder.new", "intent.desired", &desired);
        assert_direct_claim_decoys_refused("mission/new", "mission.published", &mission);
    }
}

#[test]
fn desired_claim_decoys_cannot_hide_member_or_desired_children() {
    let plain = crate::graph::parse_intent(
        "version 2\nagent \"new\" { command \"true\"; }\n",
        "alder",
    )
    .unwrap();
    let plain = serde_json::to_value(&plain.subjects["agent/alder.new"]).unwrap();
    for child in [
        "declared-priority \"normal\"",
        "declared-lane \"check\"",
        "declared-budgets {}",
    ] {
        let source = format!("version 2\nagent \"new\" {{ command \"true\"; {child}; }}\n");
        let intent = crate::graph::parse_intent(&source, "alder").unwrap();
        let mut desired = serde_json::to_value(&intent.subjects["agent/alder.new"]).unwrap();
        if child == "declared-budgets {}" {
            desired["member"]["declared_budgets"] = json!({});
        }
        let mut member_only = desired.clone();
        member_only["desired"] = plain["desired"].clone();
        let mut children_only = desired;
        children_only["member"] = plain["member"].clone();
        assert_barrier_decoys_refused("agent/alder.new", "intent.desired", &member_only);
        assert_barrier_decoys_refused("agent/alder.new", "intent.desired", &children_only);
    }
}

#[test]
fn non_agent_desired_children_are_not_declared_admission() {
    let connection = Connection::open_in_memory().unwrap();
    for child in ["declared-priority", "declared-lane", "declared-budgets"] {
        for payload in [
            json!({"kind": "resource", "children": [{"name": child}]}),
            json!({"kind": "resource", "desired": {"children": [{"name": child}]}}),
        ] {
            for body in [
                payload.clone(),
                json!({"fields": payload}),
                json!({"body": payload}),
                json!({"fields": {"body": payload}}),
            ] {
                validate_claim(&connection, "alder", "intent.desired", &body).unwrap();
            }
        }
    }
}

#[test]
fn direct_claim_semantics_precede_the_publication_barrier() {
    for (priority, expected) in [
        ("urgent", "invalid-declared-admission"),
        ("normal", "declared-admission-feature-barrier"),
    ] {
        for (subject, kind, fields) in [
            ("agent/alder.new", "intent.desired", json!({
                "kind": "agent",
                "desired": {"children": [{"name": "declared-priority", "arguments": [priority]}]}
            })),
            ("mission/new", "mission.published", json!({
                "body": {"subject": "mission/new", "declared_priority": priority}
            })),
        ] {
            let store = Store::open_memory("alder").unwrap();
            let before = store.index().unwrap();
            let error = store.append_claim(&ClaimInput {
                subject: subject.into(),
                kind: kind.into(),
                actor: Some("person/avery".into()),
                fields: serde_json::from_value(fields).unwrap(),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: None,
            }).unwrap_err();
            assert_eq!(error.code, expected, "{kind} {priority}");
            assert_eq!(store.index().unwrap(), before);
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
