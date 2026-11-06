//! Accepted claim attribution for an original native card, independent of bounded lists.
use super::*;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Locator {
    pub source_id: String,
    pub attention_kind: String,
    pub episode: String,
}

pub async fn get(
    State(state): State<AppState>, Extension(session): Extension<client_v0::ClientSession>,
    AxumPath(id): AxumPath<String>, Query(locator): Query<Locator>,
) -> Result<Json<Value>, ApiError> {
    client_v0::require_scope(&session, "read.projections")?;
    if !session.authority_actor.starts_with("person/") {
        return Err(ApiError::bad(St3Error::new("forbidden", "Attention resolutions require an authenticated person")));
    }
    let proof = blocking_store(move || Ok(state.store.attention_source_resolution(
        &crate::store::attention_controls::AttentionControlSource {
            source_id: &locator.source_id, kind: &locator.attention_kind, episode: &locator.episode,
        }, &session.authority_actor, &id,
    ))).await?.map_err(ApiError::bad)?
        .ok_or_else(|| ApiError::not_found("No accepted resolution is retained for this attention locator"))?;
    Ok(Json(json!({"claim_id":proof.claim_id,"resolved_by":proof.resolved_by,
        "person_id":proof.person,"resolved_at":client_timestamp(proof.resolved_at_unix_ms)})))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn original_native_locator_requires_exact_identity_recipient_and_accepted_receipt() {
        let root = tempfile::tempdir().unwrap();
        let state = tests_shared::state(root.path());
        let source = "agent/seat";
        let mut opening = tests_shared::probe_claim("resolution-opening", "working");
        opening.subject = source.into();
        opening.actor = Some(source.into());
        let observation = state.store.claim(opening).unwrap();
        let episode = observation.id.as_str();
        let person = "person/receiver";
        let id = client_attention_id(source, person, episode).unwrap();
        let lookup = |person: &str, id: &str| get(State(state.clone()),
            Extension(client_v0::ClientSession::local(Some(person)).unwrap()), AxumPath(id.into()),
            Query(Locator { source_id: source.into(), attention_kind: "harness-prompt".into(), episode: episode.into() }));
        assert_eq!(lookup(person, &id).await.unwrap_err().code, "not-found");
        state.store.claim(ClaimInput { subject: source.into(), kind: "harness.diagnostic".into(),
            actor: Some(person.into()), fields: serde_json::from_value(json!({"code":"native-prompt-answered","status":"allow","driver":"claude"})).unwrap(),
            evidence: vec![episode.into()], expected_subject: None,
            idempotency_key: Some(format!("native-prompt-answer:{episode}")),
        }).unwrap();
        let Json(proof) = lookup(person, &id).await.unwrap();
        assert_eq!(proof["resolved_by"], person);
        assert_eq!(proof["person_id"], person);
        assert_eq!(lookup(person, "attention/mismatch").await.unwrap_err().code, "validation-failed");
        assert_eq!(lookup("person/other", &id).await.unwrap_err().code, "validation-failed");
        let foreign_id = client_attention_id(source, "person/other", episode).unwrap();
        assert_eq!(lookup("person/other", &foreign_id).await.unwrap_err().code, "not-found");
        assert_eq!(lookup("agent/reader", &id).await.unwrap_err().code, "forbidden");
    }
}
