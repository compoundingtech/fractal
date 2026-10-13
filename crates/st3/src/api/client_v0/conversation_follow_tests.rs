use super::tests::test_state_named;
use super::*;
use std::io::Write as _;

#[test]
fn append_only_follow_keeps_native_deltas_through_harness_observations() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let mut measurements = Vec::new();
    let spans = crate::otel::capture_test_spans(|| {
        runtime.block_on(async {
            for observed in [false, true] {
                let (frames, replaces, bytes) = measure_append_only_follow(observed).await;
                measurements.push((observed, frames, replaces, bytes));
            }
        });
    });
    let mut reasons = BTreeMap::new();
    for span in &spans {
        for field in &span.attributes {
            if matches!(
                field.key.as_str(),
                "st.conversation.gap.reason" | "st.conversation.reopen.reason"
            ) {
                *reasons
                    .entry(field.value.as_str().into_owned())
                    .or_insert(0) += 1;
            }
        }
    }
    println!("follow attribution: {reasons:?}");
    for (observed, frames, replaces, bytes) in measurements {
        assert_eq!(
            replaces, 1,
            "only OPEN replaces the native transcript; observed={observed}"
        );
        assert_eq!(
            frames, 77,
            "each turn sends one delta; unchanged transcript sends nothing"
        );
        assert!(bytes < 200_000, "76 turns must stay within the delta byte budget: {bytes}");
    }
    assert!(
        !spans
            .iter()
            .any(|span| span.name == "st.conversation.reopen"),
        "append-only follow must never re-OPEN"
    );
}

async fn measure_append_only_follow(observed: bool) -> (usize, usize, usize) {
    let root = tempfile::tempdir().unwrap();
    let state = test_state_named(
        root.path(),
        if observed {
            "follow-observed"
        } else {
            "follow-native"
        },
    );
    let owner = "agent/follow-worker";
    let incarnation = "follow-runtime:i1";
    let path = root.path().join("session.jsonl");
    std::fs::write(
        &path,
        format!(
            "{}\n",
            json!({"type":"session", "id":"follow-native",
        "timestamp":"2026-10-01T12:00:00Z", "cwd":"/work/example"})
        ),
    )
    .unwrap();
    for (kind, fields) in [
        (
            "runtime.observed",
            json!({"status":"running", "runtime_id":"follow-runtime", "incarnation_id":incarnation}),
        ),
        (
            "harness.observed",
            json!({"state":"idle", "driver":"omp", "incarnation_id":incarnation}),
        ),
        (
            "harness.session-file",
            json!({"harness":"omp", "agent":owner, "incarnation_id":incarnation,
            "session_id":"follow-native", "path":path}),
        ),
    ] {
        state
            .store
            .append_claim(&ClaimInput {
                subject: owner.into(),
                kind: kind.into(),
                actor: Some(owner.into()),
                fields: serde_json::from_value(fields).unwrap(),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
    }
    let session_id = managed_session_id(owner, incarnation);
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    let follower = tokio::spawn(follow_conversation(
        state.clone(),
        ClientSession::local(Some("person/example")).unwrap(),
        "chat".into(),
        1,
        (session_id.clone(), None),
        sender,
        Queue::new(Kind::Conversation),
        (None, None),
    ));
    let mut frames = Vec::new();
    frames.push(
        tokio::time::timeout(Duration::from_secs(5), receiver.recv())
            .await
            .unwrap()
            .unwrap()
            .2,
    );
    let observe = |turn: usize, phase: &str| {
        state
            .store
            .append_local_observations_for_test(&[ClaimInput {
                subject: owner.into(),
                kind: "harness.timeline".into(),
                actor: Some(owner.into()),
                fields: serde_json::from_value(json!({"operation":"append",
                "entry_id":format!("timeline-entry/driver-{turn}-{phase}"), "sequence":turn * 2 + 1,
                "revision":1, "role":"assistant", "entry_type":"content", "final":true,
                "body":{"media_type":"text/plain", "text":"driver observation"},
                "driver":"omp", "incarnation_id":incarnation}))
                .unwrap(),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: None,
            }]);
    };
    for turn in 0..76 {
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        for role in ["user", "assistant"] {
            writeln!(
                file,
                "{}",
                json!({"type":"message", "id":format!("{role}-{turn}"),
                "timestamp":"2026-10-01T12:00:01Z", "message":{"role":role,
                "content":[{"type":"text", "text":format!("turn {turn} {}", "x".repeat(512))}]}})
            )
            .unwrap();
        }
        drop(file);
        if observed {
            observe(turn, "append");
        }
        signal_changed(&state);
        // Count every frame, including any late page from the observation-only wake.
        // Advance only when this turn arrives; the final frame budget rejects extras.
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let frame = receiver.recv().await.unwrap().2;
                let items = frame["items"].as_array().unwrap();
                let delivered = ["user", "assistant"].iter().all(|role| {
                    items.iter().any(|item| {
                        item["role"] == *role
                            && item["body"]["text"]
                                .as_str()
                                .is_some_and(|text| text.starts_with(&format!("turn {turn} ")))
                    })
                });
                if delivered && frame["replace"] == false {
                    assert_eq!(
                        items.len(),
                        4,
                        "a delta carries exactly the two appended records"
                    );
                }
                frames.push(frame);
                if delivered {
                    break;
                }
            }
        })
        .await
        .expect("both sides of the appended turn must arrive");
        // A driver observation with no native change must not resend the delivered page.
        if observed {
            observe(turn, "quiet");
            signal_changed(&state);
            if let Ok(Some((_, _, frame))) =
                tokio::time::timeout(Duration::from_millis(50), receiver.recv()).await
            {
                frames.push(frame);
            }
        }
    }
    follower.abort();
    let _ = follower.await;
    let replaces = frames
        .iter()
        .filter(|frame| frame["replace"] == true)
        .count();
    let bytes: usize = frames.iter().map(frame_bytes).sum();
    println!(
        "76 append-only turns, observed={observed}: frames={}, replace={replaces}, bytes={bytes}",
        frames.len()
    );
    (frames.len(), replaces, bytes)
}

#[tokio::test]
async fn stored_follow_still_replays_harness_entries() {
    let root = tempfile::tempdir().unwrap();
    let state = test_state_named(root.path(), "follow-stored");
    let owner = "agent/stored-worker";
    let incarnation = "stored-runtime:i1";
    state
        .store
        .append_claim(&ClaimInput {
            subject: owner.into(),
            kind: "runtime.observed".into(),
            actor: Some(owner.into()),
            fields: serde_json::from_value(
                json!({"status":"running", "runtime_id":"stored-runtime",
            "incarnation_id":incarnation}),
            )
            .unwrap(),
            evidence: vec![],
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    let follower = tokio::spawn(follow_conversation(
        state.clone(),
        ClientSession::local(Some("person/example")).unwrap(),
        "chat".into(),
        1,
        (managed_session_id(owner, incarnation), None),
        sender,
        Queue::new(Kind::Conversation),
        (None, None),
    ));
    let opened = tokio::time::timeout(Duration::from_secs(5), receiver.recv())
        .await
        .unwrap()
        .unwrap()
        .2;
    assert_eq!(opened["replace"], true);
    state
        .store
        .append_local_observations_for_test(&[ClaimInput {
            subject: owner.into(),
            kind: "harness.timeline".into(),
            actor: Some(owner.into()),
            fields: serde_json::from_value(json!({"operation":"append",
            "entry_id":"timeline-entry/stored-content", "sequence":1,
            "revision":1, "role":"assistant", "entry_type":"content", "final":true,
            "body":{"media_type":"text/plain", "text":"stored content"},
            "driver":"codex", "incarnation_id":incarnation}))
            .unwrap(),
            evidence: vec![],
            expected_subject: None,
            idempotency_key: None,
        }]);
    signal_changed(&state);
    let delta = tokio::time::timeout(Duration::from_secs(5), receiver.recv())
        .await
        .unwrap()
        .unwrap()
        .2;
    follower.abort();
    let _ = follower.await;
    assert_eq!(delta["replace"], false);
    assert_eq!(delta["items"].as_array().unwrap().len(), 1);
    assert_eq!(delta["items"][0]["id"], "timeline-entry/stored-content");
    assert_eq!(delta["items"][0]["body"]["text"], "stored content");
}
