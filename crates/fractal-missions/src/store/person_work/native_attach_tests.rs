use super::*;

const PERSON: &str = "person/operator";
const SESSION: &str = "person/operator/session/phone";
const PREPARATION: &str = "01900000-0000-7000-8000-000000000001";

fn ask_input() -> PersonAskRequest {
    serde_json::from_value(json!({
        "person":PERSON,"actor":"daemon/native-attach","title":"Attach native session",
        "reason":"Approve control of the selected existing session.",
        "idempotency_key":format!("native-attach/{PREPARATION}/{}", "a".repeat(64)),
        "request":{"version":1,"type":"decision","question":"Attach this session?",
            "why_person":"Only the assigned person can approve attaching.","answers":[
                {"id":"attach","label":"Attach","outcome":"accept","consequence":"Attach this session."},
                {"id":"decline","label":"Decline","outcome":"decline","consequence":"Do not attach."}
            ]}
    })).unwrap()
}

fn grant(store: &Store, person: &str, session: &str, scopes: &[&str], expiry: u64, trusted: bool) -> ClaimRecord {
    let input = smallclaims::claim::ClaimInput {
        subject:format!("custom/client/pairing-{}", uuid::Uuid::new_v4()),
        kind:"custom.client.pairing-completed".into(), actor:Some(person.into()),
        fields:serde_json::from_value(json!({"person_id":person,"session_actor":session,
            "credential_hash":"test-digest","expires_at_unix_ms":expiry,"scopes":scopes})).unwrap(),
        evidence:vec![],expected_subject:None,idempotency_key:None,
    };
    if trusted { store.append_client_operation_claim(&input) } else { store.append_claim(&input) }.unwrap()
}

fn fixture() -> (Store, ClaimRecord, ClaimRecord, PersonStepResponse) {
    let store = Store::open_memory("issuer").unwrap();
    let view = store.ask_person(&ask_input()).unwrap();
    let ask = request(&store.readers.get(), &view.subject).unwrap().unwrap();
    let paired = grant(&store, PERSON, SESSION, &["control.work"], u64::MAX, true);
    assert!(store.pairing_completion_is_trusted(&paired, now_ms()).unwrap());
    let input = serde_json::from_value(json!({"subject":ask.subject,"actor":PERSON,
        "summary":"Attach","evidence":[],"episode":ask.id,"idempotency_key":"answer",
        "answer":{"id":"attach"}})).unwrap();
    (store, ask, paired, input)
}

fn answer(store: &Store, input: &PersonStepResponse, paired: &ClaimRecord) -> Result<StepRunView, FractalError> {
    store.finish_person_step_kind(input, false, None, false, Some((&paired.subject, SESSION)))
}

fn receipts(store: &Store) -> i64 {
    store.readers.get().query_row("SELECT count(*) FROM claims WHERE kind='custom.client.pairing-native-attach-answer-admitted'", [], |row| row.get(0)).unwrap()
}

#[test]
fn native_attach_paired_answer_has_exact_operation_backed_receipt_and_idempotent_retry() {
    let (store, ask, paired, input) = fixture();
    answer(&store, &input, &paired).unwrap();
    answer(&store, &input, &paired).unwrap();
    assert_eq!(receipts(&store), 1);
    let done = store.claims_for_subject_kind_at(&ask.subject, "work.person-done", None, true, 1).unwrap().claims.remove(0);
    let receipt = store.claims_for_subject_kind_at(&format!("custom/client/native-attach-answer-{}", done.id),
        "custom.client.pairing-native-attach-answer-admitted", None, true, 1).unwrap().claims.remove(0);
    assert_eq!(receipt.actor, done.actor);
    assert_eq!(receipt.origin, done.origin);
    assert_eq!(receipt.body["fields"], json!({"answer_id":done.id,"ask":ask.subject,"episode":ask.id,
        "person":PERSON,"preparation":PREPARATION,"candidate_digest":"a".repeat(64),
        "grant_id":paired.id,"grant_subject":paired.subject}));
    assert!(store.client_operation_is_trusted(&receipt).unwrap());
    let mut replay = input.clone();
    replay.idempotency_key = "another-answer".into();
    assert!(answer(&store, &replay, &paired).is_err());
    replay = input.clone();
    replay.answer.as_mut().unwrap().id = Some("decline".into());
    assert!(answer(&store, &replay, &paired).is_err());
    assert_eq!(receipts(&store), 1);
}

#[test]
fn native_attach_refuses_unpaired_forged_delegated_and_unfenced_answers() {
    let (store, ask, paired, input) = fixture();
    assert!(store.finish_person_step(&input, false).is_err(), "local same-person answer");
    for actor in ["person/other", "agent/operator", "assistant/chatgpt", SESSION] {
        let mut forged = input.clone();
        forged.actor = actor.into();
        assert!(answer(&store, &forged, &paired).is_err(), "{actor}");
    }
    for episode in [None, Some("different-episode".into())] {
        let mut unfenced = input.clone();
        unfenced.episode = episode;
        assert!(answer(&store, &unfenced, &paired).is_err());
    }
    let mut unnamed = input.clone();
    unnamed.answer = None;
    assert!(answer(&store, &unnamed, &paired).is_err());
    let mut delegated = input.clone();
    delegated.delegation = Some(serde_json::from_value(json!({"person":PERSON,"policy":"policy/test",
        "message":"message/test","quote":"attach","episode":ask.id})).unwrap());
    assert!(answer(&store, &delegated, &paired).is_err());
    assert!(store.override_person_step(&input).is_err());
    assert_eq!(receipts(&store), 0);
}

#[test]
fn native_attach_refuses_wrong_grants_sessions_scopes_expiry_and_revocation() {
    let (store, _ask, paired, input) = fixture();
    assert!(store.finish_person_step_kind(&input, false, None, false, Some((&paired.subject, "person/operator/session/other"))).is_err());
    assert!(store.finish_person_step_kind(&input, false, None, false, Some(("custom/client/missing", SESSION))).is_err());
    for (person, session, scopes, expiry, trusted) in [
        ("person/other", SESSION, vec!["control.work"], u64::MAX, true),
        (PERSON, "person/operator/session/other", vec!["control.work"], u64::MAX, true),
        (PERSON, SESSION, vec!["control.pairing"], u64::MAX, true),
        (PERSON, SESSION, vec!["control.work"], 1, true),
        (PERSON, SESSION, vec!["control.work"], u64::MAX, false),
    ] {
        let invalid = grant(&store, person, session, &scopes, expiry, trusted);
        assert!(answer(&store, &input, &invalid).is_err());
    }
    store.append_client_operation_claim(&smallclaims::claim::ClaimInput {
        subject:paired.subject.clone(), kind:"custom.client.pairing-revoked".into(), actor:Some(PERSON.into()),
        fields:serde_json::from_value(json!({"completion_id":paired.id})).unwrap(),
        evidence:vec![],expected_subject:None,idempotency_key:None,
    }).unwrap();
    assert!(store.pairing_completion_is_revoked(&paired).unwrap());
    assert!(answer(&store, &input, &paired).is_err());
    assert_eq!(receipts(&store), 0);
}

#[test]
fn native_attach_defaults_expired_asks_and_assistant_admission_are_refused() {
    let (store, ask, paired, input) = fixture();
    let mut defaulted = ask_input();
    defaulted.default_answer = Some("attach".into());
    assert!(store.ask_person(&defaulted).is_err());
    store.connection.batched(|tx| -> anyhow::Result<()> {
        let view = step(tx, &ask.subject)?.unwrap();
        assert!(admit(tx, &ask, &view, &input, false, true, Some((&paired.subject, SESSION))).is_err());
        for (field, value) in [("deadline_unix_ms", json!(1)), ("deadline_unix_ms", Value::Null), ("default_answer", json!("attach"))] {
            let mut invalid = ask.clone();
            invalid.body["fields"][field] = value;
            assert!(admit(tx, &invalid, &view, &input, false, false, Some((&paired.subject, SESSION))).is_err());
        }
        Ok(())
    }).unwrap().unwrap();
    assert_eq!(receipts(&store), 0);
}

#[test]
fn native_attach_decline_and_dismissal_remain_nonaccepting_person_answers() {
    for choice in ["decline", crate::person_request::DISMISS] {
        let (store, ask, paired, mut input) = fixture();
        input.answer.as_mut().unwrap().id = Some(choice.into());
        answer(&store, &input, &paired).unwrap();
        let done = store.claims_for_subject_kind_at(&ask.subject, "work.person-done", None, true, 1).unwrap().claims.remove(0);
        assert_ne!(done.body["fields"]["answer"]["outcome"], "accept");
        assert_eq!(receipts(&store), 1);
    }
}

#[test]
fn native_attach_key_requires_canonical_uuid_and_lowercase_digest() {
    assert!(key_parts(&format!("native-attach/{PREPARATION}/{}", "a".repeat(64))).is_some());
    for key in [format!("native-attach/{PREPARATION}/{}", "A".repeat(64)),
        format!("native-attach/{PREPARATION}/{}", "g".repeat(64)),
        format!("native-attach/{PREPARATION}/{}", "a".repeat(63)),
        format!("native-attach/not-a-uuid/{}", "a".repeat(64)),
        format!("native-attach/{PREPARATION}/{}/extra", "a".repeat(64))] {
        assert!(key_parts(&key).is_none(), "{key}");
    }
}
