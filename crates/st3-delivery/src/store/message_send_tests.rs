#![allow(unused_imports)]
use super::*;
use smallclaims::principal::{ClaimSignature, FIELDS_FORMAT, Judged, fields_signing_bytes};
use st3_store::store::message_send_tests_shared_delivery::*;
use st3_store::store::message_send_tests_shared_delivery::{assert_operation_replay_parity, operation_rows, request, signed};


#[test]
fn signed_send_commits_once_and_retries_keep_the_original_signature() {
    let store = Store::open_memory("node").unwrap();
    let input = request();
    let signature = signed(&input);
    let snapshots = Arc::new(Mutex::new(Vec::new()));
    let observed = snapshots.clone();
    let _observer = store.connection.observe_commits(move |connection| {
        let counts: (u64,u64) = connection.query_row(
            "SELECT (SELECT COUNT(*) FROM claims WHERE subject='message/local-signed-send'),(SELECT COUNT(*) FROM expected_claim_signatures)",
            [], |row| Ok((row.get(0)?,row.get(1)?)),
        ).unwrap();
        observed.lock().unwrap().push(counts);
    });
    let (record, appended) = store.append_signed_message(&input, &signature).unwrap();
    assert!(appended);
    assert_eq!(
        *snapshots.lock().unwrap(),
        [(1, 0)],
        "ACK follows one committed message and consumed signature"
    );
    let held = store.claim_signature(&record.id).unwrap().unwrap();
    assert_eq!(
        serde_json::to_value(&held).unwrap(),
        serde_json::to_value(&signature).unwrap()
    );
    assert!(held.verifies(&Judged {
        id: &record.id,
        subject: &record.subject,
        kind: &record.kind,
        actor: record.actor.as_deref(),
        content: String::new(),
        fields: &record.body["fields"]
    }));
    let repeat_signature = signed(&input);
    let (repeat, appended) = store
        .append_signed_message(&input, &repeat_signature)
        .unwrap();
    assert!(!appended);
    assert_eq!(repeat.id, record.id);
    assert_eq!(
        store.claim_signature(&record.id).unwrap().unwrap().nonce,
        signature.nonce
    );
    assert!(
        !store
            .signature_nonce_used(&repeat_signature.key, &repeat_signature.nonce)
            .unwrap()
    );
    assert!(
        snapshots
            .lock()
            .unwrap()
            .iter()
            .all(|counts| *counts == (1, 0))
    );
    let mut conflict = input;
    conflict
        .fields
        .insert("content".into(), json!("A different send"));
    assert_eq!(
        store
            .append_signed_message(&conflict, &signed(&conflict))
            .unwrap_err()
            .code,
        "idempotency-mismatch"
    );
    assert_eq!(
        store
            .readers
            .get()
            .query_row(
                "SELECT COUNT(*) FROM expected_claim_signatures",
                [],
                |row| row.get::<_, u64>(0)
            )
            .unwrap(),
        0
    );
}

#[test]
fn failed_signed_send_rolls_back_its_message_and_temporary_signature() {
    let store = Store::open_memory("node").unwrap();
    store.connection.write().execute_batch(
        /* sqlite-large-value: allow-for-now lv-hafe7a9268997 owner=st3_store::store::message_send_tests TODO=prepare-body-and-predecessor-values */ "CREATE TRIGGER reject_fixture_send BEFORE INSERT ON claims
         WHEN NEW.subject='message/local-signed-send' BEGIN SELECT RAISE(ABORT,'injected send failure'); END;",
    ).unwrap();
    let input = request();
    let signature = signed(&input);
    assert!(store.append_signed_message(&input, &signature).is_err());
    assert!(store.message(&input.subject).unwrap().is_none());
    assert!(
        !store
            .signature_nonce_used(&signature.key, &signature.nonce)
            .unwrap()
    );
    assert_eq!(
        store
            .readers
            .get()
            .query_row(
                "SELECT COUNT(*) FROM expected_claim_signatures",
                [],
                |row| row.get::<_, u64>(0)
            )
            .unwrap(),
        0
    );
    store
        .connection
        .write()
        .execute_batch(/* sqlite-large-value: allow-for-now lv-hb3ec307860fa owner=st3_store::store::message_send_tests TODO=bound-or-reference-values-and-reclamation */ "DROP TRIGGER reject_fixture_send")
        .unwrap();
    assert!(store.append_signed_message(&input, &signature).unwrap().1);
    assert_eq!(
        store.message(&input.subject).unwrap().unwrap().status,
        "sent"
    );
}

#[test]
fn digest_mismatched_message_operation_repair_persists_across_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("invented-message.db");
    let store = Store::open(&path, "fixture").unwrap();
    store.project_replication_backlog().unwrap();
    let input = request();
    let signature = signed(&input);
    let (claim, _) = store.append_signed_message(&input, &signature).unwrap();
    let (operation, digest) = operation_parts(&claim.body).unwrap();
    let operation = operation.to_owned();
    let digest = digest.to_owned();
    store
        .connection
        .write()
        .execute(
            /* sqlite-large-value: allow-for-now lv-hbb4b945bb48d owner=st3_store::store::message_send_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE operations SET request_digest='invented-stale-digest' WHERE id=?1",
            [&operation],
        )
        .unwrap();
    let mut lines = Vec::new();
    store
        .project_replication_backlog_with_log("fixture/startup", |line| lines.push(line.to_owned()))
        .unwrap();
    assert!(
        lines.is_empty(),
        "receipt-only repair must be incremental: {lines:?}"
    );
    assert_eq!(
        operation_tx(&store.readers.get(), &operation)
            .unwrap()
            .unwrap(),
        (digest.clone(), claim.id.clone(), "active".into())
    );
    assert_eq!(
        store.claim_by_id(&claim.id).unwrap().unwrap().body,
        claim.body
    );
    assert_eq!(
        serde_json::to_value(store.claim_signature(&claim.id).unwrap().unwrap()).unwrap(),
        serde_json::to_value(&signature).unwrap()
    );
    drop(store);
    let reopened = Store::open(&path, "fixture").unwrap();
    let mut lines = Vec::new();
    reopened
        .project_replication_backlog_with_log("fixture/reopen", |line| lines.push(line.to_owned()))
        .unwrap();
    assert!(
        lines.is_empty(),
        "committed stale receipt repair must survive restart: {lines:?}"
    );
    assert_eq!(
        operation_tx(&reopened.readers.get(), &operation)
            .unwrap()
            .unwrap(),
        (digest, claim.id.clone(), "active".into())
    );
    assert_eq!(
        serde_json::to_value(reopened.claim_signature(&claim.id).unwrap().unwrap()).unwrap(),
        serde_json::to_value(&signature).unwrap()
    );
}

#[test]
fn resolved_message_operation_conflict_does_not_replay_for_another_receipt() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("invented-conflict.db");
    let store = Store::open(&path, "fixture").unwrap();
    store.project_replication_backlog().unwrap();
    let input = request();
    let signature = signed(&input);
    let (original, _) = store.append_signed_message(&input, &signature).unwrap();
    store.project_replication_backlog().unwrap();
    let (operation, _) = operation_parts(&original.body).unwrap();
    let operation = operation.to_owned();
    let mut body = original.body.clone();
    body["_operation"]["request_digest"] = json!("invented-conflicting-digest");
    let incoming = {
        let mut connection = store.connection.write();
        let transaction = connection.transaction().unwrap();
        let claim = append_claim_record_tx(
            &transaction,
            "invented-peer",
            &original.subject,
            "message.sent",
            original.actor.as_deref(),
            &body,
            &[],
            None,
        )
        .unwrap();
        transaction.commit().unwrap();
        claim
    };
    let mut lines = Vec::new();
    store
        .project_replication_backlog_with_log("fixture/conflict", |line| {
            lines.push(line.to_owned())
        })
        .unwrap();
    assert!(
        lines.is_empty(),
        "message conflict repair must be incremental: {lines:?}"
    );
    let resolved = operation_tx(&store.readers.get(), &operation)
        .unwrap()
        .unwrap();
    assert_eq!(resolved.2, "conflict");
    assert_eq!(
        store.claim_by_id(&original.id).unwrap().unwrap().body,
        original.body
    );
    assert_eq!(store.claim_by_id(&incoming.id).unwrap().unwrap().body, body);
    assert_eq!(
        serde_json::to_value(store.claim_signature(&original.id).unwrap().unwrap()).unwrap(),
        serde_json::to_value(&signature).unwrap()
    );
    drop(store);
    let reopened = Store::open(&path, "fixture").unwrap();
    reopened.project_replication_backlog().unwrap();
    assert_eq!(
        operation_tx(&reopened.readers.get(), &operation)
            .unwrap()
            .unwrap(),
        resolved
    );
    {
        let mut connection = reopened.connection.write();
        let transaction = connection.transaction().unwrap();
        append_claim_record_tx(
            &transaction,
            "invented-other-peer",
            &original.subject,
            "message.sent",
            original.actor.as_deref(),
            &body,
            &[],
            None,
        )
        .unwrap();
        transaction.commit().unwrap();
    }
    let mut lines = Vec::new();
    reopened
        .project_replication_backlog_with_log("fixture/new-receipt", |line| {
            lines.push(line.to_owned())
        })
        .unwrap();
    assert!(
        !lines
            .iter()
            .any(|line| line.contains("projection full replay")),
        "durably recorded conflict must not force another full-store replay: {lines:?}"
    );
    assert_eq!(
        reopened
            .claim_signature(&original.id)
            .unwrap()
            .unwrap()
            .nonce,
        signature.nonce
    );
}

#[test]
fn message_operation_reconciliation_rolls_back_with_its_frontier() {
    let store = Store::open_memory("fixture").unwrap();
    store.project_replication_backlog().unwrap();
    let (original, _) = store
        .append_signed_message(&request(), &signed(&request()))
        .unwrap();
    store.project_replication_backlog().unwrap();
    let (operation, _) = operation_parts(&original.body).unwrap();
    let before = operation_rows(&store.readers.get());
    let index = store.index().unwrap();
    let mut connection = store.connection.write();
    let tx = connection.transaction().unwrap();
    let frontier: u64 = tx
        .query_row(
            "SELECT last_good_store_index FROM projection_health WHERE aggregate='graph'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let mut body = original.body.clone();
    body["_operation"]["request_digest"] = json!("invented-rollback-digest");
    let incoming = append_claim_record_tx(
        &tx,
        "invented-peer",
        &original.subject,
        "message.sent",
        original.actor.as_deref(),
        &body,
        &[],
        None,
    )
    .unwrap();
    assert_eq!(
        try_project_simple_replication_tx(&tx, "fixture", incoming.store_index).unwrap(),
        IncrementalProjection::Projected
    );
    assert_eq!(operation_tx(&tx, operation).unwrap().unwrap().2, "conflict");
    assert_eq!(
        tx.query_row(
            "SELECT last_good_store_index FROM projection_health WHERE aggregate='graph'",
            [],
            |row| row.get::<_, u64>(0)
        )
        .unwrap(),
        frontier
    );
    tx.rollback().unwrap();
    drop(connection);
    assert_eq!(store.index().unwrap(), index);
    assert_eq!(operation_rows(&store.readers.get()), before);
    assert!(store.claim_by_id(&incoming.id).unwrap().is_none());
}

#[test]
fn message_operation_late_conflict_after_duplicate_receipt_matches_replay() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("invented-late-conflict.db");
    let mut store = Store::open(&path, "fixture").unwrap();
    store.project_replication_backlog().unwrap();
    let input = request();
    let signature = signed(&input);
    let (original, _) = store.append_signed_message(&input, &signature).unwrap();
    store.project_replication_backlog().unwrap();
    let (operation, _) = operation_parts(&original.body).unwrap();
    let mut body = original.body.clone();
    for conflicting in [false, true] {
        if conflicting {
            body["_operation"]["request_digest"] = json!("invented-late-conflict");
        }
        {
            let mut connection = store.connection.write();
            let tx = connection.transaction().unwrap();
            append_claim_record_tx(
                &tx,
                "invented-peer",
                &original.subject,
                "message.sent",
                original.actor.as_deref(),
                &body,
                &[],
                None,
            )
            .unwrap();
            tx.commit().unwrap();
        }
        FULL_REPLAYS.with(|count| count.set(0));
        store.project_replication_backlog().unwrap();
        assert_eq!(FULL_REPLAYS.with(std::cell::Cell::get), 0);
        assert_eq!(
            operation_tx(&store.readers.get(), operation)
                .unwrap()
                .unwrap()
                .2,
            if conflicting { "conflict" } else { "active" }
        );
        assert_operation_replay_parity(&store);
        assert_eq!(
            serde_json::to_value(store.claim_signature(&original.id).unwrap().unwrap()).unwrap(),
            serde_json::to_value(&signature).unwrap()
        );
        drop(store);
        store = Store::open(&path, "fixture").unwrap();
        store.project_replication_backlog().unwrap();
    }
    assert_eq!(
        store
            .append_signed_message(&input, &signed(&input))
            .unwrap_err()
            .code,
        "idempotency-conflict"
    );
    assert_eq!(
        store.claim_by_id(&original.id).unwrap().unwrap().body,
        original.body
    );
    assert_eq!(
        serde_json::to_value(store.claim_signature(&original.id).unwrap().unwrap()).unwrap(),
        serde_json::to_value(&signature).unwrap()
    );
}

#[test]
fn assistant_provenance_generic_signed_and_transaction_appends_refuse_forgery() {
    let store = Store::open_memory("node").unwrap();
    for (actor, field, value) in [
        ("assistant/chatgpt", "from", json!("person/eval")),
        ("person/eval", "from", json!("assistant/chatgpt")),
        ("person/eval", "sender_class", json!("external-assistant")),
        ("person/eval", "assistant_grant_id", json!("assistant-grant/forged")),
    ] {
        let mut input = request();
        input.actor = Some(actor.into());
        input.fields.insert(field.into(), value);
        assert_eq!(store.append_claim(&input).unwrap_err().code, "assistant-send-refused");
        assert_eq!(store.graph.append_claim_outcome(&input).unwrap_err().code, "assistant-send-refused");
        assert_eq!(store.append_signed_message(&input, &signed(&input)).unwrap_err().code, "assistant-send-refused");
        let mut connection = store.connection.lock().unwrap();
        let transaction = connection.transaction().unwrap();
        let error = append_claim_tx(&transaction, "node", &input.subject, &input.kind,
            input.actor.as_deref(), &json!({"fields":input.fields}), &[], None).unwrap_err();
        assert_eq!(error.downcast_ref::<St3Error>().unwrap().code, "assistant-send-refused");
    }
    assert!(store.latest_claim("message/local-signed-send", None).unwrap().is_none());
}

#[test]
fn assistant_provenance_stamps_authenticated_grant_and_preserves_replica_admission() {
    let store = Store::open_memory("node").unwrap();
    let grant_subject = "custom/client/assistant-v1-native-fixture";
    let grant_id = "assistant-grant/native-fixture";
    let session_actor = format!("assistant/chatgpt/session/{grant_id}");
    store.append_claim(&ClaimInput {
        subject: grant_subject.into(), kind: "custom.client.assistant.v1.granted".into(),
        actor: Some("person/eval".into()),
        fields: serde_json::from_value(json!({
            "principal":"assistant/chatgpt", "session_actor":session_actor,
            "issuer":"person/eval", "disclosure_consent":true, "policy_revision":1,
            "expires_at_unix_ms":now_ms() as u64 + 60_000,
            "scopes":["write.assistant.messages"], "recipient_allowlist":["agent/eval.recipient"],
            "grant_id":grant_id
        })).unwrap(),
        evidence: vec![], expected_subject: None, idempotency_key: None,
    }).unwrap();
    let authorization = assistant_write::AssistantWriteAuthorization {
        grant_subject: grant_subject.into(), actor: session_actor.clone(),
        owner: "person/eval".into(), policy_revision: 1, scope: "write.assistant.messages".into(),
        recipient: Some("agent/eval.recipient".into()), approval_subject: None,
    };
    let mut input = request();
    input.actor = Some("assistant/chatgpt".into());
    input.fields.insert("from".into(), json!("assistant/chatgpt"));
    input.fields.insert("sender_class".into(), json!("external-assistant"));
    input.fields.insert("assistant_grant_id".into(), json!("assistant-grant/caller-forged"));
    let (claim, appended) = store.append_assistant_claim_outcome(&input, &authorization).unwrap();
    assert!(appended);
    assert_eq!(claim.actor.as_deref(), Some("assistant/chatgpt"));
    assert_eq!(claim.body["fields"]["from"], "assistant/chatgpt");
    assert_eq!(claim.body["fields"]["assistant_owner"], "person/eval");
    assert_eq!(claim.body["fields"]["assistant_session_actor"], session_actor);
    assert_eq!(claim.body["fields"]["sender_class"], "external-assistant");
    assert_eq!(claim.body["fields"]["assistant_grant_id"], grant_id);
    assert_eq!(store.append_assistant_claim_outcome(&input, &authorization).unwrap().0.id, claim.id);
    assert!(matches!(classify_replicated_claim_with_registry(&claim, st3_schema::registry()).unwrap(),
        ReplicatedClaimAdmission::Valid));
    let mut forged = claim;
    forged.body["fields"].as_object_mut().unwrap().remove("assistant_grant_id");
    assert_eq!(classify_replicated_claim_with_registry(&forged, st3_schema::registry()).err().unwrap().code,
        "assistant-send-refused");
    let mut wrong_actor = input;
    wrong_actor.actor = Some("person/intruder".into());
    assert_eq!(store.append_assistant_claim_outcome(&wrong_actor, &authorization).unwrap_err().code, "forbidden");
}
