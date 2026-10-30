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

/// A Codex seat on `node` whose person is avery, blocked on a permission prompt. Its
/// transcript has `call-done` with its result, then `call-waiting` with none. Returns the state,
/// the seat, the prompt's episode and the transcript.
fn prompted_codex_seat(root: &Path, node: &str) -> (AppState, String, String, std::path::PathBuf) {
    use sha2::Digest as _;
    let mut state = tests::test_state_named(root, node);
    let home = root.join("home");
    let transcript = home.join(".codex/sessions/2026/10/30/answered.jsonl");
    std::fs::create_dir_all(transcript.parent().unwrap()).unwrap();
    let native_id = format!("native-{node}-codex-test");
    let lines = [
        json!({"type":"session_meta","timestamp":"2026-10-30T12:00:00Z","payload":{"id":native_id,"cwd":root,"source":"test"}}),
        json!({"type":"response_item","timestamp":"2026-10-30T12:00:01Z","payload":{"type":"function_call","name":"shell","arguments":"{\"command\":[\"make\"]}","call_id":"call-done"}}),
        json!({"type":"response_item","timestamp":"2026-10-30T12:00:02Z","payload":{"type":"function_call_output","call_id":"call-done","output":"built"}}),
        json!({"type":"response_item","timestamp":"2026-10-30T12:00:03Z","payload":{"type":"function_call","name":"shell","arguments":"{\"command\":[\"make\",\"release\"]}","call_id":"call-waiting"}}),
    ];
    std::fs::write(&transcript, lines.iter().map(|line| format!("{line}\n")).collect::<String>()).unwrap();
    state.native_session_home = Some(home);
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let source = format!("version 2\nagent \"worker\" {{ workspace {:?}; harness \"codex\" {{}} }}\n", workspace.display().to_string());
    let intent = crate::graph::parse_intent(&source, node).unwrap();
    let preview = state.store.mission(&intent, crate::model::IntentInput { kdl: source, source_name: None }).unwrap();
    state.store.apply_as(&intent, &preview.subject_tokens, "answered-seat", Some("person/avery")).unwrap();
    let seat = format!("agent/{node}.worker");
    let (incarnation, provider) = ("native-pty:one", "provider-one");
    let directory = state.state_dir.join("drivers").join(&hex::encode(sha2::Sha256::digest(seat.as_bytes()))[..24]).join("state");
    std::fs::create_dir_all(&directory).unwrap();
    let agent = seat.trim_start_matches("agent/");
    std::fs::write(directory.join("runtime.json"), json!({"agent":agent,"incarnation":provider}).to_string()).unwrap();
    std::fs::write(directory.join("binding.json"), json!({"agent":agent,"runtimeIncarnation":provider,"threadId":native_id}).to_string()).unwrap();
    append_seat_claim(&state, &seat, "runtime.observed", json!({"status":"running","runtime_id":"answered-pty","incarnation_id":incarnation,"terminal":true}));
    let asked = append_seat_claim(
        &state,
        &seat,
        "harness.observed",
        json!({"state":"blocked","driver":"codex","incarnation_id":incarnation,"evidence_incarnation":provider,
            "blocked_on":"human","ask":"permission","reason":"commandExecution","ownership_sequence":1,"transition_sequence":3}),
    );
    (state, seat, asked.id, transcript)
}

fn append_seat_claim(state: &AppState, seat: &str, kind: &str, fields: Value) -> ClaimRecord {
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
}

/// The seat's hook names `call` as the one its prompt asks about.
fn name_prompt_call(state: &AppState, seat: &str, call: &str) {
    append_seat_claim(
        state,
        seat,
        "harness.diagnostic",
        json!({"severity":"warning","status":"shown","code":"native-prompt-call","driver":"codex","ownership_sequence":1,
            "matched_line": json!({"call_id": call, "transition": 3}).to_string()}),
    );
}

/// Avery allows the prompt `episode` of `seat` through `state`, as their client would.
async fn allow_prompt(state: &AppState, seat: &str, episode: &str, key: &str) -> Result<(), ApiError> {
    let session = ClientSession::for_tests_scoped("client/tui-answer-fixture", "person/avery", &["control.attention", "read.projections"]);
    let snapshot = new_client_snapshot(state);
    let request = ActionRequest {
        api_version: CLIENT_API_VERSION.into(),
        id: format!("action/{key}"),
        action_type: "prompt.respond".into(),
        idempotency_key: format!("authored-answer-{key}"),
        fence: Fence { snapshot_id: snapshot.id.clone(), ..Default::default() },
        parameters: json!({"target_id": seat, "episode": episode, "answer": "allow"}),
    };
    action_inner(State(state.clone()), Extension(snapshot), Extension(session), Json(request), None).await.map(|_| ())
}

/// An allow `state` refuses with `code` saying `says`, recording nothing.
async fn allow_refused(state: &AppState, seat: &str, episode: &str, key: &str, code: &str, says: &str) {
    let before = state.store.index().unwrap();
    let refused = allow_prompt(state, seat, episode, key).await.unwrap_err();
    assert_eq!(refused.code, code, "{key}: {}", refused.message);
    assert!(refused.message.contains(says), "{key}: {}", refused.message);
    assert_eq!(state.store.index().unwrap(), before, "{key}: nothing was recorded");
}

/// A card may have offered allow from a read before its call's result. An answer that lets the
/// call through reads the seat's conversation again, and goes on only while it sees the named
/// call waiting: not once the call has its result, even beyond the entries a card reads, and not
/// when the conversation cannot be read. Nothing is recorded for a refused answer.
#[tokio::test]
async fn an_allow_for_a_call_that_already_has_a_result_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let (state, seat, episode, transcript) = prompted_codex_seat(root.path(), "node");

    // The prompt names the call that already has its result: allowing it is refused.
    name_prompt_call(&state, &seat, "call-done");
    assert_eq!(state.store.native_prompt_call(&seat, &episode).unwrap().as_deref(), Some("call-done"));
    allow_refused(&state, &seat, &episode, "allow-answered", "stale-fence", "already has a result").await;
    // The prompt names the call that waits, but its conversation cannot be read here any more:
    // an unreadable transcript reads as no entries, so no call waits.
    name_prompt_call(&state, &seat, "call-waiting");
    let kept = std::fs::read(&transcript).unwrap();
    std::fs::remove_file(&transcript).unwrap();
    allow_refused(&state, &seat, &episode, "allow-unreadable", "stale-fence", "that prompt ").await;
    // A result more entries back than a card reads still counts: the call no longer waits.
    let mut later = vec![json!({"type":"response_item","timestamp":"2026-10-30T12:00:04Z","payload":{"type":"function_call_output","call_id":"call-waiting","output":"released"}})];
    later.extend((0..300).map(|n| json!({"type":"response_item","timestamp":"2026-10-30T12:00:05Z","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":format!("step {n}")}]}})));
    let long = [kept.clone(), later.iter().map(|line| format!("{line}\n")).collect::<String>().into_bytes()].concat();
    std::fs::write(&transcript, &long).unwrap();
    allow_refused(&state, &seat, &episode, "allow-long-after", "stale-fence", "does not see it waiting").await;
    // Read again while it waits: the answer is recorded.
    std::fs::write(&transcript, &kept).unwrap();
    let before = state.store.index().unwrap();
    allow_prompt(&state, &seat, &episode, "allow-waiting").await.unwrap();
    assert!(state.store.index().unwrap() > before, "the answer was recorded");
}

/// Nathan answers prompts through one member for seats on every member. An allow that lands on
/// a member other than the seat's host reads the seat's conversation through the client-read
/// relay to that host, and is recorded where it lands once the named call waits there; a call
/// with its result is refused, and so is any allow when the host cannot be reached.
#[tokio::test]
async fn an_allow_for_a_remote_seat_checks_its_call_on_the_seats_host() {
    let owner_root = tempfile::tempdir().unwrap();
    let gateway_root = tempfile::tempdir().unwrap();
    let (owner, seat, episode, _) = prompted_codex_seat(owner_root.path(), "conversation-owner");
    let replicate = |gateway: &AppState| {
        gateway.store.import_replication("conversation-owner", &owner.store.export_replication(0).unwrap()).unwrap();
    };
    let mut gateway = tests::test_state_named(gateway_root.path(), "conversation-gateway");
    replicate(&gateway);
    let owner_socket = owner_root.path().join("owner-st3.sock");
    let served = owner_socket.clone();
    let owner_app = crate::api::router(owner.clone());
    tokio::spawn(async move { crate::api::serve_unix(&served, owner_app).await });
    for _ in 0..200 {
        if tokio::net::UnixStream::connect(&owner_socket).await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    gateway.client_relay =
        crate::peer::conversation_gateway_relay(&owner_socket, crate::api::raw_terminal_owner_splice, gateway_root.path())
            .await
            .unwrap();
    // A member that reaches no peer.
    let unreachable_root = tempfile::tempdir().unwrap();
    let unreachable = tests::test_state_named(unreachable_root.path(), "prompt-island");
    unreachable.store.import_replication("conversation-owner", &owner.store.export_replication(0).unwrap()).unwrap();

    // The named call has its result on the seat's host: refused.
    name_prompt_call(&owner, &seat, "call-done");
    replicate(&gateway);
    allow_refused(&gateway, &seat, &episode, "remote-answered", "stale-fence", "already has a result").await;
    // The named call waits on the seat's host, but this member cannot reach it: refused, saying so.
    name_prompt_call(&owner, &seat, "call-waiting");
    replicate(&gateway);
    unreachable.store.import_replication("conversation-owner", &owner.store.export_replication(0).unwrap()).unwrap();
    allow_refused(&unreachable, &seat, &episode, "remote-unreachable", "remote-unavailable", "").await;
    // It waits, and the host is reached: the answer is recorded here.
    let before = gateway.store.index().unwrap();
    allow_prompt(&gateway, &seat, &episode, "remote-waiting").await.unwrap();
    assert!(gateway.store.index().unwrap() > before, "the answer was recorded where it landed");
}
