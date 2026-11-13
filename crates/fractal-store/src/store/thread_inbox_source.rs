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
    pub evidence: Vec<String>,
}

/// Reuse the same source reducers as the Inbox without taking a pooled reader,
/// trusting a client-rendered row, or refolding another person's Inbox.
pub(super) fn resolve(
    store: &Store,
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
    } else if source.starts_with("agent/") {
        let mut prompts = store.harness_prompt_items_for_seats_at(
            connection, vec![source.into()], Some(&person), at,
        ).map_err(internal)?;
        let prompt = prompts.iter().position(|item| item.episode == origin.episode)
            .map(|index| prompts.remove(index));
        if prompt.is_some() {
            prompt
        } else if let Some(desired) = current_desired_row(connection, source).map_err(internal)? {
            let member = desired.member.as_deref()
                .map(serde_json::from_str::<crate::model::MemberSpec>)
                .transpose().map_err(internal)?;
            let host = member.as_ref().map(|member| member.host.as_str()).unwrap_or("unknown");
            let candidates = store.desired_harness_login_candidates_at(connection, Some(host)).map_err(internal)?;
            store.harness_login_items_at(connection, candidates, Some(&person))
                .map_err(internal)?.into_iter().find(|item| item.subject == source && item.episode == origin.episode)
        } else {
            None
        }
    } else if source.starts_with("planning-session/") {
        attention::planning_attention_item_at(connection, source).map_err(internal)?
    } else if source.starts_with("revision-proposal/") {
        attention::revision_attention_items_at(connection, source, Some(&person))
            .map_err(internal)?.into_iter().next()
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
    if item.as_ref().is_none_or(|item| item.episode != origin.episode)
        && let Some(opening) = claim_by_id_tx(connection, &origin.episode).map_err(internal)?
        && let Some(failure) = attention_snapshot::operational_failure_at(connection, &opening).map_err(internal)?
    {
        item = store.broken_gate_item(connection, failure, Some(&person), at).map_err(internal)?;
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
    let evidence = source_evidence(store, connection, origin, &item).map_err(internal)?;
    Ok(ResolvedSource { item, evidence })
}

fn source_evidence(
    store: &Store,
    connection: &Connection,
    origin: &InboxOrigin,
    item: &AttentionItemView,
) -> anyhow::Result<Vec<String>> {
    let mut evidence = BTreeSet::new();
    if let Some(claim) = claim_by_id_tx(connection, &origin.episode)?
        && (claim.subject == origin.source_id || claim.body["fields"]["owner"] == origin.source_id
            || claim.body["fields"]["targets"].as_array().is_some_and(|targets| targets.iter().any(|target| target == &origin.source_id)))
    {
        evidence.insert(claim.id);
    }
    if item.kind.starts_with("custom.") {
        // Source payloads can contain unrelated variable data. Only their provenance object
        // is needed to pin the exact admitted claims; retain the caller's transaction cut.
        let text: String = connection.query_row(
            "SELECT CASE WHEN json_type(body,'$.provenance')='object'
               THEN json_extract(body,'$.provenance') ELSE '{}' END
             FROM custom_sources WHERE subject=?1", [&origin.source_id],
            |row| row.get(0),
        )?;
        let provenance: Value = serde_json::from_str(&text)?;
        for provenance in provenance.as_object().into_iter().flat_map(|p| p.values()) {
            if let Some(id) = provenance["claim_id"].as_str() {
                evidence.insert(id.into());
            }
        }
    } else if item.kind == "harness-login" {
        if let Some(harness) = current_harness_fold_at(connection, &origin.source_id, None, true, true)? {
            evidence.insert(harness.claim);
            if let Some(claim) = store.harness_login_episode_key_at(
                connection, &origin.source_id, &harness.incarnation_id,
            )?.0 {
                evidence.insert(claim.id);
            }
        }
    } else if matches!(item.kind.as_str(), "launch-approval" | "revision-approval") {
        let kind = if item.kind == "launch-approval" { "planning-session.previewed" } else { "revision-proposal.created" };
        let id: Option<String> = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h9ee9e75a9561 owner=fractal_store::store::thread_inbox_source TODO=trace-statement-and-value-representation */ &canonical_sql(
            "SELECT id FROM claims WHERE subject=?1 AND kind=?2
             ORDER BY CANONICAL_DESC(claims) LIMIT 1",
        ))?.query_row(/* sqlite-large-value: allow-for-now lv-h6c3a55133316 owner=fractal_store::store::thread_inbox_source TODO=trace-statement-and-value-representation */ params![origin.source_id, kind], |row| row.get(0)).optional()?;
        evidence.extend(id);
    }
    Ok(evidence.into_iter().collect())
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
        store: &Store,
        connection: &Connection,
        selected: &str,
    ) -> Result<String, FractalError> {
        let canonical = store.inbox_responsible_agent_at(connection, &self.item).map_err(internal)?
            .ok_or_else(|| FractalError::new(
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
    fn login_source_decodes_its_member_and_reads_only_the_same_host() {
        let store = Store::open_memory("node").unwrap();
        let workspace = tempfile::tempdir().unwrap();
        for host in ["node", "other"] {
            let source = format!("version 2\nagent \"login\" {{ workspace {:?}; harness \"claude\" {{}} }}\n", workspace.path().display().to_string());
            let intent = crate::graph::parse_intent(&source, host).unwrap();
            let preview = store.mission(&intent, crate::model::IntentInput {
                kdl: source, source_name: None,
            }).unwrap();
            store.apply_as(&intent, &preview.subject_tokens, &format!("login-{host}"), Some("person/avery")).unwrap();
            for (kind, fields) in [
                ("runtime.observed", json!({"status":"running", "incarnation_id":"current"})),
                ("harness.observed", json!({"state":"idle", "incarnation_id":"current", "provider_auth":false})),
            ] {
                store.append_claim(&ClaimInput {
                    subject: format!("agent/{host}.login"), kind: kind.into(),
                    actor: Some(format!("agent/{host}.login")),
                    fields: serde_json::from_value(fields).unwrap(),
                    evidence: vec![], expected_subject: None, idempotency_key: None,
                }).unwrap();
            }
        }
        let item = store.attention_items(Some("person/avery")).unwrap().into_iter()
            .find(|item| item.subject == "agent/node.login" && item.kind == "harness-login").unwrap();
        let origin = InboxOrigin { source_id: item.subject, episode: item.episode };
        store.connection.batched(|tx| -> anyhow::Result<()> {
            // Dependency recording is thread-local: capture on the writer that executes
            // source resolution, rather than the caller waiting for its batched result.
            let (selected, reads) = smallclaims::touched::record(|| {
                resolve(&store, tx, "person/avery", &origin, now_ms()).unwrap()
            });
            assert_eq!(selected.item.targets, ["agent/node.login"]);
            assert!(reads.contains("actor:agent/node.login"));
            assert!(!reads.contains("actor:agent/other.login"), "exact source admission must not fold another host's login");
            Ok(())
        }).unwrap().unwrap();
    }

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
            let selected = resolve(&store, tx, person, &origin, now_ms()).unwrap();
            assert_eq!(selected.item.conversation.as_deref(), Some("agent/garden/seed"));
            assert_eq!(selected.evidence, [opening.id.clone()]);
            assert!(resolve(&store, tx, "person/other", &origin, now_ms()).is_err());
            let stale = InboxOrigin { episode: "claim/stale".into(), ..origin.clone() };
            assert!(resolve(&store, tx, person, &stale, now_ms()).is_err());
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
            assert!(resolve(&store, tx, person, &origin, now_ms()).is_err());
            Ok(())
        }).unwrap().unwrap();
    }
}
