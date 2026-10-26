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
    assert!(
        point_count >= batch_count + 127,
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
