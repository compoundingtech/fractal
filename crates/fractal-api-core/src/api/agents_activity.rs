//! Internal, read-only workspace inventory. The native page cache retains the entire
//! immutable fold, including incomplete rows, so later pages cannot forget a gap.
use super::*;

#[path = "agents_activity_runtime.rs"]
mod runtime_inventory;

#[derive(Default, Deserialize)]
pub(super) struct ActivityQuery {
    limit: Option<usize>,
    cursor: Option<String>,
    min_cut: Option<u64>,
    host: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "kebab-case")]
enum IncompleteReason {
    NoActual,
    IndeterminateAuthority,
    MissingWorkspace,
    MissingHost,
    MissingIncarnation,
    UnresolvedLiveness,
}

#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
enum ActivityStatus {
    Desired, Starting, Running, Waiting, Down, Stopped, Failed, Suspended, Retired, Unknown,
}

impl ActivityStatus {
    fn from_actual(raw: Option<&str>) -> (Self, Option<&str>) {
        let status = match raw {
            Some("desired") => Self::Desired,
            Some("starting") => Self::Starting,
            Some("running") => Self::Running,
            Some("waiting") => Self::Waiting,
            Some("down") => Self::Down,
            Some("stopped") => Self::Stopped,
            Some("failed") => Self::Failed,
            Some("suspended") => Self::Suspended,
            Some("retired") => Self::Retired,
            _ => return (Self::Unknown, raw),
        };
        (status, None)
    }
}

#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
enum ActivityLayer {
    Current,
    Historical,
}

pub(super) async fn list(
    State(state): State<AppState>,
    Query(query): Query<ActivityQuery>,
) -> Result<Json<Value>, ApiError> {
    if query.limit.is_some_and(|limit| !(1..=200).contains(&limit)) {
        return Err(client_v0::validation("limit must be between 1 and 200"));
    }
    if query.cursor.is_some() && query.min_cut.is_some() {
        return Err(client_v0::validation("min_cut is only supported on the first activity page"));
    }
    // Collection identity binds both the filter and the immutable fold's completeness.
    // Unlike a caller-provided running bit, this identity is checked against retained pages.
    let filter = query.host.as_deref()
        .map_or_else(|| "all".to_owned(), |host| format!("host-{}", urlencoding::encode(host)));
    let prefix = format!("agents-activity/{filter}/");
    let cursor = query.cursor.as_deref().map(decode_client_cursor).transpose()?;
    let (snapshot, collection, items) = if let Some(cursor) = &cursor {
        if cursor.snapshot.host_id != client_host_id(&state.node)
            || (cursor.collection != format!("{prefix}complete")
                && cursor.collection != format!("{prefix}incomplete"))
        {
            return Err(client_page_expired("the activity cursor does not match this host or filter"));
        }
        (cursor.snapshot.clone(), cursor.collection.clone(), Vec::new())
    } else {
        let reader = state.clone();
        let host = query.host.clone();
        if let Some(wanted) = query.min_cut {
            let head = state.store.index().map_err(ApiError::internal)?;
            if wanted > head {
                let mut error = agent_roster_cut_not_ready(wanted);
                error.details.insert("head".into(), json!(head));
                return Err(error);
            }
        }
        let (snapshot, items) = read_deadline::spawn_blocking(move || {
            reader.store.read_snapshot(|cut| {
                // Retain all known agent subjects and their presentation layer, but never
                // use the presentation fold's actual fields for liveness or attribution.
                let status = reader.store.agent_card_status_for_generic_reads_at(None, cut, true)?;
                let items = runtime_inventory::read(&reader.store, &status.subjects, cut, host.as_deref())?;
                Ok((client_snapshot_at(&reader, cut), items))
            })
        }).await.map_err(ApiError::internal)?.map_err(ApiError::internal)?;
        let complete = !items.iter().any(|row| row.get("reason").is_some());
        let collection = format!("{prefix}{}", if complete { "complete" } else { "incomplete" });
        (snapshot, collection, items)
    };
    let page_query = ClientListQuery {
        limit: query.limit.or_else(|| cursor.is_none().then_some(200)),
        cursor: query.cursor,
        ..Default::default()
    };
    let page = client_page_read(&state, &snapshot, &collection, items, &page_query, true)?;
    let complete = collection.ends_with("/complete") && !page.page.has_more;
    let (incomplete, agents): (Vec<_>, Vec<_>) = page.items.into_iter()
        .partition(|row| row.get("reason").is_some());
    let mut snapshot = serde_json::to_value(snapshot).map_err(ApiError::internal)?;
    snapshot["local_host_id"] = json!(client_host_id(&state.node));
    Ok(Json(json!({
        "snapshot": snapshot,
        "agents": agents,
        "incomplete": incomplete,
        "next_cursor": page.page.next_cursor,
        "complete": complete,
    })))
}

#[cfg(test)]
#[path = "agents_activity_tests.rs"]
mod tests;
