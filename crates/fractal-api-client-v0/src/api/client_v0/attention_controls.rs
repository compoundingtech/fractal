use super::*;
use crate::store::attention_controls::{AttentionControlChange, AttentionControlRequest, AttentionControlSource, current};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ReadQuery {
    source_id: String,
    attention_kind: String,
    episode: String,
    source_revision: String,
}

pub(super) async fn read(
    State(state): State<AppState>,
    Extension(session): Extension<ClientSession>,
    AxumPath(id): AxumPath<String>,
    Query(query): Query<ReadQuery>,
) -> Result<Json<Value>, ApiError> {
    require_scope(&session, "read.projections")?;
    let person = session.authority_actor.as_str();
    if !person.starts_with("person/") {
        return Err(forbidden("attention presentation controls require an authenticated person"));
    }
    let expected = crate::client_values::client_attention_id(&query.source_id, person, &query.episode)
        .map_err(ApiError::internal)?;
    if expected != id {
        return Err(validation("Attention locator does not match the native item identity"));
    }
    let source = AttentionControlSource {
        source_id: &query.source_id, kind: &query.attention_kind, episode: &query.episode,
    };
    let controls = state.store.read_attention_control(
        &source, person, &id, &query.source_revision, client_now_ms(),
    ).map_err(|error| if error.code == "stale-fence" {
        ApiError::not_found("attention source is not available for this person")
    } else { ApiError::bad(error) })?;
    let until = controls.snoozed_until_unix_ms.map(|until| {
        chrono::DateTime::from_timestamp_millis(until)
            .map(|until| until.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
            .ok_or_else(|| ApiError::internal("stored attention snooze exceeds timestamp range"))
    }).transpose()?;
    Ok(Json(json!({
        "id": id, "source_revision": query.source_revision, "revision": controls.revision,
        "dismissed": controls.dismissed, "snoozed_until": until,
    })))
}

pub(super) fn write(
    state: &AppState, session: &ClientSession, request: &ActionRequest,
    receipt_subject: &str, request_digest: &str, actor_key: &str,
) -> Result<Value, ApiError> {
    let person = session.authority_actor.as_str();
    if !person.starts_with("person/") {
        return Err(forbidden("attention presentation controls require an authenticated person"));
    }
    let p = &request.parameters;
    if p.as_object().is_none_or(|parameters| parameters.keys().any(|key| {
        !matches!(key.as_str(), "target_id" | "source_id" | "attention_kind" | "episode" | "source_revision" | "expected_control_revision")
            && !(key == "until" && request.action_type == "attention.snooze")
    })) {
        return Err(validation("unknown attention control parameter"));
    }
    let target = parameter_string(p, "target_id")?;
    let source_id = parameter_string(p, "source_id")?;
    let kind = parameter_string(p, "attention_kind")?;
    let episode = parameter_string(p, "episode")?;
    let revision = parameter_string(p, "source_revision")?;
    let expected = match p.get("expected_control_revision") {
        Some(Value::Null) => None,
        Some(Value::String(revision)) => Some(revision.as_str()),
        _ => return Err(validation("expected_control_revision must be a string or null")),
    };
    let change = match request.action_type.as_str() {
        "attention.dismiss" => AttentionControlChange::Dismiss,
        "attention.restore" => AttentionControlChange::Restore,
        "attention.snooze" => AttentionControlChange::Snooze(match p.get("until") {
            Some(Value::Null) => None,
            Some(Value::String(until)) => {
                let until = chrono::DateTime::parse_from_rfc3339(until)
                    .map_err(|_| validation("until must be an RFC3339 UTC timestamp or null"))?;
                if until.offset().local_minus_utc() != 0 {
                    return Err(validation("until must use UTC"));
                }
                Some(until.timestamp_millis())
            }
            _ => return Err(validation("until must be an RFC3339 UTC timestamp or null")),
        }),
        _ => return Err(validation("unknown attention presentation action")),
    };
    let now = client_now_ms();
    let source = AttentionControlSource { source_id: &source_id, kind: &kind, episode: &episode };
    let validate = |connection: &rusqlite::Connection| state.store.validate_attention_control_source(
        connection, &source, person, &target, &revision, now,
    );
    let result = std::sync::Mutex::new(None);
    let observe = |transaction: &rusqlite::Transaction<'_>, _: &ClaimRecord, inserted: bool| {
        if !inserted {
            return Err(St3Error::new("missing-control-receipt", "a previously accepted control has no client receipt"));
        }
        let controls = current(transaction, person, &target, &revision)
            .map_err(|error| St3Error::new("internal", error.to_string()))?;
        let value = json!({
            "kind": "action-result", "action_id": request.id,
            "operation_id": format!("operation/client-{}", &request_digest[..24]),
            "status": "completed", "affected_ids": [&target], "control_revision": controls.revision,
        });
        crate::store::insert_local_observation_tx(transaction, state.store.origin(), &ClaimInput {
            subject: receipt_subject.into(), kind: "client.action-result".into(), actor: None,
            fields: BTreeMap::from([
                ("authority_actor".into(), json!(session.authority_actor)),
                ("request_digest".into(), json!(request_digest)), ("result".into(), value.clone()),
            ]),
            evidence: vec![], expected_subject: Some(None), idempotency_key: None,
        }, now)?;
        *result.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(value);
        Ok(())
    };
    state.store.write_attention_control(&AttentionControlRequest {
        person, attention_id: &target, source_revision: &revision, expected_revision: expected,
        change, now_unix_ms: i64::try_from(now).map_err(|_| validation("server clock exceeds supported range"))?,
        key: &format!("client-attention-control:{actor_key}"), validate_source: &validate, commit_receipt: Some(&observe),
    }).map_err(ApiError::bad)?;
    let mut result = result.into_inner().unwrap_or_else(std::sync::PoisonError::into_inner)
        .ok_or_else(|| ApiError::internal("attention control did not produce a receipt"))?;
    result["snapshot_id"] = json!(new_client_snapshot(state).id);
    Ok(result)
}
