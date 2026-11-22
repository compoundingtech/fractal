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

#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
enum ActivityLayer {
    Current,
    Historical,
}

fn terminal_status(actual: &Value) -> bool {
    matches!(actual.get("status").and_then(Value::as_str), Some("stopped" | "retired" | "failed"))
}

/// Only selected runtime end claims prove that a terminal folded status has no live
/// incarnation. Read those claims by primary key, in the roster's bounded chunks;
/// do not walk each historical agent's runtime history.
fn selected_runtime_ends(
    store: &Store,
    subjects: &[crate::model::SubjectStatus],
    cut: u64,
) -> anyhow::Result<BTreeMap<String, Option<String>>> {
    let claims = subjects.iter()
        .filter(|subject| subject.projection.layer != "current"
            && subject.actual.as_ref().is_some_and(terminal_status))
        .filter_map(|subject| subject.actual_claim.as_deref())
        .collect::<Vec<_>>();
    if claims.is_empty() {
        return Ok(BTreeMap::new());
    }
    let connection = store.readers.get();
    let mut query = connection.prepare_cached(
        "SELECT id, json_extract(body, '$.fields.incarnation_id') FROM claims
         WHERE id IN (SELECT value FROM json_each(?1)) AND store_index<=?2
           AND kind='runtime.observed'
           AND json_extract(body, '$.fields.status') IN ('stopped','retired','failed')"
    )?;
    let mut ends = BTreeMap::new();
    for chunk in claims.chunks(crate::store::card_fold::CHUNK) {
        smallclaims::read_budget::check()?;
        for row in query.query_map(rusqlite::params![serde_json::to_string(chunk)?, cut], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })? {
            let (claim, incarnation) = row?;
            ends.insert(claim, incarnation);
        }
    }
    Ok(ends)
}

enum ActivityClassification<'a> {
    Complete {
        origin: &'a str,
        workspace: &'a str,
        actual: &'a Value,
        claim: &'a str,
    },
    Incomplete(IncompleteReason),
}

fn classify_activity(subject: &crate::model::SubjectStatus) -> ActivityClassification<'_> {
    let (Some(actual), Some(claim)) = (subject.actual.as_ref(), subject.actual_claim.as_deref()) else {
        return ActivityClassification::Incomplete(IncompleteReason::NoActual);
    };
    if subject.reason.as_deref() == Some(crate::store::INDETERMINATE_ACTUAL_AUTHORITY) {
        return ActivityClassification::Incomplete(IncompleteReason::IndeterminateAuthority);
    }
    let Some(origin) = subject.actual_origin.as_deref().filter(|origin| !origin.is_empty()) else {
        return ActivityClassification::Incomplete(IncompleteReason::MissingHost);
    };
    let Some(workspace) = actual.get("workspace").and_then(Value::as_str)
        .filter(|workspace| Path::new(workspace).is_absolute()) else {
        return ActivityClassification::Incomplete(IncompleteReason::MissingWorkspace);
    };
    ActivityClassification::Complete { origin, workspace, actual, claim }
}

fn activity_row(
    subject: &crate::model::SubjectStatus,
    host: Option<&str>,
    runtime_ends: &BTreeMap<String, Option<String>>,
) -> Option<Value> {
    let classification = classify_activity(subject);
    let layer = if subject.projection.layer == "current" {
        ActivityLayer::Current
    } else {
        // Presentation history is not runtime death: owners and generations can end
        // before their child runtime. Omit only resolved, terminal runtime evidence.
        if let ActivityClassification::Complete { actual, claim, .. } = &classification
            && terminal_status(actual)
            && runtime_ends.get(*claim).is_some_and(|ended| ended.as_deref().is_none_or(
                |incarnation| actual.get("incarnation_id").and_then(Value::as_str) == Some(incarnation)
            ))
        {
            return None;
        }
        ActivityLayer::Historical
    };
    // Missing workspace still has an authoritative origin. Every other incomplete
    // classification has an unknown host and must remain visible under any host filter.
    let known_origin = match &classification {
        ActivityClassification::Complete { origin, .. } => Some(*origin),
        ActivityClassification::Incomplete(IncompleteReason::MissingWorkspace) =>
            subject.actual_origin.as_deref(),
        ActivityClassification::Incomplete(_) => None,
    };
    if let (Some(host), Some(origin)) = (host, known_origin)
        && client_host_id(origin) != host
    {
        return None;
    }
    Some(match classification {
        ActivityClassification::Incomplete(reason) =>
            json!({"id": subject.subject, "layer": layer, "reason": reason}),
        ActivityClassification::Complete { origin, workspace, actual, claim } => {
            let (status, status_raw) = ActivityStatus::from_actual(actual.get("status").and_then(Value::as_str));
            let mut row = json!({
                "id": subject.subject,
                "layer": layer,
                "status": status,
                "host": client_host_id(origin),
                "workspace": workspace,
                "incarnation_id": actual.get("incarnation_id"),
                "actual_claim": claim,
                "actual_origin": origin,
            });
            if let Some(raw) = status_raw {
                row["status_raw"] = json!(raw);
            }
            row
        }
    })
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
                // Read both presentation layers with the roster's chunked desired/actual
                // fold. Runtime evidence, not owner lifecycle, decides activity membership.
                let status = reader.store.agent_card_status_for_generic_reads_at(None, cut, true)?;
                let runtime_ends = selected_runtime_ends(&reader.store, &status.subjects, cut)?;
                let items = status.subjects.iter()
                    .filter_map(|subject| activity_row(subject, host.as_deref(), &runtime_ends))
                    .collect::<Vec<_>>();
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
