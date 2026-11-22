//! Internal, read-only workspace inventory. The native page cache retains the entire
//! immutable fold, including incomplete rows, so later pages cannot forget a gap.
use super::*;

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

fn activity_row(subject: crate::model::SubjectStatus) -> Value {
    let actual = subject.actual.as_ref();
    let workspace = actual.and_then(|actual| actual.get("workspace")).and_then(Value::as_str)
        .filter(|workspace| Path::new(workspace).is_absolute());
    let origin = subject.actual_origin.as_deref().filter(|origin| !origin.is_empty());
    let reason = if actual.is_none() || subject.actual_claim.is_none() {
        Some(IncompleteReason::NoActual)
    } else if subject.reason.as_deref() == Some(crate::store::INDETERMINATE_ACTUAL_AUTHORITY) {
        Some(IncompleteReason::IndeterminateAuthority)
    } else if origin.is_none() {
        Some(IncompleteReason::MissingHost)
    } else if workspace.is_none() {
        Some(IncompleteReason::MissingWorkspace)
    } else {
        None
    };
    if let Some(reason) = reason {
        return json!({"id": subject.subject, "reason": reason});
    }
    let actual = actual.expect("complete actual");
    let (status, status_raw) = ActivityStatus::from_actual(actual.get("status").and_then(Value::as_str));
    let mut row = json!({
        "id": subject.subject,
        "status": status,
        "host": client_host_id(origin.expect("complete host")),
        "workspace": workspace.expect("complete workspace"),
        "incarnation_id": actual.get("incarnation_id"),
        "actual_claim": subject.actual_claim,
        "actual_origin": origin,
    });
    if let Some(raw) = status_raw {
        row["status_raw"] = json!(raw);
    }
    row
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
        let (snapshot, items) = tokio::task::spawn_blocking(move || {
            reader.store.read_snapshot(|cut| {
                // This is the roster's current-layer membership and chunked desired/actual fold,
                // not the presented card (whose workspace may come from the declaration).
                let status = reader.store.agent_card_status_for_generic_reads_at(None, cut, false)?;
                let items = status.subjects.into_iter()
                    .filter(|subject| host.as_ref().is_none_or(|host| subject.actual_origin.as_deref()
                        .is_some_and(|origin| client_host_id(origin) == *host)))
                    .map(activity_row).collect::<Vec<_>>();
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
