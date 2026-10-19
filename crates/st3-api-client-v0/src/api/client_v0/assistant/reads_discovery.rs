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
    let request_id = context.request_id.clone();
    let reader = state.clone();
    let pinned = snapshot.clone();
    let work = projection == "work";
    let continuation = cursor.is_some();
    let (snapshot, mut sources) = blocking_store(move || reader.store.clone().read_snapshot(|index| {
        let snapshot = if continuation { pinned } else { client_snapshot_at(&reader, index) };
        let now = client_snapshot_time(&snapshot);
        let sources = if work {
            let ids = reader.store.assistant_work_candidate_ids_page(after.as_deref(), limit, now, index)?;
            let selected = ids.iter().cloned().collect::<BTreeSet<_>>();
            let visible = reader.store.work_at_snapshot_selected(None, false, now, true, true, false, Some(&selected))?
                .into_iter().map(|item| item.subject).collect::<BTreeSet<_>>();
            ids.into_iter().map(|id| {
                let value = if visible.contains(&id) {
                    client_work_item(&reader.store, &id, None, now, index)?
                        .map(|source| {
                            let mut value = operational_reads::work_detail(&source);
                            value["evidence_refs"] = operational_reads::evidence_refs(&reader.store, &id)?;
                            Ok::<_, anyhow::Error>(value)
                        }).transpose()?
                } else { None };
                Ok((id, value))
            }).collect::<anyhow::Result<Vec<_>>>()?
        } else {
            reader.store.assistant_conversation_owners_page(after.as_deref(), limit, snapshot.store_index)?
                .into_iter().map(|(id, owner)| {
                    let incarnation = owner.incarnation.as_deref().or(owner.runtime.as_deref());
                    let value = incarnation.map(|incarnation| {
                        let session = client_session_id(&id, incarnation);
                        json!({"id":id,"agent_id":id,"session_id":session,"current_session_id":session,"read":null})
                    });
                    (id, value)
                }).collect()
        };
        Ok((snapshot, sources))
    })).await?;
    let source_more = sources.len() > limit;
    sources.truncate(limit);
    let source_count = sources.len();
    let mut items = Vec::with_capacity(source_count);
    let mut last = cursor.as_ref().and_then(|cursor| cursor.after_key.as_ref()).map(|(_, id)| id.clone());
    for (position, (id, source)) in sources.into_iter().enumerate() {
        let Some(mut item) = source else { last = Some(id); continue; };
        if !work {
            item["read"] = conversation_reads::discovery_read(state, &snapshot, &item, binding).await?;
        }
        items.push(item);
        let has_more = source_more || position + 1 < source_count;
        let candidate = value(state, &snapshot, &collection, projection, &items, Some(&id), has_more, limit, expires)?;
        let bytes = serde_json::to_vec(&json!({"api_version":CLIENT_API_VERSION,"request_id":request_id,"snapshot":snapshot,"value":candidate}))
            .map_err(ApiError::internal)?.len();
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
    async fn work_pages_fit_budget_and_continue_without_skipping_items() {
        let root = tempfile::tempdir().unwrap();
        let state = super::super::super::super::tests::test_state_named(root.path(), "work-discovery");
        let steps = (0..20).map(|i| format!("step \"step-{i:02}\" {{ agentless; goal \"A bounded goal.\"; }}\n")).collect::<String>();
        let source = format!("version 2\nmission \"paged-work\" state=\"ready\" {{ goal \"Page work.\"; {steps} }}");
        let intent = crate::graph::parse_internal_intent(&source, "work-discovery").unwrap();
        state.store.apply_internal(&intent, "paged-work-publication").unwrap();
        let run = state.store.create_mission_run(&crate::model::MissionRunRequest {
            mission:"paged-work".into(), revision:None, workspace:root.path().display().to_string(),
            requester:Some("person/operator".into()), mode:None, inputs:BTreeMap::new(), idempotency_key:"paged-work-run".into(),
        }).unwrap();
        for step in run.steps {
            state.store.set_step_state(&step.subject, "ready", None).unwrap();
        }
        let mut snapshot = new_client_snapshot(&state);
        let mut query = BTreeMap::from([("limit".into(),"20".into())]);
        let mut ids = BTreeSet::new();
        let mut pages = 0;
        loop {
            let (cut, value) = page(&state, &snapshot, "work", &query, DEFAULT_RESPONSE_BYTES, "test-policy", &test_context(&state, &snapshot)).await.unwrap();
            snapshot = cut;
            let bytes = serde_json::to_vec(&json!({"api_version":CLIENT_API_VERSION,"request_id":"","snapshot":snapshot,"value":value})).unwrap();
            assert!(bytes.len() <= DEFAULT_RESPONSE_BYTES);
            for item in value["items"].as_array().unwrap() {
                assert!(ids.insert(item["id"].as_str().unwrap().to_owned()), "cursor repeated an item");
            }
            pages += 1;
            assert!(pages <= 20, "cursor must advance");
            let Some(cursor) = value["page"]["next_cursor"].as_str() else { break; };
            assert!(value["items"].as_array().unwrap().len() < 20);
            query.insert("cursor".into(), cursor.into());
        }
        assert_eq!(ids.len(), 20);
        assert!(pages > 1);
        let error = page(&state, &new_client_snapshot(&state), "work",
            &BTreeMap::new(), 64, "test-policy", &test_context(&state, &new_client_snapshot(&state))).await.unwrap_err();
        assert_eq!(error.code, "response-budget-exceeded");
        assert!(error.message.contains("single work item `step-run/"));
    }
}
