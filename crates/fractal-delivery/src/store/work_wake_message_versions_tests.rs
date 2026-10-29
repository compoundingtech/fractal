use super::*;
use crate::DeliveryStore;

fn send(store: &Store, id: &str, to: &str, tags: Vec<String>) {
    store
        .append_claim(&ClaimInput {
            subject: format!("message/{id}"),
            kind: "message.sent".into(),
            actor: Some("daemon/runtime".into()),
            fields: BTreeMap::from([
                ("from".into(), json!("daemon/runtime")),
                ("to".into(), json!(to)),
                ("content".into(), json!("invented wake body")),
                ("status".into(), json!("sent")),
                ("tags".into(), json!(tags)),
            ]),
            evidence: vec![],
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
}

// Verbatim parent selection/order/filtering, retaining its point cache validation.
fn point_work_wakes(store: &Store, agent: &str) -> Result<Vec<MessageView>> {
    let connection = store.readers.get();
    let recipient = fractal_store::store::identities::resolve_reference_conn(
        &connection,
        &normalize_message_party(agent),
    )?;
    let bare_recipient = recipient
        .strip_prefix("agent/")
        .filter(|suffix| !suffix.contains('/'))
        .unwrap_or(&recipient);
    let mut statement = connection.prepare_cached(
        "SELECT DISTINCT sent.subject,
                (SELECT MIN(created.store_index) FROM claims created
                 WHERE created.subject=sent.subject) AS created_index
         FROM claims sent INDEXED BY claims_message_to_index
         WHERE sent.kind='message.sent'
           AND json_extract(sent.body, '$.fields.to') IN (SELECT name FROM identity_names WHERE identity=?1 UNION SELECT ?1 UNION SELECT ?2)
           AND instr(sent.body, 'st3-work:')>0
         ORDER BY created_index, sent.subject",
    )?;
    let subjects = statement
        .query_map(params![recipient, bare_recipient], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    let mut messages = Vec::with_capacity(subjects.len());
    for (subject, created_index) in subjects {
        let message = store.message_view_cached(&connection, &subject, created_index)?;
        if message.to == recipient && message.tags.iter().any(|tag| tag.starts_with("st3-work:")) {
            messages.push(message);
        }
    }
    sort_messages_canonically(&connection, &mut messages)?;
    Ok(messages)
}

#[test]
fn a_pass_reads_only_the_work_wakes_sent_since_the_last() {
    let store = Store::open_memory("fixture").unwrap();
    for index in 0..3 {
        send(&store, &format!("wake-{index}"), "agent/worker", vec![format!("st3-work:step-run/{index}@1@1@worker")]);
    }
    send(&store, "ordinary", "agent/worker", vec![]);
    let read = || super::delivery::WORK_WAKE_SUBJECTS_READ.with(std::cell::Cell::get);
    let before = read();
    assert_eq!(store.work_wake_messages_for_reconcile("agent/worker").unwrap().len(), 3);
    assert_eq!(read() - before, 3);

    let before = read();
    let again = store.work_wake_messages_for_reconcile("agent/worker").unwrap();
    assert_eq!(read() - before, 0, "nothing was sent since the last pass");
    assert_eq!(
        serde_json::to_value(&again).unwrap(),
        serde_json::to_value(point_work_wakes(&store, "agent/worker").unwrap()).unwrap()
    );

    send(&store, "wake-3", "agent/worker", vec!["st3-work:step-run/3@1@1@worker".into()]);
    send(&store, "other", "agent/other", vec!["st3-work:step-run/other@1@1@other".into()]);
    let before = read();
    let after = store.work_wake_messages_for_reconcile("agent/worker").unwrap();
    assert_eq!(read() - before, 1);
    assert_eq!(after.len(), 4);
    assert_eq!(
        serde_json::to_value(&after).unwrap(),
        serde_json::to_value(point_work_wakes(&store, "agent/worker").unwrap()).unwrap()
    );
}

#[test]
fn work_wake_batch_preserves_closed_attempts_order_and_fresh_lifecycle_without_point_queries() {
    let store = Store::open_memory("fixture").unwrap();
    for index in 0..129 {
        send(
            &store,
            &format!("wake-{index:03}"),
            "agent/worker",
            vec![format!("st3-work:step-run/{index}@1@1@worker")],
        );
    }
    send(&store, "ordinary", "agent/worker", vec![]);
    send(
        &store,
        "other",
        "agent/other",
        vec!["st3-work:step-run/other@1@1@other".into()],
    );
    // Warm through the original point path, so both calls see the same warm cache.
    let expected = point_work_wakes(&store, "agent/worker").unwrap();
    assert_eq!(expected.len(), 129);
    let before = STATEMENTS_RUN.with(std::cell::Cell::get);
    let point_result = point_work_wakes(&store, "agent/worker").unwrap();
    let point_count = STATEMENTS_RUN.with(std::cell::Cell::get) - before;
    let before = STATEMENTS_RUN.with(std::cell::Cell::get);
    let batch_result = store
        .work_wake_messages_for_reconcile("agent/worker")
        .unwrap();
    let batch_count = STATEMENTS_RUN.with(std::cell::Cell::get) - before;
    assert_eq!(
        serde_json::to_value(&batch_result).unwrap(),
        serde_json::to_value(&point_result).unwrap()
    );
    // The batch adds two constant reads (the recipient's names and the newest index).
    assert!(
        point_count >= batch_count + 125,
        "point={point_count}, batch={batch_count}"
    );
    store
        .append_claim(&ClaimInput {
            subject: "message/wake-000".into(),
            kind: "message.closed".into(),
            actor: Some("daemon/runtime".into()),
            fields: BTreeMap::from([("status".into(), json!("closed"))]),
            evidence: vec![],
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
    let actual = store
        .work_wake_messages_for_reconcile("agent/worker")
        .unwrap();
    assert_eq!(
        actual.len(),
        129,
        "closed attempts must remain for retry accounting"
    );
    assert_eq!(
        actual
            .iter()
            .find(|m| m.subject == "message/wake-000")
            .unwrap()
            .status,
        "closed"
    );
    assert_eq!(
        serde_json::to_value(&actual).unwrap(),
        serde_json::to_value(point_work_wakes(&store, "agent/worker").unwrap()).unwrap()
    );
}

#[test]
fn a_pass_keeps_its_work_wake_views_when_the_shared_message_cache_empties() {
    let store = Store::open_memory("fixture").unwrap();
    for index in 0..3 {
        send(&store, &format!("wake-{index}"), "agent/worker", vec![format!("st3-work:step-run/{index}@1@1@worker")]);
    }
    let views = || super::delivery::WORK_WAKE_VIEWS_READ.with(std::cell::Cell::get);
    let decoded = || fractal_store::store::ACTUAL_HISTORY_BODIES_DECODED.with(std::cell::Cell::get);
    let before = views();
    assert_eq!(store.work_wake_messages_for_reconcile("agent/worker").unwrap().len(), 3);
    assert_eq!(views() - before, 3);

    // What a mailbox read past the shared cache's limit does to it.
    store.fractal.message_cache.lock().unwrap().clear();
    let (before, folded) = (views(), decoded());
    let again = store.work_wake_messages_for_reconcile("agent/worker").unwrap();
    assert_eq!(views() - before, 0, "no wake changed, so no view is read again");
    assert_eq!(decoded() - folded, 0, "no message history is folded again");
    assert_eq!(
        serde_json::to_value(&again).unwrap(),
        serde_json::to_value(point_work_wakes(&store, "agent/worker").unwrap()).unwrap()
    );

    store
        .append_claim(&ClaimInput {
            subject: "message/wake-1".into(),
            kind: "message.closed".into(),
            actor: Some("daemon/runtime".into()),
            fields: BTreeMap::from([("status".into(), json!("closed"))]),
            evidence: vec![],
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
    store.fractal.message_cache.lock().unwrap().clear();
    let before = views();
    let after = store.work_wake_messages_for_reconcile("agent/worker").unwrap();
    assert_eq!(views() - before, 1, "only the closed wake is read again");
    assert_eq!(after.iter().find(|m| m.subject == "message/wake-1").unwrap().status, "closed");
    store.fractal.message_cache.lock().unwrap().clear();
    assert_eq!(
        serde_json::to_value(&after).unwrap(),
        serde_json::to_value(point_work_wakes(&store, "agent/worker").unwrap()).unwrap()
    );
}
