use super::*;
use fractal_client::Client as ClientV0;

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs ST3_COMPAT_BIN: an actual storage-version-22 thread reader"]
async fn version_22_reader_refuses_upgraded_state_without_losing_history_or_pairing() {
    retained_history_and_pairing_survive_upgrade(22).await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs ST3_COMPAT_BIN: an actual published-main storage-version-20 reader"]
async fn published_version_20_reader_refuses_upgraded_state_without_losing_history_or_pairing() {
    retained_history_and_pairing_survive_upgrade(20).await;
}

async fn retained_history_and_pairing_survive_upgrade(previous_storage_version: u32) {
    if fractal_daemon::test_support::supervise_test() { return; }
    let old = PathBuf::from(std::env::var("ST3_COMPAT_BIN").expect("a real compatibility binary matching the selected storage version"));
    let root = tempfile::tempdir().unwrap();
    let mut node = Node::new(root.path(), "thread-floor");
    node.binary = old.clone();
    if previous_storage_version == 22 {
        let config = node.root.join("config/st3/config.toml");
        let mut contents = fs::read_to_string(&config).unwrap();
        contents.push_str("thread_writes = true\n");
        fs::write(config, contents).unwrap();
    }
    node.start().await;
    let database = node.state_dir().join("claims.sqlite3");
    assert_eq!(database_schema(&database), previous_storage_version,"the selected compatibility binary must actually create the expected storage version");
    let receipt: Value = node.client().post("/v1/messages", &json!({
        "idempotency_key":"retained-floor-mail","from":PERSON,"to":"agent/fleet-reader",
        "content":"Retained before causal metadata upgrade",
    })).await.unwrap();
    let subject = receipt["subject"].as_str().unwrap().to_owned();
    let person = ClientV0::unix_as(node.socket(), PERSON);
    let begun: Value = person.post("/v1/client/pairings", &json!({
        "api_version":"st3.client.v0","device_name":"Floor fixture","person_id":PERSON,
    })).await.unwrap();
    let challenge = &begun["value"];
    let id = challenge["pairing_id"].as_str().unwrap().trim_start_matches("pairing/");
    let paired: Value = person.post(&format!("/v1/client/pairings/{id}/complete"), &json!({
        "api_version":"st3.client.v0","code":challenge["code"],"device_public_key":"b".repeat(64),
    })).await.unwrap();
    let credential = paired["value"]["credential"].as_str().unwrap().to_owned();
    let _: Value = ClientV0::unix_gateway(node.socket(), &credential).get("/v1/client/capabilities").await.unwrap();
    let private_thread = if previous_storage_version == 22 {
        Some(create_version_22_private_history(&node, &person).await)
    } else {
        None
    };
    node.stop();
    let before = {
        let connection = rusqlite::Connection::open_with_flags(&database, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let mut statement = connection.prepare("SELECT id,body,predecessors FROM claims ORDER BY id").unwrap();
        let rows = statement.query_map([],|row|Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?)))
            .unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
        rows
    };
    node.binary = PathBuf::from(st3());
    node.start().await;
    assert_eq!(database_schema(&database),23);
    let _: Value = ClientV0::unix_gateway(node.socket(), &credential).get("/v1/client/capabilities").await.unwrap();
    let retained: Value = node.client().get(&format!("/v1/messages/read/{subject}")).await.unwrap();
    assert_eq!(retained["content"],"Retained before causal metadata upgrade");
    if let Some(history) = &private_thread {
        assert_private_history_retained(&ClientV0::unix_as(node.socket(), PERSON), history).await;
    }
    node.stop();
    {
        let connection = rusqlite::Connection::open_with_flags(&database,rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        for (id,body,predecessors) in before {
            let retained: (String,String) = connection.query_row("SELECT body,predecessors FROM claims WHERE id=?1",[id],
                |row|Ok((row.get(0)?,row.get(1)?))).unwrap();
            assert_eq!(retained,(body,predecessors),"upgrade preserves exact stored history and pairing records");
        }
    }
    node.binary = old;
    assert!(!node.start_with_schema_refusal(Some(23)).await,"the previous reader must refuse this upgraded store");
    assert_eq!(database_schema(&database),23,"old-reader refusal must not alter the upgraded header");
    node.binary = PathBuf::from(st3());
    node.start().await;
    let _: Value = ClientV0::unix_gateway(node.socket(), credential).get("/v1/client/capabilities").await.unwrap();
    let retained: Value = node.client().get(&format!("/v1/messages/read/{subject}")).await.unwrap();
    assert_eq!(retained["content"],"Retained before causal metadata upgrade");
    if let Some(history) = &private_thread {
        assert_private_history_retained(&ClientV0::unix_as(node.socket(), PERSON), history).await;
    }
}


struct PrivateThreadHistory {
    summary: Value,
    timeline: Value,
    source: String,
    episode: String,
}

// Use only the old pilot's public producers. Its obsolete action fields stay here,
// never in a current production writer or compatibility adapter.
async fn create_version_22_private_history(node: &Node, person: &ClientV0) -> PrivateThreadHistory {
    let declaration = node.root.join("private-history.kdl");
    fs::write(&declaration, format!(
        "version 2\nagent \"floor/reviewer\" {{ workspace {:?}; command \"true\"; restart \"never\"; }}\n",
        node.root.to_str().unwrap(),
    )).unwrap();
    node.st_ok(&["apply", declaration.to_str().unwrap(), "--as", PERSON]);
    let ask: Value = node.client().post("/v1/work/ask", &json!({
        "person":PERSON,"actor":"agent/floor/reviewer","title":"Preserve private pilot history",
        "reason":"Review this retained conversation","new_run":"private-history",
        "idempotency_key":"private-history-source-22",
    })).await.unwrap();
    let source = ask["subject"].as_str().unwrap().to_owned();
    let attention: Value = person.get("/v1/client/now").await.unwrap();
    let row = attention["value"]["items"].as_array().unwrap().iter()
        .find(|item| item["source_id"] == source).expect("the actual old ask must appear in Inbox");
    let episode = row["episode"].as_str().unwrap().to_owned();
    let thread = "thread/floor-private-22";
    let started = version_22_thread_action(person, "thread.start", "private-start-floor-22", json!({
        "target_id":thread,"agent_id":"agent/floor/reviewer","origin_ask":source,
        "origin_ask_episode":episode,"title":"Pilot review","content":"Retained private first message",
    }), None).await;
    let first = started["value"]["affected_ids"][1].as_str().unwrap();
    version_22_thread_action(person, "thread.reply", "private-reply-floor-22", json!({
        "target_id":thread,"content":"Retained quoted response","thread_quote_id":first,
    }), None).await;
    let detail: Value = person.get("/v1/client/threads/floor-private-22").await.unwrap();
    version_22_thread_action(person, "thread.rename", "private-rename-floor-22", json!({
        "target_id":thread,"title":"Retained pilot title","title_revision":detail["value"]["title_revision"],
    }), Some(&detail["value"]["revision"])).await;
    let detail: Value = person.get("/v1/client/threads/floor-private-22").await.unwrap();
    version_22_thread_action(person, "thread.close", "private-close-floor-22", json!({
        "target_id":thread,
    }), Some(&detail["value"]["revision"])).await;
    let detail: Value = person.get("/v1/client/threads/floor-private-22").await.unwrap();
    let timeline: Value = person.get("/v1/client/threads/floor-private-22/timeline").await.unwrap();
    let history = PrivateThreadHistory { summary: detail["value"].clone(), timeline: timeline["value"].clone(), source, episode };
    assert_eq!(history.summary["title"], "Retained pilot title");
    assert_eq!(history.summary["state"], "closed");
    assert_eq!(history.summary["message_count"], 2);
    let items = history.timeline["items"].as_array().unwrap();
    assert_eq!(items.len(), 4, "two messages plus rename and close must be genuinely produced");
    assert!(history.timeline["next_cursor"].is_null(), "the captured timeline must be complete");
    assert!(items.iter().any(|item| item["content"] == "Retained quoted response" && item["thread_quote_id"] == first));
    assert!(items.iter().any(|item| item["kind"] == "thread.renamed"
        && item["previous_title"] == "Pilot review" && item["title"] == "Retained pilot title"));
    assert!(items.iter().any(|item| item["kind"] == "thread.closed"));
    history
}

async fn version_22_thread_action(person: &ClientV0, kind: &str, key: &str, parameters: Value, revision: Option<&Value>) -> Value {
    let capabilities: Value = person.get("/v1/client/capabilities").await.unwrap();
    let mut fence = json!({"snapshot_id":capabilities["snapshot"]["id"]});
    if let Some(revision) = revision {
        fence["subject_revisions"] = json!({"thread/floor-private-22":revision});
    }
    person.post("/v1/client/actions", &json!({
        "api_version":"st3.client.v0","id":format!("action/{key}"),"type":kind,
        "idempotency_key":key,"fence":fence,"parameters":parameters,
    })).await.unwrap()
}

async fn assert_private_history_retained(person: &ClientV0, history: &PrivateThreadHistory) {
    let detail: Value = person.get("/v1/client/threads/floor-private-22").await.unwrap();
    // The current semantic revision intentionally differs from the pilot's raw claim
    // tip; immutable title identity, membership, lifecycle and content must not.
    for field in ["id", "agent_id", "title", "title_revision", "state", "message_count"] {
        assert_eq!(detail["value"][field], history.summary[field], "retained {field}");
    }
    let timeline: Value = person.get("/v1/client/threads/floor-private-22/timeline").await.unwrap();
    assert_eq!(timeline["value"], history.timeline, "exact private content, quotes and lifecycle events survive upgrade");
    let selector = format!("origin_source={}&origin_episode={}",
        urlencoding::encode(&history.source), urlencoding::encode(&history.episode));
    let discovered: Value = person.get(&format!("/v1/client/threads?{selector}")).await.unwrap();
    assert!(discovered["value"]["items"].as_array().unwrap().iter()
        .any(|item| item["id"] == "thread/floor-private-22"), "historical ask origin remains discoverable through the current Inbox selector");
}
