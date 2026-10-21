//! Page-sized assistant source reads. Mission cursors seek by key; attention and roster
//! cursors reference an existing publication. Only the offline agent path caches shallow refs.
use crate::AttentionListStore;
use super::*;

#[allow(clippy::too_many_arguments)]
pub async fn page(
    state: &AppState,
    snapshot: &ClientSnapshot,
    projection: &str,
    parameters: &BTreeMap<String, String>,
    max_bytes: usize,
    binding: &str,
    context: &response_body::Context,
) -> Result<(ClientSnapshot, Value), ApiError> {
    let agent_id = if matches!(projection, "agents" | "agents-rich" | "attention" | "attention-rich") {
        allow(parameters, &["cursor", "limit", "max_response_bytes", "agent_id"], max_bytes)?;
        parameters.get("agent_id").map(|id| {
            if identifier_allowed(id) && id.starts_with("agent/") && id.len() > "agent/".len() {
                Ok(id.clone())
            } else {
                Err(validation("agent_id must be an exact canonical agent identifier"))
            }
        }).transpose()?
    } else {
        allow(parameters, METADATA_FIELDS, max_bytes)?;
        None
    };
    let limit = parameters.get("limit").map(|limit| {
        limit.parse::<usize>().ok()
            .filter(|limit| (1..=CLIENT_MAX_PAGE_ITEMS).contains(limit))
            .ok_or_else(|| validation(format!("limit must be between 1 and {CLIENT_MAX_PAGE_ITEMS}")))
    }).transpose()?;
    let query = ClientListQuery {
        limit, cursor: parameters.get("cursor").cloned(), ..Default::default()
    };
    // The native cursor's collection field also binds the disclosure class, grant and
    // policy revision. Authorization still precedes every first and subsequent page.
    let collection = match &agent_id {
        Some(id) => format!("assistant.{projection}:{binding}:agent_id={id}"),
        None => format!("assistant.{projection}:{binding}"),
    };
    let cursor = query.cursor.as_deref().map(decode_client_cursor).transpose()?;
    if let Some(cursor) = &cursor
        && (cursor.collection != collection
            || cursor.snapshot.id != snapshot.id
            || cursor.snapshot.store_index != snapshot.store_index
            || limit.is_some_and(|limit| limit != cursor.limit)
            || !(1..=CLIENT_MAX_PAGE_ITEMS).contains(&cursor.limit)
            || cursor.expires_at_unix_ms < client_now_ms())
    {
        return Err(client_page_expired("the assistant cursor does not match this collection, grant, policy, or snapshot"));
    }
    let person = owner(state)?;
    let reader = state.clone();
    let snapshot = snapshot.clone();
    let projection = projection.to_owned();
    let context = context.clone();
    blocking_store(move || reader.store.clone().read_snapshot(|index| {
        Ok(read_page(&reader, snapshot, &projection, &collection, &person, agent_id.as_deref(), &query, cursor, index, &context, max_bytes))
    })).await?
}

#[allow(clippy::too_many_arguments)]
fn read_page(
    state: &AppState,
    snapshot: ClientSnapshot,
    projection: &str,
    collection: &str,
    person: &str,
    agent_id: Option<&str>,
    query: &ClientListQuery,
    cursor: Option<ClientPageCursor>,
    index: u64,
    context: &response_body::Context,
    max_bytes: usize,
) -> Result<(ClientSnapshot, Value), ApiError> {
    let snapshot = if cursor.is_some() { snapshot } else { client_snapshot_at(state, index) };
    let now = client_snapshot_time(&snapshot);
    let rich = projection.ends_with("-rich");
    let limit = cursor.as_ref().map_or(query.limit.unwrap_or(CLIENT_DEFAULT_PAGE_ITEMS), |cursor| cursor.limit);
    let expires = cursor.as_ref().map_or_else(|| client_now_ms().saturating_add(CLIENT_PAGE_TTL_MS), |cursor| cursor.expires_at_unix_ms);
    let offset = cursor.as_ref().map_or(0, |cursor| cursor.offset);
    let mut delivered_keys = Vec::new();
    let (snapshot, items, has_more, marker) = if projection.starts_with("missions") {
        // Recheck bounded claim metadata, not cards. Read-authorization receipts and
        // unrelated custom claims must not invalidate every signed continuation.
        check_mission_source(state, cursor.as_ref(), index)?;
        // Admission can precede projection, and full replay can replace projections at
        // the same claim cut. Both fences share the page's pinned SQLite snapshot.
        let frontier = state.store.attention_projection_frontier().map_err(ApiError::internal)?;
        let generation = smallclaims::store::graph_generation(&state.store.readers.get())
            .map_err(ApiError::internal)?;
        let marker = format!("assistant-mission-keyset-v3:{frontier:?}:{generation}");
        check_source(&cursor, &marker)?;
        let mut keys = state.store.assistant_mission_collection_page_at(
            false, 0, limit.saturating_add(1),
            cursor.as_ref().and_then(|cursor| cursor.after_key.as_ref()), now,
        ).map_err(ApiError::internal)?;
        let has_more = keys.len() > limit;
        keys.truncate(limit);
        let ids = keys.iter().map(|(id, _)| id.clone()).collect::<Vec<_>>();
        // Hydrate only selected missions, explicitly retaining their relevant live run
        // even when it is older than the native recent-run preview.
        let cards = assistant_mission_list_cards_with(&state.store, &ids, now)
            .map_err(ApiError::internal)?;
        delivered_keys = keys.into_iter().map(|(id, time)| (time, id)).collect();
        (snapshot, mission_rows(&cards, rich).map_err(ApiError::internal)?, has_more, marker)
    } else if projection.starts_with("attention") {
        let (cut, published_at, rows) = published_assistant_attention_rows(&state.store, index, person)
            .ok_or_else(|| published_view_not_ready("attention"))?;
        let marker = format!("assistant-attention:{cut}:{published_at}");
        check_source(&cursor, &marker)?;
        let mut matching = rows.iter().filter(|row|
            agent_id.is_none_or(|target| attention_agent_id(row) == Some(target))).skip(offset);
        let items = matching.by_ref().take(limit)
            .map(|row| attention_row(row, rich)).collect::<anyhow::Result<Vec<_>>>()
            .map_err(ApiError::internal)?;
        if cursor.is_some() && items.is_empty() {
            return Err(client_page_expired("the attention cursor is outside its source publication"));
        }
        let has_more = matching.next().is_some();
        let snapshot = if cursor.is_some() { snapshot } else { roster_snapshot(state, cut, published_at) };
        (snapshot, items, has_more, marker)
    } else if state.store.agent_roster_refresher_running() {
        let publication = if cursor.is_some() {
            state.store.published_agent_roster_at(snapshot.store_index, false)
                .map(|(cards, time)| (snapshot.store_index, cards, time))
        } else {
            state.store.published_agent_roster(index, false)
        };
        let Some((cut, cards, published_at)) = publication else {
            state.store.request_agent_roster_refresh();
            return Err(if cursor.is_some() {
                client_page_expired("the agent publication changed; restart pagination")
            } else { published_view_not_ready("agents") });
        };
        if cut < index { state.store.request_agent_roster_refresh(); }
        let marker = format!("assistant-agents:{cut}:{published_at}");
        check_source(&cursor, &marker)?;
        // Count assistant-visible membership before applying offsets; clone only the
        // selected cards, never the whole publication or its excluded identifiers.
        let window = offset..offset.saturating_add(limit);
        let mut visible_len = 0;
        let mut selected = Vec::new();
        for card in cards.iter().filter(|card| card["id"].as_str()
            .is_some_and(|id| identifier_allowed(id) && id.starts_with("agent/")
                && agent_id.is_none_or(|target| target == id)))
        {
            if window.contains(&visible_len) {
                selected.push(card.clone());
            }
            visible_len += 1;
        }
        if cursor.is_some() && offset >= visible_len {
            return Err(client_page_expired("the agent cursor is outside its source publication"));
        }
        let snapshot = if cursor.is_some() { snapshot } else { roster_snapshot(state, cut, published_at) };
        let end = window.end.min(visible_len);
        overlay_agent_resources(&state.store, &mut selected, &snapshot.created_at).map_err(ApiError::internal)?;
        (snapshot, agent_rows(&selected, rich), end < visible_len, marker)
    } else {
        // Reuse native shallow membership, but filter before caching positions and
        // materializing selected cards so every cached ref projects to one visible row.
        let refs = if cursor.is_some() { Vec::new() } else {
            let mut refs = client_agent_page_refs(&state.store, false, index).map_err(ApiError::internal)?;
            refs.retain(|reference| reference["id"].as_str()
                .is_some_and(|id| identifier_allowed(id) && id.starts_with("agent/")
                    && agent_id.is_none_or(|target| target == id)));
            refs
        };
        let page = client_page_read_projected(
            state, &snapshot, collection, &format!("assistant.{projection}"), refs, query,
            Some((context, max_bytes)), |refs| {
                let cards = client_agent_cards_for_page(
                    &state.store, false, snapshot.store_index, &refs, &snapshot.created_at,
                )?;
                Ok(agent_rows(&cards, rich))
            },
        )?;
        return Ok((snapshot, serde_json::to_value(page).map_err(ApiError::internal)?));
    };
    let fetched = items.len();
    let page = client_pack_page(&snapshot, items, Some((context, max_bytes)), |count| {
        let has_more = has_more || count < fetched;
        let next_cursor = if has_more {
            Some(encode_client_cursor(&ClientPageCursor {
                snapshot: snapshot.clone(), collection: collection.to_owned(),
                offset: offset.saturating_add(count), limit,
                history: false, person: None, actor: None, owner_run: None,
                status: None, owner: None, state: None, native_only: false,
                items_digest: marker.clone(), before_index: None,
                after_key: count.checked_sub(1).and_then(|last| delivered_keys.get(last)).cloned(),
                expires_at_unix_ms: expires,
            })?)
        } else { None };
        Ok(ClientResourcePage {
            kind: "page".into(), collection: format!("assistant.{projection}"),
            filters: client_page_filters("", query), items: Vec::new(),
            page: ClientPageInfo {
                limit, has_more, next_cursor,
                cursor_expires_at: has_more.then(|| client_timestamp(expires)),
            },
            sync: client_sync_notice(state), replicated: None,
        })
    })?;
    Ok((snapshot, serde_json::to_value(page).map_err(ApiError::internal)?))
}

fn check_mission_source(state: &AppState, cursor: Option<&ClientPageCursor>, index: u64) -> Result<(), ApiError> {
    let Some(cursor) = cursor else { return Ok(()); };
    const CHANGE_LIMIT: usize = 512;
    let connection = state.store.readers.get();
    let mut statement = connection.prepare_cached(
        "SELECT subject,kind FROM claims WHERE store_index>?1 AND store_index<=?2
         ORDER BY store_index LIMIT ?3",
    ).map_err(ApiError::internal)?;
    let mut changes = statement.query(
        rusqlite::params![cursor.snapshot.store_index, index, CHANGE_LIMIT + 1],
    ).map_err(ApiError::internal)?;
    let mut count = 0;
    while let Some(change) = changes.next().map_err(ApiError::internal)? {
        let subject = change.get_ref(0).map_err(ApiError::internal)?.as_str().map_err(ApiError::internal)?;
        let kind = change.get_ref(1).map_err(ApiError::internal)?.as_str().map_err(ApiError::internal)?;
        // Use the canonical registry's subject families, including all step claims
        // (standalone person asks can create missions). Repairs can replace old inputs.
        let family = st3_schema::registry().subject(subject).map(|spec| spec.family.as_str());
        if count == CHANGE_LIMIT
            || matches!(family, Some("mission" | "mission-run" | "step-run" | "run-generation" | "revision-proposal"))
            || matches!(kind, "repair.applied" | "record.repaired")
        {
            return Err(client_page_expired("mission inputs changed; restart pagination"));
        }
        count += 1;
    }
    Ok(())
}

fn check_source(cursor: &Option<ClientPageCursor>, marker: &str) -> Result<(), ApiError> {
    if cursor.as_ref().is_some_and(|cursor| cursor.items_digest != marker) {
        return Err(client_page_expired("the assistant source publication changed; restart pagination"));
    }
    Ok(())
}
