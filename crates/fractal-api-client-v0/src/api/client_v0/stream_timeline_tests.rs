//! The conversation timeline reads stream items: a follower's cursor carries its stream
//! position, and each item's entries sort together.
use super::tests::test_state_named;
use super::*;

/// A stream item to a seat that arrives while the graph stands still reaches its
/// conversation: the cursor's stream position, not its graph index, says what the follower
/// has read, and the quiet check that answers an issued cursor without a read sees it. Its
/// entries sort two past the graph claim it arrived behind.
#[tokio::test]
async fn conversation_changes_carry_a_stream_item_that_arrives_while_the_graph_stands_still() {
    let root = tempfile::tempdir().unwrap();
    let state = test_state_named(root.path(), "stream-follow");
    let shard = crate::store::messages_shard::open(&state.store, root.path()).unwrap();
    let owner = "agent/stream-follow";
    let incarnation = "stream-follow-runtime:i1";
    state.store.append_claim(&ClaimInput {
        subject: owner.into(), kind: "runtime.observed".into(), actor: Some(owner.into()),
        fields: serde_json::from_value(json!({"status":"running", "runtime_id":"stream-follow-runtime", "incarnation_id":incarnation})).unwrap(),
        evidence: vec![], expected_subject: None, idempotency_key: None,
    }).unwrap();
    let session_id = managed_session_id(owner, incarnation);
    let session = ClientSession::local(Some("person/example")).unwrap();
    let (start, page) = conversation_open_local(&state, &session, &session_id, 200).unwrap();
    let cursor = start["next_cursor"].as_str().unwrap().to_owned();
    let on_stream = |value: &Value| value["items"].as_array().unwrap().iter()
        .filter(|item| item["body"]["text"] == "on the stream" || item["body"]["message_id"] == "message/00000000000000c1")
        .map(|item| (item["type"].as_str().unwrap().to_owned(), item["sequence"].as_u64().unwrap()))
        .collect::<BTreeSet<_>>();
    assert!(on_stream(&page).is_empty());

    let graph = state.store.index().unwrap();
    shard.append_claim(&ClaimInput {
        subject: "message/00000000000000c1".into(), kind: "message.sent".into(), actor: Some("agent/birch".into()),
        fields: serde_json::from_value(json!({"from":"agent/birch", "to":owner, "session_id":session_id,
            "status":"sent", "content":"on the stream"})).unwrap(),
        evidence: vec![], expected_subject: None, idempotency_key: None,
    }).unwrap();
    crate::store::messages_shard::stream_reads::record_arrivals(&state.store).unwrap();
    assert_eq!(state.store.index().unwrap(), graph, "the graph stands still");

    let delta = conversation_changes_local(&state, &session, &session_id, Some(&cursor), 0).await.unwrap();
    let expected = BTreeSet::from([("message".to_owned(), graph * 4 + 2), ("content".to_owned(), graph * 4 + 3)]);
    assert_eq!(on_stream(&delta), expected, "{delta}");
    let next = delta["next_cursor"].as_str().unwrap();
    assert_eq!(conversation_position(&state, &session_id, next).unwrap().stream_position,
        crate::store::messages_shard::stream_reads::conversation_position(&state.store).unwrap());
    let (_, reopened) = conversation_open_local(&state, &session, &session_id, 200).unwrap();
    assert_eq!(on_stream(&reopened), expected);
    let after = conversation_changes_local(&state, &session, &session_id, Some(next), 0).await.unwrap();
    assert!(on_stream(&after).is_empty(), "read once: {after}");

    // A cursor from before the stream part existed has read none of the stream.
    let old = next.rsplit_once('.').unwrap().0;
    let position = conversation_position(&state, &session_id, old).unwrap();
    assert_eq!(position.stream_position, 0);
}

/// A stream item whose accepted time does not parse shows when it arrived here, not 1970.
#[tokio::test]
async fn a_stream_item_whose_accepted_time_does_not_parse_shows_when_it_arrived() {
    let root = tempfile::tempdir().unwrap();
    let state = test_state_named(root.path(), "stream-arrived");
    let shard = crate::store::messages_shard::open(&state.store, root.path()).unwrap();
    let owner = "agent/stream-arrived";
    let incarnation = "stream-arrived-runtime:i1";
    state.store.append_claim(&ClaimInput {
        subject: owner.into(), kind: "runtime.observed".into(), actor: Some(owner.into()),
        fields: serde_json::from_value(json!({"status":"running", "runtime_id":"stream-arrived-runtime", "incarnation_id":incarnation})).unwrap(),
        evidence: vec![], expected_subject: None, idempotency_key: None,
    }).unwrap();
    let session_id = managed_session_id(owner, incarnation);
    let session = ClientSession::local(Some("person/example")).unwrap();
    // Written at a fixed time, so a time read from the claim would show it.
    shard.set_write_clock_at(1_790_000_000_000).unwrap();
    let subject = "message/00000000000000e1";
    shard.append_claim(&ClaimInput {
        subject: subject.into(), kind: "message.sent".into(), actor: Some("agent/birch".into()),
        fields: serde_json::from_value(json!({"from":"agent/birch", "to":owner, "session_id":session_id,
            "status":"sent", "title":"late", "content":"late"})).unwrap(),
        evidence: vec![], expected_subject: None, idempotency_key: None,
    }).unwrap();
    shard.connection.write()
        .execute("UPDATE claims SET accepted_at_unix_ms='not-a-time' WHERE subject=?1", [subject])
        .unwrap();
    crate::store::messages_shard::stream_reads::record_arrivals(&state.store).unwrap();
    let (_, page) = conversation_open_local(&state, &session, &session_id, 200).unwrap();
    let item = page["items"].as_array().unwrap().iter()
        .find(|item| item["type"] == "message" && item["body"]["message_id"] == subject)
        .unwrap_or_else(|| panic!("{page}"));
    let shown = item["timestamp"].as_str().unwrap();
    assert!(!shown.starts_with("1970") && shown > "2026-09-22", "shown at {shown}, not when it arrived");
}

/// Two stream items that arrive at the same graph index in the same millisecond share
/// sequences. Their shown times differ, so each item's message and content entries sort
/// together and a phone that pairs a message with the next content entry pairs them right.
/// `fixtures/clients/stream-timeline-pairs-v1.json` holds this page for the TypeScript views,
/// which read it with the phone's unchanged `conversationEntries`.
#[tokio::test]
async fn stream_items_that_share_a_graph_index_and_a_millisecond_keep_their_entries_together() {
    let root = tempfile::tempdir().unwrap();
    let state = test_state_named(root.path(), "stream-pairs");
    let shard = crate::store::messages_shard::open(&state.store, root.path()).unwrap();
    let owner = "agent/stream-pairs";
    let incarnation = "stream-pairs-runtime:i1";
    state.store.append_claim(&ClaimInput {
        subject: owner.into(), kind: "runtime.observed".into(), actor: Some(owner.into()),
        fields: serde_json::from_value(json!({"status":"running", "runtime_id":"stream-pairs-runtime", "incarnation_id":incarnation})).unwrap(),
        evidence: vec![], expected_subject: None, idempotency_key: None,
    }).unwrap();
    let session_id = managed_session_id(owner, incarnation);
    let session = ClientSession::local(Some("person/example")).unwrap();
    let graph = state.store.index().unwrap();
    shard.set_write_clock_at(1_790_000_000_000).unwrap();
    for (subject, from, text) in [
        ("message/00000000000000d1", "agent/birch", "first"),
        ("message/00000000000000d2", "agent/cedar", "second"),
    ] {
        shard.append_claim(&ClaimInput {
            subject: subject.into(), kind: "message.sent".into(), actor: Some(from.into()),
            fields: serde_json::from_value(json!({"from":from, "to":owner, "session_id":session_id,
                "status":"sent", "title":text, "content":text})).unwrap(),
            evidence: vec![], expected_subject: None, idempotency_key: None,
        }).unwrap();
    }
    crate::store::messages_shard::stream_reads::record_arrivals(&state.store).unwrap();
    assert_eq!(state.store.index().unwrap(), graph, "both arrive at one graph index");

    let (_, page) = conversation_open_local(&state, &session, &session_id, 200).unwrap();
    let mut items = page["items"].as_array().unwrap().iter()
        .filter(|item| item["sequence"].as_u64().is_some_and(|sequence| sequence / 4 == graph)
            && matches!(item["type"].as_str(), Some("message" | "content")))
        .cloned().collect::<Vec<_>>();
    assert_eq!(items.len(), 4, "{page}");
    // The page's order, which every phone's sort keeps: time, then sequence, then ID.
    let key = |item: &Value| (item["timestamp"].as_str().unwrap().to_owned(),
        item["sequence"].as_u64().unwrap(), item["id"].as_str().unwrap().to_owned());
    items.sort_by_key(key);
    let shown = items.iter().map(|item| (item["type"].as_str().unwrap(),
        item["body"][if item["type"] == "message" { "message_id" } else { "text" }].as_str().unwrap()))
        .collect::<Vec<_>>();
    assert_eq!(shown, [("message", "message/00000000000000d1"), ("content", "first"),
        ("message", "message/00000000000000d2"), ("content", "second")]);
    assert_eq!(items[0]["timestamp"], "2026-09-21T14:13:20.000Z");
    assert_eq!(items[2]["timestamp"], "2026-09-21T14:13:20.001Z");

    // The fixture names entries by their order, which is all the views' sort reads of an ID.
    let mut ids = items.iter().map(|item| item["id"].as_str().unwrap().to_owned()).collect::<Vec<_>>();
    ids.sort();
    let fixture = items.iter().map(|item| {
        let mut item = item.clone();
        let rank = ids.iter().position(|id| item["id"] == *id).unwrap();
        item["id"] = json!(format!("timeline-entry/stream-pairs/{rank}"));
        item["sequence"] = json!(item["sequence"].as_u64().unwrap() - graph * 4);
        item
    }).collect::<Vec<_>>();
    let path = test_env!("CARGO_MANIFEST_DIR", "/../../fixtures/clients/stream-timeline-pairs-v1.json");
    let expected: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(json!(fixture), expected["items"], "{}", serde_json::to_string_pretty(&json!({"items": fixture})).unwrap());
}
