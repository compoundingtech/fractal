use super::*;
use fractal_store::store::agent_review_transaction::append_agent_review_answer_tx;

const REJECTED: &str = "agent-review-answer-rejected";
const MALFORMED: &str = "invalid-agent-review-claim";

fn fixture_answer(overrides: &[(&str, Value)]) -> (Store, ClaimInput, String) {
    let (store, _, _) = fixture();
    let run = store.create_mission_run(&request("source")).unwrap();
    let owner = &run.steps[0];
    let digest = "0123456789abcdef01234567";
    let operation = format!(
        "gate-operation/{}/{digest}",
        owner.subject.replace('/', ".")
    );
    let mut fields = json!({"owner":owner.subject,"reviewer":FREE,"question":"Check output",
        "review_targets":[],"decisions":["approved","rejected"],"operation":operation,
        "mission_revision":run.revision,"step_definition":owner.definition_hash,"attempt":owner.attempt,
        "gate_kind":"agent-review","gate":"review","evidence_contract":{"proof":{"type":"text"}}});
    for (key, value) in overrides {
        fields[key] = value.clone();
    }
    let key = format!("agent-review-request:{}:{digest}", owner.subject);
    let mut connection = store.connection.write();
    let tx = connection.transaction().unwrap();
    let record = smallclaims::store::append_claim_record_tx(
        &tx,
        store.origin(),
        &operation,
        "gate.requested",
        Some(PERSON),
        &json!({"fields":fields,"_operation":{
            "id":smallclaims::store::operation_id_for_key(&key),"request_digest":"fixture"}}),
        &[],
        None,
    )
    .unwrap();
    tx.commit().unwrap();
    drop(connection);
    let mut answer: ClaimInput =
        serde_json::from_value(json!({"subject":operation,"kind":"gate.result",
        "actor":FREE,"fields":{"request":record.id,"reviewer":FREE,"decision":"approved",
            "verdict":"pass","evidence":{"proof":"Checked output"}},"evidence":[record.id]}))
        .unwrap();
    for key in [
        "operation",
        "mission_revision",
        "step_definition",
        "attempt",
    ] {
        answer.fields.insert(key.into(), fields[key].clone());
    }
    (store, answer, owner.subject.clone())
}

fn append(store: &Store, input: &ClaimInput, commit: bool) -> Result<ClaimRecord, FractalError> {
    let mut connection = store.connection.write();
    let tx = connection.transaction().unwrap();
    let before = smallclaims::store::current_index_tx(&tx).unwrap();
    let result =
        append_agent_review_answer_tx(&tx, store.origin(), input, |tx, owner, reviewer| {
            review_owner_snapshot(
                &Scope {
                    run: BTreeSet::new(),
                    intent: None,
                },
                tx,
                owner,
                reviewer,
            )
        });
    if result.is_err() {
        assert_eq!(smallclaims::store::current_index_tx(&tx).unwrap(), before);
    }
    if commit {
        tx.commit().unwrap();
    }
    result
}
#[test]
fn agent_review_answer_transaction_commit_and_rollback() {
    let (store, mut input, _) = fixture_answer(&[]);
    let rolled_back = append(&store, &input, false).unwrap();
    assert!(store.claim_by_id(&rolled_back.id).unwrap().is_none());
    input.fields.insert("decision".into(), json!("rejected"));
    input.fields.insert("verdict".into(), json!("fail"));
    input
        .fields
        .insert("reason".into(), json!("Needs revision"));
    let saved = append(&store, &input, true).unwrap();
    let record = store.claim_by_id(&saved.id).unwrap().unwrap();
    assert_eq!(record.actor.as_deref(), Some(FREE));
    assert_eq!(record.body["fields"]["evidence"]["proof"], "Checked output");
    assert_eq!(record.body["evidence"][0], input.fields["request"]);
    assert_eq!(record.body["fields"]["reason"], "Needs revision");
}
#[test]
fn agent_review_answer_transaction_invalid_fields_do_not_append() {
    let (store, input, _) = fixture_answer(&[]);
    for (key, value, code, variant) in [
        ("attempt", json!(2), REJECTED, "Binding"),
        ("decision", json!("rejected"), REJECTED, "Decision"),
        ("evidence", json!({}), REJECTED, "Evidence"),
        ("reviewer", json!(CHILD), REJECTED, "Assignment"),
        ("reason", json!(42), MALFORMED, "malformed"),
    ] {
        let mut bad = input.clone();
        bad.fields.insert(key.into(), value);
        let error = append(&store, &bad, true).unwrap_err();
        assert_eq!(error.code, code);
        assert!(error.message.contains(variant));
    }
    for actor in [None, Some(CHILD.into())] {
        let mut bad = input.clone();
        bad.actor = actor;
        let error = append(&store, &bad, true).unwrap_err();
        assert_eq!(
            error.code,
            if bad.actor.is_none() {
                MALFORMED
            } else {
                REJECTED
            }
        );
        if bad.actor.is_some() {
            assert!(error.message.contains("Actor"));
        }
    }
    let mut bad = input.clone();
    bad.fields.insert("decision".into(), json!("rejected"));
    bad.fields.insert("verdict".into(), json!("fail"));
    bad.fields.insert("reason".into(), json!(" "));
    let error = append(&store, &bad, true).unwrap_err();
    assert_eq!(error.code, REJECTED);
    assert!(error.message.contains("Reason"));
    let mut bad = input;
    bad.evidence.clear();
    assert_eq!(
        append(&store, &bad, true).unwrap_err().code,
        "invalid-agent-review-claim"
    );
}

#[test]
fn agent_review_answer_transaction_reloads_stale_binding_and_worker() {
    for binding in [
        vec![("attempt", json!(2))],
        vec![("mission_revision", json!("stale"))],
    ] {
        let (store, input, _) = fixture_answer(&binding);
        assert_eq!(
            append(&store, &input, true).unwrap_err().code,
            "agent-review-stale"
        );
    }
    let (store, input, owner) = fixture_answer(&[]);
    progress(&store, &owner, FREE);
    assert_eq!(append(&store, &input, true).unwrap_err().code, CONFLICT);
}

#[test]
fn agent_review_answer_transaction_retirement_is_blocked() {
    let (store, input, _) = fixture_answer(&[]);
    let retirement: ClaimInput = serde_json::from_value(json!({"subject":FREE,"kind":"agent.retired",
        "actor":PERSON,"fields":{"reason":"Review responsibility ended","retired_by":PERSON,
            "declaration":store.selected_desired_token(FREE).unwrap().unwrap(),"owner_host":store.origin()},
        "idempotency_key":"retire-reviewer"})).unwrap();
    fractal_store::store::append_claim_with_commit_context(
        &store.graph,
        &retirement,
        None,
        None,
        None,
        None,
        fractal_store::store::ClaimCommitContext {
            retirement_writer: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        append(&store, &input, true).unwrap_err().code,
        "reviewer-retired"
    );
}

#[test]
fn agent_review_reassignment_admission_and_imported_fold_are_fenced() {
    let (store, input, owner) = fixture_answer(&[]);
    let repair: ClaimInput = serde_json::from_value(json!({
        "subject":input.subject,"kind":"gate.reviewer-reassigned","actor":PERSON,
        "fields":{"request":input.fields["request"],"owner":owner,
            "previous_reviewer":FREE,"reviewer":CHILD}
    }))
    .unwrap();
    let error = store.append_claim(&repair).unwrap_err();
    assert_eq!(error.code, "agent-review-reassignment-forbidden");
    let mut connection = store.connection.write();
    let tx = connection.transaction().unwrap();
    // Imported history bypasses local admission; the assignment fold is still fenced.
    smallclaims::store::append_claim_record_tx(
        &tx,
        store.origin(),
        &repair.subject,
        &repair.kind,
        repair.actor.as_deref(),
        &json!({"fields":repair.fields}),
        &[],
        None,
    )
    .unwrap();
    tx.commit().unwrap();
    drop(connection);
    let mut substitute = input.clone();
    substitute.actor = Some(CHILD.into());
    substitute.fields.insert("reviewer".into(), json!(CHILD));
    let error = append(&store, &substitute, true).unwrap_err();
    assert_eq!(error.code, REJECTED);
    assert!(error.message.contains("Assignment"));
    let accepted = append(&store, &input, true).unwrap();
    assert_eq!(accepted.actor.as_deref(), Some(FREE));
}

fn database_state(connection: &Connection) -> BTreeMap<String, Vec<Vec<String>>> {
    const TABLES_SQL: &str = "SELECT name FROM sqlite_schema WHERE type='table'";
    let tables = connection
        .prepare(/* sqlite-large-value: allow-for-now lv-hc04950ca3e3d owner=fractal_missions::store::missions::run_review::answer_tests TODO=trace-statement-and-value-representation */ TABLES_SQL)
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    tables
        .into_iter()
        .map(|table| {
            let mut statement = connection
                .prepare(/* sqlite-large-value: allow-for-now lv-h21a64752ecb4 owner=fractal_missions::store::missions::run_review::answer_tests TODO=trace-statement-and-value-representation */ &format!("SELECT * FROM \"{}\"", table.replace('"', "\"\"")))
                .unwrap();
            let columns = statement.column_count();
            let mut rows = statement
                .query_map([], |row| {
                    (0..columns)
                        .map(|index| row.get_ref(index).map(|value| format!("{value:?}")))
                        .collect::<rusqlite::Result<Vec<_>>>()
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            rows.sort();
            (table, rows)
        })
        .collect()
}

#[test]
fn agent_review_answer_transaction_settles_retries_and_changed_requests() {
    let (store, mut input, _) = fixture_answer(&[]);
    input.idempotency_key = Some("answer-once".into());
    let original = append(&store, &input, true).unwrap();
    let before = database_state(&store.connection.write());
    assert_eq!(append(&store, &input, true).unwrap().id, original.id);
    assert_eq!(database_state(&store.connection.write()), before);
    for rejected in [false, true] {
        let mut changed = input.clone();
        if rejected {
            changed.fields.insert("decision".into(), json!("rejected"));
            changed.fields.insert("verdict".into(), json!("fail"));
            changed
                .fields
                .insert("reason".into(), json!("Needs revision"));
        } else {
            changed
                .fields
                .insert("evidence".into(), json!({"proof":"Different evidence"}));
        }
        assert_eq!(
            append(&store, &changed, true).unwrap_err().code,
            "idempotency-mismatch"
        );
        assert_eq!(database_state(&store.connection.write()), before);
    }
}

#[test]
fn agent_review_answer_transaction_post_insert_error_rolls_back_before_outer_commit() {
    let (store, mut input, _) = fixture_answer(&[]);
    input.idempotency_key = Some("post-insert-error".into());
    let request = store
        .claim_by_id(input.fields["request"].as_str().unwrap())
        .unwrap()
        .unwrap();
    let (key, _) = smallclaims::fleet::MemberKey::generate().unwrap();
    let signature = smallclaims::principal::ClaimSignature::sign(
        &key,
        &smallclaims::principal::content_digest(
            &request.subject,
            &request.kind,
            request.actor.as_deref(),
            &request.body,
        ),
        PERSON,
        None,
        vec![],
        0,
    );
    {
        let mut connection = store.connection.write();
        let tx = connection.transaction().unwrap();
        // A genuine signature on an existing claim consumes this nonce.
        smallclaims::store::principals::store_claim_signature_tx(&tx, &request.id, &signature)
            .unwrap();
        tx.execute(/* sqlite-large-value: allow-for-now lv-hde1a4495a8b4 owner=fractal_missions::store::missions::run_review::answer_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO expected_claim_signatures(subject,kind,actor,signature) VALUES(?1,?2,?3,?4)",
            params![input.subject, input.kind, input.actor, serde_json::to_string(&signature).unwrap()]).unwrap();
        tx.commit().unwrap();
    }
    let before = database_state(&store.connection.write());
    // append() commits the outer transaction even when the writer returns an error.
    assert_eq!(
        append(&store, &input, true).unwrap_err().code,
        "device-signature-replayed"
    );
    assert_eq!(database_state(&store.connection.write()), before);
    let operation = smallclaims::store::claim_operation(&input)
        .unwrap()
        .unwrap()
        .0;
    assert!(
        smallclaims::store::operation_tx(&store.connection.write(), &operation)
            .unwrap()
            .is_none()
    );
}
