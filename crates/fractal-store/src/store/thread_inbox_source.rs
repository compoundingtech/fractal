//! Exact current Inbox source admission. Callers already hold the writer connection.
use super::*;

#[derive(Clone, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct InboxOrigin {
    pub source_id: String,
    pub episode: String,
}

pub(super) struct ResolvedSource {
    pub item: AttentionItemView,
    pub agent: Option<String>,
    pub evidence: Vec<String>,
}

/// Reuse the same source reducers as the Inbox without taking a pooled reader,
/// trusting a client-rendered row, or refolding another person's Inbox.
pub(super) fn resolve(
    connection: &Connection,
    person: &str,
    origin: &InboxOrigin,
    at: u128,
) -> Result<ResolvedSource, FractalError> {
    let unavailable = || FractalError::new("thread-unavailable", "the Inbox source is unavailable");
    if origin.source_id.is_empty() || origin.source_id.len() > 512
        || origin.episode.is_empty() || origin.episode.len() > 512
    {
        return Err(unavailable());
    }
    let person = identities::resolve_reference_conn(connection, person)?;
    let source = origin.source_id.as_str();
    let mut item = if source.starts_with("custom/") {
        custom_item(connection, source).map_err(internal)?
    } else if source.starts_with("step-run/") {
        attention_snapshot::person_attention_item(connection, source, at).map_err(internal)?
    } else {
        None
    };
    // A step/run/loop may instead be the owner of a current human gate.
    if item.as_ref().is_none_or(|item| item.episode != origin.episode)
        && let Some(request) = claim_by_id_tx(connection, &origin.episode).map_err(internal)?
        && request.kind == "gate.requested"
        && let Some(review) = current_human_review(connection, request).map_err(internal)?
    {
        item = Some(attention::attention_item_from_review_at(connection, review).map_err(internal)?);
    }
    let item = item.filter(|item| item.subject == source && item.episode == origin.episode
        && item.requested_at_unix_ms <= at
        && identities::resolve_reference_conn(connection, &item.person)
            .is_ok_and(|owner| owner == person))
        .ok_or_else(unavailable)?;
    if let Some(run) = item.mission_run.as_deref()
        && !person_work::run_live(connection, run, None, item.kind == "fault").map_err(internal)?
    {
        return Err(unavailable());
    }
    let agent = item.conversation.as_deref()
        .map(|agent| identities::resolve_reference_conn(connection, agent))
        .transpose()?;
    let evidence = claim_by_id_tx(connection, &origin.episode).map_err(internal)?
        .filter(|claim| claim.subject == source || claim.body["fields"]["owner"] == source)
        .map(|claim| vec![claim.id]).unwrap_or_default();
    Ok(ResolvedSource { item, agent, evidence })
}

fn custom_item(connection: &Connection, source: &str) -> anyhow::Result<Option<AttentionItemView>> {
    if fractal_schema::notice::is_reserved_subject(source) {
        return Ok(None);
    }
    let row: Option<(String, String)> = connection.query_row(
        "SELECT body,requested_at FROM custom_sources WHERE subject=?1 AND active=1",
        [source], |row| Ok((row.get(0)?, row.get(1)?)),
    ).optional()?;
    row.map(|(body, at)| {
        custom::attention_item(&serde_json::from_str(&body)?, at.parse()?)
    }).transpose()
}


impl ResolvedSource {
    pub(super) fn require_agent(
        &self,
        connection: &Connection,
        selected: &str,
    ) -> Result<String, FractalError> {
        let canonical = self.agent.as_deref().ok_or_else(|| FractalError::new(
            "thread-responsibility-unassigned",
            "this Inbox source has no configured responsible agent",
        ))?;
        let selected = identities::resolve_reference_conn(connection, selected)?;
        if canonical != selected || !person_work::declaration_live(connection, &selected).map_err(internal)? {
            return Err(unavailable());
        }
        Ok(selected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_exact_custom_source_keeps_its_owner_episode_and_creator() {
        let store = Store::open_memory("thread-source").unwrap();
        let person = "person/operator";
        let source = "custom/garden/review/v1/origin";
        let manifest = serde_json::from_str(include_str!("../../../../examples/fractal/custom-review.json")).unwrap();
        store.register_custom_kind(&custom::RegistrationRequest {
            manifest, actor: "agent/garden/seed".into(),
        }).unwrap();
        let (opening, _) = store.append_claim_outcome(&ClaimInput {
            subject: source.into(), kind: "custom.garden.review.v1.requested".into(),
            actor: Some("agent/garden/seed".into()),
            fields: serde_json::from_value(json!({
                "title":"Review", "detail":"Choose the next step", "recipient":person,
            })).unwrap(),
            evidence: vec![], expected_subject: None, idempotency_key: Some("source-open".into()),
        }).unwrap();
        let origin = InboxOrigin { source_id: source.into(), episode: opening.id.clone() };
        // Actual source projection, examined on the same connection a writer admits against.
        store.connection.batched(|tx| -> anyhow::Result<()> {
            let selected = resolve(tx, person, &origin, now_ms()).unwrap();
            assert_eq!(selected.agent.as_deref(), Some("agent/garden/seed"));
            assert_eq!(selected.evidence, [opening.id.clone()]);
            assert!(resolve(tx, "person/other", &origin, now_ms()).is_err());
            let stale = InboxOrigin { episode: "claim/stale".into(), ..origin.clone() };
            assert!(resolve(tx, person, &stale, now_ms()).is_err());
            Ok(())
        }).unwrap().unwrap();
        store.append_claim(&ClaimInput {
            subject: source.into(), kind: "custom.garden.review.v1.answered".into(),
            actor: Some(person.into()),
            fields: serde_json::from_value(json!({
                "request": opening.id, "selection":"keep", "text":"Proceed.",
            })).unwrap(),
            evidence: vec![], expected_subject: None, idempotency_key: Some("source-close".into()),
        }).unwrap();
        store.connection.batched(|tx| -> anyhow::Result<()> {
            assert!(resolve(tx, person, &origin, now_ms()).is_err());
            Ok(())
        }).unwrap().unwrap();
    }
}
