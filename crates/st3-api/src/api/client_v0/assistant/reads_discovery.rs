//! Keyset discovery pages: select a bounded source slice, then fit the complete envelope.
use super::*;

pub(super) async fn page(
    state: &AppState, snapshot: &ClientSnapshot, projection: &str,
    parameters: &BTreeMap<String, String>, max_bytes: usize, binding: &str,
    context: &response_body::Context,
) -> Result<(ClientSnapshot, Value), ApiError> {
    allow(parameters, METADATA_FIELDS, max_bytes)?;
    owner(state)?;
    let requested_limit = parameters.get("limit").map(|value| value.parse::<usize>().ok()
        .filter(|limit| (1..=50).contains(limit))
        .ok_or_else(|| validation("limit must be between 1 and 50"))).transpose()?;
    let collection = format!("assistant.{projection}:{binding}");
    let cursor = parameters.get("cursor").map(|value| decode_client_cursor(value)).transpose()?;
    if let Some(cursor) = &cursor {
        if cursor.collection != collection || cursor.snapshot.id != snapshot.id
            || cursor.snapshot.store_index != snapshot.store_index
            || cursor.expires_at_unix_ms < client_now_ms()
            || !(1..=50).contains(&cursor.limit)
            || requested_limit.is_some_and(|limit| limit != cursor.limit)
            || cursor.items_digest != "assistant-source-keyset-v1"
        {
            return Err(client_page_expired("the discovery cursor does not match its collection, grant, policy or snapshot"));
        }
    }
    let limit = cursor.as_ref().map_or(requested_limit.unwrap_or(10), |cursor| cursor.limit);
    let expires = cursor.as_ref().map_or_else(|| client_now_ms().saturating_add(CLIENT_PAGE_TTL_MS), |cursor| cursor.expires_at_unix_ms);
    let after = cursor.as_ref().and_then(|cursor| cursor.after_key.as_ref()).map(|(_, id)| id.clone());
    let reader = state.clone();
    let pinned = snapshot.clone();
    let continuation = cursor.is_some();
    let (snapshot, mut sources) = blocking_store(move || reader.store.clone().read_snapshot(|index| {
        let snapshot = if continuation { pinned } else { client_snapshot_at(&reader, index) };
        let sources: Vec<_> = reader.store.assistant_conversation_owners_page(after.as_deref(), limit, snapshot.store_index)?
                .into_iter().map(|(id, owner)| {
                    let incarnation = owner.incarnation.as_deref().or(owner.runtime.as_deref());
                    let value = incarnation.map(|incarnation| {
                        let session = client_session_id(&id, incarnation);
                        json!({"id":id,"agent_id":id,"session_id":session,"current_session_id":session,"read":null})
                    });
                    (id, value)
                }).collect();
        Ok((snapshot, sources))
    })).await?;
    // Source selection is bounded, but not yet a delivered page. Fit each item with its
    // candidate continuation using the gateway codec; advance only past delivered/skipped sources.
    let source_more = sources.len() > limit;
    sources.truncate(limit);
    let source_count = sources.len();
    let mut items = Vec::with_capacity(source_count);
    let mut last = cursor.as_ref().and_then(|cursor| cursor.after_key.as_ref()).map(|(_, id)| id.clone());
    for (position, (id, source)) in sources.into_iter().enumerate() {
        let Some(mut item) = source else { last = Some(id); continue; };
        item["read"] = conversation_reads::discovery_read(state, &snapshot, &item, binding).await?;
        items.push(item);
        let has_more = source_more || position + 1 < source_count;
        let candidate = value(state, &snapshot, &collection, projection, &items, Some(&id), has_more, limit, expires)?;
        let bytes = context.value_envelope_size(&snapshot, &candidate)?;
        if bytes > max_bytes {
            items.pop();
            if items.is_empty() {
                return Err(budget_exceeded(format!("single {projection} item `{id}` cannot fit the approved response byte budget")));
            }
            let result = value(state, &snapshot, &collection, projection, &items, last.as_deref(), true, limit, expires)?;
            return Ok((snapshot, result));
        }
        last = Some(id);
    }
    let result = value(state, &snapshot, &collection, projection, &items, last.as_deref(), source_more, limit, expires)?;
    Ok((snapshot, result))
}

#[allow(clippy::too_many_arguments)]
fn value(
    state: &AppState, snapshot: &ClientSnapshot, collection: &str, projection: &str,
    items: &[Value], after: Option<&str>, has_more: bool, limit: usize, expires: u128,
) -> Result<Value, ApiError> {
    let next = if has_more {
        Some(encode_client_cursor(&ClientPageCursor {
            snapshot:snapshot.clone(), collection:collection.into(), offset:0, limit,
            history:false, person:None, actor:None, owner_run:None, status:None, owner:None,
            state:None, native_only:false, items_digest:"assistant-source-keyset-v1".into(),
            before_index:None, after_key:after.map(|id| (0,id.to_owned())), expires_at_unix_ms:expires,
        })?)
    } else { None };
    Ok(json!({"kind":"page","collection":format!("assistant.{projection}"),"filters":{},"items":items,
        "page":{"limit":limit,"has_more":has_more,"next_cursor":next,
            "cursor_expires_at":has_more.then(|| client_timestamp(expires))},
        "sync":client_sync_notice(state),"replicated":null}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn discovery_packs_exact_envelopes_and_resumes_after_last_delivered_item() {
        use axum::response::IntoResponse as _;

        let root = tempfile::tempdir().unwrap();
        let (state, _, _, _) = super::super::tests::managed_conversation(root.path(), "discovery text");
        let mut expected = BTreeSet::from(["agent/reads-codex".to_owned()]);
        for number in 0..12 {
            let id = format!("agent/page-{number:02}");
            state.store.append_claim(&ClaimInput {
                subject:id.clone(), kind:"runtime.observed".into(), actor:Some(id.clone()),
                fields:serde_json::from_value(json!({"status":"running", "runtime_id":"page-runtime", "incarnation_id":"page-incarnation"})).unwrap(),
                evidence:Vec::new(), expected_subject:None, idempotency_key:None,
            }).unwrap();
            expected.insert(id);
        }
        let mut snapshot = new_client_snapshot(&state);
        let context = response_body::Context {
            state:state.clone(), client_request:true, observation_report:false,
            request_id:format!("request/{}\n", "escaped-\"".repeat(24)),
            client_snapshot:Some(snapshot.clone()), server:None, roster:false,
        };
        let mut parameters = BTreeMap::from([("limit".into(), "20".into())]);
        let mut delivered = BTreeSet::new();
        let mut pages = 0;
        loop {
            let (cut, value) = page(&state, &snapshot, "conversations", &parameters,
                DEFAULT_RESPONSE_BYTES, "test-grant-policy", &context).await.unwrap();
            snapshot = cut;
            let items = value["items"].as_array().unwrap();
            assert!(!items.is_empty());
            for item in items {
                assert!(delivered.insert(item["id"].as_str().unwrap().to_owned()), "continuation repeated an item");
            }
            let measured = context.value_envelope_size(&snapshot, &value).unwrap();
            let response = context.wrap((Extension(snapshot.clone()), Json(value.clone())).into_response()).await.unwrap();
            let encoded = axum::body::to_bytes(response.into_body(), MAX_RESPONSE_BYTES).await.unwrap();
            assert_eq!(encoded.len(), measured, "packer and delivery use identical envelope framing");
            assert!(encoded.len() <= DEFAULT_RESPONSE_BYTES);
            pages += 1;
            assert!(pages <= expected.len());
            let Some(next) = value["page"]["next_cursor"].as_str() else { break; };
            assert!(items.len() < 20, "byte budget, not count limit, packs this page");
            let cursor = decode_client_cursor(next).unwrap();
            assert_eq!(cursor.after_key.unwrap().1, items.last().unwrap()["id"].as_str().unwrap());
            parameters.insert("cursor".into(), next.into());
        }
        assert!(pages > 1);
        assert_eq!(delivered, expected, "byte-shortened pages neither skip nor duplicate source identities");
    }
}
