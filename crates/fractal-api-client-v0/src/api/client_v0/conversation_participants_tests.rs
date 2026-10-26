use super::*;

#[test]
fn daemon_authorship_is_truthful_without_widening_legacy_sender_or_guessing_broadcasts() {
    let root = tempfile::tempdir().unwrap();
    let state = super::tests::test_state_named(root.path(), "message-author-fixture");
    let send = |subject: &str, actor: Option<&str>, from: &str, to: &str| {
        state.store.append_claim(&ClaimInput {
                subject:subject.into(),kind:"message.sent".into(),actor:actor.map(str::to_owned),
                fields:serde_json::from_value(json!({"from":from,"to":to,"content":"Identical invented text","status":"sent"})).unwrap(),
                evidence:vec![],expected_subject:None,idempotency_key:None,
            }).unwrap()
    };
    let daemon = send(
        "message/runtime",
        Some("daemon/runtime"),
        "daemon/runtime",
        "agent/eval.worker",
    );
    let body = session_message_body(&state, &daemon, true);
    assert_eq!(body["author"], "daemon/runtime");
    assert!(body.get("from").is_none());
    assert_eq!(body["to"], "agent/eval.worker");
    assert_eq!(body["recipients"], json!(["agent/eval.worker"]));
    let legacy = session_message_body(&state, &daemon, false);
    assert!(legacy.get("author").is_none());
    assert!(legacy.get("recipients").is_none());
    for recipient in ["agent/one", "person/two", "step/three"] {
        let claim = send(
            &format!("message/{}", recipient.replace('/', "-")),
            Some("person/fixture"),
            "person/fixture",
            recipient,
        );
        let body = session_message_body(&state, &claim, true);
        assert_eq!(body["author"], "person/fixture");
        assert_eq!(body["from"], "person/fixture");
        assert_eq!(body["recipients"], json!([recipient]));
        assert_eq!(body["message_id"], claim.subject);
    }
    let unknown = send(
        "message/unknown",
        None,
        "person/fixture",
        "agent/eval.worker",
    );
    assert!(
        session_message_body(&state, &unknown, true)
            .get("author")
            .is_none()
    );
    let session = ClientSession::local(Some("person/fixture")).unwrap();
    let mut item = json!({"type":"message","body":body});
    conversation_participants::legacy_item(&mut item, &session);
    assert_eq!(item["body"], legacy);
    let mut future =
        json!({"type":"future-message-kind","body":{"author":"opaque","recipients":"opaque"}});
    let original = future.clone();
    conversation_participants::legacy_item(&mut future, &session);
    assert_eq!(future, original);
}

#[test]
fn participant_negotiation_is_exact_and_legacy_relays_remove_only_new_message_fields() {
    let root = tempfile::tempdir().unwrap();
    let state = super::tests::test_state_named(root.path(), "participant-negotiation-fixture");
    for (features, negotiated) in [
        ("", false),
        ("conversation-message-participants.v2", false),
        ("conversation-message-participants.v1-extra", false),
        (
            "conversation-blocks.v1, conversation-message-participants.v1",
            true,
        ),
    ] {
        let request = Request::builder()
            .uri("/v1/client/capabilities")
            .header("x-st3-features", features)
            .body(Body::empty())
            .unwrap();
        let session = authenticate(&state, &request, "unix").unwrap();
        assert_eq!(session.conversation_message_participants, negotiated);
        let message = json!({"type":"message", "body": {
            "message_id":"message/fixture", "from":"person/fixture", "to":"agent/worker",
            "author":"person/fixture", "recipients":["agent/worker"],
            "sender_class":"external-assistant", "assistant_grant_id":"grant/fixture"
        }});
        let opaque =
            json!({"type":"future-kind", "body":{"author":"opaque","recipients":"opaque"}});
        let mut page = json!({"items":[message.clone(), opaque.clone()]});
        conversation_blocks::legacy(&mut page, &session);
        let mut expected = message;
        if !negotiated {
            expected["body"].as_object_mut().unwrap().remove("author");
            expected["body"]
                .as_object_mut()
                .unwrap()
                .remove("recipients");
        }
        assert_eq!(page["items"][0], expected);
        assert_eq!(page["items"][1], opaque);
    }
}
