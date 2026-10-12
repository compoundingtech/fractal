use super::*;

const ROOT: &str = "agent/example/root";
const PERSON: &str = "person/avery";
const SOURCE: &str =
    "version 2\nagent \"example/root\" { accountable \"person/avery\"; harness \"codex\" {}; }";

fn intent() -> NormalizedIntent {
    crate::graph::parse_internal_intent(SOURCE, "alder").unwrap()
}

#[test]
fn accountable_root_publication_requires_same_person_and_round_trips() {
    let store = Store::open_memory("alder").unwrap();
    let intent = intent();
    let expected = BTreeMap::from([(ROOT.into(), Vec::new())]);
    for actor in [None, Some("agent/example/parent"), Some("person/robin")] {
        let error = store
            .apply_as(&intent, &expected, "refused", actor)
            .unwrap_err();
        assert_eq!(error.code, "accountable-root-forbidden");
        assert!(store.desired_subject_with_writer(ROOT).unwrap().is_none());
    }
    store
        .apply_as(&intent, &expected, "root", Some(PERSON))
        .unwrap();
    let (desired, author) = store.desired_subject_with_writer(ROOT).unwrap().unwrap();
    assert_eq!(desired, intent.subjects[ROOT]);
    assert_eq!(author.as_deref(), Some(PERSON));
    let claim = store
        .latest_claim(ROOT, Some("intent.desired"))
        .unwrap()
        .unwrap();
    assert_eq!(claim.body["accountable"], PERSON);
    let (_, tree) = store.agent_declaration(ROOT, None).unwrap().unwrap();
    let rendered = crate::graph::render_agent_desired_kdl(&tree).unwrap();
    assert_eq!(
        crate::graph::parse_internal_intent(&rendered, "alder")
            .unwrap()
            .subjects[ROOT]
            .accountable
            .as_deref(),
        Some(PERSON)
    );
    store.rebuild_claim_projections().unwrap();
    assert_eq!(
        store.desired_subject_with_writer(ROOT).unwrap().unwrap().0,
        desired
    );
}

#[test]
fn accountable_root_raw_and_replica_paths_reject_forgery_and_unknown_stays_unknown() {
    let store = Store::open_memory("alder").unwrap();
    let intent = intent();
    let body = serde_json::to_value(&intent.subjects[ROOT]).unwrap();
    for actor in [None, Some("agent/example/child"), Some("person/robin")] {
        let error = store
            .connection
            .batched(|tx| {
                append_claim_tx(tx, "alder", ROOT, "intent.desired", actor, &body, &[], None)
            })
            .unwrap()
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("only the named accountable person")
        );
    }
    let generic =
        json!({"fields": {"kind": "agent", "desired": body["desired"], "accountable": PERSON}});
    assert!(
        store
            .connection
            .batched(|tx| append_claim_tx(
                tx,
                "alder",
                ROOT,
                "intent.desired",
                Some(PERSON),
                &generic,
                &[],
                None
            ))
            .unwrap()
            .is_err()
    );
    let record = store
        .connection
        .batched(|tx| {
            append_claim_tx(
                tx,
                "alder",
                ROOT,
                "intent.desired",
                Some(PERSON),
                &body,
                &[],
                None,
            )
        })
        .unwrap()
        .unwrap();
    assert!(matches!(
        classify_replicated_claim_with_registry(&record, st3_schema::registry()).unwrap(),
        ReplicatedClaimAdmission::Valid
    ));
    for actor in [None, Some("agent/example/child"), Some("person/robin")] {
        let mut forged = record.clone();
        forged.actor = actor.map(str::to_owned);
        assert_eq!(
            classify_replicated_claim_with_registry(&forged, st3_schema::registry())
                .err()
                .unwrap()
                .code,
            "accountable-root-forbidden"
        );
    }
    for mutate in [
        "missing-field",
        "different-field",
        "missing-child",
        "mission-root",
    ] {
        let mut forged = record.clone();
        match mutate {
            "missing-field" => {
                forged.body.as_object_mut().unwrap().remove("accountable");
            }
            "different-field" => forged.body["accountable"] = json!("person/robin"),
            "missing-child" => forged.body["desired"]["children"]
                .as_array_mut()
                .unwrap()
                .retain(|v| v["name"] != "accountable"),
            _ => forged.body["owner_run"] = json!("mission-run/example/run"),
        }
        assert_eq!(
            classify_replicated_claim_with_registry(&forged, st3_schema::registry())
                .err()
                .unwrap()
                .code,
            "invalid-accountable-root",
            "{mutate}"
        );
    }
    let mut old = st3_schema::registry().clone();
    old.claims
        .get_mut("intent.desired")
        .unwrap()
        .fields
        .remove("accountable");
    assert!(matches!(
        classify_replicated_claim_with_registry(&record, &old).unwrap(),
        ReplicatedClaimAdmission::UnknownField
    ));
}

#[test]
fn accountable_root_grammar_rejects_nonpersons_duplicates_and_nested_values() {
    for child in [
        "accountable \"avery\"",
        "accountable \"agent/example\"",
        "accountable \"person/a/b\"",
        "accountable \"person/\"",
        "accountable \"person/avery\" extra=\"x\"",
        "accountable \"person/avery\" { name \"x\"; }",
        "accountable \"person/avery\" \"person/robin\"",
        "accountable \"person/avery\"; accountable \"person/avery\"",
    ] {
        let source =
            format!("version 2\nagent \"example/root\" {{ {child}; harness \"codex\" {{}}; }}");
        assert!(
            crate::graph::parse_internal_intent(&source, "alder").is_err(),
            "{child}"
        );
    }
    let mission = "version 2\nmission \"example/work\" state=\"ready\" { goal \"Work.\"; agent \"worker\" { accountable \"person/avery\"; command \"true\"; }; step \"work\" { assigned-to \"agent/worker\"; goal \"Work.\"; }; }";
    assert_eq!(
        crate::graph::parse_internal_intent(mission, "alder")
            .unwrap_err()
            .code,
        "invalid-accountable-root"
    );
    let mut desired = intent().subjects[ROOT].clone();
    desired.owner_run = Some("mission-run/example/run".into());
    assert!(
        st3_schema::accountable::validate_claim(
            ROOT,
            "intent.desired",
            &serde_json::to_value(desired).unwrap(),
            Some(PERSON)
        )
        .is_err()
    );
}
