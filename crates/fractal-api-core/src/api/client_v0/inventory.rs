//! Screen inventories use the same authorized reads as HTTP, on the collection snapshot.
use crate::LimitsStore;
use super::*;


pub fn validate(request: &CollectionSubscribe) -> Result<(), ApiError> {
    if request.collection != "usage" && (request.since_ms.is_some() || request.until_ms.is_some()) {
        return Err(validation("period boundaries are supported for usage only"));
    }
    if matches!(
        request.collection.as_str(),
        "clients" | "usage" | "machines" | "sessions" | "terminals"
    ) && (request.person.is_some() || request.actor.is_some())
    {
        return Err(validation(
            "this inventory does not accept person or actor filters",
        ));
    }
    if request.collection == "devices" && request.actor.is_some() {
        return Err(validation("devices select a person, not an actor"));
    }
    if request
        .since_ms
        .zip(Some(request.until_ms.unwrap_or(client_now_ms() as u64)))
        .is_some_and(|(since, until)| since > until)
    {
        return Err(validation("usage start must be before its end"));
    }
    Ok(())
}

/// Shared serialization keeps the HTTP and followed usage reports identical.
pub fn usage_value(store: &Store, since_ms: u64, until_ms: u64) -> anyhow::Result<Value> {
    let crate::store::AttributedUsage {
        mut rows,
        agent_messages,
        limits,
        accounts,
    } = store.attributed_usage(since_ms, until_ms)?;
    for row in &mut rows {
        if let Some(fields) = row.as_object_mut() {
            fields.retain(|_, value| value.as_str() != Some(""));
        }
    }
    let limits = limits
        .into_iter()
        .map(|limit| {
            let mut value = serde_json::to_value(limit).expect("account limit is serializable");
            if let Some(fields) = value.as_object_mut() {
                fields.retain(|_, value| !value.is_null());
            }
            value
        })
        .collect::<Vec<_>>();
    Ok(
        json!({"since_ms":since_ms, "until_ms":until_ms, "rows":rows, "limits":limits, "accounts":accounts, "agent_messages":agent_messages}),
    )
}

pub fn items(
    state: &AppState,
    session: &ClientSession,
    request: &CollectionSubscribe,
    snapshot: &ClientSnapshot,
) -> anyhow::Result<(Vec<Value>, bool)> {
    let limit = request.limit.unwrap_or(CLIENT_DEFAULT_PAGE_ITEMS);
    if let Some(deadline) = &request.inventory_deadline {
        *deadline.lock().expect("inventory deadline") = session.grant_expires_at;
    }
    let mut items = match request.collection.as_str() {
        "clients" => {
            let mut rows = client_connection_items(state, snapshot)
                .map_err(|error| anyhow::anyhow!(error.message))?;
            if let Some(deadline) = &request.inventory_deadline {
                let expiry = super::super::client_presence::next_expiry(&rows);
                let mut deadline = deadline.lock().expect("inventory deadline");
                *deadline = deadline.iter().copied().chain(expiry).min();
            }
            for row in &mut rows {
                // The grouping tuple is stable across connect/disconnect and has no ambiguous separators.
                let identity =
                    json!([row["member"], row["actor"], row["client"], row["via"]]).to_string();
                row["id"] = json!(format!(
                    "connected-client/{}",
                    hex::encode(Sha256::digest(identity.as_bytes()))
                ));
                row["kind"] = json!("connected-client");
                row["updated_at"] = row["last_seen"].clone();
                row["revision"] = json!(hex::encode(Sha256::digest(row.to_string().as_bytes())));
            }
            rows
        }
        "usage" => {
            // The graph clock can predate this period on an idle or empty member.
            // Open periods end at wall time, just as the authorized HTTP read does.
            let until = request.until_ms.unwrap_or(client_now_ms() as u64);
            let since = request.since_ms.unwrap_or(until.saturating_sub(86_400_000));
            anyhow::ensure!(since <= until, "usage start must be before its end");
            let mut value = usage_value(&state.store, since, until)?;
            let rows = value["rows"].as_array_mut().expect("usage rows");
            let has_more = rows.len() > limit;
            rows.truncate(limit);
            let revision = hex::encode(Sha256::digest(value.to_string().as_bytes()));
            return Ok((
                vec![
                    json!({"id":"usage/current", "kind":"usage", "revision":revision,
                "updated_at":snapshot.created_at, "value":value}),
                ],
                has_more,
            ));
        }
        "machines" => machine_resources(state, false, snapshot, session)?,
        "devices" => {
            let person = person_filter(session, request.person.as_deref())
                .map_err(|error| anyhow::anyhow!(error.message))?
                .filter(|person| person.starts_with("person/"))
                .ok_or_else(|| {
                    anyhow::anyhow!("device inventory requires an explicitly authenticated person")
                })?;
            let mut rows = device_resources(state, snapshot, &person)
                .map_err(|error| anyhow::anyhow!(error.message))?;
            rows.retain(|row| row["state"] == "active");
            rows
        }
        "sessions" => {
            let managed = managed_native_session_ids(&state.store, snapshot.store_index)?;
            let discovery = crate::external_sessions::discover_fresh(
                state.native_session_home.as_deref(),
                false,
            )?;
            let mut rows = discovery
                .sessions
                .into_iter()
                .filter(|session| !managed.contains(&session.native_id))
                .map(external_session_resource)
                .collect::<Vec<_>>();
            rows.extend(
                discovery
                    .unresolved_processes
                    .into_iter()
                    .map(|process| unresolved_session_resource(process, &snapshot.created_at)),
            );
            rows.sort_by(|left, right| {
                right["updated_at"]
                    .as_str()
                    .cmp(&left["updated_at"].as_str())
                    .then_with(|| left["id"].as_str().cmp(&right["id"].as_str()))
            });
            rows
        }
        "terminals" => {
            let mut rows = terminal_resources_for_owner(state, false, snapshot, session, None)?;
            rows.retain(|row| row["terminal_id"].is_string());
            rows
        }
        _ => unreachable!(),
    };
    if request.collection == "devices"
        && let Some(deadline) = &request.inventory_deadline
    {
        let expiry = items
            .iter()
            .filter_map(|row| row["expires_at"].as_str())
            .filter_map(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
            .map(|at| at.timestamp_millis().max(0) as u128)
            .min();
        let mut deadline = deadline.lock().expect("inventory deadline");
        *deadline = deadline.iter().copied().chain(expiry).min();
    }
    if let Some(processes) = &request.native_processes {
        *processes.lock().expect("native process coverage") = items
            .iter()
            .filter_map(|row| row.pointer("/process/pid").and_then(Value::as_u64))
            .map(|pid| pid as u32)
            .collect();
    }
    let has_more = items.len() > limit;
    items.truncate(limit);
    Ok((items, has_more))
}

/// Schedule actual membership expirations, without a timer that rereads idle inventories.
pub fn next_expiry(subscription: &CollectionSubscription) -> Option<u128> {
    subscription
        .request
        .inventory_deadline
        .as_ref()
        .and_then(|deadline| *deadline.lock().expect("inventory deadline"))
}

