use super::*;

const PERSON: &str = "person/avery";
const AGENT: &str = "agent/commit-context.reviewer";
const THREAD: &str = "thread/commit-context";
const EXISTING_MESSAGE: &str = "message/commit-context-existing";

fn message(subject: &str) -> ClaimInput {
    let mut input = ClaimInput {
        subject: subject.into(),
        kind: "message.sent".into(),
        actor: Some(PERSON.into()),
        fields: BTreeMap::from([
            ("thread_id".into(), json!(THREAD)),
            ("from".into(), json!(PERSON)),
            ("to".into(), json!(AGENT)),
            ("content".into(), json!("A concrete thread delivery test.")),
            ("status".into(), json!("sent")),
        ]),
        evidence: Vec::new(),
        expected_subject: None,
        idempotency_key: Some(subject.into()),
    };
    thread_delivery::add_reader_marker(&mut input);
    input
}

fn append(store: &Store, input: &ClaimInput, context: ClaimCommitContext<'_>) -> Result<(ClaimRecord, bool), FractalError> {
    append_claim_with_commit_context(&store.graph, input, None, None, None, None, context)
}

fn typed_context() -> ClaimCommitContext<'static> {
    ClaimCommitContext { thread_delivery: true, ..Default::default() }
}

fn store() -> Store {
    let store = Store::open_memory("commit-context").unwrap();
    store.configure_thread_write_rollout([]);
    thread_metadata::apply(
        &store, PERSON, THREAD, "create-commit-context",
        thread_metadata::Edit::Create { agent: AGENT, title: "Commit context" },
    ).unwrap();
    append(&store, &message(EXISTING_MESSAGE), typed_context()).unwrap();
    store
}

// Use a real declaration and a schema-valid terminal claim, rather than replacing the
// lifecycle reader or its admission callback. The seats crate owns the public retire API.
fn retire(store: &Store) -> ClaimRecord {
    let intent = crate::graph::parse_test_intent(
        "version 2\nagent \"reviewer\" { command \"true\"; }\n", &store.origin,
    ).unwrap();
    let declaration = store.connection.batched(|tx| {
        append_claim_tx(
            tx, &store.origin, AGENT, "intent.desired", None,
            &serde_json::to_value(&intent.subjects[AGENT]).unwrap(), &[], None,
        ).map_err(claim_append_error)
    }).unwrap().unwrap();
    let input = ClaimInput {
        subject: AGENT.into(),
        kind: retirements::KIND.into(),
        actor: Some(PERSON.into()),
        fields: BTreeMap::from([
            ("reason".into(), json!("Finished the review.")),
            ("retired_by".into(), json!(PERSON)),
            ("owner_host".into(), json!(store.origin)),
            ("declaration".into(), json!(declaration.id)),
        ]),
        evidence: vec![declaration.id],
        expected_subject: None,
        idempotency_key: Some("retire-commit-context".into()),
    };
    validate_claim_input(&input).unwrap();
    append(store, &input, ClaimCommitContext { retirement_writer: true, ..Default::default() }).unwrap().0
}

fn write_counts(connection: &Connection) -> [u64; 5] {
    ["claims", "batches", "operations", "expected_claim_signatures", "local_native_delivery_receipts_v2"]
        .map(|table| connection.query_row(/* sqlite-large-value: allow-for-now lv-hb8649a6d1afe owner=fractal_store::store::claim_commit_context_tests TODO=trace-statement-and-value-representation */ &format!("SELECT COUNT(*) FROM {table}"), [], |row| row.get::<_, u64>(0)).unwrap())
}

fn create_thread_tx(store: &Store, tx: &Transaction<'_>, thread: &str, first: &str) -> ClaimRecord {
    append_claim_tx(
        tx, &store.origin, thread, "thread.created", Some(PERSON),
        &json!({"fields": {"person": PERSON, "agent": AGENT, "title": "Atomic first delivery", "first_message": first}, "evidence": []}),
        &[], None,
    ).map_err(claim_append_error).unwrap()
}

#[test]
fn existing_transaction_typed_delivery_uses_thread_batch_and_returns_actual_receipt() {
    let store = Store::open_memory("commit-context").unwrap();
    store.graph.ensure_principal_key(PERSON).unwrap();
    let input = message("message/commit-context-first");
    let (created, sent) = store.connection.batched(|tx| -> Result<_, FractalError> {
        let created = create_thread_tx(&store, tx, THREAD, &input.subject);
        let (sent, appended) = append_claim_with_commit_context_tx(
            &store.graph, tx, &input, Some(&created.batch_id), typed_context(),
        )?;
        assert!(appended);
        assert_eq!(sent.batch_id, created.batch_id);
        assert_eq!(sent.subject, input.subject);
        assert_eq!(sent.body["fields"]["thread_id"], THREAD);
        assert_eq!(claim_by_id_tx(tx, &sent.id).map_err(internal)?.unwrap().id, sent.id);
        let (replayed, appended) = append_claim_with_commit_context_tx(
            &store.graph, tx, &input, Some(&created.batch_id), typed_context(),
        )?;
        assert!(!appended);
        assert_eq!(replayed.id, sent.id);
        assert_eq!(replayed.batch_id, created.batch_id);
        Ok((created, sent))
    }).unwrap().unwrap();
    assert_eq!(store.latest_claim_id(THREAD).unwrap().as_deref(), Some(created.id.as_str()));
    assert_eq!(store.latest_claim_id(&input.subject).unwrap().as_deref(), Some(sent.id.as_str()));
    assert_eq!(store.operation_claim(input.idempotency_key.as_deref().unwrap()).unwrap().unwrap().id, sent.id);
}

#[test]
fn caller_rollback_removes_thread_message_batch_and_operation() {
    let store = Store::open_memory("commit-context").unwrap();
    store.graph.ensure_principal_key(PERSON).unwrap();
    let before = write_counts(&store.readers.get());
    let input = message("message/commit-context-rollback");
    let error = store.connection.batched(|tx| -> Result<(), FractalError> {
        let created = create_thread_tx(&store, tx, THREAD, &input.subject);
        let (sent, appended) = append_claim_with_commit_context_tx(
            &store.graph, tx, &input, Some(&created.batch_id), typed_context(),
        )?;
        assert!(appended);
        assert_eq!(sent.batch_id, created.batch_id);
        assert!(latest_claim_id_tx(tx, THREAD).map_err(internal)?.is_some());
        assert!(latest_claim_id_tx(tx, &input.subject).map_err(internal)?.is_some());
        Err(FractalError::new("caller-rollback", "the caller refuses its enclosing operation"))
    }).unwrap().unwrap_err();
    assert_eq!(error.code, "caller-rollback");
    assert_eq!(write_counts(&store.readers.get()), before);
    assert!(store.latest_claim_id(THREAD).unwrap().is_none());
    assert!(store.latest_claim_id(&input.subject).unwrap().is_none());
    assert!(store.operation_claim(input.idempotency_key.as_deref().unwrap()).unwrap().is_none());
}

#[derive(Clone, Copy, Debug)]
enum Gate {
    None,
    Authority,
    Admission,
}

struct Rejection {
    name: &'static str,
    mutate: fn(&mut ClaimInput),
    retired: bool,
    gate: Gate,
    code: &'static str,
}

fn rejection_cases() -> Vec<Rejection> {
    let mut cases = vec![
        Rejection { name: "unknown kind", mutate: |i| i.kind = "message.not-registered".into(), retired: false, gate: Gate::None, code: "unknown-claim-kind" },
        Rejection { name: "malformed subject", mutate: |i| i.subject = "message/has whitespace".into(), retired: false, gate: Gate::None, code: "invalid-claim-subject" },
        Rejection { name: "wrong subject family", mutate: |i| i.subject = "agent/wrong-family".into(), retired: false, gate: Gate::None, code: "invalid-claim-subject" },
        Rejection { name: "malformed actor", mutate: |i| i.actor = Some("not-a-subject".into()), retired: false, gate: Gate::None, code: "invalid-claim-actor" },
        Rejection { name: "missing status", mutate: |i| { i.fields.remove("status"); }, retired: false, gate: Gate::None, code: "missing-claim-field" },
        Rejection { name: "wrong status enum", mutate: |i| { i.fields.insert("status".into(), json!("read")); }, retired: false, gate: Gate::None, code: "invalid-claim-field" },
        Rejection { name: "wrong content type", mutate: |i| { i.fields.insert("content".into(), json!(17)); }, retired: false, gate: Gate::None, code: "invalid-claim-field" },
        Rejection { name: "wrong thread type", mutate: |i| { i.fields.insert("thread_id".into(), json!(17)); }, retired: false, gate: Gate::None, code: "invalid-claim-field" },
        Rejection { name: "wrong thread reference family", mutate: |i| { i.fields.insert("thread_id".into(), json!("message/not-a-thread")); }, retired: false, gate: Gate::None, code: "invalid-subject-reference" },
        Rejection { name: "wrong quote reference family", mutate: |i| { i.fields.insert("thread_quote_id".into(), json!(THREAD)); }, retired: false, gate: Gate::None, code: "invalid-subject-reference" },
        Rejection { name: "wrong ask reference family", mutate: |i| { i.fields.insert("thread_origin_ask".into(), json!(THREAD)); }, retired: false, gate: Gate::None, code: "invalid-subject-reference" },
        Rejection { name: "malformed sender reference", mutate: |i| { i.fields.insert("from".into(), json!("not-a-subject")); }, retired: false, gate: Gate::None, code: "invalid-subject-reference" },
        Rejection { name: "malformed recipient reference", mutate: |i| { i.fields.insert("to".into(), json!("not-a-subject")); }, retired: false, gate: Gate::None, code: "invalid-subject-reference" },
        Rejection { name: "assistant actor", mutate: |i| i.actor = Some("assistant/chatgpt".into()), retired: false, gate: Gate::None, code: "assistant-send-refused" },
        Rejection { name: "assistant sender", mutate: |i| { i.fields.insert("from".into(), json!("assistant/chatgpt")); }, retired: false, gate: Gate::None, code: "assistant-send-refused" },
        // The real typed credential checker is installed at each hook, not a test stub.
        Rejection { name: "expired authority", mutate: |_| {}, retired: false, gate: Gate::Authority, code: "thread-authority-expired" },
        Rejection { name: "expired admission", mutate: |_| {}, retired: false, gate: Gate::Admission, code: "thread-authority-expired" },
        Rejection { name: "retired recipient", mutate: |_| {}, retired: true, gate: Gate::None, code: "recipient-retired" },
        Rejection { name: "missing evidence", mutate: |i| i.evidence.push("claim/missing-commit-context".into()), retired: false, gate: Gate::None, code: "missing-evidence" },
        Rejection { name: "stale subject", mutate: |i| i.expected_subject = Some(Some("claim/not-the-head".into())), retired: false, gate: Gate::None, code: "stale-subject" },
        Rejection { name: "occupied empty subject CAS", mutate: |i| { i.subject = EXISTING_MESSAGE.into(); i.expected_subject = Some(None); }, retired: false, gate: Gate::None, code: "stale-subject" },
        Rejection { name: "duplicate send with new operation", mutate: |i| i.subject = EXISTING_MESSAGE.into(), retired: false, gate: Gate::None, code: "invalid-message-transition" },
        Rejection { name: "changed idempotency body", mutate: |i| i.idempotency_key = Some(EXISTING_MESSAGE.into()), retired: false, gate: Gate::None, code: "idempotency-mismatch" },
        Rejection { name: "retirement cannot mask key mismatch", mutate: |i| i.idempotency_key = Some(EXISTING_MESSAGE.into()), retired: true, gate: Gate::None, code: "idempotency-mismatch" },
        Rejection { name: "authority precedes key mismatch", mutate: |i| i.idempotency_key = Some(EXISTING_MESSAGE.into()), retired: false, gate: Gate::Authority, code: "thread-authority-expired" },
        Rejection { name: "retirement precedes missing evidence", mutate: |i| i.evidence.push("claim/missing-commit-context".into()), retired: true, gate: Gate::None, code: "recipient-retired" },
        Rejection { name: "evidence precedes CAS", mutate: |i| { i.evidence.push("claim/missing-commit-context".into()); i.expected_subject = Some(Some("claim/not-the-head".into())); }, retired: false, gate: Gate::None, code: "missing-evidence" },
    ];
    for (name, mutate) in [
        ("forged sender class", (|i: &mut ClaimInput| { i.fields.insert("sender_class".into(), json!("external-assistant")); }) as fn(&mut ClaimInput)),
        ("forged grant", |i: &mut ClaimInput| { i.fields.insert("assistant_grant_id".into(), json!("assistant-grant/forged")); }),
        ("forged owner", |i: &mut ClaimInput| { i.fields.insert("assistant_owner".into(), json!(PERSON)); }),
        ("forged session", |i: &mut ClaimInput| { i.fields.insert("assistant_session_actor".into(), json!("forged")); }),
        ("provenance precedes schema", |i: &mut ClaimInput| { i.fields.insert("assistant_owner".into(), json!(17)); i.fields.remove("status"); }),
    ] {
        cases.push(Rejection { name, mutate, retired: false, gate: Gate::None, code: "assistant-send-refused" });
    }
    cases
}

fn receipt(input: &mut ClaimInput, kind: &str, status: &str, actor: Option<&str>) {
    input.subject = EXISTING_MESSAGE.into();
    input.kind = kind.into();
    input.actor = actor.map(str::to_owned);
    input.fields = BTreeMap::from([("status".into(), json!(status))]);
}

#[test]
fn transaction_local_and_normal_typed_context_reject_identical_requests_without_writes() {
    let mut cases = rejection_cases();
    cases.extend([
        Rejection { name: "orphan mail custody", mutate: |i| { receipt(i, "message.read", "read", Some(AGENT)); i.fields.insert("acting_for".into(), json!(AGENT)); }, retired: false, gate: Gate::None, code: "wrong-message-recipient" },
        Rejection { name: "retired staging", mutate: |i| receipt(i, "message.staged", "staged", Some(AGENT)), retired: true, gate: Gate::None, code: "wrong-message-recipient" },
        Rejection { name: "retired delivery", mutate: |i| receipt(i, "message.delivered", "delivered", Some(AGENT)), retired: true, gate: Gate::None, code: "wrong-message-recipient" },
        Rejection { name: "retired read without custody", mutate: |i| receipt(i, "message.read", "read", Some(AGENT)), retired: true, gate: Gate::None, code: "wrong-message-recipient" },
        Rejection { name: "retired close without custody", mutate: |i| receipt(i, "message.closed", "closed", Some(AGENT)), retired: true, gate: Gate::None, code: "wrong-message-recipient" },
        Rejection { name: "missing read actor", mutate: |i| receipt(i, "message.read", "read", None), retired: false, gate: Gate::None, code: "missing-message-actor" },
        Rejection { name: "premature agent read", mutate: |i| receipt(i, "message.read", "read", Some(AGENT)), retired: false, gate: Gate::None, code: "invalid-message-transition" },
        Rejection { name: "expiry actor restriction", mutate: |i| { receipt(i, "message.expired", "expired", Some(PERSON)); i.fields.insert("reason".into(), json!("no-live-recipient")); }, retired: false, gate: Gate::None, code: "invalid-message-expiry" },
        Rejection { name: "expiry requires runtime writer", mutate: |i| { receipt(i, "message.expired", "expired", Some("daemon/runtime")); i.fields.insert("reason".into(), json!("no-live-recipient")); }, retired: false, gate: Gate::None, code: "invalid-message-expiry" },
    ]);
    for case in cases {
        for in_transaction in [false, true] {
            let store = store();
            if case.retired { retire(&store); }
            let mut input = message("message/commit-context-rejected");
            (case.mutate)(&mut input);
            let before = write_counts(&store.readers.get());
            let expired_credential = |connection: &Connection| {
                thread_metadata::PersonAuthority::LocalWeb { expires_at: 0 }.check(&store, connection, PERSON)
            };
            let context = ClaimCommitContext {
                thread_delivery: true,
                authority: matches!(case.gate, Gate::Authority).then_some(&expired_credential as &ClaimAdmission<'_>),
                admission: matches!(case.gate, Gate::Admission).then_some(&expired_credential as &ClaimAdmission<'_>),
                ..Default::default()
            };
            let error = if in_transaction {
                store.connection.batched(|tx| {
                    let error = append_claim_with_commit_context_tx(&store.graph, tx, &input, None, context).unwrap_err();
                    assert_eq!(write_counts(tx), before, "{} changed caller transaction", case.name);
                    // Commit the caller transaction after catching the refusal: absence of
                    // writes must not depend on propagating the error to force rollback.
                    Ok::<_, FractalError>(error)
                }).unwrap().unwrap()
            } else {
                append(&store, &input, context).unwrap_err()
            };
            assert_eq!(error.code, case.code, "{}; transaction-local={in_transaction}", case.name);
            assert_eq!(write_counts(&store.readers.get()), before, "{}; transaction-local={in_transaction}", case.name);
            assert!(store.latest_claim_id("message/commit-context-rejected").unwrap().is_none());
        }
    }
}

#[test]
fn untyped_context_refuses_every_reserved_thread_field_in_both_paths() {
    for field in fractal_schema::threads::MESSAGE_RELATION_FIELDS {
        for in_transaction in [false, true] {
            let store = Store::open_memory("commit-context").unwrap();
            let mut input = message("message/commit-context-untyped");
            for reserved in fractal_schema::threads::MESSAGE_RELATION_FIELDS {
                input.fields.remove(*reserved);
            }
            // Even null is reserved. Refusal must precede schema and provenance checks.
            input.fields.insert((*field).into(), Value::Null);
            input.fields.insert("assistant_owner".into(), json!(17));
            let before = write_counts(&store.readers.get());
            let error = if in_transaction {
                store.connection.batched(|tx| {
                    let error = append_claim_with_commit_context_tx(&store.graph, tx, &input, None, Default::default()).unwrap_err();
                    assert_eq!(write_counts(tx), before);
                    Ok::<_, FractalError>(error)
                }).unwrap().unwrap()
            } else {
                append(&store, &input, Default::default()).unwrap_err()
            };
            assert_eq!(error.code, "thread-delivery-required", "{field}; transaction-local={in_transaction}");
            assert_eq!(write_counts(&store.readers.get()), before);
        }
    }
}

#[test]
fn transaction_local_effect_operation_preserves_first_send_envelope_and_replay() {
    let store = Store::open_memory("commit-context").unwrap();
    store.graph.ensure_principal_key(PERSON).unwrap();
    let mut input = message("message/commit-context-effect");
    input.idempotency_key = None;
    let key = "thread-start-message:commit-context-effect";
    let mut expected_body = json!({"fields": input.fields, "evidence": input.evidence});
    smallclaims::store::idempotency::attach(&mut expected_body, key).unwrap();
    let effect_operation = smallclaims::store::operation_parts(&expected_body).unwrap();
    let sent = store.connection.batched(|tx| -> Result<_, FractalError> {
        let created = create_thread_tx(&store, tx, THREAD, &input.subject);
        let (sent, appended) = append_claim_with_commit_context_tx(
            &store.graph, tx, &input, Some(&created.batch_id),
            ClaimCommitContext { thread_delivery: true, effect_operation: Some(effect_operation), ..Default::default() },
        )?;
        assert!(appended);
        assert_eq!(sent.body, expected_body);
        assert_eq!(sent.batch_id, created.batch_id);
        let before = write_counts(tx);
        let (replayed, appended) = append_claim_with_commit_context_tx(
            &store.graph, tx, &input, Some(&created.batch_id),
            ClaimCommitContext { thread_delivery: true, effect_operation: Some(effect_operation), ..Default::default() },
        )?;
        assert!(!appended);
        assert_eq!(replayed.id, sent.id);
        assert_eq!(replayed.body, expected_body);
        assert_eq!(write_counts(tx), before);
        Ok(sent)
    }).unwrap().unwrap();
    // A secondary effect uses the local-request cache identity, not a claim-operation key.
    let connection = store.readers.get();
    let operation = smallclaims::store::operation_tx(&connection, effect_operation.0)
        .unwrap().expect("the effect operation is durably registered");
    assert_eq!(operation, (effect_operation.1.to_owned(), sent.id.clone(), "active".into()));
    assert_eq!(smallclaims::store::idempotency::original_claim(&connection, key)
        .unwrap().as_deref(), Some(sent.id.as_str()));
}
