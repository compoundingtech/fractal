//! Optional receipt links on selected native user entries, never reconstructed from text.
use super::*;

pub fn annotate<'a>(
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
