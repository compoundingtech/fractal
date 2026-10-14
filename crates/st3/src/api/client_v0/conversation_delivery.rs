//! Optional receipt links on selected native user entries, never reconstructed from text.
use super::*;

pub(super) fn annotate<'a>(
    state: &AppState,
    snapshot: &ClientSnapshot,
    session: &ClientSession,
    session_id: &str,
    owner: Option<&str>,
    items: impl Iterator<Item = &'a mut Value>,
) -> Result<(), ApiError> {
    let Some(owner) = owner.filter(|_| session.conversation_delivery_links) else {
        return Ok(());
    };
    let mut items = items.take(CLIENT_MAX_PAGE_ITEMS + 1).collect::<Vec<_>>();
    if items.len() > CLIENT_MAX_PAGE_ITEMS {
        return Ok(());
    }
    let eligible = |item: &Value| {
        item["type"] == "message"
            && item["role"] == "user"
            && item["id"]
                .as_str()
                .is_some_and(|id| id.starts_with("timeline-entry/native-"))
    };
    let ids: BTreeSet<String> = items
        .iter()
        .filter(|item| eligible(item))
        .filter_map(|item| {
            item["body"]["message_id"]
                .as_str()
                .filter(|id| id.len() <= 256)
                .map(str::to_owned)
        })
        .collect();
    let receipts = state
        .store
        .conversation_delivery_receipts(session_id, owner, &ids, snapshot.store_index)
        .map_err(ApiError::internal)?;
    let mut links = crate::conversation_delivery::unique_links(session_id, receipts);
    let mut seen = BTreeSet::new();
    for item in items.iter().filter(|item| eligible(item)) {
        if let Some(id) = item["body"]["message_id"].as_str()
            && !seen.insert(id)
        {
            links.remove(id);
        }
    }
    for item in &mut items {
        if eligible(item)
            && let Some(link) = item["body"]["message_id"]
                .as_str()
                .and_then(|id| links.get(id))
        {
            item["delivers"] = json!(link);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_delivery_annotation_requires_negotiation_and_exact_receipt_identity() {
        let root = tempfile::tempdir().unwrap();
        let state = super::super::tests::test_state_named(root.path(), "delivery-annotation");
        let owner = "agent/eval.worker";
        let session_id = "session/external-0123456789abcdef01234567";
        crate::mailbox::tests::ready(&state.store, "session-1");
        let fence = state
            .store
            .bind_mailbox(&crate::mailbox::Fence::new(owner, "session-1", "delivery"))
            .unwrap();
        let input = |kind: &str, fields: Value| ClaimInput {
            subject: "message/linked".into(),
            kind: kind.into(),
            actor: Some(
                if kind == "message.sent" {
                    "person/fixture"
                } else {
                    owner
                }
                .into(),
            ),
            fields: serde_json::from_value(fields).unwrap(),
            evidence: vec![],
            expected_subject: None,
            idempotency_key: Some(format!("annotation:{kind}")),
        };
        state.store.append_claim(&input("message.sent", json!({"status":"sent","from":"person/fixture","to":owner,"content":"Invented mail"}))).unwrap();
        for status in ["staged", "delivered"] {
            state
                .store
                .append_mailbox_receipt_outcome(
                    &input(&format!("message.{status}"), json!({"status":status})),
                    &fence,
                )
                .unwrap();
        }
        state
            .store
            .append_native_mailbox_receipt_outcome(
                &input("message.read", json!({"status":"read"})),
                &fence,
                Some(&crate::conversation_delivery::NativeReceipt {
                    session_id: session_id.into(),
                    message_id: "native-qualified".into(),
                }),
            )
            .unwrap();
        let snapshot = new_client_snapshot(&state);
        let mut session = ClientSession::local(Some("person/fixture")).unwrap();
        let base = json!({"id":"timeline-entry/native-1","sequence":1,"revision":1,
            "timestamp":"2026-10-14T00:00:00Z","role":"user","type":"message","final":true,
            "body":{"message_id":"native-qualified","reply_to":null}});
        let mut items = vec![base.clone()];
        annotate(
            &state,
            &snapshot,
            &session,
            session_id,
            Some(owner),
            items.iter_mut(),
        )
        .unwrap();
        assert_eq!(items[0], base);
        session.conversation_delivery_links = true;
        annotate(
            &state,
            &snapshot,
            &session,
            session_id,
            Some(owner),
            items.iter_mut(),
        )
        .unwrap();
        assert_eq!(items[0]["delivers"], "message/linked");
        assert_eq!(items[0]["body"], base["body"]);
        let mut unmatched = vec![base.clone()];
        unmatched[0]["body"]["message_id"] = json!("native-without-receipt");
        annotate(
            &state,
            &snapshot,
            &session,
            session_id,
            Some(owner),
            unmatched.iter_mut(),
        )
        .unwrap();
        assert!(unmatched[0].get("delivers").is_none());
        let mut duplicated = vec![base.clone(), base.clone()];
        annotate(
            &state,
            &snapshot,
            &session,
            session_id,
            Some(owner),
            duplicated.iter_mut(),
        )
        .unwrap();
        assert!(duplicated.iter().all(|item| item.get("delivers").is_none()));
        session.conversation_delivery_links = false;
        let mut page = json!({"items":items});
        conversation_blocks::legacy(&mut page, &session);
        assert_eq!(page["items"][0], base);
    }
}
