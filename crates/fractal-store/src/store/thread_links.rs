//! Thread links use reserved records so neither ask nor thread metadata revisions change.

use super::*;
use thread_metadata::{creation_claim, latest_applied_claim};

pub struct AttachAsk<'a> {
    pub thread: &'a str,
    pub ask: &'a str,
    pub episode: &'a str,
    pub key: &'a str,
    pub decision: Option<DecisionHistory<'a>>,
}

pub struct DecisionHistory<'a> {
    pub predecessor: &'a str,
    pub episode: &'a str,
    pub reason: &'a str,
}

pub fn ask_link_subject(ask: &str, episode: &str) -> Result<String, FractalError> {
    canonical_hash(&("thread.ask-link.v1", ask, episode))
        .map(|digest| format!("thread/link/{digest}"))
        .map_err(internal)
}

/// Attach an ask as its bound requesting agent. The actor is established by the daemon.
pub fn attach_ask(
    store: &Store,
    bound: &str,
    request: AttachAsk<'_>,
) -> Result<(ClaimRecord, bool), FractalError> {
    let AttachAsk {
        thread,
        ask,
        episode,
        key,
        decision,
    } = request;
    let unavailable =
        || FractalError::new("thread-unavailable", "the thread or ask is unavailable");
    if !bound.starts_with("agent/") || ask.is_empty() || episode.is_empty() {
        return Err(unavailable());
    }
    let mut fields = BTreeMap::from([
        ("thread_id".into(), json!(thread)),
        ("ask".into(), json!(ask)),
        ("ask_episode".into(), json!(episode)),
    ]);
    if let Some(history) = &decision {
        if history.predecessor == ask
            || history.reason.trim().is_empty()
            || history.reason.len() > 1024
        {
            return Err(FractalError::new(
                "invalid-thread-decision",
                "a decision revision needs another ask and a reason",
            ));
        }
        fields.insert("decision_predecessor".into(), json!(history.predecessor));
        fields.insert(
            "decision_predecessor_episode".into(),
            json!(history.episode),
        );
        fields.insert("decision_reason".into(), json!(history.reason));
    }
    let input = ClaimInput {
        subject: ask_link_subject(ask, episode)?,
        kind: "thread.ask-linked".into(),
        actor: Some(bound.into()),
        fields,
        evidence: vec![episode.into()],
        expected_subject: None,
        idempotency_key: Some(key.into()),
    };
    let creation =
        |connection: &Connection| creation_claim(connection, thread)?.ok_or_else(unavailable);
    let ask_claim = |connection: &Connection| {
        person_work::request(connection, ask)
            .map_err(internal)?
            .filter(|claim| claim.id == episode)
            .ok_or_else(unavailable)
    };
    let authority = |connection: &Connection| {
        store.thread_write_rollout(connection)?;
        let created = creation(connection)?;
        let original = ask_claim(connection)?;
        let agent = created.body["fields"]["agent"]
            .as_str()
            .ok_or_else(unavailable)?;
        let owner = created.body["fields"]["person"]
            .as_str()
            .ok_or_else(unavailable)?;
        let assigned = person_work::step(connection, ask)
            .map_err(internal)?
            .and_then(|step| step.assigned_to)
            .ok_or_else(unavailable)?;
        let identity = |actor| identities::resolve_reference_conn(connection, actor);
        let bound = identity(bound)?;
        if identity(agent)? != bound
            || identity(original.actor.as_deref().ok_or_else(unavailable)?)? != bound
            || identity(&assigned)? != identity(owner)?
        {
            return Err(unavailable());
        }
        Ok(())
    };
    let admission = |connection: &Connection| {
        // The operation digest predates this guard. Preserve existing retry receipts
        // and check link uniqueness inside the serialized new-write path.
        let linked: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM claims WHERE subject=?1 AND kind='thread.ask-linked')",
            [&input.subject],
            |row| row.get(0),
        ).map_err(internal)?;
        if linked {
            return Err(FractalError::new(
                "thread-ask-already-linked",
                "this ask episode already has a thread link",
            ));
        }
        if !person_work::current(connection, &ask_claim(connection)?, now_ms()).map_err(internal)? {
            return Err(unavailable());
        }
        if let Some(history) = &decision {
            let created = creation(connection)?;
            let is_origin = thread_metadata::origin_pair(&created)
                == Some((history.predecessor, history.episode));
            let predecessor_link = ask_link_subject(history.predecessor, history.episode)?;
            let is_linked =
                latest_applied_claim(connection, &predecessor_link, "thread.ask-linked")?
                    .is_some_and(|claim| {
                        claim.body["fields"]["thread_id"].as_str() == Some(thread)
                            && claim.body["fields"]["ask_episode"].as_str() == Some(history.episode)
                    });
            if !is_origin && !is_linked {
                return Err(unavailable());
            }
        }
        Ok(())
    };
    store.graph.ensure_principal_key(bound)?;
    append_claim_with_commit_context(
        &store.graph,
        &input,
        None,
        None,
        None,
        None,
        ClaimCommitContext {
            authority: Some(&authority),
            admission: Some(&admission),
            ..Default::default()
        },
    )
}
