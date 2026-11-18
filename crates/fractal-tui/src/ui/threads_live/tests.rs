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

/// An exact Inbox source association. `agent` is the historical source agent, never the
/// first-Start recipient: that comes only from the canonical start context.
fn origin(context: ThreadInterest, agent: Option<&str>, title: &str) -> Origin {
    Origin {
        context,
        agent: agent.map(Into::into),
        title: title.into(),
    }
}

fn preview_snapshot(id: &str) -> Snapshot {
    Snapshot {
        id: id.into(),
        ..snapshot()
    }
}

fn retired(original: &str) -> fractal_client::ThreadStartRecipientBasis {
    fractal_client::ThreadStartRecipientBasis::Fallback {
        original_agent_id: original.into(),
        retirement: fractal_client::ThreadStartRetirement {
            claim_id: format!("claim/retired/{original}"),
            kind: fractal_client::ThreadStartRetirementKind::AgentRetired,
            state: fractal_client::ThreadStartRetirementState::Retired,
        },
    }
}

/// The generated canonical DTO exactly as `thread_start_context` returns it.
fn start_context_envelope(
    source: &str,
    episode: &str,
    recipient: &str,
    basis: fractal_client::ThreadStartRecipientBasis,
    at: &str,
) -> Envelope<fractal_client::ThreadStartContext> {
    Envelope {
        value: fractal_client::ThreadStartContext {
            kind: fractal_client::ThreadStartContextKind::ThreadStartContext,
            source_id: source.into(),
            episode: episode.into(),
            start_recipient: fractal_client::ThreadStartRecipient {
                agent_id: recipient.into(),
                basis,
            },
        },
        snapshot: preview_snapshot(at),
        request_id: "request/start-context".into(),
        api_version: fractal_client::API_VERSION.into(),
    }
}

/// The one canonical read the model queued for `item` (other items' reads are dropped).
fn take_start_context_read(threads: &mut Threads, item: &str) -> StartContextRead {
    let mut reads = threads.take_start_context_reads();
    let at = reads
        .iter()
        .position(|read| read.item == item)
        .unwrap_or_else(|| panic!("a canonical start-context read for {item}"));
    reads.swap_remove(at)
}

/// Ask for and answer the canonical preview of `item`'s exact current or saved source.
fn install_start_context(
    threads: &mut Threads,
    item: &str,
    recipient: &str,
    basis: fractal_client::ThreadStartRecipientBasis,
) {
    threads.request_start_context(item);
    let read = take_start_context_read(threads, item);
    let ThreadInterest::Inbox { source, episode } =
        threads.start_origin(item).unwrap().context.clone()
    else {
        panic!("an Inbox origin")
    };
    threads.start_context_loaded(
        &read.item,
        &read.key,
        Ok(start_context_envelope(
            &source,
            &episode,
            recipient,
            basis,
            "snapshot/start-context",
        )),
    );
    assert_eq!(
        threads
            .start_context(item)
            .map(|context| context.value.start_recipient.agent_id.as_str()),
        Some(recipient)
    );
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
        origin(ask.clone(), Some("agent/atlas"), "Worker test"),
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
    use fractal_daemon::{PersonWorkStore as _, SeatsStoreCore as _};
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
    let ask = store
        .ask_person(&fractal_daemon::model::PersonAskRequest {
            legacy_request: None,
            person: person.into(),
            title: "Review the worker".into(),
            reason: "Discuss the review without answering the ask.".into(),
            actor: agent.into(),
            step: None,
            new_run: None,
            incarnation: None,
            request: None,
            deadline_ms: None,
            default_answer: None,
            priority: None,
            idempotency_key: "native-thread-origin".into(),
        })
        .unwrap();
    let ask_item = store
        .attention_items(Some(person))
        .unwrap()
        .into_iter()
        .find(|item| item.subject == ask.subject)
        .unwrap();
    let state = fractal_daemon::api::AppState {
        browser_pairing_login: None,
        seat_admission: Default::default(),
        owning_person: Some(person.into()),
        store: store.clone(),
        notify: Arc::new(tokio::sync::Notify::new()),
        event_notify: tokio::sync::watch::channel(0).0,
        node: "native-thread-test".into(),
        api_socket: socket.clone(),
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

    fn capture(ui: &mut super::super::Ui, pane: &Pane, name: &str) {
        let Ok(dir) = std::env::var("THREAD_PREVIEW_CAPTURE_DIR") else {
            return;
        };
        let area = Rect::new(0, 0, 120, 40);
        let mut screen = Buffer::empty(area);
        ui.draw_pane(&mut screen, area, pane);
        let mut text = String::from(
            "HARNESS — production Ui::draw_pane + disposable real daemon socket, not the installed TUI binary\n",
        );
        for row in screen.content.chunks(120) {
            text.push_str(&row.iter().map(|cell| cell.symbol()).collect::<String>());
            text.push('\n');
        }
        std::fs::write(std::path::Path::new(&dir).join(format!("{name}.txt")), text).unwrap();
    }
    let inbox = Pane::Home(Some(ask.subject.clone()));
    let context = ThreadInterest::Inbox {
        source: ask.subject.clone(),
        episode: ask_item.episode.clone(),
    };
    ui.threads.origins.insert(
        ask.subject.clone(),
        origin(context.clone(), Some(agent), &ask_item.title),
    );
    // The recipient comes from the real daemon's canonical start-context read, never the card.
    ui.threads.request_start_context(&ask.subject);
    let read = take_start_context_read(&mut ui.threads, &ask.subject);
    let preview = read
        .read(&client)
        .await
        .expect("real canonical start context");
    assert_eq!(preview.value.start_recipient.agent_id, agent);
    assert_eq!(
        preview.value.start_recipient.basis,
        fractal_client::ThreadStartRecipientBasis::Original
    );
    let preview_fence = preview.snapshot.id.clone();
    ui.threads
        .start_context_loaded(&read.item, &read.key, Ok(preview));
    ui.threads.lists.insert(context, Vec::new());
    ui.open_in_glass(inbox.clone(), super::super::glass::Open::Tab);
    capture(&mut ui, &inbox, "harness-socket-start-before");
    ui.click(Hit::Thread(
        inbox.key(),
        thread::Tap::Start(ask.subject.clone()),
    ));
    ui.paste("Start the review discussion.".into());
    capture(&mut ui, &inbox, "harness-socket-start-draft");
    ui.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    ui.click(Hit::Thread(
        inbox.key(),
        thread::Tap::Start(ask.subject.clone()),
    ));
    ui.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let start = ui
        .threads
        .writes()
        .pop()
        .expect("native first-message start action");
    let Mutation::Start(parameters) = &start.mutation else {
        panic!("native Start")
    };
    assert_eq!(
        parameters.agent_id, agent,
        "the submitted recipient is the displayed canonical one"
    );
    assert_eq!(
        start.fence.snapshot_id, preview_fence,
        "the Start fence is the canonical preview snapshot"
    );
    let started_id = start.id.clone();
    let receipt = start.perform(client.clone()).await;
    assert!(
        matches!(receipt.confirmation, Confirmation::Applied(_)),
        "real first message committed"
    );
    ui.threads
        .complete(receipt, &mut ui.conversation_state.drafts);
    let timeline = client
        .thread_timeline(&started_id, None, Some(50))
        .await
        .unwrap();
    assert_eq!(
        timeline.value.items.len(),
        1,
        "start commits exactly one first message"
    );
    ui.threads.window(
        ThreadInterest::Timeline(started_id),
        timeline.snapshot,
        true,
        ThreadCollectionRows::Timeline(timeline.value.items.clone()),
        vec![],
        timeline
            .value
            .items
            .iter()
            .map(|event| event.id.clone())
            .collect(),
        false,
    );
    capture(&mut ui, &inbox, "harness-socket-start-after");
    assert!(
        store
            .attention_items(Some(person))
            .unwrap()
            .iter()
            .any(|item| item.subject == ask.subject),
        "starting a discussion never answers the ask"
    );
    let pane = Pane::Thread(id.into());
    ui.open_in_glass(pane.clone(), super::super::glass::Open::Tab);
    let host = pane.key();
    capture(&mut ui, &pane, "harness-socket-reply-before");
    ui.click(Hit::Thread(host.clone(), thread::Tap::Compose(id.into())));
    ui.paste("Please check the worker test.".into());
    capture(&mut ui, &pane, "harness-socket-compose-before-send");
    ui.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let write = ui.threads.writes().pop().expect("native reply action");
    let receipt = write.perform(client.clone()).await;
    assert_eq!(
        ui.threads
            .complete(receipt, &mut ui.conversation_state.drafts),
        None,
    );
    apply_window(&mut ui, &mut stream, id).await;
    capture(&mut ui, &pane, "harness-socket-reply-after");

    let area = Rect::new(0, 0, 100, 24);
    let mut screen = Buffer::empty(area);
    ui.draw_pane(&mut screen, area, &pane);
    let visible: String = screen.content.iter().map(|cell| cell.symbol()).collect();
    assert!(
        visible.contains("Please check the worker test."),
        "{visible}"
    );
    assert!(visible.contains("You"), "{visible}");

    capture(&mut ui, &pane, "harness-socket-close-before");
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
    capture(&mut ui, &pane, "harness-socket-close-after");
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
    capture(&mut ui, &pane, "harness-socket-quoted-reply-before");
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
    // Refresh the closed reply before renaming so every mutation uses its observed revision.
    apply_window(&mut ui, &mut stream, id).await;
    ui.threads.put_summary(client.thread_get(id).await.unwrap());
    capture(&mut ui, &pane, "harness-socket-quoted-reply-after");
    ui.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    // A thread tab types bare letters into its reply box, so its actions take Alt.
    ui.key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::ALT));
    ui.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
    ui.paste("Worker review complete".into());
    capture(&mut ui, &pane, "harness-socket-rename-before");
    ui.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let rename = ui.threads.writes().pop().expect("native rename action");
    assert!(matches!(rename.mutation, Mutation::Rename(_)));
    let receipt = rename.perform(client.clone()).await;
    assert!(
        matches!(receipt.confirmation, Confirmation::Applied(_)),
        "real Rename committed"
    );
    ui.threads
        .complete(receipt, &mut ui.conversation_state.drafts);
    apply_window(&mut ui, &mut stream, id).await;
    ui.threads.put_summary(client.thread_get(id).await.unwrap());
    capture(&mut ui, &pane, "harness-socket-rename-after");
    assert_eq!(
        client.thread_get(id).await.unwrap().value.title,
        "Worker review complete"
    );
    ui.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    capture(&mut ui, &pane, "harness-socket-reopen-before");
    ui.key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::ALT));
    let reopen = ui
        .threads
        .writes()
        .pop()
        .expect("native alt+z reopens the tab's closed thread");
    assert!(matches!(reopen.mutation, Mutation::Reopen));
    let receipt = reopen.perform(client.clone()).await;
    assert!(
        matches!(receipt.confirmation, Confirmation::Applied(_)),
        "real Reopen committed"
    );
    ui.threads
        .complete(receipt, &mut ui.conversation_state.drafts);
    apply_window(&mut ui, &mut stream, id).await;
    ui.threads.put_summary(client.thread_get(id).await.unwrap());
    capture(&mut ui, &pane, "harness-socket-reopen-after");
    assert_eq!(
        client.thread_get(id).await.unwrap().value.state,
        ThreadState::Open
    );
    server.abort();
}

#[test]
fn inbox_start_keeps_the_accepted_thread_in_the_draft_slot() {
    let mut threads = Threads::default();
    enable_editing(&mut threads);
    let mut world = super::super::demo::test_world();
    let super::super::view::Load::Ready(agents) = &mut world.agents else {
        panic!()
    };
    agents[0].id = "agent/atlas".into();
    agents[0].name = "Atlas".into();
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
        origin(context.clone(), Some(older.agent_id.as_str()), &older.title),
    );
    install_start_context(
        &mut threads,
        "attention/ask",
        "agent/atlas",
        fractal_client::ThreadStartRecipientBasis::Original,
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
        .position(|line| line.trim_end() == "▾ Worker test · New discussion · Draft")
        .unwrap();
    assert_eq!(
        draft[slot + 1].trim_end(),
        "New discussion to Atlas",
        "the canonical recipient's display label is visible before the first send"
    );
    assert_eq!(draft[slot + 2].trim_end(), "Original source recipient");
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
    let Mutation::Start(parameters) = &write.mutation else {
        panic!("first-message Start")
    };
    assert_eq!(parameters.agent_id, "agent/atlas");
    assert_eq!(write.fence.snapshot_id, "snapshot/start-context");
    let queued = lines(&threads, &texts);
    let action_row = draft
        .iter()
        .position(|line| line.contains("Send first message"))
        .unwrap();
    let panel_end = draft
        .iter()
        .position(|line| line.contains("Refresh recipient"))
        .unwrap();
    let mut pending_layout = draft.clone();
    pending_layout[action_row] = "Send first message…".into();
    // While sending, the recipient preview is frozen to the exact packet recipient.
    pending_layout.splice(
        slot + 1..=panel_end,
        ["Original delivery to Atlas is pending or unconfirmed. Retrying keeps this exact recipient.".to_owned()],
    );
    assert_eq!(
        queued, pending_layout,
        "only the action row and the frozen recipient line change while sending; draft identity, composer and sibling order stay fixed"
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
    assert_eq!(
        accepted[slot].trim_end(),
        "▌▾ Worker test · 1 message · unread"
    );
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
    threads.failure(context.clone(), None, "Ask access expired".into(), true);
    let refused = lines(&threads, &texts);
    assert!(refused.iter().any(|line| line == "Ask access expired"));
    assert!(!refused.iter().any(|line| line.contains("Worker test")));
    // A refused collection read is connection-level: it keeps the shared wording and never
    // claims access to one thread ended.
    threads.failure(
        context.clone(),
        Some(fractal_client::ErrorCode::Forbidden),
        "Ask access expired".into(),
        true,
    );
    assert_eq!(
        threads.failures[&context],
        "not allowed: Ask access expired"
    );
    assert!(
        !lines(&threads, &texts)
            .iter()
            .any(|line| line.contains("no longer have access"))
    );
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
fn enter_that_queues_nothing_says_why_and_keeps_the_draft() {
    let mut threads = observed_threads(ThreadState::Open);
    let pane = Pane::Thread("thread/worker".into());
    let cursor = edit::Cursor::default();
    let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
    let mut texts = HashMap::new();
    threads.tap(
        &pane.key(),
        thread::Tap::Compose("thread/worker".into()),
        &mut texts,
    );
    assert!(threads.key(&pane, enter, &cursor, &mut texts));
    assert!(threads.writes().is_empty());
    assert_eq!(threads.take_refusal().as_deref(), Some(NOTHING_TO_SEND));
    assert_eq!(threads.take_refusal(), None, "said once");

    threads.paste(&pane, "First reply", &cursor, &mut texts);
    assert!(threads.key(&pane, enter, &cursor, &mut texts));
    assert_eq!(threads.writes().len(), 1);
    assert_eq!(
        threads.take_refusal(),
        None,
        "a queued reply is not refused"
    );
    assert!(threads.key(&pane, enter, &cursor, &mut texts));
    assert!(threads.writes().is_empty());
    assert_eq!(
        threads.take_refusal().as_deref(),
        Some("The original action is still being sent")
    );

    // Before the thread's summary has loaded, the reply has no context yet.
    let mut loading = Threads::default();
    enable_editing(&mut loading);
    let early = Input::Reply("thread/loading".into());
    texts.insert(early.key(), "Too early".into());
    loading.drafts.entry(early.key()).or_default();
    loading.submit(&pane.key(), early.clone(), &texts);
    assert!(loading.writes().is_empty());
    assert_eq!(
        loading.take_refusal().as_deref(),
        Some("The thread context is not ready")
    );
    assert_eq!(texts[&early.key()], "Too early", "the draft is kept");
}

#[test]
fn the_shell_flashes_a_refused_thread_send_as_a_warning() {
    let mut ui = super::super::Ui::new(super::super::demo::test_world());
    ui.world.link = super::super::Link::Live;
    ui.threads = observed_threads(ThreadState::Open);
    let pane = Pane::Thread("thread/worker".into());
    ui.open_in_glass(pane.clone(), super::super::glass::Open::Tab);
    let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
    ui.click(Hit::Thread(
        pane.key(),
        thread::Tap::Compose("thread/worker".into()),
    ));
    ui.key(enter);
    let said = |ui: &super::super::Ui| ui.flash.as_ref().map(|(text, _)| text.clone());
    assert_eq!(said(&ui).as_deref(), Some(NOTHING_TO_SEND));
    assert_eq!(ui.flash_severity, Some(super::super::Severity::Warn));
    super::super::flash::capture_shell(&ui, "p1-6-empty-enter-warn");

    ui.flash = None;
    ui.paste("Please check the worker test.".into());
    ui.key(enter);
    assert_eq!(ui.threads.writes().len(), 1);
    assert_eq!(said(&ui), None, "a queued reply flashes no refusal");
    ui.key(enter);
    assert_eq!(
        said(&ui).as_deref(),
        Some("The original action is still being sent")
    );
    assert_eq!(ui.flash_severity, Some(super::super::Severity::Warn));
    super::super::flash::capture_shell(&ui, "p1-6-pending-enter-warn");

    // The Send button reaches the same refusal as Enter.
    ui.flash = None;
    ui.click(Hit::Thread(
        pane.key(),
        thread::Tap::Send("thread/worker".into()),
    ));
    assert_eq!(
        said(&ui).as_deref(),
        Some("The original action is still being sent")
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
            origin(context.clone(), Some("agent/atlas"), "Review"),
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
        origin(context.clone(), Some("agent/atlas"), "Review"),
    );
    install_start_context(
        &mut threads,
        item,
        "agent/atlas",
        fractal_client::ThreadStartRecipientBasis::Original,
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
            origin(context.clone(), Some("agent/atlas"), "Review"),
        );
        install_start_context(
            &mut threads,
            item,
            "agent/atlas",
            fractal_client::ThreadStartRecipientBasis::Original,
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
            origin(
                ThreadInterest::Inbox {
                    source: "agent/example".into(),
                    episode: "claim/feedback".into(),
                },
                Some("agent/atlas"),
                "Review",
            ),
        );
        install_start_context(
            &mut threads,
            item,
            "agent/atlas",
            fractal_client::ThreadStartRecipientBasis::Original,
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
    // A step-run feedback source has no historical source agent; its recipient is canonical.
    threads.origins.insert(
        item.into(),
        origin(
            ThreadInterest::Inbox {
                source: "step-run/feedback".into(),
                episode: "claim/feedback".into(),
            },
            None,
            "Review",
        ),
    );
    install_start_context(
        &mut threads,
        item,
        "agent/atlas",
        fractal_client::ThreadStartRecipientBasis::Configured,
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
    assert_eq!(start.agent_id, "agent/atlas");
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
        let source = ThreadInterest::Inbox {
            source: "agent/example".into(),
            episode: "claim/notice".into(),
        };
        threads.origins.insert(
            "attention/notice".into(),
            origin(source, Some("agent/atlas"), "Notice"),
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
        // An agent's conversation types bare letters, so its thread actions take Alt there.
        let chord = if matches!(pane, Pane::Agent(_)) {
            KeyModifiers::ALT
        } else {
            KeyModifiers::NONE
        };
        assert!(
            matches!(threads.shortcut(&pane, KeyEvent::new(KeyCode::Char('z'), chord)), Some(thread::Tap::Close(id)) if id == first),
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
    threads.failure(timeline.clone(), None, THREAD_UPDATES_PAUSED.into(), false);
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
    assert!(paused.contains(PAUSED_NOTICE));
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
    assert!(!resumed.contains(PAUSED_NOTICE));
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
    let close = || KeyEvent::new(KeyCode::Char('z'), KeyModifiers::ALT);
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
            doc.lines[target.line].to_string().contains(" alt+z "),
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
fn ordinary_agent_composer_keeps_thread_shortcut_letters_as_text() {
    let mut ui = super::super::Ui::new(super::super::demo::test_world());
    let agent = "agent/example/atlas/builder";
    let pane = Pane::Agent(Some(agent.into()));
    ui.open_in_glass(pane.clone(), super::super::glass::Open::Tab);
    ui.threads = observed_threads(ThreadState::Open);
    ui.threads.set_access(ThreadAccess {
        mark_read: true,
        ..ui.threads.access
    });
    ui.threads
        .open
        .insert(pane.key(), ["thread/worker".into()].into());
    ui.threads
        .selected
        .insert(pane.key(), "thread/worker".into());
    ui.click(Hit::Composer);
    ui.paste("Please ".into());
    for letter in "zinc never help".chars() {
        ui.key(KeyEvent::new(KeyCode::Char(letter), KeyModifiers::NONE));
    }
    assert!(
        ui.threads.writes().is_empty(),
        "typing in the ordinary composer must not close or mark a selected thread"
    );
    assert!(
        ui.threads.focused.is_none(),
        "typing n must not open the selected thread's rename editor"
    );
    assert_eq!(
        ui.conversation_state.drafts[agent],
        "Please zinc never help"
    );
}

#[test]
fn expanding_a_writable_thread_transfers_composer_ownership_without_losing_either_draft() {
    let mut ui = super::super::Ui::new(super::super::demo::test_world());
    let agent = "agent/example/atlas/builder";
    let pane = Pane::Agent(Some(agent.into()));
    ui.open_in_glass(pane.clone(), super::super::glass::Open::Tab);
    ui.threads = observed_threads(ThreadState::Open);
    ui.click(Hit::Composer);
    ui.paste("Keep the ordinary draft.".into());
    ui.click(Hit::Thread(
        pane.key(),
        thread::Tap::Toggle("thread/worker".into()),
    ));
    assert!(!ui.editing, "the visible thread composer now owns the keys");
    for letter in "zinc never help".chars() {
        ui.key(KeyEvent::new(KeyCode::Char(letter), KeyModifiers::NONE));
    }
    assert_eq!(
        ui.conversation_state.drafts[agent],
        "Keep the ordinary draft."
    );
    assert_eq!(
        ui.conversation_state.drafts[&Input::Reply("thread/worker".into()).key()],
        "zinc never help"
    );
    assert!(
        ui.threads.writes().is_empty(),
        "typing queues no thread actions"
    );
}

#[test]
fn conversation_find_keeps_selected_thread_shortcut_letters_as_search_text() {
    let mut ui = super::super::Ui::new(super::super::demo::test_world());
    let agent = "agent/example/atlas/builder";
    let pane = Pane::Agent(Some(agent.into()));
    ui.open_in_glass(pane.clone(), super::super::glass::Open::Tab);
    ui.threads = observed_threads(ThreadState::Open);
    ui.threads
        .open
        .insert(pane.key(), ["thread/worker".into()].into());
    ui.threads.set_access(ThreadAccess {
        mark_read: true,
        ..ui.threads.access
    });
    ui.threads
        .selected
        .insert(pane.key(), "thread/worker".into());
    ui.find_in(agent, "");
    for letter in "zinc never help".chars() {
        ui.key(KeyEvent::new(KeyCode::Char(letter), KeyModifiers::NONE));
    }
    assert!(
        ui.threads.writes().is_empty(),
        "search never mutates threads"
    );
    assert!(
        ui.threads.focused.is_none(),
        "search never opens a rename editor"
    );
    assert_eq!(ui.find.as_ref().unwrap().query, "zinc never help");
    assert_eq!(
        ui.main_pane(),
        pane,
        "p in a search must not open another tab"
    );
}

#[test]
fn selected_thread_alt_chords_still_dispatch_from_agent_list_focus() {
    for (letter, state) in [
        ('n', ThreadState::Open),
        ('z', ThreadState::Open),
        ('z', ThreadState::Closed),
    ] {
        let mut ui = super::super::Ui::new(super::super::demo::test_world());
        let pane = Pane::Agent(Some("agent/example/atlas/builder".into()));
        ui.open_in_glass(pane.clone(), super::super::glass::Open::Tab);
        let was_open = matches!(state, ThreadState::Open);
        ui.threads = observed_threads(state);
        ui.threads
            .open
            .insert(pane.key(), ["thread/worker".into()].into());
        ui.threads
            .selected
            .insert(pane.key(), "thread/worker".into());
        assert!(!ui.editing);
        ui.key(KeyEvent::new(KeyCode::Char(letter), KeyModifiers::ALT));
        if letter == 'n' {
            assert!(matches!(ui.threads.focused, Some((_, Input::Rename(_)))));
        } else {
            let write = ui
                .threads
                .writes()
                .pop()
                .expect("alt+z lifecycle dispatch from list focus");
            assert!(matches!(
                (was_open, write.mutation),
                (true, Mutation::Close) | (false, Mutation::Reopen)
            ));
        }
    }
}

/// A writable thread, open and selected in an agent's conversation or open as its own tab,
/// in a fresh Ui with nothing being typed.
fn writable_thread_in(pane: &Pane) -> super::super::Ui {
    let mut ui = super::super::Ui::new(super::super::demo::test_world());
    ui.open_in_glass(pane.clone(), super::super::glass::Open::Tab);
    ui.threads = observed_threads(ThreadState::Open);
    ui.threads.set_access(ThreadAccess {
        mark_read: true,
        ..ui.threads.access
    });
    if matches!(pane, Pane::Agent(_)) {
        ui.threads
            .open
            .insert(pane.key(), ["thread/worker".into()].into());
        ui.threads
            .selected
            .insert(pane.key(), "thread/worker".into());
    }
    assert!(!ui.editing && ui.threads.focused.is_none());
    ui
}

#[test]
fn agent_and_thread_tab_bare_action_letters_type_instead_of_acting() {
    let agent = "agent/example/atlas/builder";
    let id = "thread/worker";
    let conversation = Pane::Agent(Some(agent.into()));
    let mut ui = writable_thread_in(&conversation);
    for letter in "znvpr".chars() {
        ui.key(KeyEvent::new(KeyCode::Char(letter), KeyModifiers::NONE));
    }
    assert!(
        ui.threads.writes().is_empty(),
        "bare letters queue no thread action"
    );
    assert!(
        ui.threads.focused.is_none(),
        "a bare n opens no rename editor"
    );
    assert_eq!(ui.main_pane(), conversation, "a bare p opens no tab");
    assert_eq!(
        ui.conversation_state.drafts[agent], "znvpr",
        "every letter starts the agent message"
    );

    let tab = Pane::Thread(id.into());
    let mut ui = writable_thread_in(&tab);
    for letter in "znvpr".chars() {
        ui.key(KeyEvent::new(KeyCode::Char(letter), KeyModifiers::NONE));
    }
    assert!(
        ui.threads.writes().is_empty(),
        "bare letters queue no thread action"
    );
    assert!(
        matches!(&ui.threads.focused, Some((host, Input::Reply(thread))) if host == &tab.key() && thread == id),
        "the first letter opens the tab's reply box, never its rename editor"
    );
    assert_eq!(ui.main_pane(), tab);
    assert_eq!(
        ui.conversation_state.drafts[&Input::Reply(id.into()).key()],
        "znvpr"
    );
}

#[test]
fn agent_and_thread_tab_thread_actions_dispatch_on_alt_chords() {
    let id = "thread/worker";
    let alt = |letter| KeyEvent::new(KeyCode::Char(letter), KeyModifiers::ALT);
    for pane in [
        Pane::Agent(Some("agent/example/atlas/builder".into())),
        Pane::Thread(id.into()),
    ] {
        let mut ui = writable_thread_in(&pane);
        ui.key(alt('z'));
        let close = ui.threads.writes().pop().expect("alt+z closes the thread");
        assert!(matches!(close.mutation, Mutation::Close), "{pane:?}");

        let mut ui = writable_thread_in(&pane);
        ui.key(alt('n'));
        assert!(
            matches!(&ui.threads.focused, Some((host, Input::Rename(thread))) if host == &pane.key() && thread == id),
            "alt+n opens the rename editor in {pane:?}"
        );

        let ui = writable_thread_in(&pane);
        assert_eq!(
            ui.threads.shortcut(&pane, alt('v')),
            Some(thread::Tap::MarkVisibleRead(id.into())),
            "alt+v marks the visible replies read in {pane:?}"
        );

        // In an agent alt+p opens the thread's tab; in that tab it has nothing to open.
        let mut ui = writable_thread_in(&pane);
        ui.key(alt('p'));
        assert_eq!(ui.main_pane(), Pane::Thread(id.into()), "{pane:?}");
        assert!(ui.threads.writes().is_empty());
    }
}

#[test]
fn an_active_message_box_keeps_alt_chords_from_thread_actions() {
    let id = "thread/worker";
    let conversation = Pane::Agent(Some("agent/example/atlas/builder".into()));
    let mut ui = writable_thread_in(&conversation);
    ui.click(Hit::Composer);
    ui.key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::ALT));
    assert!(
        ui.threads.writes().is_empty(),
        "the agent's message box keeps the keys"
    );

    let tab = Pane::Thread(id.into());
    let mut ui = writable_thread_in(&tab);
    ui.click(Hit::Thread(tab.key(), thread::Tap::Compose(id.into())));
    for letter in ['z', 'n', 'v', 'r'] {
        ui.key(KeyEvent::new(KeyCode::Char(letter), KeyModifiers::ALT));
    }
    assert!(
        ui.threads.writes().is_empty(),
        "the reply box keeps the keys"
    );
    assert!(matches!(&ui.threads.focused, Some((_, Input::Reply(_)))));
}

#[test]
fn inbox_thread_detail_keeps_bare_action_letters() {
    let id = "thread/worker";
    let inbox = Pane::Home(Some(id.into()));
    let mut threads = observed_threads(ThreadState::Open);
    threads.set_access(ThreadAccess {
        mark_read: true,
        ..threads.access
    });
    let cases = [
        ('z', thread::Tap::Close as fn(String) -> thread::Tap),
        ('n', thread::Tap::Rename),
        ('v', thread::Tap::MarkVisibleRead),
        ('p', thread::Tap::OpenTab),
    ];
    for (letter, tap) in cases {
        assert_eq!(
            threads.shortcut(
                &inbox,
                KeyEvent::new(KeyCode::Char(letter), KeyModifiers::NONE)
            ),
            Some(tap(id.into())),
            "a bare {letter} acts in an Inbox item"
        );
        assert_eq!(
            threads.shortcut(
                &inbox,
                KeyEvent::new(KeyCode::Char(letter), KeyModifiers::ALT)
            ),
            None,
            "alt+{letter} is not an Inbox thread key"
        );
    }
    let close = threads
        .shortcut(
            &inbox,
            KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE),
        )
        .unwrap();
    threads.tap(&inbox.key(), close, &mut HashMap::new());
    let write = threads
        .writes()
        .pop()
        .expect("a bare z closes from the Inbox");
    assert!(matches!(write.mutation, Mutation::Close));
}

#[test]
fn retry_takes_alt_where_letters_type_and_stays_bare_in_the_inbox() {
    let id = "thread/worker";
    let agent = Pane::Agent(Some("agent/atlas".into()));
    let tab = Pane::Thread(id.into());
    let inbox = Pane::Home(Some(id.into()));
    let mut threads = observed_threads(ThreadState::Open);
    let mut texts = HashMap::new();
    threads.lifecycle(&tab.key(), id.into(), Mutation::Close);
    let original = threads.writes().pop().unwrap();
    threads.complete(
        Receipt {
            write: original.clone(),
            confirmation: Confirmation::Failed(ClientError::Transport(
                "lost acknowledgement".into(),
            )),
        },
        &mut texts,
    );
    threads.open.insert(agent.key(), [id.into()].into());
    threads.selected.insert(agent.key(), id.into());
    let retry = Some(thread::Tap::Retry(original.key.clone()));
    let r = |modifiers| KeyEvent::new(KeyCode::Char('r'), modifiers);
    for pane in [&agent, &tab] {
        assert_eq!(
            threads.shortcut(pane, r(KeyModifiers::NONE)),
            None,
            "a bare r types in {pane:?}"
        );
        assert_eq!(
            threads.shortcut(pane, r(KeyModifiers::ALT)),
            retry,
            "alt+r retries in {pane:?}"
        );
    }
    assert_eq!(threads.shortcut(&inbox, r(KeyModifiers::NONE)), retry);
    assert_eq!(threads.shortcut(&inbox, r(KeyModifiers::ALT)), None);
    let doc = threads.mount(
        &tab,
        100,
        &edit::Cursor::default(),
        &super::super::demo::test_world(),
        &texts,
    );
    let button = doc
        .lines
        .iter()
        .map(ToString::to_string)
        .find(|line| line.contains("Retry Close thread"))
        .unwrap();
    assert!(button.contains(" alt+r "), "{button}");
}

/// An agent conversation with two tool blocks and, under them, two listed threads.
fn agent_with_tools_and_threads() -> (super::super::Ui, Pane, &'static str) {
    use super::super::view::{Body, Entry, Load, ToolState};
    let mut ui = super::super::Ui::new(super::super::demo::test_world());
    let agent = "agent/example/atlas/builder";
    let pane = Pane::Agent(Some(agent.into()));
    ui.open_in_glass(pane.clone(), super::super::glass::Open::Tab);
    let tool = |id: &str| Entry {
        id: id.into(),
        at: "10:00".into(),
        body: Body::Tool {
            title: id.into(),
            state: ToolState::Ok,
            output: vec![],
        },
    };
    ui.world
        .conversations
        .insert(agent.into(), Load::Ready(vec![tool("one"), tool("two")]));
    ui.threads = observed_threads(ThreadState::Open);
    ui.threads.window(
        ThreadInterest::Agent(agent.into()),
        snapshot(),
        true,
        ThreadCollectionRows::Summaries(vec![
            summary(),
            ThreadSummary {
                id: "thread/sibling".into(),
                title: "Sibling".into(),
                ..summary()
            },
        ]),
        vec![],
        vec!["thread/worker".into(), "thread/sibling".into()],
        false,
    );
    ui.content.focused.insert(agent.into(), "one".into());
    (ui, pane, agent)
}

fn ctrl(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::CONTROL)
}

#[test]
fn ctrl_arrows_walk_tools_then_thread_headings_in_screen_order() {
    let (mut ui, pane, agent) = agent_with_tools_and_threads();
    let focus = |ui: &super::super::Ui| {
        (
            ui.content.focused.get(agent).cloned(),
            ui.threads.focused_row(&pane, &ui.conversation_state.drafts),
        )
    };
    let tool = |id: &str| (Some(id.to_owned()), None);
    let row = |id: &str| (None, Some(id.to_owned()));
    let mut seen = vec![focus(&ui)];
    for _ in 0..4 {
        ui.key(ctrl(KeyCode::Down));
        seen.push(focus(&ui));
    }
    assert_eq!(
        seen,
        [
            tool("one"),
            tool("two"),
            row("thread/worker"),
            row("thread/sibling"),
            row("thread/sibling")
        ],
        "tools first, then thread headings, stopping at the last"
    );
    assert_eq!(
        ui.content.scroll_to.borrow().get(agent).map(String::as_str),
        Some("thread/sibling"),
        "a focused heading is scrolled into view like a focused tool"
    );
    let collapsed = ui.threads.mount(
        &pane,
        100,
        &edit::Cursor::default(),
        &ui.world,
        &ui.conversation_state.drafts,
    );
    let lines: Vec<String> = collapsed.lines.iter().map(ToString::to_string).collect();
    assert!(
        lines.iter().any(|line| line.starts_with("▌▸ Sibling")),
        "{lines:#?}"
    );
    assert_eq!(lines.iter().filter(|line| line.starts_with('▌')).count(), 1);
    for expected in [row("thread/worker"), tool("two"), tool("one"), tool("one")] {
        ui.key(ctrl(KeyCode::Up));
        assert_eq!(focus(&ui), expected);
    }
    assert!(
        ui.threads.writes().is_empty(),
        "moving focus changes no thread"
    );
}

#[test]
fn ctrl_enter_toggles_the_focused_thread_and_keeps_the_heading_focused() {
    let (mut ui, pane, _) = agent_with_tools_and_threads();
    ui.key(ctrl(KeyCode::Down));
    ui.key(ctrl(KeyCode::Down));
    assert_eq!(
        ui.threads
            .focused_row(&pane, &ui.conversation_state.drafts)
            .as_deref(),
        Some("thread/worker")
    );
    ui.key(ctrl(KeyCode::Enter));
    assert!(ui.threads.open[&pane.key()].contains("thread/worker"));
    assert!(
        ui.threads.focused.is_none(),
        "expanding by keyboard keeps the keys off the reply box"
    );
    assert!(!ui.editing);
    assert!(
        !ui.conversation_state.expanded.contains("two"),
        "the tool above stays as it was"
    );
    ui.key(ctrl(KeyCode::Enter));
    assert!(
        !ui.threads
            .open
            .get(&pane.key())
            .is_some_and(|open| open.contains("thread/worker"))
    );
    assert_eq!(
        ui.threads
            .focused_row(&pane, &ui.conversation_state.drafts)
            .as_deref(),
        Some("thread/worker"),
        "collapsing leaves the heading focused, so Ctrl+Enter expands it again"
    );
    assert!(ui.threads.writes().is_empty());
}

#[test]
fn thread_chords_act_on_the_keyboard_focused_heading_even_collapsed() {
    let (mut ui, pane, _) = agent_with_tools_and_threads();
    for _ in 0..3 {
        ui.key(ctrl(KeyCode::Down));
    }
    assert_eq!(
        ui.threads
            .focused_row(&pane, &ui.conversation_state.drafts)
            .as_deref(),
        Some("thread/sibling")
    );
    assert!(
        !ui.threads.open.contains_key(&pane.key()),
        "the focused heading is collapsed"
    );
    ui.key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::ALT));
    let write = ui
        .threads
        .writes()
        .pop()
        .expect("alt+z closes the focused thread");
    assert_eq!(write.id, "thread/sibling");
    assert!(matches!(write.mutation, Mutation::Close));
    ui.key(ctrl(KeyCode::Up));
    ui.key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::ALT));
    assert!(
        matches!(&ui.threads.focused, Some((_, Input::Rename(id))) if id == "thread/worker"),
        "alt+n renames the newly focused heading, not the one before it"
    );
}

#[test]
fn ctrl_arrows_leave_message_boxes_without_losing_their_drafts() {
    let (mut ui, pane, agent) = agent_with_tools_and_threads();
    ui.click(Hit::Composer);
    ui.paste("Keep the ordinary draft".into());
    assert!(ui.editing);
    ui.key(ctrl(KeyCode::Down));
    assert!(!ui.editing, "the composer lets go of the keys");
    assert_eq!(
        ui.conversation_state.drafts[agent],
        "Keep the ordinary draft"
    );
    ui.key(ctrl(KeyCode::Down));
    assert_eq!(
        ui.threads
            .focused_row(&pane, &ui.conversation_state.drafts)
            .as_deref(),
        Some("thread/worker")
    );
    ui.key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::ALT));
    assert_eq!(
        ui.threads.writes().pop().map(|write| write.id).as_deref(),
        Some("thread/worker")
    );
    assert_eq!(
        ui.conversation_state.drafts[agent], "Keep the ordinary draft",
        "the chord typed nothing"
    );

    // A thread reply box lets go the same way and keeps its own draft.
    ui.click(Hit::Thread(
        pane.key(),
        thread::Tap::Compose("thread/worker".into()),
    ));
    ui.paste("Keep the reply".into());
    assert!(ui.threads.editing_in(&pane));
    ui.key(ctrl(KeyCode::Down));
    assert!(!ui.threads.editing_in(&pane));
    assert_eq!(
        ui.threads
            .focused_row(&pane, &ui.conversation_state.drafts)
            .as_deref(),
        Some("thread/sibling")
    );
    assert_eq!(
        ui.conversation_state.drafts[&Input::Reply("thread/worker".into()).key()],
        "Keep the reply"
    );
    assert_eq!(
        ui.conversation_state.drafts[agent],
        "Keep the ordinary draft"
    );

    // A letter goes back to the ordinary composer, as before.
    ui.key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE));
    assert!(ui.editing);
    assert!(ui.conversation_state.drafts[agent].starts_with("Keep the ordinary draft"));
    assert!(ui.conversation_state.drafts[agent].contains('z'));
    assert!(ui.threads.writes().is_empty(), "a typed z closes nothing");
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
        origin(context.clone(), Some("agent/atlas"), "Notice"),
    );
    install_start_context(
        &mut threads,
        item,
        "agent/atlas",
        fractal_client::ThreadStartRecipientBasis::Original,
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
        origin(
            ThreadInterest::Inbox {
                source: "agent/example".into(),
                episode: "claim/fault".into(),
            },
            Some("agent/atlas"),
            "Fault",
        ),
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
            origin(
                ThreadInterest::Inbox {
                    source: "agent/example".into(),
                    episode: "claim/feedback".into(),
                },
                Some("agent/atlas"),
                "Review",
            ),
        );
        // "missing-recipient": the exact source is known but its canonical recipient preview
        // has not loaded, so nothing may be prepared or sent.
        if scenario != "missing-recipient" {
            install_start_context(
                &mut threads,
                item,
                "agent/atlas",
                fractal_client::ThreadStartRecipientBasis::Original,
            );
        }
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
            "missing-recipient" => {}
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
        origin(
            ThreadInterest::Inbox {
                source: "agent/example".into(),
                episode: "claim/feedback".into(),
            },
            Some("agent/atlas"),
            "Review",
        ),
    );
    install_start_context(
        &mut threads,
        item,
        "agent/atlas",
        fractal_client::ThreadStartRecipientBasis::Original,
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
    assert_eq!(parameters.agent_id, "agent/atlas");
    assert_eq!(parameters.content, "Feedback");
}

#[test]
fn retained_start_keeps_its_captured_context_through_replacement_and_removal() {
    let mut threads = observed_threads(ThreadState::Open);
    let item = "attention/notice";
    threads.origins.insert(
        item.into(),
        origin(
            ThreadInterest::Inbox {
                source: "agent/example".into(),
                episode: "claim/source".into(),
            },
            Some("agent/atlas"),
            "Fault context",
        ),
    );
    install_start_context(
        &mut threads,
        item,
        "agent/atlas",
        fractal_client::ThreadStartRecipientBasis::Original,
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
        origin(replacement, Some("agent/replacement"), "Replacement title"),
    );
    let home = Pane::Home(Some(item.into()));
    assert_eq!(threads.context(&home), Some(captured.clone()));
    assert!(
        matches!(threads.start_input(item), Some(Input::Start { episode, .. }) if episode == "claim/source")
    );
    // A pending packet is immutable: no new preview is read for it, whatever the card says now.
    threads.request_start_context(item);
    assert!(threads.take_start_context_reads().is_empty());
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
            rendered.contains("Fault context · New discussion · Draft"),
            "{rendered}"
        );
        assert!(
            rendered.contains("Original delivery to agent/atlas is pending or unconfirmed"),
            "{rendered}"
        );
        assert!(!rendered.contains("Replacement title"), "{rendered}");
        assert!(!rendered.contains("agent/replacement"), "{rendered}");
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
    assert!(doc.lines.iter().any(|line| {
        line.to_string()
            .contains("Fault context · New discussion · Draft")
    }));
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

/// Preview-only evidence. These are in-process production mounts, NOT the production binary
/// transport path. Run with THREAD_PREVIEW_CAPTURE_DIR and --ignored --nocapture.
#[test]
#[ignore = "writes explicitly requested preview evidence"]
fn thread_preview_evidence() {
    use ratatui::{buffer::Buffer, layout::Rect};
    let dir = std::path::PathBuf::from(
        std::env::var("THREAD_PREVIEW_CAPTURE_DIR").expect("THREAD_PREVIEW_CAPTURE_DIR"),
    );
    std::fs::create_dir_all(&dir).unwrap();
    let mut ui = super::super::Ui::new(super::super::demo::test_world());
    ui.threads = observed_threads(ThreadState::Open);
    let pane = Pane::Thread("thread/worker".into());
    ui.open_in_glass(pane.clone(), super::super::glass::Open::Tab);
    fn capture(
        ui: &mut super::super::Ui,
        pane: &Pane,
        dir: &std::path::Path,
        name: &str,
    ) -> String {
        let area = Rect::new(0, 0, 120, 40);
        let mut screen = Buffer::empty(area);
        ui.draw_pane(&mut screen, area, pane);
        let mut text =
            String::from("HARNESS, NOT PRODUCTION BINARY PATH — production Ui::draw_pane\n");
        for row in screen.content.chunks(120) {
            text.push_str(&row.iter().map(|cell| cell.symbol()).collect::<String>());
            text.push('\n');
        }
        std::fs::write(dir.join(format!("{name}.txt")), &text).unwrap();
        text
    }
    // This preview uses the ordinary wall-clock renderer, not the fixed unit-test clock.
    // Keep its message on that same clock so captured relative times remain realistic.
    let accepted_at = chrono::Utc::now().timestamp_millis() - 120_000;
    let message: ThreadTimelineEvent = serde_json::from_value(serde_json::json!({
        "id":"message/preview", "actor_id":"agent/atlas", "accepted_at_unix_ms":accepted_at,
        "kind":"message", "content":"Please review the sensor calibration.", "thread_quote_id":null
    }))
    .unwrap();
    ui.threads.window(
        ThreadInterest::Timeline("thread/worker".into()),
        snapshot(),
        true,
        ThreadCollectionRows::Timeline(vec![message]),
        vec![],
        vec!["message/preview".into()],
        false,
    );
    ui.threads.hover(Some("message/preview"));
    let hovered = capture(&mut ui, &pane, &dir, "harness-hover-reply");
    assert!(hovered.contains("↩ Reply"));
    assert!(
        hovered.contains("2m ago"),
        "preview message uses the rendering clock: {hovered}"
    );
    ui.threads.set_access(ThreadAccess {
        mark_read: true,
        ..ui.threads.access
    });
    capture(&mut ui, &pane, &dir, "harness-v-before");
    ui.key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::ALT));
    let read = ui
        .threads
        .writes()
        .pop()
        .expect("alt+v dispatches visible-message read");
    assert!(matches!(read.mutation, Mutation::MarkRead(_)));
    capture(&mut ui, &pane, &dir, "harness-v-pending");
    // Dispatch-only fixture: settle its retained action before the independent lifecycle
    // scenarios below. This is a simulated rejection, not an accepted server-side read.
    assert_eq!(
        ui.threads
            .complete(
                Receipt {
                    write: read,
                    confirmation: Confirmation::Rejected
                },
                &mut ui.conversation_state.drafts,
            )
            .as_deref(),
        Some("The original thread action was rejected"),
    );
    ui.click(Hit::Thread(
        pane.key(),
        thread::Tap::Compose("thread/worker".into()),
    ));
    ui.paste("Keep this unsent draft.".into());
    let focused = capture(&mut ui, &pane, &dir, "harness-focused-composer");
    assert!(focused.contains("Keep this unsent draft.") && focused.contains("Send"));
    ui.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    ui.click(Hit::Thread(
        pane.key(),
        thread::Tap::Rename("thread/worker".into()),
    ));
    assert!(capture(&mut ui, &pane, &dir, "harness-rename").contains("Worker test"));
    ui.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    ui.click(Hit::Thread(
        pane.key(),
        thread::Tap::Close("thread/worker".into()),
    ));
    let close = ui.threads.writes().pop().expect("production close action");
    assert!(matches!(close.mutation, Mutation::Close));
    capture(&mut ui, &pane, &dir, "harness-close-pending");

    let item = "attention/preview";
    let inbox = Pane::Home(Some(item.into()));
    let context = ThreadInterest::Inbox {
        source: "agent/atlas".into(),
        episode: "claim/preview".into(),
    };
    ui.threads.origins.insert(
        item.into(),
        origin(context.clone(), Some("agent/atlas"), "Calibration review"),
    );
    install_start_context(
        &mut ui.threads,
        item,
        "agent/atlas",
        fractal_client::ThreadStartRecipientBasis::Original,
    );
    ui.threads.lists.insert(context.clone(), Vec::new());
    ui.open_in_glass(inbox.clone(), super::super::glass::Open::Tab);
    capture(&mut ui, &inbox, &dir, "harness-start-before");
    ui.click(Hit::Thread(inbox.key(), thread::Tap::Start(item.into())));
    ui.paste("First message stays inline before sending.".into());
    assert!(
        capture(&mut ui, &inbox, &dir, "harness-inline-first-message")
            .contains("First message stays inline")
    );
    ui.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    let canonical = Input::Start {
        item: item.into(),
        episode: "claim/preview".into(),
    }
    .key();
    ui.conversation_state
        .drafts
        .insert(format!("chat:{item}"), "Different recovered draft".into());
    let before = ui.conversation_state.drafts.clone();
    ui.click(Hit::Thread(inbox.key(), thread::Tap::Start(item.into())));
    assert_eq!(ui.conversation_state.drafts, before);
    assert!(
        capture(&mut ui, &inbox, &dir, "harness-draft-collision").contains("Both drafts are kept")
    );
    ui.click(Hit::Thread(
        inbox.key(),
        thread::Tap::CombineSavedDrafts(item.into()),
    ));
    assert!(ui.conversation_state.drafts[&canonical].contains("Different recovered draft"));
    assert!(ui.conversation_state.drafts[&canonical].contains("First message stays inline"));
    capture(&mut ui, &inbox, &dir, "harness-draft-recovered");
    assert!(ui.threads.writes().is_empty(), "draft recovery never sends");
    ui.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let start = ui
        .threads
        .writes()
        .pop()
        .expect("first-message start dispatch");
    let Mutation::Start(parameters) = start.mutation else {
        panic!("start mutation")
    };
    assert_eq!(parameters.agent_id, "agent/atlas");
    assert!(parameters.content.contains("Different recovered draft"));
    assert!(parameters.content.contains("First message stays inline"));
    capture(&mut ui, &inbox, &dir, "harness-start-pending");
    println!("HARNESS evidence: {}", dir.display());
}

#[test]
fn lifecycle_success_flashes_only_after_server_confirmation_with_host_keys() {
    for pane in [
        Pane::Agent(Some("agent/example/atlas/builder".into())),
        Pane::Thread("thread/worker".into()),
        Pane::Home(Some("thread/worker".into())),
    ] {
        for action in ["close", "reopen", "rename"] {
            let mut ui = writable_thread_in(&pane);
            if action == "reopen" {
                ui.threads = observed_threads(ThreadState::Closed);
            }
            let title = ui.threads.summary("thread/worker").unwrap().title.clone();
            let tap = match action {
                "close" => thread::Tap::Close("thread/worker".into()),
                "reopen" => thread::Tap::Reopen("thread/worker".into()),
                _ => thread::Tap::Rename("thread/worker".into()),
            };
            ui.click(Hit::Thread(pane.key(), tap));
            if action == "rename" {
                ui.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
                ui.paste("A clearer title".into());
                ui.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
            }
            let write = ui.threads.writes().pop().expect("lifecycle action queued");
            assert!(ui.flash.is_none(), "no optimistic success");
            let repeated = write.clone();
            ui.complete_thread(Receipt {
                write,
                confirmation: Confirmation::Applied(None),
            });
            let expected = match action {
                "close" => format!(
                    "Closed “{title}” · {} reopens",
                    if matches!(pane, Pane::Home(_)) {
                        "z"
                    } else {
                        "alt+z"
                    }
                ),
                "reopen" => format!("Reopened “{title}”"),
                _ => "Renamed to “A clearer title”".into(),
            };
            assert_eq!(
                ui.flash.as_ref().map(|(message, _)| message),
                Some(&expected)
            );
            assert_eq!(ui.flash_severity, Some(super::super::Severity::Info));
            ui.flash = None;
            ui.complete_thread(Receipt {
                write: repeated,
                confirmation: Confirmation::Applied(None),
            });
            assert!(
                ui.flash.is_none(),
                "duplicate receipt is not another success"
            );
        }
    }
}

fn rendered_lines(threads: &Threads, pane: &Pane, texts: &HashMap<String, String>) -> Vec<String> {
    threads
        .mount(
            pane,
            100,
            &edit::Cursor::default(),
            &super::super::demo::test_world(),
            texts,
        )
        .lines
        .iter()
        .map(ToString::to_string)
        .collect()
}

/// The headline and st's reason are separate lines, and a refused action never claims that read
/// access ended.
fn assert_refusal_lines(lines: &[String], headline: &str) {
    let at = lines
        .iter()
        .position(|line| line == headline)
        .unwrap_or_else(|| panic!("{headline} is its own line: {lines:#?}"));
    assert_eq!(lines[at + 1], "Grant was revoked", "{lines:#?}");
    assert!(
        !lines
            .iter()
            .any(|line| line.contains("no longer have access") || line.contains("not allowed")),
        "{lines:#?}"
    );
}

#[test]
fn definite_lifecycle_refusals_name_the_action_and_put_the_reason_on_its_own_line() {
    for (state, mutation, headline) in [
        (
            ThreadState::Open,
            Mutation::Close,
            "Couldn't close this thread.",
        ),
        (
            ThreadState::Closed,
            Mutation::Reopen,
            "Couldn't reopen this thread.",
        ),
    ] {
        let mut threads = observed_threads(state);
        let id = "thread/worker";
        let pane = Pane::Thread(id.into());
        let mut texts = HashMap::new();
        threads.lifecycle(&pane.key(), id.into(), mutation.clone());
        let write = threads.writes().pop().unwrap();
        assert_eq!(
            threads.complete(
                Receipt {
                    write,
                    confirmation: Confirmation::Failed(forbidden()),
                },
                &mut texts,
            ),
            Some(headline.into())
        );
        // A definite refusal settles the action: nothing is retained or offered for retry.
        assert!(threads.pending_actions.is_empty());
        let doc = threads.mount(
            &pane,
            100,
            &edit::Cursor::default(),
            &super::super::demo::test_world(),
            &texts,
        );
        assert!(
            !doc.targets
                .iter()
                .any(|target| matches!(&target.hit, Hit::Thread(_, thread::Tap::Retry(_))))
        );
        assert_refusal_lines(&rendered_lines(&threads, &pane, &texts), headline);
        // Acting on the thread again replaces the old refusal.
        threads.lifecycle(&pane.key(), id.into(), mutation);
        assert_eq!(threads.writes().len(), 1);
        assert!(
            !rendered_lines(&threads, &pane, &texts)
                .iter()
                .any(|line| line == headline)
        );
    }
}

#[test]
fn definite_action_refusals_keep_the_server_reason_unprefixed() {
    for code in [
        fractal_client::ErrorCode::Forbidden,
        fractal_client::ErrorCode::NotFound,
    ] {
        let ClientError::Api(_, _, mut envelope) = forbidden() else {
            unreachable!()
        };
        envelope.code = code.clone();
        envelope.message = "The thread is unavailable".into();
        let error = ClientError::Api(code, envelope.message.clone(), envelope);
        let refusal = ActionRefusal::of(&Mutation::Close, &error).unwrap();
        assert_eq!(refusal.headline, "Couldn't close this thread.");
        assert_eq!(refusal.reason, "The thread is unavailable");
    }
}

#[test]
fn a_definitely_refused_rename_keeps_its_title_draft_editable() {
    let mut threads = observed_threads(ThreadState::Open);
    let id = "thread/worker";
    let pane = Pane::Thread(id.into());
    let mut texts = HashMap::new();
    let key = Input::Rename(id.into()).key();
    threads.tap(&pane.key(), thread::Tap::Rename(id.into()), &mut texts);
    texts.insert(key.clone(), "Renamed worker".into());
    threads.tap(&pane.key(), thread::Tap::Rename(id.into()), &mut texts);
    let write = threads.writes().pop().unwrap();
    assert!(matches!(write.mutation, Mutation::Rename(_)));
    assert_eq!(
        threads.complete(
            Receipt {
                write,
                confirmation: Confirmation::Failed(forbidden()),
            },
            &mut texts,
        ),
        Some("Couldn't rename this thread.".into())
    );
    assert!(!threads.drafts[&key].locked());
    assert!(threads.drafts[&key].delivery.is_none());
    assert_eq!(texts[&key], "Renamed worker");
    assert_refusal_lines(
        &rendered_lines(&threads, &pane, &texts),
        "Couldn't rename this thread.",
    );
}

#[test]
fn a_definitely_refused_reply_keeps_its_draft_and_says_the_reply_was_not_sent() {
    let mut threads = observed_threads(ThreadState::Open);
    let id = "thread/worker";
    let pane = Pane::Thread(id.into());
    let mut texts = HashMap::new();
    let key = Input::Reply(id.into()).key();
    threads.tap(&pane.key(), thread::Tap::Compose(id.into()), &mut texts);
    threads.paste(&pane, "Refused reply", &edit::Cursor::default(), &mut texts);
    threads.submit(&pane.key(), Input::Reply(id.into()), &texts);
    let write = threads.writes().pop().unwrap();
    assert_eq!(
        threads.complete(
            Receipt {
                write,
                confirmation: Confirmation::Failed(forbidden()),
            },
            &mut texts,
        ),
        Some("Couldn't send your reply.".into())
    );
    assert!(!threads.drafts[&key].locked());
    assert_eq!(texts[&key], "Refused reply");
    assert_refusal_lines(
        &rendered_lines(&threads, &pane, &texts),
        "Couldn't send your reply.",
    );
}

#[test]
fn unconfirmed_or_server_side_action_failures_keep_their_retry_and_wording() {
    let internal = || {
        let ClientError::Api(_, message, mut envelope) = forbidden() else {
            unreachable!()
        };
        envelope.code = fractal_client::ErrorCode::Internal;
        ClientError::Api(fractal_client::ErrorCode::Internal, message, envelope)
    };
    for error in [ClientError::Transport("lost receipt".into()), internal()] {
        let mut threads = observed_threads(ThreadState::Open);
        let id = "thread/worker";
        let pane = Pane::Thread(id.into());
        let mut texts = HashMap::new();
        threads.lifecycle(&pane.key(), id.into(), Mutation::Close);
        let write = threads.writes().pop().unwrap();
        let said = error.human();
        assert_eq!(
            threads.complete(
                Receipt {
                    write: write.clone(),
                    confirmation: Confirmation::Failed(error),
                },
                &mut texts,
            ),
            Some(said)
        );
        assert!(threads.pending_actions[id].unknown);
        let lines = rendered_lines(&threads, &pane, &texts);
        assert!(
            lines
                .iter()
                .any(|line| line
                    == "Close thread: outcome not confirmed. Retry the original action.")
        );
        assert!(!lines.iter().any(|line| line.starts_with("Couldn't")));
        // A later refusal cannot answer the original request it may have applied.
        threads.retry(&write.key);
        let retried = threads.writes().pop().unwrap();
        assert_eq!(retried.key, write.key);
        assert_eq!(
            threads.complete(
                Receipt {
                    write: retried,
                    confirmation: Confirmation::Failed(forbidden()),
                },
                &mut texts,
            ),
            Some(forbidden().human())
        );
        assert!(threads.pending_actions[id].unknown);
        assert!(
            !rendered_lines(&threads, &pane, &texts)
                .iter()
                .any(|line| line.starts_with("Couldn't"))
        );
    }
}

#[test]
fn lifecycle_failure_flashes_warn_and_never_a_success_notice() {
    let pane = Pane::Thread("thread/worker".into());
    let mut ui = writable_thread_in(&pane);
    ui.click(Hit::Thread(
        pane.key(),
        thread::Tap::Close("thread/worker".into()),
    ));
    let write = ui.threads.writes().pop().unwrap();
    ui.complete_thread(Receipt {
        write,
        confirmation: Confirmation::Rejected,
    });
    assert_eq!(ui.flash_severity, Some(super::super::Severity::Warn));
    assert_eq!(
        ui.flash.as_ref().unwrap().0,
        "The original thread action was rejected"
    );
    assert!(ui.threads.take_completion_notice().is_none());
}

#[test]
fn definite_action_refusal_headlines_flash_as_warnings() {
    let pane = Pane::Thread("thread/worker".into());
    for (state, tap, headline) in [
        (
            ThreadState::Open,
            thread::Tap::Close("thread/worker".into()),
            "Couldn't close this thread.",
        ),
        (
            ThreadState::Closed,
            thread::Tap::Reopen("thread/worker".into()),
            "Couldn't reopen this thread.",
        ),
    ] {
        let mut ui = writable_thread_in(&pane);
        ui.threads = observed_threads(state);
        ui.click(Hit::Thread(pane.key(), tap));
        let write = ui.threads.writes().pop().unwrap();
        ui.complete_thread(Receipt {
            write,
            confirmation: Confirmation::Failed(forbidden()),
        });
        assert_eq!(ui.flash.as_ref().unwrap().0, headline);
        assert_eq!(ui.flash_severity, Some(super::super::Severity::Warn));
        assert!(ui.threads.take_completion_notice().is_none());
    }
}

#[test]
fn a_refused_thread_read_says_access_ended_with_the_reason() {
    assert_eq!(
        thread_read_error(forbidden()),
        "You no longer have access to this thread. Grant was revoked."
    );
    let lost = ClientError::Transport("lost".into());
    assert_eq!(
        thread_read_error(ClientError::Transport("lost".into())),
        lost.human()
    );
    let mut threads = observed_threads(ThreadState::Open);
    let id = "thread/worker";
    let pane = Pane::Thread(id.into());
    let texts = HashMap::new();
    let summary = ThreadInterest::Summary(id.into());
    threads.failure(
        summary.clone(),
        Some(fractal_client::ErrorCode::Forbidden),
        "Grant was revoked.".into(),
        true,
    );
    assert_eq!(
        threads.failures[&summary],
        "You no longer have access to this thread. Grant was revoked."
    );
    let timeline = ThreadInterest::Timeline(id.into());
    threads.failure(
        timeline.clone(),
        Some(fractal_client::ErrorCode::Forbidden),
        "Grant was revoked".into(),
        true,
    );
    let lines = rendered_lines(&threads, &pane, &texts);
    assert!(
        lines
            .iter()
            .any(|line| line == "You no longer have access to this thread. Grant was revoked."),
        "{lines:#?}"
    );
    assert!(!lines.iter().any(|line| line.contains("not allowed")));
    // Any other failure of the same read keeps the shared wording.
    threads.failure(
        timeline.clone(),
        Some(fractal_client::ErrorCode::NotFound),
        "thread/worker".into(),
        true,
    );
    assert_eq!(threads.failures[&timeline], "it is gone: thread/worker");
}

fn request_history_page(cursor: Option<&str>) -> fractal_client::RequestHistoryPage {
    fractal_client::RequestHistoryPage {
        items: vec![],
        relations: vec![],
        source_episodes: vec![],
        boundary_items: vec![],
        source_support: Some(fractal_client::HistorySourceSupport::PersonStep),
        next_cursor: cursor.map(str::to_owned),
        complete: true,
    }
}

#[test]
fn request_history_pagination_is_explicit_bounded_and_keeps_the_server_cursor() {
    let mut threads = observed_threads(ThreadState::Open);
    threads.read_request_history("thread/worker".into(), true);
    threads.read_request_history("thread/worker".into(), true);
    let mut reads = threads.take_request_history_reads();
    assert_eq!(
        reads.len(),
        1,
        "one request in flight, never an automatic page walk"
    );
    let first = reads.pop().unwrap();
    assert!(first.cursor.is_none());
    threads.request_history_loaded(
        &first.id,
        &first.key,
        Ok(request_history_page(Some("opaque/server-cursor"))),
    );
    assert!(threads.take_request_history_reads().is_empty());
    threads.read_request_history(first.id.clone(), false);
    assert!(
        threads.request_history[&first.id].page.is_none(),
        "old authorization is not shown during a fresh read"
    );
    let next = threads.take_request_history_reads().pop().unwrap();
    assert_eq!(next.cursor.as_deref(), Some("opaque/server-cursor"));
    threads.request_history_loaded(&next.id, &next.key, Ok(request_history_page(None)));
    threads.read_request_history(next.id.clone(), false);
    assert!(
        threads.take_request_history_reads().is_empty(),
        "a terminal page cannot fetch again"
    );
    threads.read_request_history(next.id.clone(), true);
    assert!(
        threads
            .take_request_history_reads()
            .pop()
            .unwrap()
            .cursor
            .is_none()
    );
}

#[test]
fn request_history_revocation_and_departure_discard_pages_and_late_results() {
    let mut threads = observed_threads(ThreadState::Open);
    let id = "thread/worker";
    threads.read_request_history(id.into(), true);
    let read = threads.take_request_history_reads().pop().unwrap();
    threads.failure(
        ThreadInterest::Summary(id.into()),
        Some(fractal_client::ErrorCode::Forbidden),
        "access revoked".into(),
        true,
    );
    threads.request_history_loaded(id, &read.key, Ok(request_history_page(None)));
    assert!(!threads.request_history.contains_key(id));
    let mut threads = observed_threads(ThreadState::Open);
    threads.read_request_history(id.into(), true);
    let read = threads.take_request_history_reads().pop().unwrap();
    threads.keep_windows(&[]);
    threads.request_history_loaded(id, &read.key, Ok(request_history_page(None)));
    assert!(!threads.request_history.contains_key(id));
}

fn wide_rendering(threads: &Threads, pane: &Pane, texts: &HashMap<String, String>) -> String {
    threads
        .mount(
            pane,
            240,
            &edit::Cursor::default(),
            &super::super::demo::test_world(),
            texts,
        )
        .lines
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n")
}

fn refused(code: fractal_client::ErrorCode, wire: &str, message: &str) -> ClientError {
    let envelope = serde_json::from_value(serde_json::json!({
        "api_version":fractal_client::API_VERSION,"error_version":"st3.client.error.v0",
        "request_id":"request/refused","code":wire,"message":message,
        "retryable":false,"details":{}
    }))
    .unwrap();
    ClientError::Api(code, message.into(), Box::new(envelope))
}

fn resolved_source() -> ThreadInterest {
    ThreadInterest::Inbox {
        source: "step-run/resolved".into(),
        episode: "original:1:1".into(),
    }
}

const FALLBACK_LINE: &str = "Fallback recipient · original source agent agent/atlas is retired. Existing discussions and participants are unchanged.";

#[test]
fn canonical_previews_show_and_submit_the_exact_recipient_for_every_basis() {
    struct Case {
        label: &'static str,
        item: &'static str,
        source: &'static str,
        episode: &'static str,
        historical_agent: Option<&'static str>,
        current: bool,
        recipient: &'static str,
        basis: fractal_client::ThreadStartRecipientBasis,
        basis_line: &'static str,
    }
    for case in [
        Case {
            label: "original recipient of a current ask",
            item: "attention/ask",
            source: "step-run/ask",
            episode: "claim/ask",
            historical_agent: Some("agent/atlas"),
            current: true,
            recipient: "agent/atlas",
            basis: fractal_client::ThreadStartRecipientBasis::Original,
            basis_line: "Original source recipient",
        },
        Case {
            label: "configured recipient of a saved system notice",
            item: "attention/system",
            source: "system/fleet-health",
            episode: "claim/system",
            historical_agent: None,
            current: false,
            recipient: "agent/chief-of-staff",
            basis: fractal_client::ThreadStartRecipientBasis::Configured,
            basis_line: "Configured source recipient",
        },
        Case {
            label: "fallback for a saved typed notice with no listed agent",
            item: "attention/notice",
            source: "notice/retired-source",
            episode: "claim/notice",
            historical_agent: None,
            current: false,
            recipient: "agent/chief-of-staff",
            basis: retired("agent/atlas"),
            basis_line: FALLBACK_LINE,
        },
        Case {
            label: "fallback for a resolved ask whose source agent retired",
            item: "attention/resolved",
            source: "step-run/resolved",
            episode: "original:1:1",
            historical_agent: Some("agent/atlas"),
            current: false,
            recipient: "agent/chief-of-staff",
            basis: retired("agent/atlas"),
            basis_line: FALLBACK_LINE,
        },
    ] {
        let Case {
            label,
            item,
            source,
            episode,
            historical_agent,
            current,
            recipient,
            basis,
            basis_line,
        } = case;
        let mut threads = observed_threads(ThreadState::Open);
        let source_origin = origin(
            ThreadInterest::Inbox {
                source: source.into(),
                episode: episode.into(),
            },
            historical_agent,
            "Original request",
        );
        if current {
            threads.origins.insert(item.into(), source_origin);
        } else {
            threads.saved_origins.insert(item.into(), source_origin);
        }
        let pane = Pane::Home(Some(item.into()));
        let host = pane.key();
        let mut texts = HashMap::new();
        assert!(threads.can_start(item), "{label}");
        assert!(
            threads.begin(&host, item, &mut texts),
            "{label}: the draft opens while the preview loads"
        );
        let input = threads.start_input(item).unwrap();
        let read = take_start_context_read(&mut threads, item);
        let loading = wide_rendering(&threads, &pane, &texts);
        assert!(
            loading.contains("Checking recipient before first send…"),
            "{label}: {loading}"
        );
        assert!(
            !loading.contains("New discussion to "),
            "{label}: {loading}"
        );
        texts.insert(input.key(), "First discussion".into());
        threads.submit(&host, input.clone(), &texts);
        assert!(
            threads.writes().is_empty(),
            "{label}: nothing is sent before the canonical preview"
        );

        threads.start_context_loaded(
            &read.item,
            &read.key,
            Ok(start_context_envelope(
                source,
                episode,
                recipient,
                basis.clone(),
                "snapshot/preview",
            )),
        );
        assert!(
            threads.writes().is_empty(),
            "{label}: a loaded preview never sends by itself"
        );
        let shown = wide_rendering(&threads, &pane, &texts);
        assert!(
            shown.contains(&format!("New discussion to {recipient}")),
            "{label}: {shown}"
        );
        assert!(shown.contains(basis_line), "{label}: {shown}");

        let first = threads
            .prepare_write(&host, &input, "First discussion", &Draft::default())
            .unwrap();
        let second = threads
            .prepare_write(&host, &input, "Second discussion", &Draft::default())
            .unwrap();
        let (Mutation::Start(a), Mutation::Start(b)) = (&first.mutation, &second.mutation) else {
            panic!("{label}: independent Starts")
        };
        assert_ne!(
            a.target_id, b.target_id,
            "{label}: every new Start has its own target"
        );
        assert_ne!(
            first.key, second.key,
            "{label}: every new Start has its own idempotency key"
        );
        for (write, start) in [(&first, a), (&second, b)] {
            assert_eq!(
                start.agent_id, recipient,
                "{label}: the submitted recipient is the displayed one"
            );
            assert_eq!(write.agent, recipient, "{label}");
            assert_eq!(
                write.fence.snapshot_id, "snapshot/preview",
                "{label}: fenced by the canonical preview"
            );
            assert_eq!(start.origin.source_id, source, "{label}");
            assert_eq!(start.origin.episode, episode, "{label}");
        }
        assert_eq!(
            threads.start_origin(item).unwrap().agent.as_deref(),
            historical_agent,
            "{label}: the historical source agent stays separate from the recipient"
        );
        if let fractal_client::ThreadStartRecipientBasis::Fallback {
            original_agent_id, ..
        } = &basis
        {
            assert_ne!(
                &a.agent_id, original_agent_id,
                "{label}: a fallback never sends to the retired agent"
            );
        }
        threads.submit(&host, input, &texts);
        let sent = threads
            .writes()
            .pop()
            .expect("the person's Send dispatches once");
        let Mutation::Start(sent) = sent.mutation else {
            panic!("{label}: Start")
        };
        assert_eq!(sent.agent_id, recipient, "{label}");
        assert_eq!(sent.content, "First discussion", "{label}");
        assert_eq!(
            threads.summary("thread/worker").unwrap(),
            &summary(),
            "{label}: existing discussion untouched"
        );
    }
}

#[test]
fn start_recipient_uses_agent_display_metadata_without_changing_the_canonical_recipient() {
    const RECIPIENT: &str = "agent/dotfiles/manager";
    for metadata in [Some("Chief of Staff"), Some("   "), None] {
        let mut world = super::super::demo::test_world();
        let mut agent = world.agents.items().first().unwrap().clone();
        agent.id = RECIPIENT.into();
        agent.name = metadata.unwrap_or_default().into();
        world.agents = view::Load::Ready(if metadata.is_some() {
            vec![agent]
        } else {
            Vec::new()
        });
        let mut threads = observed_threads(ThreadState::Open);
        let item = "attention/resolved";
        threads.saved_origins.insert(
            item.into(),
            origin(resolved_source(), Some("agent/atlas"), "Original request"),
        );
        install_start_context(&mut threads, item, RECIPIENT, retired("agent/atlas"));
        let pane = Pane::Home(Some(item.into()));
        let host = pane.key();
        let mut texts = HashMap::new();
        assert!(threads.begin(&host, item, &mut texts));
        let input = threads.start_input(item).unwrap();
        texts.insert(input.key(), "New discussion".into());
        let render = |threads: &Threads, texts: &HashMap<String, String>| {
            threads
                .mount(&pane, 240, &edit::Cursor::default(), &world, texts)
                .lines
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n")
        };
        let expected = metadata
            .filter(|name| !name.trim().is_empty())
            .unwrap_or(RECIPIENT);
        let shown = render(&threads, &texts);
        assert!(
            shown.contains(&format!("New discussion to {expected}")),
            "{shown}"
        );
        assert!(shown.contains(FALLBACK_LINE), "{shown}");
        threads.submit(&host, input, &texts);
        let write = threads
            .writes()
            .pop()
            .expect("explicit Send dispatches once");
        let Mutation::Start(start) = &write.mutation else {
            panic!("Start")
        };
        assert_eq!(start.agent_id, RECIPIENT);
        assert_eq!(write.agent, RECIPIENT);
        assert_eq!(start.origin.source_id, "step-run/resolved");
        assert_eq!(start.origin.episode, "original:1:1");
        assert_eq!(write.fence.snapshot_id, "snapshot/start-context");
        let pending = render(&threads, &texts);
        assert!(
            pending.contains(&format!(
                "Original delivery to {expected} is pending or unconfirmed."
            )),
            "{pending}"
        );
    }
}

#[test]
fn offline_original_stays_the_recipient_while_retired_origin_uses_the_canonical_upstream() {
    for (state, recipient, label, basis, basis_line) in [
        (
            view::AgentState::Stopped,
            "agent/atlas",
            "Atlas",
            fractal_client::ThreadStartRecipientBasis::Original,
            "Original source recipient",
        ),
        (
            view::AgentState::Retired,
            "agent/dotfiles/manager",
            "Chief of Staff",
            retired("agent/atlas"),
            FALLBACK_LINE,
        ),
    ] {
        let mut world = super::super::demo::test_world();
        let mut original = world.agents.items().first().unwrap().clone();
        let mut upstream = original.clone();
        original.id = "agent/atlas".into();
        original.name = "Atlas".into();
        original.state = state;
        original.terminal = false;
        upstream.id = "agent/dotfiles/manager".into();
        upstream.name = "Chief of Staff".into();
        world.agents = view::Load::Ready(vec![original, upstream]);
        let mut threads = observed_threads(ThreadState::Open);
        let item = "attention/resolved";
        let pane = Pane::Home(Some(item.into()));
        let host = pane.key();
        threads.saved_origins.insert(
            item.into(),
            origin(resolved_source(), Some("agent/atlas"), "Original request"),
        );
        install_start_context(&mut threads, item, recipient, basis);
        let mut texts = HashMap::new();
        assert!(threads.begin(&host, item, &mut texts));
        let input = threads.start_input(item).unwrap();
        texts.insert(input.key(), "Independent discussion".into());
        let shown = threads
            .mount(&pane, 240, &edit::Cursor::default(), &world, &texts)
            .lines
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            shown.contains(&format!("New discussion to {label}")),
            "{shown}"
        );
        assert!(shown.contains(basis_line), "{shown}");
        if recipient == "agent/atlas" {
            assert!(!shown.contains("Fallback recipient"), "{shown}");
            assert!(
                !shown.contains("New discussion to Chief of Staff"),
                "{shown}"
            );
        }
        assert!(
            threads.writes().is_empty(),
            "the preview is shown before any send"
        );
        threads.submit(&host, input, &texts);
        let write = threads.writes().pop().expect("explicit first Send");
        let Mutation::Start(start) = &write.mutation else {
            panic!("Start")
        };
        assert_eq!(start.agent_id, recipient);
        assert_eq!(write.agent, recipient);
        assert_eq!(start.origin.source_id, "step-run/resolved");
        assert_eq!(start.origin.episode, "original:1:1");
    }
}

#[test]
fn resolved_asks_authored_steps_and_notices_start_independent_discussions() {
    for kind in ["person-step", "authored-step", "notice"] {
        let mut threads = observed_threads(ThreadState::Open);
        let id = "attention/resolved";
        let host = format!("home:{id}");
        threads.saved_origins.insert(
            id.into(),
            origin(resolved_source(), Some("agent/atlas"), kind),
        );
        install_start_context(
            &mut threads,
            id,
            "agent/atlas",
            fractal_client::ThreadStartRecipientBasis::Original,
        );
        let mut texts = HashMap::new();
        threads
            .start_with_text(&host, id, "First discussion", &mut texts)
            .unwrap();
        let first = threads.writes().pop().unwrap();
        let Mutation::Start(a) = &first.mutation else {
            panic!("{kind}: Start")
        };
        threads.complete(
            Receipt {
                write: first.clone(),
                confirmation: Confirmation::Applied(Some(Envelope {
                    value: ThreadSummary {
                        id: a.target_id.clone(),
                        ..summary()
                    },
                    snapshot: snapshot(),
                    request_id: "request/first".into(),
                    api_version: fractal_client::API_VERSION.into(),
                })),
            },
            &mut texts,
        );
        if threads.start_context(id).is_none() {
            install_start_context(
                &mut threads,
                id,
                "agent/atlas",
                fractal_client::ThreadStartRecipientBasis::Original,
            );
        }
        threads
            .start_with_text(&host, id, "Second discussion", &mut texts)
            .unwrap();
        let second = threads.writes().pop().unwrap();
        let Mutation::Start(b) = &second.mutation else {
            panic!("{kind}: Start")
        };
        assert_ne!(a.target_id, b.target_id, "{kind}");
        assert_ne!(first.key, second.key, "{kind}");
        assert_eq!(a.origin, b.origin, "{kind}");
        assert_eq!(
            a.agent_id, "agent/atlas",
            "{kind}: an offline original is not retired or redirected"
        );
        assert_eq!(b.agent_id, "agent/atlas", "{kind}");
        assert_eq!(
            threads.summary("thread/worker").unwrap(),
            &summary(),
            "{kind}: existing discussion is untouched"
        );
    }
}

#[test]
fn auto_resolved_sources_start_through_their_typed_key_and_canonical_recipient() {
    let mut threads = observed_threads(ThreadState::Open);
    threads.auto_resolved_sources(&fractal_client::AutoResolvedAsks {
        items: vec![fractal_client::AutoResolvedAsk {
            resolution_id: "resolution/1".into(),
            title: "Auto-resolved review".into(),
            requester_id: "agent/atlas".into(),
            resolved_at: "2026-11-17T00:00:00Z".into(),
            target_id: "step-run/auto".into(),
            episode: "claim/auto".into(),
            default_answer: None,
            request: None,
            action_parameters: None,
        }],
        complete: true,
    });
    let item = "auto-resolved/resolution/1";
    assert_eq!(
        threads.start_origin(item).unwrap().agent,
        None,
        "a requester is not a recipient"
    );
    install_start_context(
        &mut threads,
        item,
        "agent/chief-of-staff",
        retired("agent/atlas"),
    );
    let mut texts = HashMap::new();
    threads
        .start_with_text(
            &format!("home:{item}"),
            item,
            "Why was this auto-resolved?",
            &mut texts,
        )
        .unwrap();
    let Mutation::Start(start) = threads.writes().pop().unwrap().mutation else {
        panic!("Start")
    };
    assert_eq!(start.agent_id, "agent/chief-of-staff");
    assert_eq!(start.origin.source_id, "step-run/auto");
    assert_eq!(start.origin.episode, "claim/auto");
}

fn golden_start_refusal(name: &str) -> ClientError {
    let mut envelopes: BTreeMap<String, fractal_client::ErrorEnvelope> = serde_json::from_str(
        include_str!("../../../../../docs/st3/client-v0/fixtures/thread-start-refusals.json"),
    )
    .unwrap();
    let envelope = envelopes.remove(name).expect("canonical refusal fixture");
    let expected = match name {
        "recipient-unassigned" => fractal_client::ErrorCode::RecipientUnassigned,
        "recipient-retired" => fractal_client::ErrorCode::RecipientRetired,
        "stale-snapshot" => fractal_client::ErrorCode::StaleFence,
        _ => panic!("unexpected golden refusal"),
    };
    assert_eq!(envelope.code, expected);
    assert!(!envelope.retryable);
    ClientError::Api(
        envelope.code.clone(),
        envelope.message.clone(),
        Box::new(envelope),
    )
}

fn assert_terminal_post_start_refusal(code: &str, expected: &str) {
    let mut threads = observed_threads(ThreadState::Open);
    let item = "attention/resolved";
    let pane = Pane::Home(Some(item.into()));
    let host = pane.key();
    threads.saved_origins.insert(
        item.into(),
        origin(resolved_source(), Some("agent/atlas"), "Original request"),
    );
    install_start_context(
        &mut threads,
        item,
        "agent/chief-of-staff",
        retired("agent/atlas"),
    );
    let input = threads.start_input(item).unwrap();
    let mut texts = HashMap::new();
    threads
        .start_with_text(&host, item, "Please keep this draft", &mut texts)
        .unwrap();
    let write = threads
        .writes()
        .pop()
        .expect("the explicit Send posts Start");
    assert!(matches!(&write.mutation, Mutation::Start(_)));
    assert!(threads.writes().is_empty());
    assert!(threads.take_start_context_reads().is_empty());
    let notice = threads
        .complete(
            Receipt {
                write,
                confirmation: Confirmation::Failed(golden_start_refusal(code)),
            },
            &mut texts,
        )
        .unwrap();
    assert!(notice.contains(expected), "{notice}");
    assert!(notice.contains("draft is kept"), "{notice}");
    assert!(threads.start_context(item).is_none());
    assert!(
        threads
            .drafts
            .get(&input.key())
            .and_then(|draft| draft.delivery.as_ref())
            .is_none(),
        "a definitively refused POST is not retained as an unknown delivery"
    );
    for _ in 0..3 {
        threads.submit(&host, input.clone(), &texts);
        assert!(
            threads.writes().is_empty(),
            "no automatic or stale-recipient POST"
        );
        assert!(
            threads.take_start_context_reads().is_empty(),
            "no automatic preview re-read"
        );
    }
    let visible = wide_rendering(&threads, &pane, &texts);
    assert!(visible.contains(&notice), "{visible}");
    assert!(!visible.contains("Checking recipient"), "{visible}");
    assert!(!visible.contains("New discussion to "), "{visible}");
    assert_eq!(texts[&input.key()], "Please keep this draft");
}

#[test]
fn post_start_recipient_retired_keeps_the_draft_without_resending_or_refreshing() {
    assert_terminal_post_start_refusal("recipient-retired", "recipient is retired");
}

#[test]
fn post_start_recipient_unassigned_keeps_the_draft_without_resending_or_refreshing() {
    assert_terminal_post_start_refusal("recipient-unassigned", "Setup is required");
}

#[test]
fn post_start_unassigned_after_prior_unknown_keeps_the_original_packet() {
    let mut threads = observed_threads(ThreadState::Open);
    let item = "attention/resolved";
    let pane = Pane::Home(Some(item.into()));
    let host = pane.key();
    threads.saved_origins.insert(
        item.into(),
        origin(resolved_source(), None, "Original request"),
    );
    install_start_context(
        &mut threads,
        item,
        "agent/chief-of-staff",
        retired("agent/atlas"),
    );
    let input = threads.start_input(item).unwrap();
    let mut texts = HashMap::new();
    threads
        .start_with_text(&host, item, "Keep this signed request", &mut texts)
        .unwrap();
    let write = threads.writes().pop().unwrap();
    let error = ClientError::Protocol("The first POST receipt was lost".into());
    threads.complete(
        Receipt {
            write: write.clone(),
            confirmation: Confirmation::Failed(error),
        },
        &mut texts,
    );
    assert!(threads.writes().is_empty());
    assert!(threads.take_start_context_reads().is_empty());
    let delivery = threads.drafts[&input.key()].delivery.as_ref().unwrap();
    assert!(delivery.unknown);
    assert_eq!(delivery.write.key, write.key);
    let visible = wide_rendering(&threads, &pane, &texts);
    assert!(visible.contains("pending or unconfirmed"), "{visible}");
    assert!(!visible.contains("Checking recipient"), "{visible}");
    threads.retry(&write.key);
    let retried = threads.writes().pop().unwrap();
    assert_eq!(retried.key, write.key);
    let (Mutation::Start(original), Mutation::Start(retry)) = (&write.mutation, &retried.mutation)
    else {
        panic!("Start")
    };
    assert_eq!(original, retry);
    let refusal = golden_start_refusal("recipient-unassigned");
    assert_eq!(refusal.applied(), Some("none"));
    threads.complete(
        Receipt {
            write: retried,
            confirmation: Confirmation::Failed(refusal),
        },
        &mut texts,
    );
    let retained = threads.drafts[&input.key()].delivery.as_ref().unwrap();
    assert!(
        retained.unknown,
        "a later applied:none refusal cannot erase prior ambiguity"
    );
    assert_eq!(retained.write.key, write.key);
    assert!(threads.take_start_context_reads().is_empty());
    assert_eq!(texts[&input.key()], "Keep this signed request");
}

#[test]
fn stale_recipient_refusal_refreshes_and_shows_the_new_recipient_before_a_fresh_send() {
    for (stale, stale_basis, fresh, fresh_basis) in [
        (
            "agent/atlas",
            fractal_client::ThreadStartRecipientBasis::Original,
            "agent/chief-of-staff",
            retired("agent/atlas"),
        ),
        (
            "agent/chief-of-staff",
            retired("agent/atlas"),
            "agent/deputy",
            retired("agent/atlas"),
        ),
    ] {
        let mut threads = observed_threads(ThreadState::Open);
        let item = "attention/resolved";
        let pane = Pane::Home(Some(item.into()));
        let host = pane.key();
        threads.saved_origins.insert(
            item.into(),
            origin(resolved_source(), Some("agent/atlas"), "Original request"),
        );
        install_start_context(&mut threads, item, stale, stale_basis);
        let input = threads.start_input(item).unwrap();
        let mut texts = HashMap::new();
        threads
            .start_with_text(&host, item, "Please follow up", &mut texts)
            .unwrap();
        let refused_write = threads.writes().pop().unwrap();
        assert!(matches!(&refused_write.mutation, Mutation::Start(_)));
        assert_eq!(refused_write.agent, stale);
        let notice = threads
            .complete(
                Receipt {
                    write: refused_write.clone(),
                    confirmation: Confirmation::Failed(golden_start_refusal("stale-snapshot")),
                },
                &mut texts,
            )
            .unwrap();
        assert!(notice.starts_with("Start was refused:"), "{notice}");
        assert!(
            notice.contains("review it before sending. Your draft is kept."),
            "{notice}"
        );
        assert!(
            threads.writes().is_empty(),
            "{stale}: a refusal never re-sends or retargets"
        );
        assert_eq!(texts[&input.key()], "Please follow up", "the draft is kept");
        assert!(
            threads
                .drafts
                .get(&input.key())
                .and_then(|draft| draft.delivery.as_ref())
                .is_none(),
            "the refused packet is dropped"
        );
        assert!(
            threads.start_context(item).is_none(),
            "the stale recipient is no longer offered"
        );
        let refresh = take_start_context_read(&mut threads, item);
        let refreshing = wide_rendering(&threads, &pane, &texts);
        assert!(
            !refreshing.contains(&format!("New discussion to {stale}")),
            "{refreshing}"
        );
        assert!(
            refreshing.contains("Checking recipient before first send…"),
            "{refreshing}"
        );
        assert!(
            threads
                .start_with_text(&host, item, "Please follow up", &mut texts)
                .is_err()
        );
        assert!(
            threads.writes().is_empty(),
            "no send until the refreshed recipient is shown"
        );

        threads.start_context_loaded(
            &refresh.item,
            &refresh.key,
            Ok(start_context_envelope(
                "step-run/resolved",
                "original:1:1",
                fresh,
                fresh_basis,
                "snapshot/refreshed",
            )),
        );
        assert!(
            threads.writes().is_empty(),
            "the refreshed recipient is shown, never sent automatically"
        );
        let refreshed = wide_rendering(&threads, &pane, &texts);
        assert!(
            refreshed.contains(&format!("New discussion to {fresh}")),
            "{refreshed}"
        );
        assert!(
            !refreshed.contains(&format!("New discussion to {stale}")),
            "{refreshed}"
        );
        assert!(refreshed.contains(FALLBACK_LINE), "{refreshed}");

        threads
            .start_with_text(&host, item, "Please follow up", &mut texts)
            .unwrap();
        let fresh_write = threads.writes().pop().unwrap();
        let (Mutation::Start(old), Mutation::Start(new)) =
            (&refused_write.mutation, &fresh_write.mutation)
        else {
            panic!("Starts")
        };
        assert_eq!(new.agent_id, fresh);
        assert_eq!(fresh_write.fence.snapshot_id, "snapshot/refreshed");
        assert_ne!(
            new.target_id, old.target_id,
            "a fresh Start is a new target"
        );
        assert_ne!(
            fresh_write.key, refused_write.key,
            "a fresh Start is a new key"
        );
        assert_eq!(new.origin, old.origin);
    }

    // A server-side rejection also drops its packet and re-reads the recipient without sending.
    let mut threads = observed_threads(ThreadState::Open);
    let item = "attention/resolved";
    threads.saved_origins.insert(
        item.into(),
        origin(resolved_source(), None, "Original request"),
    );
    install_start_context(
        &mut threads,
        item,
        "agent/chief-of-staff",
        retired("agent/atlas"),
    );
    let mut texts = HashMap::new();
    threads
        .start_with_text(
            &format!("home:{item}"),
            item,
            "Please follow up",
            &mut texts,
        )
        .unwrap();
    let write = threads.writes().pop().unwrap();
    assert_eq!(
        threads
            .complete(
                Receipt {
                    write,
                    confirmation: Confirmation::Rejected
                },
                &mut texts
            )
            .as_deref(),
        Some("The original thread action was rejected")
    );
    take_start_context_read(&mut threads, item);
    assert!(threads.start_context(item).is_none());
    assert!(threads.writes().is_empty());
}

#[test]
fn pending_or_unknown_start_retries_keep_the_exact_packet_recipient() {
    let mut threads = observed_threads(ThreadState::Open);
    let item = "attention/notice";
    let pane = Pane::Home(Some(item.into()));
    let host = pane.key();
    threads
        .saved_origins
        .insert(item.into(), origin(resolved_source(), None, "Notice"));
    install_start_context(
        &mut threads,
        item,
        "agent/chief-of-staff",
        retired("agent/atlas"),
    );
    let mut texts = HashMap::new();
    threads
        .start_with_text(&host, item, "What happened?", &mut texts)
        .unwrap();
    let original = threads.writes().pop().unwrap();
    threads.complete(
        Receipt {
            write: original.clone(),
            confirmation: Confirmation::Failed(ClientError::Transport("lost receipt".into())),
        },
        &mut texts,
    );
    threads.request_start_context(item);
    assert!(
        threads.take_start_context_reads().is_empty(),
        "an unknown packet is never re-previewed"
    );
    threads.start_context_loaded(
        item,
        "late/unrequested",
        Ok(start_context_envelope(
            "step-run/resolved",
            "original:1:1",
            "agent/deputy",
            retired("agent/atlas"),
            "snapshot/late",
        )),
    );
    let rendered = wide_rendering(&threads, &pane, &texts);
    assert!(
        rendered.contains("Original delivery to agent/chief-of-staff is pending or unconfirmed. Retrying keeps this exact recipient."),
        "{rendered}"
    );
    assert!(!rendered.contains("agent/deputy"), "{rendered}");
    assert!(!rendered.contains("Refresh recipient"), "{rendered}");
    threads.tap(&host, thread::Tap::Retry(original.key.clone()), &mut texts);
    let retry = threads.writes().pop().expect("the original retry");
    assert_eq!(retry.key, original.key);
    assert_eq!(retry.id, original.id);
    assert_eq!(retry.fence, original.fence);
    assert_eq!(retry.agent, "agent/chief-of-staff");
    let Mutation::Start(parameters) = retry.mutation else {
        panic!("original Start")
    };
    assert_eq!(parameters.agent_id, "agent/chief-of-staff");
    assert_eq!(parameters.content, "What happened?");
}

#[test]
fn late_or_mismatched_start_contexts_are_rejected() {
    let mut threads = observed_threads(ThreadState::Open);
    let item = "attention/resolved";
    let other = "attention/other";
    let other_pane = Pane::Home(Some(other.into()));
    threads.saved_origins.insert(
        item.into(),
        origin(resolved_source(), None, "Original request"),
    );
    threads.saved_origins.insert(
        other.into(),
        origin(
            ThreadInterest::Inbox {
                source: "step-run/other".into(),
                episode: "claim/other".into(),
            },
            None,
            "Other",
        ),
    );
    let exact = |recipient: &str| {
        start_context_envelope(
            "step-run/resolved",
            "original:1:1",
            recipient,
            retired("agent/atlas"),
            "snapshot/preview",
        )
    };
    let other_exact = start_context_envelope(
        "step-run/other",
        "claim/other",
        "agent/chief-of-staff",
        fractal_client::ThreadStartRecipientBasis::Configured,
        "snapshot/preview",
    );
    threads.request_start_context(item);
    threads.request_start_context(other);
    threads.request_start_context(item);
    let reads = threads.take_start_context_reads();
    assert_eq!(reads.len(), 2, "one read per item while pending");
    let a = reads.iter().find(|read| read.item == item).unwrap();
    let b = reads.iter().find(|read| read.item == other).unwrap();
    let mut texts = HashMap::new();
    assert!(
        threads.begin(&other_pane.key(), other, &mut texts),
        "the draft opens while its read is pending"
    );
    assert!(
        threads.take_start_context_reads().is_empty(),
        "begin does not duplicate a pending read"
    );

    // Another member's read cannot answer this item, in either direction.
    threads.start_context_loaded(item, &b.key, Ok(exact("agent/chief-of-staff")));
    threads.start_context_loaded(other, &a.key, Ok(other_exact.clone()));
    assert!(threads.start_context(item).is_none());
    assert!(threads.start_context(other).is_none());

    // A result for a different source is refused and said.
    threads.start_context_loaded(
        item,
        &a.key,
        Ok(start_context_envelope(
            "step-run/other",
            "original:1:1",
            "agent/chief-of-staff",
            retired("agent/atlas"),
            "snapshot/preview",
        )),
    );
    assert!(threads.start_context(item).is_none());
    assert_eq!(
        threads.start_context_failures[item],
        "The Inbox source changed. Check its recipient again."
    );

    // A result for a different episode of the same source is refused.
    threads.request_start_context(item);
    let wrong_episode = take_start_context_read(&mut threads, item);
    threads.start_context_loaded(
        item,
        &wrong_episode.key,
        Ok(start_context_envelope(
            "step-run/resolved",
            "original:1:2",
            "agent/chief-of-staff",
            retired("agent/atlas"),
            "snapshot/preview",
        )),
    );
    assert!(threads.start_context(item).is_none());

    // A superseded read's late answer is ignored; only the current read installs.
    threads.request_start_context(item);
    let superseded = take_start_context_read(&mut threads, item);
    threads.keep_inbox_sources(&[]);
    assert!(
        threads.saved_origins.is_empty(),
        "live bounds saved sources to its current keys"
    );
    threads.saved_origins.insert(
        item.into(),
        origin(resolved_source(), None, "Original request"),
    );
    threads.request_start_context(item);
    let current = take_start_context_read(&mut threads, item);
    threads.start_context_loaded(item, &superseded.key, Ok(exact("agent/deputy")));
    assert!(threads.start_context(item).is_none());
    threads.start_context_loaded(item, &current.key, Ok(exact("agent/chief-of-staff")));
    assert_eq!(
        threads
            .start_context(item)
            .unwrap()
            .value
            .start_recipient
            .agent_id,
        "agent/chief-of-staff"
    );

    // A pruned source's late result never brings it back.
    assert!(!threads.can_start(other));
    threads.start_context_loaded(other, &b.key, Ok(other_exact));
    assert!(threads.start_context(other).is_none());

    // A refused read says why and offers nothing to send.
    threads.saved_origins.insert(
        other.into(),
        origin(
            ThreadInterest::Inbox {
                source: "step-run/other".into(),
                episode: "claim/other".into(),
            },
            None,
            "Other",
        ),
    );
    threads.request_start_context(other);
    let failed = take_start_context_read(&mut threads, other);
    threads.start_context_loaded(
        other,
        &failed.key,
        Err("You no longer have access to this source.".into()),
    );
    assert!(threads.start_context(other).is_none());
    let rendered = wide_rendering(&threads, &other_pane, &texts);
    assert!(
        rendered.contains("You no longer have access to this source."),
        "{rendered}"
    );
    assert!(rendered.contains("Refresh recipient"), "{rendered}");

    // A preview of an older episode is not offered once the source moves on.
    threads.saved_origins.insert(
        item.into(),
        origin(
            ThreadInterest::Inbox {
                source: "step-run/resolved".into(),
                episode: "original:2:1".into(),
            },
            None,
            "Original request",
        ),
    );
    assert!(threads.start_context(item).is_none());
    let input = threads.start_input(item).unwrap();
    assert!(
        threads
            .prepare_write(&format!("home:{item}"), &input, "Hello", &Draft::default())
            .is_err()
    );
    assert!(threads.writes().is_empty(), "no context result ever sends");
}

#[test]
fn a_saved_source_alone_never_authorizes_a_start() {
    let mut threads = observed_threads(ThreadState::Open);
    let item = "attention/resolved";
    threads.saved_origins.insert(
        item.into(),
        origin(resolved_source(), Some("agent/atlas"), "Original request"),
    );
    let input = threads.start_input(item).unwrap();
    assert!(
        threads
            .prepare_write(&format!("home:{item}"), &input, "Hello", &Draft::default())
            .is_err()
    );
    let mut texts = HashMap::new();
    assert!(
        threads
            .start_with_text(&format!("home:{item}"), item, "Hello", &mut texts)
            .is_err()
    );
    assert!(texts.is_empty());
    assert!(threads.writes().is_empty());
}

/// Layout evidence for the canonical first-Start recipient. Production `Ui::draw_pane` on an
/// in-process model with fixture data, NOT the installed binary or a daemon transport. Run with
/// THREAD_PREVIEW_CAPTURE_DIR and --ignored --nocapture.
#[test]
#[ignore = "writes explicitly requested recipient layout evidence"]
fn start_recipient_layout_evidence() {
    use ratatui::{buffer::Buffer, layout::Rect};
    const FIXTURE: &str = "FIXTURE: retired source agent/atlas · canonical fallback agent/chief-of-staff · stale refusal refreshes to agent/deputy";
    let dir = std::path::PathBuf::from(
        std::env::var("THREAD_PREVIEW_CAPTURE_DIR").expect("THREAD_PREVIEW_CAPTURE_DIR"),
    );
    std::fs::create_dir_all(&dir).unwrap();
    fn capture(
        ui: &mut super::super::Ui,
        pane: &Pane,
        dir: &std::path::Path,
        name: &str,
    ) -> String {
        let area = Rect::new(0, 0, 120, 40);
        let mut screen = Buffer::empty(area);
        ui.draw_pane(&mut screen, area, pane);
        let mut text = format!(
            "HARNESS, NOT PRODUCTION BINARY PATH — production Ui::draw_pane, in-process fixture model\n{FIXTURE}\n"
        );
        for row in screen.content.chunks(120) {
            text.push_str(&row.iter().map(|cell| cell.symbol()).collect::<String>());
            text.push('\n');
        }
        std::fs::write(dir.join(format!("{name}.txt")), &text).unwrap();
        text
    }
    let mut ui = super::super::Ui::new(super::super::demo::test_world());
    ui.threads = observed_threads(ThreadState::Open);

    // History: the existing discussion keeps its participant and shows the original request.
    let tab = Pane::Thread("thread/worker".into());
    ui.open_in_glass(tab.clone(), super::super::glass::Open::Tab);
    let accepted_at = chrono::Utc::now().timestamp_millis() - 120_000;
    let message: ThreadTimelineEvent = serde_json::from_value(serde_json::json!({
        "id":"message/history", "actor_id":"agent/atlas", "accepted_at_unix_ms":accepted_at,
        "kind":"message", "content":"I will look at the calibration drift.", "thread_quote_id":null
    }))
    .unwrap();
    ui.threads.window(
        ThreadInterest::Timeline("thread/worker".into()),
        snapshot(),
        true,
        ThreadCollectionRows::Timeline(vec![message]),
        vec![],
        vec!["message/history".into()],
        false,
    );
    ui.threads
        .read_request_history("thread/worker".into(), true);
    let read = ui.threads.take_request_history_reads().pop().unwrap();
    ui.threads.request_history_loaded(
        &read.id,
        &read.key,
        Ok(fractal_client::RequestHistoryPage {
            items: vec![fractal_client::RequestHistoryItem {
                key: fractal_client::SourceEpisode {
                    source_id: "step-run/resolved".into(),
                    episode: "original:1:1".into(),
                },
                payload: fractal_client::HistoricalPayload::Available {
                    title: Some("Calibration review".into()),
                    text: "Please review the sensor calibration.".into(),
                    request: None,
                    references: Vec::new(),
                    provenance: fractal_client::RequestProvenance::RuntimeAsk {
                        asking_claim_id: "claim/asking".into(),
                    },
                },
                state: fractal_client::EpisodeState::Available {
                    status: "resolved".into(),
                    answers: Vec::new(),
                },
                complete: true,
            }],
            ..request_history_page(None)
        }),
    );
    let history = capture(&mut ui, &tab, &dir, "recipient-history-related-requests");
    assert!(
        history.contains("Related requests") && history.contains("original:1:1"),
        "{history}"
    );

    // Fallback composer for the resolved item whose source agent retired.
    let item = "attention/resolved";
    let inbox = Pane::Home(Some(item.into()));
    ui.threads.saved_origins.insert(
        item.into(),
        origin(resolved_source(), Some("agent/atlas"), "Calibration review"),
    );
    ui.threads.lists.insert(resolved_source(), Vec::new());
    ui.open_in_glass(inbox.clone(), super::super::glass::Open::Tab);
    ui.click(Hit::Thread(inbox.key(), thread::Tap::Start(item.into())));
    let read = take_start_context_read(&mut ui.threads, item);
    ui.paste("Who owns the calibration follow-up now?".into());
    let checking = capture(&mut ui, &inbox, &dir, "recipient-fallback-checking");
    assert!(
        checking.contains("Checking recipient before first send…"),
        "{checking}"
    );
    ui.threads.start_context_loaded(
        &read.item,
        &read.key,
        Ok(start_context_envelope(
            "step-run/resolved",
            "original:1:1",
            "agent/chief-of-staff",
            retired("agent/atlas"),
            "snapshot/preview",
        )),
    );
    let composer = capture(&mut ui, &inbox, &dir, "recipient-fallback-composer");
    assert!(
        composer.contains("New discussion to agent/chief-of-staff"),
        "{composer}"
    );
    assert!(composer.contains("Fallback recipient"), "{composer}");
    assert!(
        ui.threads.writes().is_empty(),
        "showing the recipient never sends"
    );
    ui.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let start = ui.threads.writes().pop().expect("the person's Send");
    assert_eq!(start.agent, "agent/chief-of-staff");
    capture(&mut ui, &inbox, &dir, "recipient-fallback-pending");
    let notice = ui.threads.complete(
        Receipt {
            write: start,
            confirmation: Confirmation::Failed(refused(
                fractal_client::ErrorCode::NotFound,
                "not-found",
                "The selected recipient is no longer current.",
            )),
        },
        &mut ui.conversation_state.drafts,
    );
    assert!(notice.is_some_and(|notice| notice.starts_with("Start was refused:")));
    let read = take_start_context_read(&mut ui.threads, item);
    let refreshing = capture(&mut ui, &inbox, &dir, "recipient-stale-refused-refreshing");
    assert!(
        refreshing.contains("Checking recipient before first send…"),
        "{refreshing}"
    );
    ui.threads.start_context_loaded(
        &read.item,
        &read.key,
        Ok(start_context_envelope(
            "step-run/resolved",
            "original:1:1",
            "agent/deputy",
            retired("agent/atlas"),
            "snapshot/refreshed",
        )),
    );
    let refreshed = capture(
        &mut ui,
        &inbox,
        &dir,
        "recipient-stale-refreshed-before-send",
    );
    assert!(
        refreshed.contains("New discussion to agent/deputy"),
        "{refreshed}"
    );
    assert!(
        refreshed.contains("Who owns the calibration follow-up now?"),
        "{refreshed}"
    );
    assert!(
        ui.threads.writes().is_empty(),
        "the refreshed recipient waits for the person's Send"
    );
    println!("HARNESS recipient evidence: {}", dir.display());
}

#[test]
fn terminal_start_preview_refusals_are_visible_and_never_send_or_loop() {
    for (code, expected) in [
        ("recipient-retired", "recipient is retired"),
        ("recipient-unassigned", "Setup is required"),
    ] {
        let mut threads = observed_threads(ThreadState::Open);
        let item = "attention/system-notice";
        threads.saved_origins.insert(
            item.into(),
            origin(
                ThreadInterest::Inbox {
                    source: "notice/system".into(),
                    episode: "claim/opening".into(),
                },
                None,
                "System notice",
            ),
        );
        let pane = Pane::Home(Some(item.into()));
        let mut texts = HashMap::new();
        assert!(threads.begin(&pane.key(), item, &mut texts));
        let read = take_start_context_read(&mut threads, item);
        let notice = start_context_error(golden_start_refusal(code));
        assert!(notice.contains(expected), "{notice}");
        threads.start_context_loaded(&read.item, &read.key, Err(notice.clone()));
        let input = threads.start_input(item).unwrap();
        texts.insert(
            input.key(),
            "Keep this message until there is a recipient.".into(),
        );
        for _ in 0..3 {
            threads.submit(&pane.key(), input.clone(), &texts);
            assert!(threads.writes().is_empty());
            assert!(
                threads.take_start_context_reads().is_empty(),
                "no retry loop after a refusal"
            );
            assert!(
                threads.start_context(item).is_none(),
                "no invented recipient"
            );
        }
        let doc = threads.mount(
            &pane,
            120,
            &edit::Cursor::default(),
            &super::super::demo::test_world(),
            &texts,
        );
        let visible = doc
            .lines
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(visible.contains(&notice), "{visible}");
        assert!(!visible.contains("New discussion to "), "{visible}");
        assert_eq!(
            texts[&input.key()],
            "Keep this message until there is a recipient."
        );
    }
}
