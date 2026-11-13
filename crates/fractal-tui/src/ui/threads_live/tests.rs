fn enable_editing(threads: &mut Threads) {
    threads.enable(true);
    threads.set_access(ThreadAccess {
        start: true,
        reply: true,
        rename: true,
        close: true,
        reopen: true,
        mark_read: false,
    });
}

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
    let ask = ThreadInterest::Inbox {
        source: "step-run/example/ask".into(),
        episode: "claim/ask".into(),
    };
    assert!(threads.interests(&agent).is_empty());

    enable_editing(&mut threads);
    threads.origins.insert(
        "attention/ask".into(),
        Origin {
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
                .interests(pane)
                .contains(&ThreadInterest::Summary(thread.into()))
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
        browser_pairing_login: None,
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
    enable_editing(&mut ui.threads); // The disposable member keeps public capability advertisement off.
    ui.threads.put_summary(client.thread_get(id).await.unwrap());
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
        ui.threads
            .complete(receipt, &mut ui.conversation_state.drafts),
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

    ui.click(Hit::Thread(host.clone(), thread::Tap::Close(id.into())));
    let mut close = ui.threads.writes().pop().expect("native close action");
    ui.threads.bind(&mut close, client.clone()).unwrap();
    let receipt = close.clone().perform(client.clone()).await;
    assert!(
        matches!(receipt.confirmation, Confirmation::Applied(_)),
        "real Close committed"
    );
    // Lose only the reply after the real mutation, then deliver the live summary first.
    ui.threads.complete(
        Receipt {
            write: close.clone(),
            confirmation: Confirmation::Failed(ClientError::Transport(
                "lost Close acknowledgement".into(),
            )),
        },
        &mut ui.conversation_state.drafts,
    );
    ui.threads.put_summary(client.thread_get(id).await.unwrap());
    ui.click(Hit::Thread(
        host.clone(),
        thread::Tap::Retry(close.key.clone()),
    ));
    let retry = ui.threads.writes().pop().expect("original Close retry");
    assert_eq!(retry.key, close.key);
    assert_eq!(retry.fence, close.fence);
    assert_eq!(
        ui.threads.complete(
            retry.perform(client.clone()).await,
            &mut ui.conversation_state.drafts
        ),
        None
    );
    let timeline = client
        .thread_timeline(id, None, Some(50))
        .await
        .unwrap()
        .value;
    assert_eq!(
        timeline
            .items
            .iter()
            .filter(|event| matches!(event.change, ThreadTimelineChange::Closed))
            .count(),
        1,
        "retry must not create another lifecycle event"
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
    assert!(
        matches!(ui.threads.focused.as_ref(), Some((where_, Input::Reply(thread))) if where_ == &host && thread == id),
        "closing must preserve the active ordinary reply composer"
    );
    assert!(visible.contains("Send"), "{visible}");
    let quoted = client
        .thread_timeline(id, None, Some(50))
        .await
        .unwrap()
        .value
        .items
        .into_iter()
        .find(|event| matches!(event.change, ThreadTimelineChange::Message { .. }))
        .unwrap()
        .id;
    ui.click(Hit::Thread(
        host,
        thread::Tap::Reply {
            thread: id.into(),
            message: quoted.clone(),
        },
    ));
    ui.paste("A person can follow up while closed.".into());
    ui.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let reply = ui
        .threads
        .writes()
        .pop()
        .expect("closed person reply from native composer");
    let outcome = ui.threads.complete(
        reply.perform(client.clone()).await,
        &mut ui.conversation_state.drafts,
    );
    assert_eq!(
        outcome, None,
        "the real person route must allow replies without Reopen"
    );
    assert_eq!(
        client.thread_get(id).await.unwrap().value.state,
        ThreadState::Closed
    );
    let timeline = client
        .thread_timeline(id, None, Some(50))
        .await
        .unwrap()
        .value;
    assert!(timeline.items.iter().any(|event| matches!(&event.change,ThreadTimelineChange::Message { content,thread_quote_id } if content == "A person can follow up while closed." && thread_quote_id.as_ref() == Some(&quoted))));
    assert!(
        !timeline
            .items
            .iter()
            .any(|event| matches!(event.change, ThreadTimelineChange::Reopened))
    );
    server.abort();
}

#[test]
fn inbox_start_keeps_the_accepted_thread_in_the_draft_slot() {
    let mut threads = Threads::default();
    enable_editing(&mut threads);
    let world = super::super::demo::test_world();
    let cursor = edit::Cursor::default();
    let mut texts = HashMap::new();
    let inbox = Pane::Home(Some("attention/ask".into()));
    let context = ThreadInterest::Inbox {
        source: "step-run/example/ask".into(),
        episode: "claim/ask".into(),
    };
    let older = summary();
    threads.origins.insert(
        "attention/ask".into(),
        Origin {
            context: context.clone(),
            agent: older.agent_id.clone(),
            title: older.title.clone(),
            snapshot: snapshot().id,
        },
    );
    threads.window(
        context.clone(),
        snapshot(),
        true,
        ThreadCollectionRows::Summaries(vec![older.clone()]),
        vec![],
        vec![older.id.clone()],
        false,
    );
    threads.tap(
        &inbox.key(),
        thread::Tap::Start("attention/ask".into()),
        &mut texts,
    );
    assert!(threads.paste(&inbox, "First message", &cursor, &mut texts));
    let lines = |threads: &Threads, texts: &HashMap<String, String>| {
        threads
            .mount(&inbox, 100, &cursor, &world, texts)
            .lines
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
    };
    let draft = lines(&threads, &texts);
    let slot = draft
        .iter()
        .position(|line| line == "▾ Worker test · Draft · to agent/atlas")
        .unwrap();
    assert!(
        draft[slot + 1..]
            .iter()
            .any(|line| line.starts_with("▸ Worker test"))
    );
    assert!(threads.key(
        &inbox,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        &cursor,
        &mut texts
    ));
    let write = threads.writes().pop().unwrap();
    assert_ne!(write.id, older.id);
    let queued = lines(&threads, &texts);
    let action_row = draft
        .iter()
        .position(|line| line.contains("Send first message"))
        .unwrap();
    let mut pending_layout = draft.clone();
    pending_layout[action_row] = "Send first message…".into();
    assert_eq!(
        queued, pending_layout,
        "only the existing action row changes while sending; draft identity, composer and sibling positions stay fixed"
    );
    let pending_doc = threads.mount(&inbox, 100, &cursor, &world, &texts);
    assert!(
        !pending_doc
            .targets
            .iter()
            .any(|target| matches!(&target.hit, Hit::Thread(_, thread::Tap::Start(_)))),
        "the pending action must not retain an active Send target"
    );
    let new = ThreadSummary {
        id: write.id.clone(),
        ..older.clone()
    };
    threads.window(
        context.clone(),
        snapshot(),
        true,
        ThreadCollectionRows::Summaries(vec![older.clone(), new.clone()]),
        vec![],
        vec![older.id.clone(), new.id.clone()],
        false,
    );
    assert_eq!(lines(&threads, &texts), queued);
    threads.window(
        context.clone(),
        snapshot(),
        true,
        ThreadCollectionRows::Summaries(vec![older.clone()]),
        vec![],
        vec![older.id.clone()],
        false,
    );
    threads.complete(
        Receipt {
            write,
            confirmation: Confirmation::Applied(Some(Envelope {
                value: new.clone(),
                snapshot: snapshot(),
                request_id: String::new(),
                api_version: fractal_client::API_VERSION.into(),
            })),
        },
        &mut texts,
    );
    threads.histories.insert(new.id.clone(), History::default());
    let accepted = lines(&threads, &texts);
    assert_eq!(accepted[slot], "▾ Worker test · 1 message · unread");
    assert!(
        accepted[slot + 1..]
            .iter()
            .any(|line| line.starts_with("▸ Worker test"))
    );

    threads.window(
        context.clone(),
        snapshot(),
        true,
        ThreadCollectionRows::Summaries(vec![older.clone(), new.clone()]),
        vec![],
        vec![older.id, new.id],
        false,
    );
    assert_eq!(lines(&threads, &texts), accepted);
    threads.failure(context, "Ask access expired".into(), true);
    let refused = lines(&threads, &texts);
    assert!(refused.iter().any(|line| line == "Ask access expired"));
    assert!(!refused.iter().any(|line| line.contains("Worker test")));
}

#[test]
fn mark_visible_read_fences_the_observed_incoming_messages() {
    let mut threads = Threads::default();
    enable_editing(&mut threads);
    let id = "thread/worker";
    let observed = snapshot();
    threads.put_summary(Envelope {
        api_version: fractal_client::API_VERSION.into(),
        request_id: "request/test".into(),
        snapshot: observed.clone(),
        value: summary(),
    });
    let message = |id: &str, actor: &str| {
        serde_json::from_value::<ThreadTimelineEvent>(serde_json::json!({
            "id": id, "actor_id": actor, "accepted_at_unix_ms": 1_000,
            "kind": "message", "content": id, "thread_quote_id": null
        }))
        .unwrap()
    };
    threads.histories.insert(
        id.into(),
        History {
            head: vec![
                message("message/offscreen", "agent/atlas"),
                message("message/own", "person/operator"),
                message("message/shown", "agent/atlas"),
            ],
            earlier: vec![message("message/older", "agent/atlas")],
            ..History::default()
        },
    );
    let visible = [
        "message/own",
        "message/shown",
        "message/older",
        "message/other-thread",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    assert!(!threads.mark_visible_read("home:ask", id.into(), &visible));
    threads.set_access(ThreadAccess {
        mark_read: true,
        ..threads.access
    });
    assert!(!threads.mark_visible_read("home:ask", id.into(), &HashSet::new()));
    assert!(threads.mark_visible_read("home:ask", id.into(), &visible));
    assert!(!threads.mark_visible_read("home:ask", id.into(), &visible));
    let [write]: [Write; 1] = threads
        .writes()
        .try_into()
        .ok()
        .expect("one explicit write");
    let Mutation::MarkRead(parameters) = write.mutation else {
        panic!("expected thread.read")
    };
    assert_eq!(parameters.target_id, id);
    assert_eq!(
        parameters.through_message_id.as_deref(),
        Some("message/shown")
    );
    assert_eq!(
        parameters.seen_message_ids,
        ["message/shown", "message/older"]
    );
    assert_eq!(write.fence.snapshot_id, observed.id);
    assert!(write.fence.subject_revisions.is_empty());
}

fn observed_threads(state: ThreadState) -> Threads {
    let mut threads = Threads::default();
    enable_editing(&mut threads);
    threads.put_summary(Envelope {
        value: ThreadSummary { state, ..summary() },
        snapshot: snapshot(),
        request_id: "request/test".into(),
        api_version: fractal_client::API_VERSION.into(),
    });
    threads
        .histories
        .insert("thread/worker".into(), History::default());
    threads
}

fn forbidden() -> ClientError {
    let envelope = serde_json::from_value(serde_json::json!({
        "api_version":fractal_client::API_VERSION,"error_version":"st3.client.error.v0",
        "request_id":"request/test","code":"forbidden","message":"Grant was revoked",
        "retryable":false,"details":{}
    }))
    .unwrap();
    ClientError::Api(
        fractal_client::ErrorCode::Forbidden,
        "Grant was revoked".into(),
        Box::new(envelope),
    )
}

#[test]
fn original_close_retry_survives_changed_summary_collapse_and_later_refusal() {
    let mut threads = observed_threads(ThreadState::Open);
    let mut texts = HashMap::new();
    let id = "thread/worker";
    let pane = Pane::Thread(id.into());
    threads.lifecycle(&pane.key(), id.into(), Mutation::Close);
    let original = threads.writes().pop().unwrap();
    assert!(
        threads
            .complete(
                Receipt {
                    write: original.clone(),
                    confirmation: Confirmation::Failed(ClientError::Transport(
                        "lost acknowledgement".into()
                    ))
                },
                &mut texts
            )
            .is_some()
    );
    threads.put_summary(Envelope {
        value: ThreadSummary {
            state: ThreadState::Closed,
            revision: "claim/new".into(),
            ..summary()
        },
        snapshot: snapshot(),
        request_id: "request/new".into(),
        api_version: fractal_client::API_VERSION.into(),
    });
    threads.lifecycle(&pane.key(), id.into(), Mutation::Reopen);
    assert!(
        threads.writes().is_empty(),
        "a new action cannot replace an unconfirmed Close"
    );
    threads.tap(&pane.key(), thread::Tap::Toggle(id.into()), &mut texts);
    threads.tap(&pane.key(), thread::Tap::Toggle(id.into()), &mut texts);
    let doc = threads.mount(
        &pane,
        100,
        &edit::Cursor::default(),
        &super::super::demo::test_world(),
        &texts,
    );
    assert!(
        doc.lines
            .iter()
            .any(|line| line.to_string().contains("Retry Close thread"))
    );
    assert!(doc.targets.iter().any(|target| matches!(&target.hit, Hit::Thread(_, thread::Tap::Retry(key)) if key == &original.key)));
    threads.tap(
        &pane.key(),
        thread::Tap::Retry(original.key.clone()),
        &mut texts,
    );
    let retried = threads.writes().pop().unwrap();
    assert_eq!(retried.key, original.key);
    assert_eq!(retried.fence, original.fence);
    assert!(matches!(retried.mutation, Mutation::Close));
    threads.complete(
        Receipt {
            write: retried,
            confirmation: Confirmation::Failed(forbidden()),
        },
        &mut texts,
    );
    assert!(
        threads.pending_actions[id].unknown,
        "a later403 cannot answer the lost original request"
    );
    threads.enable(false);
    enable_editing(&mut threads);
    assert!(
        threads.pending_actions[id].unknown,
        "reconnection must not discard the original action"
    );
    threads.tap(
        &pane.key(),
        thread::Tap::Retry(original.key.clone()),
        &mut texts,
    );
    let retried = threads.writes().pop().unwrap();
    assert_eq!(retried.key, original.key);
    threads.complete(
        Receipt {
            write: retried,
            confirmation: Confirmation::Applied(None),
        },
        &mut texts,
    );
    assert!(!threads.pending_actions.contains_key(id));
}

#[test]
fn a_pending_reply_without_loaded_history_keeps_fallback_status_and_retry() {
    let mut threads = observed_threads(ThreadState::Open);
    let id = "thread/worker";
    let tab = Pane::Thread(id.into());
    let agent = Pane::Agent(Some("agent/atlas".into()));
    let mut texts = HashMap::new();
    threads.tap(&tab.key(), thread::Tap::Compose(id.into()), &mut texts);
    threads.paste(
        &tab,
        "Retain this reply",
        &edit::Cursor::default(),
        &mut texts,
    );
    threads.submit(&tab.key(), Input::Reply(id.into()), &texts);
    let original = threads.writes().pop().unwrap();
    threads.histories.remove(id);
    threads
        .open
        .entry(agent.key())
        .or_default()
        .insert(id.into());
    threads
        .lists
        .insert(ThreadInterest::Agent("agent/atlas".into()), vec![id.into()]);
    let mount = |threads: &Threads, pane: &Pane, texts: &HashMap<String, String>| {
        threads.mount(
            pane,
            100,
            &edit::Cursor::default(),
            &super::super::demo::test_world(),
            texts,
        )
    };
    for pane in [&tab, &agent] {
        let doc = mount(&threads, pane, &texts);
        let lines = doc
            .lines
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        assert!(lines.iter().any(|line| line.contains("Loading thread")));
        assert_eq!(
            lines.iter().filter(|line| *line == "Send reply…").count(),
            1,
            "a missing editor must not hide its retained request"
        );
        assert!(!doc.targets.iter().any(|target| matches!(
            &target.hit,
            Hit::Thread(_, thread::Tap::Send(_) | thread::Tap::Compose(_))
        )));
    }
    threads.complete(
        Receipt {
            write: original.clone(),
            confirmation: Confirmation::Failed(ClientError::Transport("lost receipt".into())),
        },
        &mut texts,
    );
    for pane in [&tab, &agent] {
        let doc = mount(&threads, pane, &texts);
        assert!(doc.targets.iter().any(|target| matches!(&target.hit, Hit::Thread(_, thread::Tap::Retry(key)) if key == &original.key)));
    }
}

#[test]
fn an_unfocused_pending_reply_uses_the_existing_divider_without_growing_the_panel() {
    let mut threads = observed_threads(ThreadState::Closed);
    let id = "thread/worker";
    let pane = Pane::Thread(id.into());
    let mut texts = HashMap::new();
    threads.tap(
        &pane.key(),
        thread::Tap::Reply {
            thread: id.into(),
            message: "message/question".into(),
        },
        &mut texts,
    );
    threads.paste(
        &pane,
        "Original quoted draft",
        &edit::Cursor::default(),
        &mut texts,
    );
    threads.tap(
        &pane.key(),
        thread::Tap::LeaveComposer(id.into()),
        &mut texts,
    );
    let mount = |threads: &Threads, texts: &HashMap<String, String>| {
        threads.mount(
            &pane,
            100,
            &edit::Cursor::default(),
            &super::super::demo::test_world(),
            texts,
        )
    };
    let lines = |doc: &Doc| {
        doc.lines
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
    };
    let before = lines(&mount(&threads, &texts));
    let divider = before
        .iter()
        .position(|line| line == &"─".repeat(100))
        .unwrap();
    threads.tap(&pane.key(), thread::Tap::Compose(id.into()), &mut texts);
    threads.submit(&pane.key(), Input::Reply(id.into()), &texts);
    let original = threads.writes().pop().unwrap();
    threads.tap(
        &pane.key(),
        thread::Tap::LeaveComposer(id.into()),
        &mut texts,
    );
    let queued = mount(&threads, &texts);
    let mut expected = before;
    let status = lines(&queued)[divider].clone();
    assert!(status.starts_with("─ Send reply… "));
    assert_eq!(text::width(&status), 100);
    expected[divider] = status;
    assert_eq!(
        lines(&queued),
        expected,
        "the existing quote, draft and actions keep their rows without focus"
    );
    assert!(!queued.targets.iter().any(|target| matches!(
        &target.hit,
        Hit::Thread(_, thread::Tap::Send(_) | thread::Tap::Compose(_))
    )));
    assert_eq!(
        threads.drafts[&Input::Reply(id.into()).key()]
            .delivery
            .as_ref()
            .unwrap()
            .write
            .key,
        original.key
    );
}

#[test]
fn a_pending_reply_uses_the_existing_action_row_without_a_second_send_target() {
    let mut threads = observed_threads(ThreadState::Closed);
    let pane = Pane::Thread("thread/worker".into());
    let mut texts = HashMap::new();
    threads.tap(
        &pane.key(),
        thread::Tap::Reply {
            thread: "thread/worker".into(),
            message: "message/question".into(),
        },
        &mut texts,
    );
    threads.paste(
        &pane,
        "Reply without reopening",
        &edit::Cursor::default(),
        &mut texts,
    );
    let mount = |threads: &Threads, texts: &HashMap<String, String>| {
        threads.mount(
            &pane,
            100,
            &edit::Cursor::default(),
            &super::super::demo::test_world(),
            texts,
        )
    };
    let lines = |doc: &Doc| {
        doc.lines
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
    };
    let before = lines(&mount(&threads, &texts));
    let action_row = before
        .iter()
        .position(|line| line.contains("Send"))
        .unwrap();
    threads.submit(&pane.key(), Input::Reply("thread/worker".into()), &texts);
    let original = threads.writes().pop().unwrap();
    let queued = mount(&threads, &texts);
    let mut expected = before;
    expected[action_row] = "Send reply…".into();
    assert_eq!(
        lines(&queued),
        expected,
        "quote, draft and lifecycle geometry stay fixed; only the action row changes"
    );
    assert!(!queued.targets.iter().any(|target| matches!(
        &target.hit,
        Hit::Thread(_, thread::Tap::Send(_) | thread::Tap::Compose(_))
    )));
    threads.submit(&pane.key(), Input::Reply("thread/worker".into()), &texts);
    assert!(
        threads.writes().is_empty(),
        "a pending request cannot queue another Send"
    );
    threads.put_summary(Envelope {
        value: ThreadSummary {
            state: ThreadState::Closed,
            revision: "claim/feed".into(),
            ..summary()
        },
        snapshot: snapshot(),
        request_id: "request/feed".into(),
        api_version: fractal_client::API_VERSION.into(),
    });
    assert_eq!(lines(&mount(&threads, &texts)), expected);
    assert_eq!(
        threads.drafts[&Input::Reply("thread/worker".into()).key()]
            .delivery
            .as_ref()
            .unwrap()
            .write
            .key,
        original.key
    );
}

#[test]
fn a_closed_summary_keeps_the_active_reply_focus_and_quote() {
    let mut threads = observed_threads(ThreadState::Open);
    let id = "thread/worker";
    let pane = Pane::Thread(id.into());
    let mut texts = HashMap::new();
    threads.tap(
        &pane.key(),
        thread::Tap::Reply {
            thread: id.into(),
            message: "message/question".into(),
        },
        &mut texts,
    );
    threads.paste(
        &pane,
        "Continue this discussion",
        &edit::Cursor::default(),
        &mut texts,
    );
    let input = Input::Reply(id.into());
    threads.put_summary(Envelope {
        value: ThreadSummary {
            state: ThreadState::Closed,
            revision: "claim/closed".into(),
            ..summary()
        },
        snapshot: snapshot(),
        request_id: "request/closed".into(),
        api_version: fractal_client::API_VERSION.into(),
    });
    assert!(
        threads.focused == Some((pane.key(), input.clone())),
        "a closed summary must preserve the exact focused pane and reply input"
    );
    assert_eq!(
        threads.drafts[&input.key()].quote.as_deref(),
        Some("message/question")
    );
    threads.submit(&pane.key(), input, &texts);
    let write = threads.writes().pop().unwrap();
    let Mutation::Reply(parameters) = write.mutation else {
        panic!("reply")
    };
    assert_eq!(parameters.content, "Continue this discussion");
    assert_eq!(
        parameters.thread_quote_id.as_deref(),
        Some("message/question")
    );
    assert_eq!(threads.summary(id).unwrap().state, ThreadState::Closed);
}

#[test]
fn a_confirmed_reply_does_not_consume_a_concurrent_unknown_close() {
    let mut threads = observed_threads(ThreadState::Open);
    let mut texts = HashMap::new();
    let id = "thread/worker";
    let pane = Pane::Thread(id.into());
    threads.lifecycle(&pane.key(), id.into(), Mutation::Close);
    let close = threads.writes().pop().unwrap();
    threads.complete(
        Receipt {
            write: close.clone(),
            confirmation: Confirmation::Failed(ClientError::Transport("lost receipt".into())),
        },
        &mut texts,
    );
    threads.tap(&pane.key(), thread::Tap::Compose(id.into()), &mut texts);
    threads.paste(
        &pane,
        "Reply while Close settles",
        &edit::Cursor::default(),
        &mut texts,
    );
    threads.submit(&pane.key(), Input::Reply(id.into()), &texts);
    let reply = threads.writes().pop().unwrap();
    assert!(matches!(reply.mutation, Mutation::Reply(_)));
    threads.complete(
        Receipt {
            write: reply,
            confirmation: Confirmation::Applied(None),
        },
        &mut texts,
    );
    assert!(threads.pending_actions[id].unknown);
    threads.retry(&close.key);
    let retried = threads.writes().pop().unwrap();
    assert_eq!(retried.key, close.key);
    assert_eq!(retried.fence, close.fence);
    assert!(matches!(retried.mutation, Mutation::Close));
}

#[test]
fn read_retry_keeps_the_original_shown_ids_even_after_the_view_moves() {
    let mut threads = observed_threads(ThreadState::Open);
    threads.set_access(ThreadAccess {
        mark_read: true,
        ..threads.access
    });
    let id = "thread/worker";
    let event = |id: &str| {
        serde_json::from_value::<ThreadTimelineEvent>(serde_json::json!({
            "kind":"message","id":id,"actor_id":"agent/atlas","accepted_at_unix_ms":1,
            "content":"Reply","thread_quote_id":null
        }))
        .unwrap()
    };
    threads.histories.get_mut(id).unwrap().head = vec![event("message/first")];
    let visible = ["message/first".into()].into();
    assert!(threads.mark_visible_read("thread:test", id.into(), &visible));
    let original = threads.writes().pop().unwrap();
    threads.complete(
        Receipt {
            write: original.clone(),
            confirmation: Confirmation::Failed(ClientError::Transport("lost".into())),
        },
        &mut HashMap::new(),
    );
    threads
        .histories
        .get_mut(id)
        .unwrap()
        .head
        .insert(0, event("message/later"));
    assert!(!threads.mark_visible_read("thread:test", id.into(), &["message/later".into()].into()));
    threads.retry(&original.key);
    let retry = threads.writes().pop().unwrap();
    let Mutation::MarkRead(request) = retry.mutation else {
        panic!("read must remain read")
    };
    assert_eq!(request.seen_message_ids, ["message/first"]);
    assert_eq!(request.through_message_id.as_deref(), Some("message/first"));
    assert_eq!(retry.fence, original.fence);
}

#[test]
fn metadata_receipts_require_an_immutable_claim_id_not_an_observed_cas_revision() {
    let mut threads = observed_threads(ThreadState::Open);
    let id = "thread/worker";
    threads.lifecycle(&Pane::Thread(id.into()).key(), id.into(), Mutation::Close);
    let write = threads.writes().pop().unwrap();
    let result = |metadata| {
        let mut value = serde_json::json!({
            "kind": "action-result", "action_id": format!("action/{}", write.key),
            "operation_id": "operation/close", "status": "completed",
            "affected_ids": [id], "snapshot_id": "snapshot/receipt",
        });
        value.as_object_mut().unwrap().extend(metadata);
        serde_json::from_value::<fractal_client::ActionResult>(value).unwrap()
    };
    assert!(!write.matches_receipt(&result(serde_json::Map::from_iter([(
        "metadata_revision".into(),
        serde_json::json!("old-shape")
    ),]))));
    assert!(!write.matches_receipt(&result(serde_json::Map::from_iter([(
        "metadata_claim_id".into(),
        serde_json::json!("")
    ),]))));
    assert!(write.matches_receipt(&result(serde_json::Map::from_iter([(
        "metadata_claim_id".into(),
        serde_json::json!("claim/closed")
    ),]))));
}

#[test]
fn closed_person_composer_keeps_original_quote_and_payload_until_settled() {
    let mut threads = observed_threads(ThreadState::Closed);
    let id = "thread/worker";
    let pane = Pane::Thread(id.into());
    let mut texts = HashMap::new();
    let cursor = edit::Cursor::default();
    threads.tap(
        &pane.key(),
        thread::Tap::Reply {
            thread: id.into(),
            message: "message/quoted".into(),
        },
        &mut texts,
    );
    assert!(threads.paste(&pane, "Closed reply", &cursor, &mut texts));
    assert!(threads.key(
        &pane,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        &cursor,
        &mut texts
    ));
    let original = threads
        .writes()
        .pop()
        .expect("a closed thread accepts an ordinary person reply");
    threads.complete(
        Receipt {
            write: original.clone(),
            confirmation: Confirmation::Failed(ClientError::Protocol(
                "truncated acknowledgement".into(),
            )),
        },
        &mut texts,
    );
    threads.tap(
        &pane.key(),
        thread::Tap::Reply {
            thread: id.into(),
            message: "message/different".into(),
        },
        &mut texts,
    );
    threads.paste(&pane, "changed", &cursor, &mut texts);
    threads.submit(&pane.key(), Input::Reply(id.into()), &texts);
    let retry = threads.writes().pop().unwrap();
    assert_eq!(retry.key, original.key);
    let Mutation::Reply(reply) = retry.mutation else {
        panic!("reply")
    };
    assert_eq!(reply.content, "Closed reply");
    assert_eq!(reply.thread_quote_id.as_deref(), Some("message/quoted"));
    assert_eq!(threads.summary(id).unwrap().state, ThreadState::Closed);
    threads.complete(
        Receipt {
            write: original.clone(),
            confirmation: Confirmation::Failed(forbidden()),
        },
        &mut texts,
    );
    assert!(threads.drafts[&Input::Reply(id.into()).key()].locked());
    // An authoritative rejection settles the old intent; the draft remains editable.
    threads.complete(
        Receipt {
            write: original,
            confirmation: Confirmation::Rejected,
        },
        &mut texts,
    );
    assert!(!threads.drafts[&Input::Reply(id.into()).key()].locked());
}

#[test]
fn first_definitive_refusal_releases_the_intent_and_dispatch_stays_on_its_original_client() {
    let mut threads = observed_threads(ThreadState::Open);
    let id = "thread/worker";
    threads.lifecycle("thread:test", id.into(), Mutation::Close);
    let mut original = threads.writes().pop().unwrap();
    let client = Client::unix("/tmp/thread-original.sock");
    threads.bind(&mut original, client.clone()).unwrap();
    threads.complete(
        Receipt {
            write: original.clone(),
            confirmation: Confirmation::Failed(ClientError::Transport("lost".into())),
        },
        &mut HashMap::new(),
    );
    threads.retry(&original.key);
    let mut retry = threads.writes().pop().unwrap();
    threads
        .bind(&mut retry, Client::unix("/tmp/thread-other.sock"))
        .unwrap();
    assert_eq!(
        format!("{:?}", retry.client.as_ref().unwrap()),
        format!("{client:?}")
    );
    threads.complete(
        Receipt {
            write: retry,
            confirmation: Confirmation::Applied(None),
        },
        &mut HashMap::new(),
    );
    threads.lifecycle("thread:test", id.into(), Mutation::Close);
    let next = threads.writes().pop().unwrap();
    assert_ne!(next.key, original.key);
    threads.complete(
        Receipt {
            write: next,
            confirmation: Confirmation::Failed(forbidden()),
        },
        &mut HashMap::new(),
    );
    assert!(threads.pending_actions.is_empty());
}

#[test]
fn starting_adopts_compatible_legacy_drafts_and_reports_conflicts_without_losing_either() {
    let item = "attention/feedback";
    let pane = Pane::Home(Some(item.into()));
    let input = Input::Start {
        item: item.into(),
        episode: "claim/feedback".into(),
    };
    let context = ThreadInterest::Inbox {
        source: "agent/example".into(),
        episode: "claim/feedback".into(),
    };
    for (canonical, legacy, selected) in [
        (Some(""), "Recovered legacy draft", "Recovered legacy draft"),
        (None, "Recovered legacy draft", "Recovered legacy draft"),
        (Some("Same draft"), "Same draft", "Same draft"),
        (Some("Canonical draft"), "", "Canonical draft"),
    ] {
        let mut threads = observed_threads(ThreadState::Open);
        threads.origins.insert(
            item.into(),
            Origin {
                context: context.clone(),
                agent: "agent/atlas".into(),
                title: "Review".into(),
                snapshot: snapshot().id,
            },
        );
        let mut texts = HashMap::from([(format!("chat:{item}"), legacy.into())]);
        if let Some(canonical) = canonical {
            texts.insert(input.key(), canonical.into());
        }
        assert!(threads.begin(&pane.key(), item, &mut texts));
        assert_eq!(texts[&input.key()], selected);
        assert!(!texts.contains_key(&format!("chat:{item}")));
        assert!(
            threads.writes().is_empty(),
            "draft adoption alone never sends"
        );
    }
    let mut threads = observed_threads(ThreadState::Open);
    threads.origins.insert(
        item.into(),
        Origin {
            context: context.clone(),
            agent: "agent/atlas".into(),
            title: "Review".into(),
            snapshot: snapshot().id,
        },
    );
    threads.lists.insert(context.clone(), Vec::new());
    let mut texts = HashMap::from([
        (input.key(), "Canonical draft".into()),
        (format!("chat:{item}"), "Different legacy draft".into()),
    ]);
    let before = texts.clone();
    threads.tap(&pane.key(), thread::Tap::Start(item.into()), &mut texts);
    assert_eq!(texts, before);
    assert!(threads.focused.is_none());
    assert!(threads.writes().is_empty());
    assert_eq!(threads.failures[&context], DRAFT_COLLISION);
    let doc = threads.mount(
        &pane,
        120,
        &edit::Cursor::default(),
        &super::super::demo::test_world(),
        &texts,
    );
    let rendered = doc
        .lines
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(rendered.contains("Both drafts are kept"), "{rendered}");
    let combine = doc
        .targets
        .iter()
        .find_map(|target| match &target.hit {
            Hit::Thread(host, tap @ thread::Tap::CombineSavedDrafts(_)) => {
                Some((host.clone(), tap.clone()))
            }
            _ => None,
        })
        .expect("the collision exposes a reachable normal native action");
    threads.tap(&combine.0, combine.1, &mut texts);
    assert_eq!(
        texts[&input.key()],
        "Canonical draft\n\n--- Earlier saved draft ---\n\nDifferent legacy draft"
    );
    assert!(!texts.contains_key(&format!("chat:{item}")));
    assert!(
        matches!(&threads.focused, Some((host, mode)) if host == &pane.key() && mode == &input)
    );
    assert_eq!(threads.failures[&context], DRAFT_COMBINED);
    assert!(
        threads.writes().is_empty(),
        "combining is local editing, never Send"
    );
    let combined = texts.clone();
    assert!(
        threads
            .start_with_text(&pane.key(), item, "Different revision feedback", &mut texts)
            .is_err()
    );
    assert_eq!(
        texts, combined,
        "a refused feedback handoff cannot overwrite combined saved text"
    );
    assert!(threads.writes().is_empty());
    assert!(threads.paste(&pane, "Editable: ", &edit::Cursor::default(), &mut texts));
}

#[test]
fn locked_start_requests_never_offer_or_apply_saved_draft_combination() {
    let item = "attention/feedback";
    let pane = Pane::Home(Some(item.into()));
    let input = Input::Start {
        item: item.into(),
        episode: "claim/feedback".into(),
    };
    let context = ThreadInterest::Inbox {
        source: "agent/example".into(),
        episode: "claim/feedback".into(),
    };
    for unknown in [false, true] {
        let mut threads = observed_threads(ThreadState::Open);
        threads.origins.insert(
            item.into(),
            Origin {
                context: context.clone(),
                agent: "agent/atlas".into(),
                title: "Review".into(),
                snapshot: snapshot().id,
            },
        );
        threads.lists.insert(context.clone(), Vec::new());
        let mut texts = HashMap::new();
        threads
            .start_with_text(&pane.key(), item, "Original delivery", &mut texts)
            .unwrap();
        let original = threads.writes().pop().unwrap();
        if unknown {
            threads.complete(
                Receipt {
                    write: original.clone(),
                    confirmation: Confirmation::Failed(ClientError::Transport(
                        "lost receipt".into(),
                    )),
                },
                &mut texts,
            );
        }
        texts.insert(format!("chat:{item}"), "Different saved text".into());
        let before = texts.clone();
        threads.key(
            &pane,
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
            &edit::Cursor::default(),
            &mut texts,
        );
        threads.tap(&pane.key(), thread::Tap::Start(item.into()), &mut texts);
        let doc = threads.mount(
            &pane,
            120,
            &edit::Cursor::default(),
            &super::super::demo::test_world(),
            &texts,
        );
        assert!(!doc.targets.iter().any(|target| matches!(
            &target.hit,
            Hit::Thread(_, thread::Tap::CombineSavedDrafts(_))
        )));
        threads.tap(
            &pane.key(),
            thread::Tap::CombineSavedDrafts(item.into()),
            &mut texts,
        );
        assert_eq!(texts, before);
        let retained = &threads.drafts[&input.key()]
            .delivery
            .as_ref()
            .unwrap()
            .write;
        assert_eq!(retained.key, original.key);
        assert_eq!(retained.fence, original.fence);
        assert!(
            matches!(&retained.mutation, Mutation::Start(start) if start.content == "Original delivery")
        );
        assert_eq!(threads.failures[&context], DRAFT_LOCKED_COLLISION);
        assert!(threads.writes().is_empty());
    }
}

#[test]
fn feedback_preflights_both_saved_drafts_and_only_adopts_after_a_request_is_retained() {
    let item = "attention/feedback";
    let input = Input::Start {
        item: item.into(),
        episode: "claim/feedback".into(),
    };
    for (canonical, legacy, refusal) in [
        ("", "Different legacy", "collision"),
        ("Feedback", "Different legacy", "collision"),
        ("Different canonical", "Feedback", "collision"),
        ("", "Feedback", "too-long"),
        ("Feedback", "Feedback", "busy"),
        ("", "Feedback", "none"),
        ("Feedback", "Feedback", "none"),
    ] {
        let mut threads = observed_threads(ThreadState::Open);
        threads.origins.insert(
            item.into(),
            Origin {
                context: ThreadInterest::Inbox {
                    source: "agent/example".into(),
                    episode: "claim/feedback".into(),
                },
                agent: "agent/atlas".into(),
                title: "Review".into(),
                snapshot: snapshot().id,
            },
        );
        let mut texts = HashMap::from([
            (input.key(), canonical.into()),
            (format!("chat:{item}"), legacy.into()),
            ("revision:original".into(), "Feedback".into()),
        ]);
        if refusal == "busy" {
            threads
                .start_with_text("home:attention/feedback", item, "Feedback", &mut texts)
                .unwrap();
            threads.writes();
            texts.insert(format!("chat:{item}"), legacy.into());
        }
        let feedback = if refusal == "too-long" {
            "x".repeat(fractal_client::device::INLINE_MAX_BYTES + 1)
        } else {
            "Feedback".into()
        };
        let before = texts.clone();
        let focused = threads.focused.clone();
        let result =
            threads.start_with_text("home:attention/feedback", item, &feedback, &mut texts);
        if refusal == "none" {
            result.unwrap();
            assert_eq!(texts[&input.key()], "Feedback");
            assert!(!texts.contains_key(&format!("chat:{item}")));
            let write = threads.writes().pop().unwrap();
            assert_eq!(
                threads.drafts[&input.key()]
                    .delivery
                    .as_ref()
                    .unwrap()
                    .write
                    .key,
                write.key
            );
        } else {
            let error = result.unwrap_err();
            if refusal == "collision" {
                assert_eq!(error, DRAFT_COLLISION);
            }
            assert_eq!(texts, before, "{refusal}");
            assert!(threads.focused == focused);
            assert!(threads.writes().is_empty());
        }
        assert_eq!(texts["revision:original"], "Feedback");
    }
}

#[test]
fn feedback_adopts_only_an_empty_or_identical_original_thread_draft() {
    let mut threads = observed_threads(ThreadState::Open);
    let item = "attention/feedback";
    let input = Input::Start {
        item: item.into(),
        episode: "claim/feedback".into(),
    };
    threads.origins.insert(
        item.into(),
        Origin {
            context: ThreadInterest::Inbox {
                source: "step-run/feedback".into(),
                episode: "claim/feedback".into(),
            },
            agent: "agent/atlas".into(),
            title: "Review".into(),
            snapshot: snapshot().id,
        },
    );
    let mut texts = HashMap::from([(format!("chat:{item}"), "Old private draft".into())]);
    assert!(
        threads
            .start_with_text(
                "home:attention/feedback",
                item,
                "Revision feedback",
                &mut texts
            )
            .is_err()
    );
    assert_eq!(texts[&format!("chat:{item}")], "Old private draft");
    assert!(!texts.contains_key(&input.key()));
    texts.clear();
    threads
        .start_with_text(
            "home:attention/feedback",
            item,
            "Revision feedback",
            &mut texts,
        )
        .unwrap();
    let write = threads.writes().pop().unwrap();
    let Mutation::Start(start) = write.mutation else {
        panic!("durable thread start")
    };
    assert_eq!(start.content, "Revision feedback");
    assert!(threads.drafts[&input.key()].delivery.is_some());
}

#[test]
fn internal_error_without_not_applied_answer_keeps_the_original_action() {
    let mut threads = observed_threads(ThreadState::Open);
    threads.lifecycle("thread:test", "thread/worker".into(), Mutation::Close);
    let write = threads.writes().pop().unwrap();
    let ClientError::Api(_, message, mut envelope) = forbidden() else {
        unreachable!()
    };
    envelope.code = fractal_client::ErrorCode::Internal;
    let error = ClientError::Api(fractal_client::ErrorCode::Internal, message, envelope);
    threads.complete(
        Receipt {
            write,
            confirmation: Confirmation::Failed(error),
        },
        &mut HashMap::new(),
    );
    assert!(threads.pending_actions["thread/worker"].unknown);
}

#[test]
fn embedded_thread_disclosures_are_independent_in_agent_mission_and_inbox_hosts() {
    for pane in [
        Pane::Agent(Some("agent/atlas".into())),
        Pane::Mission(Some("mission/example".into())),
        Pane::Home(Some("attention/notice".into())),
    ] {
        let mut threads = observed_threads(ThreadState::Open);
        let first = "thread/worker";
        let second = "thread/sibling";
        let origin = ThreadInterest::Inbox {
            source: "agent/example".into(),
            episode: "claim/notice".into(),
        };
        threads.origins.insert(
            "attention/notice".into(),
            Origin {
                context: origin,
                agent: "agent/atlas".into(),
                title: "Notice".into(),
                snapshot: snapshot().id,
            },
        );
        let context = threads.context(&pane).unwrap();
        threads.window(
            context,
            snapshot(),
            true,
            ThreadCollectionRows::Summaries(vec![
                summary(),
                ThreadSummary {
                    id: second.into(),
                    title: "Sibling thread".into(),
                    ..summary()
                },
            ]),
            vec![],
            vec![first.into(), second.into()],
            false,
        );
        for (id, content) in [
            (first, "First thread body"),
            (second, "Sibling thread body"),
        ] {
            let event = serde_json::from_value(serde_json::json!({
                "kind":"message", "id":format!("message/{id}"), "actor_id":"agent/atlas",
                "accepted_at_unix_ms":1000,"content":content,"thread_quote_id":null,
            }))
            .unwrap();
            threads.histories.insert(
                id.into(),
                History {
                    head: vec![event],
                    ..History::default()
                },
            );
        }
        let mut texts = HashMap::new();
        threads.tap(&pane.key(), thread::Tap::Toggle(first.into()), &mut texts);
        threads.tap(&pane.key(), thread::Tap::Toggle(second.into()), &mut texts);
        threads.paste(&pane, "Sibling draft", &edit::Cursor::default(), &mut texts);
        let rendered = |threads: &Threads, texts: &HashMap<String, String>| {
            threads
                .mount(
                    &pane,
                    100,
                    &edit::Cursor::default(),
                    &super::super::demo::test_world(),
                    texts,
                )
                .lines
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n")
        };
        let both = rendered(&threads, &texts);
        assert!(
            both.contains("First thread body") && both.contains("Sibling thread body"),
            "{both}"
        );
        let interests = threads.interests(&pane);
        for id in [first, second] {
            assert!(interests.contains(&ThreadInterest::Summary(id.into())));
            assert!(interests.contains(&ThreadInterest::Timeline(id.into())));
        }
        threads.tap(&pane.key(), thread::Tap::Toggle(first.into()), &mut texts);
        let after = rendered(&threads, &texts);
        assert!(
            !after.contains("First thread body") && after.contains("Sibling thread body"),
            "{after}"
        );
        assert!(after.contains("Sibling draft"));
        assert!(
            matches!(&threads.focused, Some((host, Input::Reply(id))) if host == &pane.key() && id == second)
        );
        assert_eq!(texts[&Input::Reply(second.into()).key()], "Sibling draft");
        let interests = threads.interests(&pane);
        assert!(!interests.contains(&ThreadInterest::Summary(first.into())));
        assert!(!interests.contains(&ThreadInterest::Timeline(first.into())));
        assert!(interests.contains(&ThreadInterest::Summary(second.into())));
        assert!(interests.contains(&ThreadInterest::Timeline(second.into())));
        threads.tap(&pane.key(), thread::Tap::Toggle(first.into()), &mut texts);
        let restored = rendered(&threads, &texts);
        assert!(restored.contains("First thread body") && restored.contains("Sibling thread body"));
        assert_eq!(texts[&Input::Reply(second.into()).key()], "Sibling draft");
        threads.tap(
            &pane.key(),
            thread::Tap::LeaveComposer(first.into()),
            &mut texts,
        );
        assert!(
            matches!(threads.shortcut(&pane, KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE)), Some(thread::Tap::Close(id)) if id == first),
            "leaving a composer retains its exact selected thread for shortcuts"
        );
    }
}

#[test]
fn paused_live_updates_keep_cached_thread_content_and_draft_until_a_fresh_frame_resumes_them() {
    let mut threads = observed_threads(ThreadState::Open);
    let pane = Pane::Thread("thread/worker".into());
    let mut texts = HashMap::new();
    threads.tap(
        &pane.key(),
        thread::Tap::Compose("thread/worker".into()),
        &mut texts,
    );
    threads.paste(
        &pane,
        "Keep this local draft",
        &edit::Cursor::default(),
        &mut texts,
    );
    let timeline = ThreadInterest::Timeline("thread/worker".into());
    threads.failure(
        timeline.clone(),
        "Live thread updates paused: shared live-window budget; resumes automatically".into(),
        false,
    );
    let render = |threads: &Threads| {
        threads
            .mount(
                &pane,
                100,
                &edit::Cursor::default(),
                &super::super::demo::test_world(),
                &texts,
            )
            .lines
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    };
    let paused = render(&threads);
    assert!(paused.contains("Live thread updates paused"));
    assert!(paused.contains("Keep this local draft"));
    assert!(!paused.contains("Loading thread"));
    threads.window(
        timeline,
        snapshot(),
        true,
        ThreadCollectionRows::Timeline(vec![]),
        vec![],
        vec![],
        false,
    );
    let resumed = render(&threads);
    assert!(!resumed.contains("Live thread updates paused"));
    assert!(resumed.contains("Keep this local draft"));
}

#[test]
fn expanded_threads_keep_one_honest_keyboard_context_after_escape_and_independent_collapse() {
    let mut threads = observed_threads(ThreadState::Open);
    let pane = Pane::Agent(Some("agent/atlas".into()));
    let first = "thread/worker";
    let second = "thread/sibling";
    threads.window(
        ThreadInterest::Agent("agent/atlas".into()),
        snapshot(),
        true,
        ThreadCollectionRows::Summaries(vec![
            summary(),
            ThreadSummary {
                id: second.into(),
                title: "Sibling".into(),
                ..summary()
            },
        ]),
        vec![],
        vec![first.into(), second.into()],
        false,
    );
    threads.histories.insert(second.into(), History::default());
    let mut texts = HashMap::new();
    threads.tap(&pane.key(), thread::Tap::Toggle(first.into()), &mut texts);
    threads.tap(&pane.key(), thread::Tap::Toggle(second.into()), &mut texts);
    threads.paste(&pane, "Sibling draft", &edit::Cursor::default(), &mut texts);
    assert!(threads.key(
        &pane,
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
        &edit::Cursor::default(),
        &mut texts
    ));
    let close = || KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE);
    assert!(
        matches!(threads.shortcut(&pane, close()), Some(thread::Tap::Close(id)) if id == second)
    );
    let doc = threads.mount(
        &pane,
        100,
        &edit::Cursor::default(),
        &super::super::demo::test_world(),
        &texts,
    );
    for id in [first, second] {
        let target = doc.targets.iter().find(|target| matches!(&target.hit, Hit::Thread(_, thread::Tap::Close(found)) if found == id)).unwrap();
        assert_eq!(
            doc.lines[target.line].to_string().contains(" Z "),
            id == second,
            "only the selected panel advertises its keyboard shortcut"
        );
    }
    threads.tap(&pane.key(), thread::Tap::Toggle(first.into()), &mut texts);
    assert!(
        matches!(threads.shortcut(&pane, close()), Some(thread::Tap::Close(id)) if id == second)
    );
    let action = threads.shortcut(&pane, close()).unwrap();
    threads.tap(&pane.key(), action, &mut texts);
    assert_eq!(threads.writes().pop().unwrap().id, second);
    assert_eq!(texts[&Input::Reply(second.into()).key()], "Sibling draft");
    threads.tap(&pane.key(), thread::Tap::Toggle(first.into()), &mut texts);
    threads.tap(&pane.key(), thread::Tap::Compose(second.into()), &mut texts);
    threads.key(
        &pane,
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
        &edit::Cursor::default(),
        &mut texts,
    );
    threads.tap(&pane.key(), thread::Tap::Toggle(second.into()), &mut texts);
    assert!(threads.open[&pane.key()].contains(first));
    assert!(!threads.selected.contains_key(&pane.key()));
    assert!(
        threads.shortcut(&pane, close()).is_none(),
        "collapsing the selected container cannot silently target its sibling"
    );
    assert!(threads.writes().is_empty());
}

#[test]
fn accepting_a_new_inbox_thread_keeps_the_existing_expanded_thread_and_its_draft() {
    let mut threads = observed_threads(ThreadState::Open);
    let pane = Pane::Home(Some("attention/notice".into()));
    let item = "attention/notice";
    let first = "thread/worker";
    let context = ThreadInterest::Inbox {
        source: "agent/example".into(),
        episode: "claim/notice".into(),
    };
    threads.origins.insert(
        item.into(),
        Origin {
            context: context.clone(),
            agent: "agent/atlas".into(),
            title: "Notice".into(),
            snapshot: snapshot().id,
        },
    );
    threads.lists.insert(context, vec![first.into()]);
    let mut texts = HashMap::new();
    threads.tap(&pane.key(), thread::Tap::Toggle(first.into()), &mut texts);
    threads.paste(
        &pane,
        "Keep the existing draft",
        &edit::Cursor::default(),
        &mut texts,
    );
    threads.begin(&pane.key(), item, &mut texts);
    let start_input = threads.start_input(item).unwrap();
    threads.paste(
        &pane,
        "First message in new thread",
        &edit::Cursor::default(),
        &mut texts,
    );
    threads.tap(&pane.key(), thread::Tap::Toggle(first.into()), &mut texts);
    assert!(
        matches!(&threads.focused, Some((host, input)) if host == &pane.key() && input == &start_input),
        "collapsing a sibling must not blur the separate Start draft"
    );
    assert_eq!(texts[&start_input.key()], "First message in new thread");
    threads.tap(&pane.key(), thread::Tap::Toggle(first.into()), &mut texts);
    threads.begin(&pane.key(), item, &mut texts);
    threads.submit(&pane.key(), start_input, &texts);
    let write = threads.writes().pop().unwrap();
    let new_id = write.id.clone();
    threads.complete(
        Receipt {
            write,
            confirmation: Confirmation::Applied(Some(Envelope {
                value: ThreadSummary {
                    id: new_id.clone(),
                    title: "Notice".into(),
                    ..summary()
                },
                snapshot: snapshot(),
                request_id: "request/new".into(),
                api_version: fractal_client::API_VERSION.into(),
            })),
        },
        &mut texts,
    );
    threads.histories.insert(new_id.clone(), History::default());
    assert!(threads.open[&pane.key()].contains(first));
    assert!(threads.open[&pane.key()].contains(&new_id));
    assert_eq!(
        texts[&Input::Reply(first.into()).key()],
        "Keep the existing draft"
    );
    let interests = threads.interests(&pane);
    for id in [first, &new_id] {
        assert!(interests.contains(&ThreadInterest::Summary(id.into())));
        assert!(interests.contains(&ThreadInterest::Timeline(id.into())));
    }
    let doc = threads.mount(
        &pane,
        100,
        &edit::Cursor::default(),
        &super::super::demo::test_world(),
        &texts,
    );
    let rendered = doc
        .lines
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        rendered.contains("▾ Worker test") && rendered.contains("▾ Notice"),
        "{rendered}"
    );
    assert!(rendered.contains("Keep the existing draft"));
}

#[test]
fn inbox_summary_timeline_and_lists_all_remain_requested() {
    let mut threads = observed_threads(ThreadState::Open);
    let item = "attention/notice";
    let pane = Pane::Home(Some(item.into()));
    threads.origins.insert(
        item.into(),
        Origin {
            context: ThreadInterest::Inbox {
                source: "agent/example".into(),
                episode: "claim/fault".into(),
            },
            agent: "agent/atlas".into(),
            title: "Fault".into(),
            snapshot: snapshot().id,
        },
    );
    threads
        .open
        .entry(pane.key())
        .or_default()
        .insert("thread/worker".into());
    let interests = threads.interests(&pane);
    assert_eq!(interests.len(), 4);
    assert!(interests.contains(&ThreadInterest::RecentlyClosed));
    assert!(interests.contains(&ThreadInterest::Summary("thread/worker".into())));
    assert!(interests.contains(&ThreadInterest::Timeline("thread/worker".into())));
}

#[test]
fn feedback_refusals_leave_both_drafts_unchanged_and_success_is_retained() {
    let item = "attention/feedback";
    for scenario in [
        "unavailable",
        "too-long",
        "busy",
        "different-draft",
        "missing-recipient",
    ] {
        let mut threads = observed_threads(ThreadState::Open);
        threads.origins.insert(
            item.into(),
            Origin {
                context: ThreadInterest::Inbox {
                    source: "agent/example".into(),
                    episode: "claim/feedback".into(),
                },
                agent: "agent/atlas".into(),
                title: "Review".into(),
                snapshot: snapshot().id,
            },
        );
        let input = Input::Start {
            item: item.into(),
            episode: "claim/feedback".into(),
        };
        let text = if scenario == "too-long" {
            "x".repeat(fractal_client::device::INLINE_MAX_BYTES + 1)
        } else {
            "Feedback".into()
        };
        let mut texts = HashMap::from([("revision:original".into(), text.clone())]);
        match scenario {
            "unavailable" => threads.enable(false),
            "missing-recipient" => {
                threads.origins.clear();
            }
            "different-draft" => {
                texts.insert(input.key(), "Existing thread draft".into());
            }
            "busy" => {
                threads
                    .start_with_text("home:attention/feedback", item, &text, &mut texts)
                    .unwrap();
                threads.writes();
            }
            _ => {}
        }
        let before = texts.clone();
        let focused = threads.focused.clone();
        let prior_key = threads
            .drafts
            .get(&input.key())
            .and_then(|draft| draft.delivery.as_ref())
            .map(|delivery| delivery.write.key.clone());
        assert!(
            threads
                .start_with_text("home:attention/feedback", item, &text, &mut texts)
                .is_err(),
            "{scenario}"
        );
        assert_eq!(texts, before, "{scenario}");
        assert!(threads.focused == focused, "{scenario}");
        assert_eq!(
            threads
                .drafts
                .get(&input.key())
                .and_then(|draft| draft.delivery.as_ref())
                .map(|delivery| delivery.write.key.clone()),
            prior_key,
            "{scenario}"
        );
        assert!(threads.writes().is_empty(), "{scenario}");
    }
    let mut threads = observed_threads(ThreadState::Open);
    threads.origins.insert(
        item.into(),
        Origin {
            context: ThreadInterest::Inbox {
                source: "agent/example".into(),
                episode: "claim/feedback".into(),
            },
            agent: "agent/atlas".into(),
            title: "Review".into(),
            snapshot: snapshot().id,
        },
    );
    let mut texts = HashMap::new();
    threads
        .start_with_text("home:attention/feedback", item, "Feedback", &mut texts)
        .unwrap();
    let write = threads.writes().pop().unwrap();
    let retained = threads.drafts[&Input::Start {
        item: item.into(),
        episode: "claim/feedback".into(),
    }
    .key()]
        .delivery
        .as_ref()
        .unwrap();
    assert_eq!(retained.write.key, write.key);
    let Mutation::Start(parameters) = &write.mutation else {
        panic!("thread.start")
    };
    assert_eq!(parameters.origin.source_id, "agent/example");
    assert_eq!(parameters.content, "Feedback");
}

#[test]
fn retained_start_keeps_its_captured_context_through_replacement_and_removal() {
    let mut threads = observed_threads(ThreadState::Open);
    let item = "attention/notice";
    threads.origins.insert(
        item.into(),
        Origin {
            context: ThreadInterest::Inbox {
                source: "agent/example".into(),
                episode: "claim/source".into(),
            },
            agent: "agent/atlas".into(),
            title: "Fault context".into(),
            snapshot: snapshot().id,
        },
    );
    let mut texts = HashMap::new();
    threads
        .start_with_text("home:attention/notice", item, "Please explain", &mut texts)
        .unwrap();
    let original = threads.writes().pop().unwrap();
    let captured = ThreadInterest::Inbox {
        source: "agent/example".into(),
        episode: "claim/source".into(),
    };
    let replacement = ThreadInterest::Inbox {
        source: "agent/replacement".into(),
        episode: "claim/replacement".into(),
    };
    threads.lists.insert(captured.clone(), Vec::new());
    threads.lists.insert(replacement.clone(), Vec::new());
    threads.origins.insert(
        item.into(),
        Origin {
            context: replacement,
            agent: "agent/replacement".into(),
            title: "Replacement title".into(),
            snapshot: snapshot().id,
        },
    );
    let home = Pane::Home(Some(item.into()));
    assert_eq!(threads.context(&home), Some(captured.clone()));
    assert!(
        matches!(threads.start_input(item), Some(Input::Start { episode, .. }) if episode == "claim/source")
    );
    let assert_captured = |threads: &Threads, pane: &Pane, texts: &HashMap<String, String>| {
        let doc = threads.mount(
            pane,
            100,
            &edit::Cursor::default(),
            &super::super::demo::test_world(),
            texts,
        );
        let rendered = doc
            .lines
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            rendered.contains("Fault context · Draft · to agent/atlas"),
            "{rendered}"
        );
        assert!(!rendered.contains("Replacement title"), "{rendered}");
        assert!(!rendered.contains("to agent/replacement"), "{rendered}");
    };
    assert_captured(&threads, &home, &texts);
    threads.complete(
        Receipt {
            write: original.clone(),
            confirmation: Confirmation::Failed(ClientError::Transport("lost".into())),
        },
        &mut texts,
    );
    let pane = Pane::Agent(Some("agent/atlas".into()));
    threads
        .lists
        .insert(ThreadInterest::Agent("agent/atlas".into()), Vec::new());
    assert_captured(&threads, &home, &texts);
    assert_captured(&threads, &pane, &texts);
    threads.origins.clear();
    assert_captured(&threads, &home, &texts);
    assert_captured(&threads, &pane, &texts);
    let doc = threads.mount(
        &pane,
        100,
        &edit::Cursor::default(),
        &super::super::demo::test_world(),
        &texts,
    );
    assert!(
        doc.lines
            .iter()
            .any(|line| line.to_string().contains("Fault context · Draft"))
    );
    assert!(doc.targets.iter().any(|target| matches!(&target.hit,Hit::Thread(_,thread::Tap::Retry(key)) if key == &original.key)));
    threads.tap(
        &pane.key(),
        thread::Tap::Retry(original.key.clone()),
        &mut texts,
    );
    let retry = threads.writes().pop().unwrap();
    assert_eq!(retry.key, original.key);
    assert_eq!(retry.id, original.id);
    assert_eq!(retry.fence, original.fence);
    let Mutation::Start(parameters) = retry.mutation else {
        panic!("original start")
    };
    assert_eq!(parameters.agent_id, "agent/atlas");
    assert_eq!(parameters.title, "Fault context");
    assert_eq!(parameters.origin.source_id, "agent/example");
    assert_eq!(parameters.origin.episode, "claim/source");
}

#[test]
fn read_only_thread_history_has_no_composer_or_new_mutations_but_keeps_original_retry() {
    let mut threads = observed_threads(ThreadState::Open);
    threads.lifecycle("thread:test", "thread/worker".into(), Mutation::Close);
    let original = threads.writes().pop().unwrap();
    threads.complete(
        Receipt {
            write: original.clone(),
            confirmation: Confirmation::Failed(ClientError::Transport("lost".into())),
        },
        &mut HashMap::new(),
    );
    threads.set_access(ThreadAccess::default());
    let pane = Pane::Thread("thread/worker".into());
    let mut texts = HashMap::new();
    threads.tap(
        &pane.key(),
        thread::Tap::Compose("thread/worker".into()),
        &mut texts,
    );
    threads.tap(
        &pane.key(),
        thread::Tap::Rename("thread/worker".into()),
        &mut texts,
    );
    threads.lifecycle(&pane.key(), "thread/worker".into(), Mutation::Reopen);
    assert!(threads.writes().is_empty());
    assert!(!threads.editing_in(&pane));
    let doc = threads.mount(
        &pane,
        100,
        &edit::Cursor::default(),
        &super::super::demo::test_world(),
        &texts,
    );
    assert!(!doc.targets.iter().any(|target| matches!(
        target.hit,
        Hit::Thread(
            _,
            thread::Tap::Compose(_)
                | thread::Tap::Reply { .. }
                | thread::Tap::Rename(_)
                | thread::Tap::Close(_)
                | thread::Tap::Reopen(_)
        )
    )));
    assert!(doc.targets.iter().any(|target| matches!(&target.hit,Hit::Thread(_,thread::Tap::Retry(key)) if key == &original.key)));
    threads.tap(
        &pane.key(),
        thread::Tap::Retry(original.key.clone()),
        &mut texts,
    );
    assert_eq!(threads.writes().pop().unwrap().key, original.key);
}

#[tokio::test]
async fn paired_native_transport_replays_the_frozen_signed_packet_byte_for_byte() {
    use axum::{
        Json, Router,
        body::Bytes,
        http::{HeaderMap, StatusCode},
        response::IntoResponse,
        routing::{get, post},
    };
    use fractal_client::device::{Device, KeyAlgorithm, SigningKey};
    use std::sync::{Arc, Mutex};

    // This is a transport/signature-retention receiver, not a fake authorization proof.
    // Real enrollment and grant rejection are exercised by the daemon's paired router tests.
    let requests = Arc::new(Mutex::new(Vec::<Vec<u8>>::new()));
    let received = requests.clone();
    let app = Router::new().route("/v1/client/actions",post(move |headers:HeaderMap, body:Bytes| {
        let received = received.clone();
        async move {
            assert_eq!(headers["authorization"],"Bearer fixture-device-credential");
            let value:serde_json::Value = serde_json::from_slice(&body).unwrap();
            let count = { let mut requests = received.lock().unwrap(); requests.push(body.to_vec()); requests.len() };
            if count == 1 {
                return (StatusCode::BAD_GATEWAY,Json(serde_json::json!({
                    "api_version":fractal_client::API_VERSION,"error_version":"st3.client.error.v0",
                    "request_id":"request/test","code":"internal","message":"The receipt was lost",
                    "retryable":true,"details":{"applied":"unknown"}
                }))).into_response();
            }
            Json(serde_json::json!({
                "api_version":fractal_client::API_VERSION,"request_id":"request/test","snapshot":snapshot(),
                "value":{"kind":"action-result","action_id":value["id"],"operation_id":"claim/operation",
                    "status":"completed","snapshot_id":snapshot().id,
                    "affected_ids":["thread/worker","message/captured"]}
            })).into_response()
        }
    })).route("/v1/client/threads/{id}",get(|| async {
        Json(serde_json::json!({"api_version":fractal_client::API_VERSION,"request_id":"request/test","snapshot":snapshot(),"value":summary()}))
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let session = serde_json::from_value(serde_json::json!({
        "kind":"person-paired-session","device_id":"device/test","person_id":"person/operator",
        "session_actor":"person/operator","credential":"fixture-device-credential",
        "scopes":["read.projections","read.conversation","control.messages"],"expires_at":"2099-01-01T00:00:00Z",
        "device_key_chain":["claim/device"]
    })).unwrap();
    let device = Device {
        endpoint,
        allow_public_http: false,
        person_root_fingerprint: None,
        session,
        signing_key: Some(SigningKey::generate(KeyAlgorithm::Ed25519).unwrap()),
    };
    let client = device.client().unwrap();
    let mut threads = observed_threads(ThreadState::Closed);
    let pane = Pane::Thread("thread/worker".into());
    let mut texts = HashMap::new();
    threads.tap(
        &pane.key(),
        thread::Tap::Compose("thread/worker".into()),
        &mut texts,
    );
    threads.paste(
        &pane,
        "Signed retained reply",
        &edit::Cursor::default(),
        &mut texts,
    );
    threads.submit(&pane.key(), Input::Reply("thread/worker".into()), &texts);
    let mut write = threads.writes().pop().unwrap();
    threads.bind(&mut write, client.clone()).unwrap();
    let Mutation::Reply(prepared) = &write.mutation else {
        panic!("reply")
    };
    assert!(
        prepared.signature.is_some(),
        "prepare must happen before the first POST"
    );
    let key = write.key.clone();
    assert!(
        threads
            .complete(write.perform(client.clone()).await, &mut texts)
            .is_some()
    );
    threads.retry(&key);
    let mut retry = threads.writes().pop().unwrap();
    threads.bind(&mut retry, client.clone()).unwrap();
    assert_eq!(
        threads.complete(retry.perform(client).await, &mut texts),
        None
    );
    let packets = requests.lock().unwrap();
    assert_eq!(packets.len(), 2);
    assert_eq!(
        packets[0], packets[1],
        "including snapshot, subject intent, nonce and signature"
    );
    let payload: serde_json::Value = serde_json::from_slice(&packets[0]).unwrap();
    assert!(
        payload["parameters"]["signature"]["signature"]
            .as_str()
            .is_some_and(|value| !value.is_empty())
    );
    server.abort();
}
