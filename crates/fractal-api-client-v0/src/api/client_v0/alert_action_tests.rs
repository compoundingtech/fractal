//! Shared dispatcher controls, using authored sessions rather than real device grants.
use super::*;

#[tokio::test]
async fn delegated_prompt_actions_share_one_source_contract_and_cannot_choose_the_actor() {
    for client in ["web", "tui", "ios"] {
        let root = tempfile::tempdir().unwrap();
        let state = tests::test_state_named(root.path(), "alder");
        let (denial, proof) =
            crate::store::alert_action_controls::seed_denial(&state.store, &["close-item"]);
        let snapshot = new_client_snapshot(&state);
        let session = ClientSession::for_tests_scoped(
            &format!("client/{client}-authored-fixture"),
            "agent/alder.helper",
            &["control.attention"],
        );
        let parameters = json!({"target_id":"agent/alder.worker","episode":denial.id,"answer":"dismiss","delegation":proof});
        let typed: fractal_client::PromptRespondParameters =
            serde_json::from_value(parameters.clone()).unwrap();
        assert_eq!(serde_json::to_value(typed).unwrap(), parameters);
        let request = ActionRequest {
            api_version: CLIENT_API_VERSION.into(),
            id: format!("action/{client}-dismiss"),
            action_type: "prompt.respond".into(),
            idempotency_key: format!("authored-{client}-delegated-dismiss"),
            fence: Fence {
                snapshot_id: snapshot.id.clone(),
                ..Default::default()
            },
            parameters,
        };
        // Main's ordinary answers remain text; supplying proof narrows the
        // source action to dismiss rather than granting those ordinary answers.
        let before = state.store.index().unwrap();
        for answer in ["allow", "approve", "use-all", r#"{"0":[1]}"#] {
            let mut refused = request.clone();
            refused.idempotency_key = format!("authored-{client}-refused-{answer}");
            refused.parameters["answer"] = json!(answer);
            assert_eq!(
                action_inner(
                    State(state.clone()),
                    Extension(snapshot.clone()),
                    Extension(session.clone()),
                    Json(refused),
                    None
                )
                .await
                .unwrap_err()
                .code,
                "delegation-refused"
            );
            assert_eq!(state.store.index().unwrap(), before);
        }
        let mut forged = request.clone();
        forged.parameters["actor"] = json!("person/avery");
        assert_eq!(
            action_inner(
                State(state.clone()),
                Extension(snapshot.clone()),
                Extension(session.clone()),
                Json(forged),
                None
            )
            .await
            .unwrap_err()
            .code,
            "validation-failed"
        );
        let Json(first) = action_inner(
            State(state.clone()),
            Extension(snapshot.clone()),
            Extension(session.clone()),
            Json(request.clone()),
            None,
        )
        .await
        .unwrap();
        let Json(again) = action_inner(
            State(state.clone()),
            Extension(snapshot),
            Extension(session),
            Json(request),
            None,
        )
        .await
        .unwrap();
        assert_eq!(first["affected_ids"], again["affected_ids"]);
        assert_eq!(first["affected_ids"], json!(["agent/alder.worker"]));
        let answer = state
            .store
            .claims_for("agent/alder.worker", Some("harness.diagnostic"))
            .unwrap()
            .into_iter()
            .find(|claim| claim.body["fields"]["code"] == "native-prompt-answered")
            .unwrap();
        assert_eq!(answer.actor.as_deref(), Some("agent/alder.helper"));
        assert_eq!(answer.body["fields"]["acted_for"], "person/avery");
        assert_eq!(
            state
                .store
                .messages(Some("agent/alder.worker"), true)
                .unwrap()
                .len(),
            1
        );
    }
}

/// A card may have offered allow from a read before its call's result. An answer that lets the
/// call through reads the seat's conversation again, and goes on only while it sees the named
/// call waiting: not once the call has its result, even beyond the entries a card reads, and not
/// when the conversation cannot be read. Nothing is recorded for a refused answer.
#[tokio::test]
async fn an_allow_for_a_call_that_already_has_a_result_is_refused() {
    use sha2::Digest as _;
    let root = tempfile::tempdir().unwrap();
    let mut state = tests::test_state_named(root.path(), "node");
    let home = root.path().join("home");
    let transcript = home.join(".codex/sessions/2026/10/30/answered.jsonl");
    std::fs::create_dir_all(transcript.parent().unwrap()).unwrap();
    let native_id = "native-answered-codex-test";
    let lines = [
        json!({"type":"session_meta","timestamp":"2026-10-30T12:00:00Z","payload":{"id":native_id,"cwd":root.path(),"source":"test"}}),
        json!({"type":"response_item","timestamp":"2026-10-30T12:00:01Z","payload":{"type":"function_call","name":"shell","arguments":"{\"command\":[\"make\"]}","call_id":"call-done"}}),
        json!({"type":"response_item","timestamp":"2026-10-30T12:00:02Z","payload":{"type":"function_call_output","call_id":"call-done","output":"built"}}),
        json!({"type":"response_item","timestamp":"2026-10-30T12:00:03Z","payload":{"type":"function_call","name":"shell","arguments":"{\"command\":[\"make\",\"release\"]}","call_id":"call-waiting"}}),
    ];
    std::fs::write(&transcript, lines.iter().map(|line| format!("{line}\n")).collect::<String>()).unwrap();
    state.native_session_home = Some(home);
    let workspace = tempfile::tempdir().unwrap();
    let source = format!(
        "version 2\nagent \"worker\" {{ workspace {:?}; harness \"codex\" {{}} }}\n",
        workspace.path().display().to_string()
    );
    let intent = crate::graph::parse_intent(&source, "node").unwrap();
    let preview = state
        .store
        .mission(&intent, crate::model::IntentInput { kdl: source, source_name: None })
        .unwrap();
    state
        .store
        .apply_as(&intent, &preview.subject_tokens, "answered-seat", Some("person/avery"))
        .unwrap();
    let seat = "agent/node.worker";
    let (incarnation, provider) = ("native-pty:one", "provider-one");
    let directory = state
        .state_dir
        .join("drivers")
        .join(&hex::encode(sha2::Sha256::digest(seat.as_bytes()))[..24])
        .join("state");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join("runtime.json"), json!({"agent":"node.worker","incarnation":provider}).to_string()).unwrap();
    std::fs::write(
        directory.join("binding.json"),
        json!({"agent":"node.worker","runtimeIncarnation":provider,"threadId":native_id}).to_string(),
    )
    .unwrap();
    let append = |kind: &str, fields: Value| {
        state
            .store
            .append_claim(&ClaimInput {
                subject: seat.into(),
                kind: kind.into(),
                actor: Some(seat.into()),
                fields: serde_json::from_value(fields).unwrap(),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap()
    };
    append("runtime.observed", json!({"status":"running","runtime_id":"answered-pty","incarnation_id":incarnation,"terminal":true}));
    let asked = append(
        "harness.observed",
        json!({"state":"blocked","driver":"codex","incarnation_id":incarnation,"evidence_incarnation":provider,
            "blocked_on":"human","ask":"permission","reason":"commandExecution","ownership_sequence":1,"transition_sequence":3}),
    );
    let named = |call: &str| {
        append(
            "harness.diagnostic",
            json!({"severity":"warning","status":"shown","code":"native-prompt-call","driver":"codex","ownership_sequence":1,
                "matched_line": json!({"call_id": call, "transition": 3}).to_string()}),
        );
    };
    let session = ClientSession::for_tests_scoped("client/tui-answer-fixture", "person/avery", &["control.attention"]);
    let answer = |key: &str| {
        let snapshot = new_client_snapshot(&state);
        let request = ActionRequest {
            api_version: CLIENT_API_VERSION.into(),
            id: format!("action/{key}"),
            action_type: "prompt.respond".into(),
            idempotency_key: format!("authored-answer-{key}"),
            fence: Fence { snapshot_id: snapshot.id.clone(), ..Default::default() },
            parameters: json!({"target_id": seat, "episode": asked.id, "answer": "allow"}),
        };
        action_inner(State(state.clone()), Extension(snapshot), Extension(session.clone()), Json(request), None)
    };

    let refused = |key: &'static str, says: &'static str| {
        let answer = &answer;
        let state = &state;
        async move {
            let before = state.store.index().unwrap();
            let refused = answer(key).await.unwrap_err();
            assert_eq!(refused.code, "stale-fence", "{key}: {}", refused.message);
            assert!(refused.message.contains(says), "{key}: {}", refused.message);
            assert_eq!(state.store.index().unwrap(), before, "{key}: nothing was recorded");
        }
    };

    // The prompt names the call that already has its result: allowing it is refused.
    named("call-done");
    assert_eq!(state.store.native_prompt_call(seat, &asked.id).unwrap().as_deref(), Some("call-done"));
    refused("allow-answered", "already has a result").await;
    // The prompt names the call that waits, but its conversation cannot be read here any more.
    named("call-waiting");
    let kept = std::fs::read(&transcript).unwrap();
    std::fs::remove_file(&transcript).unwrap();
    refused("allow-unreadable", "can only be answered in the seat's terminal").await;
    // Read again, it waits: the answer is recorded.
    std::fs::write(&transcript, &kept).unwrap();
    let before = state.store.index().unwrap();
    let _recorded = answer("allow-waiting").await.unwrap();
    assert!(state.store.index().unwrap() > before, "the answer was recorded");
    // A result more entries back than a card reads still counts: the call no longer waits.
    let mut file = std::fs::OpenOptions::new().append(true).open(&transcript).unwrap();
    let mut later = vec![json!({"type":"response_item","timestamp":"2026-10-30T12:00:04Z","payload":{"type":"function_call_output","call_id":"call-waiting","output":"released"}})];
    later.extend((0..300).map(|n| json!({"type":"response_item","timestamp":"2026-10-30T12:00:05Z","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":format!("step {n}")}]}})));
    std::io::Write::write_all(&mut file, later.iter().map(|line| format!("{line}\n")).collect::<String>().as_bytes()).unwrap();
    refused("allow-long-after", "does not see it waiting").await;
}
