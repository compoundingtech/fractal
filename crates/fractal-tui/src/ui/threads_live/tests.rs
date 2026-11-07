use super::*;

fn summary() -> ThreadSummary {
    serde_json::from_value(serde_json::json!({
        "id":"thread/worker", "agent_id":"agent/atlas", "title":"Worker test",
        "revision":"claim/current", "title_revision":"claim/title", "state":"open",
        "message_count":1,"unread":true
    }))
    .unwrap()
}

fn snapshot() -> Snapshot {
    serde_json::from_value(serde_json::json!({
        "id":"snapshot/thread", "host_id":"host/test", "store_index":7,
        "projection_version":"1","created_at":"2026-11-07T00:00:00Z"
    }))
    .unwrap()
}

#[test]
fn live_head_keeps_the_rows_between_it_and_a_loaded_older_page() {
    fn event(id: &str) -> ThreadTimelineEvent {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "actor_id": "agent/atlas",
            "accepted_at_unix_ms": 1_000,
            "kind": "message",
            "content": id,
        }))
        .unwrap()
    }

    let mut threads = Threads::default();
    let id = "thread/worker";
    threads.histories.insert(
        id.into(),
        History {
            head: vec![event("message/3"), event("message/2")],
            earlier: vec![event("message/1")],
            paged: true,
            ..History::default()
        },
    );
    threads.window(
        ThreadInterest::Timeline(id.into()),
        snapshot(),
        false,
        ThreadCollectionRows::Timeline(vec![event("message/4")]),
        vec!["message/2".into()],
        vec!["message/4".into(), "message/3".into()],
        true,
    );
    assert_eq!(
        threads.histories[id]
            .rows()
            .map(|event| event.id.as_str())
            .collect::<Vec<_>>(),
        ["message/4", "message/3", "message/2", "message/1"],
    );
}

#[test]
fn native_mounts_share_the_same_timeline_and_a_direct_reply() {
    let mut threads = Threads::default();
    let world = super::super::demo::test_world();
    let cursor = edit::Cursor::default();
    let mut texts = HashMap::new();
    let thread = "thread/worker";
    let agent = Pane::Agent(Some("agent/atlas".into()));
    let mission = Pane::Mission(Some("mission/example".into()));
    let inbox = Pane::Home(Some("attention/ask".into()));
    let tab = Pane::Thread(thread.into());
    let ask = ThreadInterest::Ask {
        id: "step-run/example/ask".into(),
        episode: "claim/ask".into(),
    };
    assert!(threads.interests(&agent).is_empty());

    threads.enable(true);
    threads.asks.insert(
        "attention/ask".into(),
        Ask {
            context: ask.clone(),
            agent: "agent/atlas".into(),
            title: "Worker test".into(),
            snapshot: snapshot().id,
        },
    );
    for interest in [
        ask,
        ThreadInterest::Agent("agent/atlas".into()),
        ThreadInterest::Mission("mission/example".into()),
    ] {
        threads.window(
            interest,
            snapshot(),
            true,
            ThreadCollectionRows::Summaries(vec![summary()]),
            vec![],
            vec![thread.into()],
            false,
        );
    }
    let message: ThreadTimelineEvent = serde_json::from_value(serde_json::json!({
        "id":"message/question","actor_id":"agent/atlas","accepted_at_unix_ms":1_000,
        "kind":"message","content":"Could you review the worker test?","thread_quote_id":null
    }))
    .unwrap();
    threads.window(
        ThreadInterest::Timeline(thread.into()),
        snapshot(),
        true,
        ThreadCollectionRows::Timeline(vec![message]),
        vec![],
        vec!["message/question".into()],
        false,
    );

    for pane in [&inbox, &agent, &mission] {
        let doc = threads.mount(pane, 100, &cursor, &world, &texts);
        assert!(doc.lines.iter().any(|line| {
            line.to_string()
                .contains("▸ Worker test · 1 message · unread")
        }));
        assert!(
            !doc.lines
                .iter()
                .any(|line| line.to_string().contains("Could you review"))
        );
        assert!(
            threads
                .tap(&pane.key(), thread::Tap::Toggle(thread.into()), &mut texts)
                .is_none()
        );
        assert!(
            threads
                .mount(pane, 100, &cursor, &world, &texts)
                .lines
                .iter()
                .any(|line| line.to_string().contains("Could you review"))
        );
    }
    assert!(
        threads
            .mount(&tab, 100, &cursor, &world, &texts)
            .lines
            .iter()
            .any(|line| line.to_string().contains("Could you review"))
    );
    assert!(
        threads
            .tap(
                &inbox.key(),
                thread::Tap::Reply {
                    thread: thread.into(),
                    message: "message/question".into()
                },
                &mut texts,
            )
            .is_none()
    );
    assert!(threads.key(
        &inbox,
        KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE),
        &cursor,
        &mut texts,
    ));
    assert!(threads.key(
        &inbox,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        &cursor,
        &mut texts,
    ));
    let write = threads.writes().pop().expect("one native reply");
    let Mutation::Reply(reply) = write.mutation else {
        panic!("expected thread reply")
    };
    assert_eq!(reply.target_id, thread);
    assert_eq!(reply.content, "y");
    assert_eq!(reply.thread_quote_id.as_deref(), Some("message/question"));
}
#[tokio::test]
async fn native_mount_sends_and_closes_through_the_real_daemon_socket() {
    use fractal_client::{CollectionEvent, CollectionStream};
    use fractal_daemon::SeatsStoreCore as _;
    use ratatui::{buffer::Buffer, layout::Rect};
    use std::sync::Arc;

    let root = tempfile::tempdir().unwrap();
    let socket = root.path().join("threads.sock");
    let person = "person/operator";
    let agent = "agent/garden/reviewer";
    let id = "thread/worker";
    let store = Arc::new(
        fractal_daemon::store::Store::open(&root.path().join("graph.db"), "native-thread-test")
            .unwrap(),
    );
    // This disposable member has no peers. Production enables the writer at daemon startup
    // only after the fleet reader rollout has been verified.
    store.configure_thread_write_rollout([]);
    let intent = fractal_daemon::graph::parse_intent(
        r#"version 2
agent "garden/reviewer" { workspace "/work/garden"; harness "omp" {} }
"#,
        store.origin(),
    )
    .unwrap();
    store.apply_internal(&intent, "declare-agent").unwrap();
    fractal_daemon::store::thread_metadata::apply(
        &store,
        person,
        id,
        "thread-fixture",
        fractal_daemon::store::thread_metadata::Edit::Create {
            agent,
            title: "Worker test",
        },
    )
    .unwrap();
    let state = fractal_daemon::api::AppState {
        seat_admission: Default::default(),
        owning_person: Some(person.into()),
        store,
        notify: Arc::new(tokio::sync::Notify::new()),
        event_notify: tokio::sync::watch::channel(0).0,
        node: "native-thread-test".into(),
        state_dir: root.path().into(),
        pty_root: root.path().join("pty"),
        pty_binary: root.path().join("unused-pty"),
        fleet_id: None,
        configured_peers: vec![],
        client_relay: None,
        native_session_home: None,
        harness_question_sink: None,
        assistant_receipts: Default::default(),
        private_notes: Default::default(),
        planner_default: Default::default(),
    };
    let path = socket.clone();
    let (ready, started) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        fractal_daemon::api::serve_unix_with_ready(
            &path,
            fractal_daemon::api::router(state),
            || {
                let _ = ready.send(());
            },
        )
        .await
        .unwrap();
    });
    tokio::time::timeout(std::time::Duration::from_secs(3), started)
        .await
        .unwrap()
        .unwrap();
    let client = Client::unix_as(&socket, person);
    let mut stream = client.collection_stream().await.unwrap();
    stream
        .subscribe_thread_timeline("native-history", id, 50)
        .await
        .unwrap();

    async fn apply_window(ui: &mut super::super::Ui, stream: &mut CollectionStream, id: &str) {
        let result = tokio::time::timeout(std::time::Duration::from_secs(5), stream.next_event())
            .await
            .unwrap()
            .unwrap();
        let Some(CollectionEvent::ThreadWindow {
            snapshot,
            replace,
            rows,
            removes,
            order,
            has_more,
            ..
        }) = result
        else {
            panic!("expected real thread frame, got {result:?}")
        };
        ui.threads.window(
            ThreadInterest::Timeline(id.into()),
            snapshot,
            replace,
            rows,
            removes,
            order,
            has_more,
        );
    }
    let mut world = super::super::demo::test_world();
    world.person = person.into();
    let mut ui = super::super::Ui::new(world);
    ui.threads.enable(true); // The disposable member keeps public capability advertisement off.
    ui.threads.detail(id, client.thread_get(id).await);
    apply_window(&mut ui, &mut stream, id).await;

    let pane = Pane::Thread(id.into());
    ui.open_in_glass(pane.clone(), super::super::glass::Open::Tab);
    let host = pane.key();
    ui.click(Hit::Thread(host.clone(), thread::Tap::Compose(id.into())));
    ui.paste("Please check the worker test.".into());
    ui.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let write = ui.threads.writes().pop().expect("native reply action");
    let receipt = write.perform(client.clone()).await;
    assert_eq!(
        ui.threads.complete(receipt, &mut ui.conversation_state.drafts),
        None,
    );
    apply_window(&mut ui, &mut stream, id).await;

    let area = Rect::new(0, 0, 100, 24);
    let mut screen = Buffer::empty(area);
    ui.draw_pane(&mut screen, area, &pane);
    let visible: String = screen.content.iter().map(|cell| cell.symbol()).collect();
    assert!(
        visible.contains("Please check the worker test."),
        "{visible}"
    );
    assert!(visible.contains("You"), "{visible}");

    ui.click(Hit::Thread(host, thread::Tap::Close(id.into())));
    let close = ui.threads.writes().pop().expect("native close action");
    let receipt = close.perform(client.clone()).await;
    assert_eq!(
        ui.threads.complete(receipt, &mut ui.conversation_state.drafts),
        None,
    );
    apply_window(&mut ui, &mut stream, id).await;
    assert_eq!(
        client.thread_get(id).await.unwrap().value.state,
        ThreadState::Closed
    );
    let mut closed = Buffer::empty(area);
    ui.draw_pane(&mut closed, area, &pane);
    let visible: String = closed.content.iter().map(|cell| cell.symbol()).collect();
    assert!(
        visible.contains("Please check the worker test."),
        "{visible}"
    );
    assert!(visible.contains("Reopen thread"), "{visible}");
    assert!(!visible.contains("Message thread"), "{visible}");
    server.abort();
}
