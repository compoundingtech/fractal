use super::*;
use crate::model::DeclaredBudgets;

const OLD_DECLARATIONS: &str = r#"version 2
agent "example/old" {
  workspace "/work/old"
  harness "omp" {}
}
mission "old" state="ready" {
  goal "Preserve old declarations."
  step "work" {
    assigned-to "agent/example/old"
    goal "Keep working."
  }
}
"#;

fn declaration_claims(store: &Store) -> Vec<(String, String, String)> {
    let connection = store.connection.lock().unwrap();
    let mut statement = connection.prepare("SELECT id,kind,body FROM claims WHERE kind IN ('intent.desired','mission.published') ORDER BY id").unwrap();
    statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
}

#[test]
fn declared_admission_unchanged_old_declarations_keep_claim_ids_and_body_bytes() {
    let store = Store::open_memory("node").unwrap();
    let intent = crate::graph::parse_intent(OLD_DECLARATIONS, "node").unwrap();
    let input = || IntentInput {
        kdl: OLD_DECLARATIONS.into(),
        source_name: None,
    };
    let first = store.mission(&intent, input()).unwrap();
    store
        .apply(&intent, &first.subject_tokens, "old-first")
        .unwrap();
    let claims = declaration_claims(&store);
    assert_eq!(claims.len(), 2);
    assert!(
        claims
            .iter()
            .all(|(_, _, body)| !body.contains("declared_priority")
                && !body.contains("declared_lane")
                && !body.contains("declared_budgets"))
    );
    let canonical = serde_json::to_vec(&intent.subjects).unwrap();
    let desired_hash = canonical_hash(&intent.subjects).unwrap();
    let mission = serde_json::to_vec(&intent.missions["old"]).unwrap();
    let revision = intent.missions["old"].revision.clone();
    let parsed_again = crate::graph::parse_intent(OLD_DECLARATIONS, "node").unwrap();
    assert_eq!(
        serde_json::to_vec(&parsed_again.subjects).unwrap(),
        canonical
    );
    assert_eq!(
        canonical_hash(&parsed_again.subjects).unwrap(),
        desired_hash
    );
    assert_eq!(
        serde_json::to_vec(&parsed_again.missions["old"]).unwrap(),
        mission
    );
    assert_eq!(parsed_again.missions["old"].revision, revision);
    let second = store.mission(&parsed_again, input()).unwrap();
    store
        .apply(&parsed_again, &second.subject_tokens, "old-second")
        .unwrap();
    assert_eq!(
        declaration_claims(&store),
        claims,
        "republish must neither replace claims nor change stored bodies"
    );
}

#[test]
fn declared_admission_invalid_typed_publication_writes_nothing() {
    let store = Store::open_memory("node").unwrap();
    let mut intent = crate::graph::parse_intent(OLD_DECLARATIONS, "node").unwrap();
    intent.missions.get_mut("old").unwrap().declared_budgets = Some(DeclaredBudgets {
        cpu_seconds: Some(0),
        ..Default::default()
    });
    let error = store
        .apply(&intent, &BTreeMap::new(), "invalid-typed")
        .unwrap_err();
    assert_eq!(error.code, "invalid-declared-admission");
    assert_eq!(error.details["field"], "declared_budgets.cpu_seconds");
    assert!(declaration_claims(&store).is_empty());
}

#[test]
fn declared_admission_direct_claim_invalid_values_write_nothing() {
    let store = Store::open_memory("node").unwrap();
    let intent = crate::graph::parse_intent(OLD_DECLARATIONS, "node").unwrap();
    for patch in [
        json!({"declared_priority":"urgent"}),
        json!({"declared_budgets":{"cpu_seconds":0}}),
        json!({"declared_budgets":{"memory_max":"18446744073709551615T"}}),
        json!({"declared_lane":null}),
    ] {
        let mut body = serde_json::to_value(&intent.missions["old"]).unwrap();
        body.as_object_mut()
            .unwrap()
            .extend(patch.as_object().unwrap().clone());
        let input = ClaimInput {
            subject: "mission/old".into(),
            kind: "mission.published".into(),
            actor: Some("person/avery".into()),
            fields: BTreeMap::from([("body".into(), body)]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        };
        assert!(
            store
                .append_claim(&input)
                .unwrap_err()
                .to_string()
                .contains("invalid-declared-admission")
        );
        assert!(declaration_claims(&store).is_empty());
    }
}
