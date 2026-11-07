//! Durable thread metadata. Client adapters authenticate the actor before calling this layer.

use super::*;

#[path = "thread_message_window.rs"]
mod message_window;
pub use message_window::{ThreadMessage, TimelineItem, TimelinePage, mixed_timeline};

pub enum Edit<'a> {
    Create {
        agent: &'a str,
        title: &'a str,
    },
    Rename {
        title: &'a str,
        title_revision: &'a str,
        revision: &'a str,
    },
    Close {
        revision: &'a str,
    },
    Reopen {
        revision: &'a str,
    },
}

/// Server-established authority, never deserialized from an action body.
pub enum PersonAuthority<'a> {
    TrustedLocal,
    BoundAgent {
        actor: &'a str,
    },
    LocalWeb {
        expires_at: u128,
    },
    Paired {
        grant: &'a str,
        completion: &'a str,
        actor: &'a str,
    },
}

impl PersonAuthority<'_> {
    pub(super) fn check(&self, connection: &Connection, person: &str) -> Result<(), FractalError> {
        let forbidden = || {
            FractalError::new(
                "thread-authority-expired",
                "the thread action credential is unavailable",
            )
        };
        match self {
            Self::TrustedLocal => Ok(()),
            Self::BoundAgent { .. } => Err(forbidden()),
            Self::LocalWeb { expires_at } if *expires_at > now_ms() => Ok(()),
            Self::LocalWeb { .. } => Err(forbidden()),
            Self::Paired {
                grant,
                completion,
                actor,
            } => {
                let find = |kind| {
                    connection
                        .query_row(
                            &latest_claim_of_kind_query(),
                            params![grant, kind],
                            claim_from_row,
                        )
                        .optional()
                        .map_err(internal)
                };
                let pair = find("custom.client.pairing-completed")?.ok_or_else(forbidden)?;
                let fields = &pair.body["fields"];
                if pair.id != *completion
                    || fields["person_id"].as_str() != Some(person)
                    || fields["session_actor"].as_str() != Some(actor)
                    || !fields["expires_at_unix_ms"]
                        .as_u64()
                        .is_some_and(|expiry| u128::from(expiry) > now_ms())
                    || !fields["scopes"].as_array().is_some_and(|scopes| {
                        scopes.iter().any(|scope| scope == "control.messages")
                    })
                    || find("custom.client.pairing-revoked")?.is_some()
                {
                    return Err(forbidden());
                }
                Ok(())
            }
        }
    }
}

/// Admit an edit as the authenticated person and return the original receipt on retry.
/// This only writes metadata. The claim supplies the timeline actor, time and operation ID.
pub fn apply(
    store: &Store,
    person: &str,
    subject: &str,
    key: &str,
    edit: Edit<'_>,
) -> Result<(ClaimRecord, bool), FractalError> {
    apply_inner(store, person, subject, key, edit, PersonAuthority::TrustedLocal, false)
}

pub fn apply_as(
    store: &Store,
    person: &str,
    subject: &str,
    key: &str,
    edit: Edit<'_>,
    principal: PersonAuthority<'_>,
) -> Result<(ClaimRecord, bool), FractalError> {
    apply_inner(store, person, subject, key, edit, principal, true)
}

fn apply_inner(
    store: &Store,
    person: &str,
    subject: &str,
    key: &str,
    edit: Edit<'_>,
    principal: PersonAuthority<'_>,
    require_rollout: bool,
) -> Result<(ClaimRecord, bool), FractalError> {
    if !person.starts_with("person/") {
        return Err(FractalError::new(
            "thread-owner-required",
            "a person must manage a thread",
        ));
    }
    let (kind, fields, revision) = match edit {
        Edit::Create { agent, title } => (
            "thread.created",
            BTreeMap::from([
                ("person".into(), json!(person)),
                ("agent".into(), json!(agent)),
                ("title".into(), json!(title)),
            ]),
            None,
        ),
        Edit::Rename {
            title,
            title_revision,
            revision,
        } => {
            let previous_title =
                rename_snapshot(store, person, subject, key, title_revision, &principal)?;
            (
                "thread.title-set",
                BTreeMap::from([
                    ("title".into(), json!(title)),
                    ("previous_title".into(), json!(previous_title)),
                    ("title_revision".into(), json!(title_revision)),
                ]),
                Some(revision),
            )
        }
        Edit::Close { revision } => (
            "thread.state-set",
            BTreeMap::from([("state".into(), json!("closed"))]),
            Some(revision),
        ),
        Edit::Reopen { revision } => (
            "thread.state-set",
            BTreeMap::from([("state".into(), json!("open"))]),
            Some(revision),
        ),
    };
    let input = ClaimInput {
        subject: subject.into(),
        kind: kind.into(),
        actor: Some(person.into()),
        fields,
        evidence: Vec::new(),
        expected_subject: Some(revision.map(str::to_owned)),
        idempotency_key: Some(key.into()),
    };
    let authority = |connection: &Connection| {
        if require_rollout {
            store.thread_write_rollout(connection)?;
        }
        principal.check(connection, person)?;
        owned_creation(connection, subject, person, revision.is_none()).map(|_| ())
    };

    let admit = |connection: &Connection| {
        if let Some(observed) = input.fields.get("title_revision").and_then(Value::as_str) {
            let title = match latest_applied_claim(connection, subject, "thread.title-set")? {
                Some(title) => title,
                None => latest_applied_claim(connection, subject, "thread.created")?
                    .ok_or_else(stale_title)?,
            };
            if title.id != observed {
                return Err(stale_title());
            }
        }
        if let Some(requested) = input.fields.get("state").and_then(Value::as_str) {
            let current_revision: String = connection
                .query_row(
                    &canonical_sql(smallclaims::store::LATEST_CLAIM_QUERY),
                    [subject],
                    |row| row.get(0),
                )
                .map_err(internal)?;
            if Some(current_revision.as_str()) == revision {
                let lifecycle = latest_applied_claim(connection, subject, "thread.state-set")?;
                let current = lifecycle
                    .as_ref()
                    .and_then(|claim| claim.body["fields"]["state"].as_str())
                    .unwrap_or("open");
                if current == requested {
                    return Err(FractalError::new(
                        "thread-state-unchanged",
                        "the thread is already in the requested state",
                    ));
                }
            }
        }
        Ok(())
    };

    store.graph.ensure_principal_key(person)?;
    append_claim_with_commit_context(
        &store.graph,
        &input,
        None,
        None,
        None,
        None,
        ClaimCommitContext {
            authority: Some(&authority),
            admission: Some(&admit),
            ..Default::default()
        },
    )
}

fn unavailable() -> FractalError {
    FractalError::new("thread-unavailable", "the thread is unavailable")
}

/// Conflicting creation has no established owner. Do not expose its existence before
/// verifying membership in a conversation.
pub(super) fn creation_claim(
    connection: &Connection,
    subject: &str,
) -> Result<Option<ClaimRecord>, FractalError> {
    latest_applied_claim(connection, subject, "thread.created").map_err(|error| {
        if error.code == "idempotency-conflict" {
            unavailable()
        } else {
            error
        }
    })
}

fn owned_creation(
    connection: &Connection,
    subject: &str,
    person: &str,
    creating: bool,
) -> Result<Option<ClaimRecord>, FractalError> {
    let Some(creator) = creation_claim(connection, subject)? else {
        return if creating {
            Ok(None)
        } else {
            Err(unavailable())
        };
    };
    let owner = creator.body["fields"]["person"]
        .as_str()
        .ok_or_else(unavailable)?;
    if identities::resolve_reference_conn(connection, owner)?
        != identities::resolve_reference_conn(connection, person)?
    {
        return Err(unavailable());
    }
    Ok(Some(creator))
}

fn stale_title() -> FractalError {
    FractalError::new(
        "stale-thread-title",
        "the thread title changed; refresh before renaming",
    )
}

fn rename_snapshot(
    store: &Store,
    person: &str,
    subject: &str,
    key: &str,
    observed: &str,
    principal: &PersonAuthority<'_>,
) -> Result<String, FractalError> {
    let connection = store.readers.get();
    principal.check(&connection, person)?;
    owned_creation(&connection, subject, person, false)?;
    if let Some(claim) = claim_by_id_tx(&connection, observed).map_err(internal)?
        && claim.subject == subject
        && matches!(claim.kind.as_str(), "thread.created" | "thread.title-set")
        && let Some(title) = claim.body["fields"]["title"].as_str()
    {
        return Ok(title.to_owned());
    }
    drop(connection);
    // A retry can still reconstruct the original request after the old title body is gone.
    // The normal transaction checks authority and the complete request digest before replay.
    if let Some(receipt) = store.operation_claim(key).map_err(internal)?
        && receipt.subject == subject
        && receipt.kind == "thread.title-set"
        && receipt.body["fields"]["title_revision"].as_str() == Some(observed)
        && let Some(title) = receipt.body["fields"]["previous_title"].as_str()
    {
        return Ok(title.to_owned());
    }
    Err(stale_title())
}

#[derive(Debug, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "kind")]
pub enum Change {
    #[serde(rename = "thread.renamed")]
    Renamed {
        previous_title: String,
        title: String,
    },
    #[serde(rename = "thread.closed")]
    Closed,
    #[serde(rename = "thread.reopened")]
    Reopened,
}

#[derive(Debug, PartialEq, Eq, serde::Serialize)]
pub struct Event {
    #[serde(rename = "id")]
    pub operation_id: String,
    #[serde(rename = "actor_id")]
    pub actor: String,
    pub accepted_at_unix_ms: u128,
    #[serde(flatten)]
    pub change: Change,
}

impl Event {
    /// Interpret an admitted, canonical claim. Duplicate selection belongs to the claim reader.
    pub fn from_claim(claim: ClaimRecord) -> Option<Self> {
        let change = match claim.kind.as_str() {
            "thread.title-set" => Change::Renamed {
                previous_title: claim.body["fields"]["previous_title"].as_str()?.into(),
                title: claim.body["fields"]["title"].as_str()?.into(),
            },
            "thread.state-set" => match claim.body["fields"]["state"].as_str()? {
                "closed" => Change::Closed,
                "open" => Change::Reopened,
                _ => return None,
            },
            _ => return None,
        };
        Some(Self {
            operation_id: claim.operation_id?,
            actor: claim.actor?,
            accepted_at_unix_ms: claim.accepted_at_unix_ms,
            change,
        })
    }
}

/// Return the latest effective lifecycle event, or no event for a thread still in its
/// initial state. The caller must authorize the conversation before using this point read.
pub fn latest_lifecycle_event(store: &Store, subject: &str) -> Result<Option<Event>, FractalError> {
    let connection = store.readers.get();
    let Some(claim) = latest_applied_claim(&connection, subject, "thread.state-set")? else {
        return Ok(None);
    };
    Event::from_claim(claim).map(Some).ok_or_else(|| {
        FractalError::new(
            "invalid-thread-event",
            "the selected lifecycle claim has no event",
        )
    })
}

pub(super) fn latest_applied_claim(
    connection: &Connection,
    subject: &str,
    kind: &str,
) -> Result<Option<ClaimRecord>, FractalError> {
    let selected = connection
        .prepare_cached(&canonical_sql(&format!(
            "SELECT {CLAIM_COLUMNS}, operations.state
             FROM claims INDEXED BY claims_subject_kind_accepted_index
             JOIN operations ON operations.id=json_extract(claims.body, '$._operation.id')
             WHERE claims.subject=?1 AND claims.kind=?2
               AND (operations.state='conflict' OR operations.canonical_claim_id=claims.id)
               AND {UNREPAIRED_CLAIM}
             ORDER BY CANONICAL_DESC(claims) LIMIT 1"
        )))
        .map_err(internal)?
        .query_row(params![subject, kind], |row| {
            Ok((claim_from_row(row)?, row.get::<_, String>(10)?))
        })
        .optional()
        .map_err(internal)?;
    let Some((claim, state)) = selected else {
        return Ok(None);
    };
    if state == "conflict" {
        return Err(FractalError::new(
            "idempotency-conflict",
            "the thread metadata operation has conflicting requests",
        ));
    }
    Ok(Some(claim))
}

#[derive(serde::Serialize)]
pub struct Summary {
    pub id: String,
    pub agent_id: String,
    pub title: String,
    pub title_revision: String,
    pub revision: String,
    pub state: &'static str,
}

/// Read metadata for the authenticated owner. All fields come from one graph snapshot.
pub fn summary(store: &Store, person: &str, subject: &str) -> Result<Summary, FractalError> {
    store
        .graph
        .read_snapshot_unindexed(|| Ok(summary_conn(&store.readers.get(), person, subject)))
        .map_err(internal)?
}

fn summary_conn(
    connection: &Connection,
    person: &str,
    subject: &str,
) -> Result<Summary, FractalError> {
    let creation = owned_creation(connection, subject, person, false)?.ok_or_else(unavailable)?;
    let state = match latest_applied_claim(connection, subject, "thread.state-set")?
        .as_ref()
        .and_then(|claim| claim.body["fields"]["state"].as_str())
    {
        Some("closed") => "closed",
        Some("open") | None => "open",
        _ => {
            return Err(FractalError::new(
                "invalid-thread-event",
                "the thread state is invalid",
            ));
        }
    };
    let title = latest_applied_claim(connection, subject, "thread.title-set")?
        .unwrap_or_else(|| creation.clone());
    let revision = connection
        .query_row(
            &canonical_sql(smallclaims::store::LATEST_CLAIM_QUERY),
            [subject],
            |row| row.get(0),
        )
        .map_err(internal)?;
    Ok(Summary {
        id: subject.into(),
        agent_id: creation.body["fields"]["agent"]
            .as_str()
            .ok_or_else(unavailable)?
            .into(),
        title: title.body["fields"]["title"]
            .as_str()
            .ok_or_else(unavailable)?
            .into(),
        title_revision: title.id,
        revision,
        state,
    })
}

#[derive(serde::Serialize)]
pub struct SummaryPage {
    pub items: Vec<Summary>,
    pub next_cursor: Option<String>,
}

/// Read one exact conversation, with cursor scope independent of agent incarnation.
pub fn list_conversation(
    store: &Store,
    person: &str,
    agent: &str,
    cursor: Option<&str>,
    limit: usize,
) -> Result<SummaryPage, FractalError> {
    store.graph.read_snapshot_unindexed(|| Ok((|| {
        let connection = store.readers.get();
        let person = identities::resolve_reference_conn(&connection, person)?;
        let agent = identities::resolve_reference_conn(&connection, agent)?;
        let scope = canonical_hash(&("thread.conversation.v1", &person, &agent)).map_err(internal)?;
        let after = cursor.map(|cursor| {
            cursor.split_once(':').filter(|(id, subject)| *id == scope && subject.starts_with("thread/") && subject.len() <= 512)
                .map(|(_, subject)| subject)
                .ok_or_else(|| FractalError::new("invalid-thread-cursor", "the cursor belongs to another conversation"))
        }).transpose()?.unwrap_or_default();
        let limit = limit.clamp(1, 100);
        let mut query = connection.prepare_cached(
            "SELECT DISTINCT claims.subject
             FROM claims INDEXED BY claims_thread_conversation_index
             JOIN operations ON operations.canonical_claim_id=claims.id AND operations.state='active'
             WHERE claims.kind='thread.created'
               AND json_extract(claims.body, '$.fields.person')=?1
               AND json_extract(claims.body, '$.fields.agent')=?2
               AND claims.subject>?3
               AND NOT EXISTS (SELECT 1 FROM projection_digest_repaired_claims WHERE id=claims.id)
             ORDER BY claims.subject LIMIT ?4"
        ).map_err(internal)?;
        let mut subjects = query.query_map(params![person, agent, after, limit + 1], |row| row.get::<_, String>(0))
            .map_err(internal)?.collect::<rusqlite::Result<Vec<_>>>().map_err(internal)?;
        let has_more = subjects.len() > limit;
        subjects.truncate(limit);
        let next_cursor = subjects.last().filter(|_| has_more).map(|subject| format!("{scope}:{subject}"));
        let items = subjects.into_iter().map(|subject| {
            let summary = summary_conn(&connection, &person, &subject)?;
            if summary.agent_id != agent {
                return Err(unavailable());
            }
            Ok(summary)
        }).collect::<Result<Vec<_>, FractalError>>()?;
        Ok(SummaryPage { items, next_cursor })
    })())).map_err(internal)?
}

#[derive(serde::Serialize)]
pub struct HistoryPage {
    pub items: Vec<Event>,
    pub next_cursor: Option<String>,
}

/// One owner-scoped page of accepted metadata events, newest first.
pub fn history(
    store: &Store,
    person: &str,
    subject: &str,
    cursor: Option<&str>,
    limit: usize,
) -> Result<HistoryPage, FractalError> {
    store.graph.read_snapshot_unindexed(|| Ok((|| {
        let connection = store.readers.get();
        owned_creation(&connection, subject, person, false)?;
        let through: u64 = connection.query_row(
            "SELECT MAX(store_index) FROM claims WHERE subject=?1",
            [subject], |row| row.get(0),
        ).map_err(internal)?;
        let before = cursor.map(|cursor| {
            let (revision, claim) = cursor.split_once(':')
                .filter(|(_, claim)| !claim.is_empty() && claim.len() <= 512)
                .ok_or_else(|| FractalError::new("invalid-thread-cursor", "the thread cursor is invalid"))?;
            if revision.parse::<u64>().ok() != Some(through) {
                return Err(FractalError::new("stale-thread-cursor", "the thread changed; refresh the timeline"));
            }
            Ok(claim)
        }).transpose()?;
        let limit = limit.clamp(1, 100);
        let at = before.map(|id| -> Result<_, FractalError> {
            let claim = claim_by_id_tx(&connection, id).map_err(internal)?
                .filter(|claim| claim.subject == subject
                    && matches!(claim.kind.as_str(), "thread.title-set" | "thread.state-set"))
                .ok_or_else(|| FractalError::new("invalid-thread-cursor", "the thread cursor is unavailable"))?;
            Ok(claim.accepted_at_unix_ms.to_string())
        }).transpose()?;
        let bound = if at.is_some() {
            format!("AND (length(claims.accepted_at_unix_ms), claims.accepted_at_unix_ms) <= (length(?3), ?3)
                     AND EXISTS (SELECT 1 FROM claims anchor WHERE anchor.id=?2 AND {})",
                    smallclaims::store::canonical::after_sql("anchor", "claims"))
        } else {
            String::new()
        };
        let sql = canonical_sql(&format!(
            "SELECT {CLAIM_COLUMNS}, operations.state
             FROM claims INDEXED BY claims_subject_accepted_index
             JOIN operations ON operations.id=json_extract(claims.body, '$._operation.id')
             WHERE claims.subject=?1 AND claims.kind IN ('thread.title-set', 'thread.state-set')
               AND (operations.state='conflict' OR operations.canonical_claim_id=claims.id)
               AND {UNREPAIRED_CLAIM} {bound}
             ORDER BY CANONICAL_DESC(claims) LIMIT ?4"
        ));
        let mut statement = connection.prepare_cached(&sql).map_err(internal)?;
        let rows = statement.query_map(params![subject, before, at, limit + 1], |row| {
            Ok((claim_from_row(row)?, row.get::<_, String>(10)?))
        }).map_err(internal)?;
        let mut items = Vec::new();
        let mut next_cursor = None;
        let mut has_more = false;
        for row in rows {
            let (claim, state) = row.map_err(internal)?;
            if items.len() == limit {
                has_more = true;
                break;
            }
            if state == "conflict" {
                return Err(FractalError::new("idempotency-conflict", "a thread metadata operation has conflicting requests"));
            }
            next_cursor = Some(format!("{through}:{}", claim.id));
            items.push(Event::from_claim(claim).ok_or_else(|| FractalError::new("invalid-thread-event", "the selected metadata claim has no event"))?);
        }
        if !has_more {
            next_cursor = None;
        }
        Ok(HistoryPage { items, next_cursor })
    })())).map_err(internal)?
}

pub struct StartFromAsk<'a> {
    pub thread: &'a str,
    pub title: &'a str,
    pub person: &'a str,
    pub ask: &'a str,
    pub episode: &'a str,
    pub key: &'a str,
    /// Canonical message prepared by the caller. This store entry point is not a client action.
    pub first_message: &'a ClaimInput,
}

/// Atomically admit a thread and its first ordinary graph message from a current agent ask.
/// This convenience entry point is for trusted local callers.
pub fn start_from_ask(
    store: &Store,
    start: StartFromAsk<'_>,
) -> Result<(ClaimRecord, bool), FractalError> {
    start_from_ask_as(store, start, PersonAuthority::TrustedLocal, None)
}

pub fn start_from_ask_as(
    store: &Store,
    start: StartFromAsk<'_>,
    principal: PersonAuthority<'_>,
    signature: Option<&smallclaims::principal::ClaimSignature>,
) -> Result<(ClaimRecord, bool), FractalError> {
    let StartFromAsk {
        thread,
        title,
        person,
        ask,
        episode,
        key,
        first_message,
    } = start;
    let agent = first_message
        .fields
        .get("to")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let content = first_message
        .fields
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !person.starts_with("person/")
        || !agent.starts_with("agent/")
        || !thread.starts_with("thread/")
        || first_message.kind != "message.sent"
        || first_message.actor.as_deref() != Some(person)
        || first_message.fields.get("from").and_then(Value::as_str) != Some(person)
        || content.trim().is_empty()
        || content.len() > 8 * 1024
        || content.starts_with("doc/")
        || first_message.fields.contains_key("attachments")
        || first_message
            .fields
            .get("in_reply_to")
            .is_some_and(|value| !value.is_null())
        || first_message.fields.contains_key("thread_quote_id")
        || title.trim().is_empty()
        || title.len() > 512
        || key.is_empty()
        || key.len() > 512
    {
        return Err(FractalError::new(
            "invalid-thread-start",
            "thread start requires a plain inline first message to the asking agent",
        ));
    }
    let mut first_message = first_message.clone();
    for (name, value) in [
        ("thread_id", thread),
        ("thread_origin_ask", ask),
        ("thread_origin_episode", episode),
    ] {
        if first_message
            .fields
            .get(name)
            .is_some_and(|current| current.as_str() != Some(value))
        {
            return Err(FractalError::new(
                "invalid-thread-start",
                "the first message context differs from the thread origin",
            ));
        }
        first_message.fields.insert(name.into(), json!(value));
    }
    if signature.is_some()
        && first_message.fields.get("title").and_then(Value::as_str) != Some(title)
    {
        return Err(FractalError::new(
            "invalid-thread-start",
            "the signed first message must name the thread title",
        ));
    }
    let encoded_signature =
        super::thread_delivery::verified_signature(&first_message, &principal, signature)?;
    let operation_id = operation_id_for_key(key);
    let digest = canonical_hash(&(
        "thread.start.v1",
        thread,
        title,
        person,
        ask,
        episode,
        &first_message.subject,
        &first_message.fields,
    ))
    .map_err(internal)?;
    let creation_body = json!({
        "fields": {
            "person": person, "agent": agent, "title": title,
            "origin_ask": ask, "origin_ask_episode": episode, "first_message": first_message.subject,
        },
        "evidence": [episode],
        "_operation": {"id": operation_id, "request_digest": digest},
    });
    let message_body = json!({"fields": first_message.fields, "evidence": []});
    store.graph.ensure_principal_key(person)?;
    store
        .connection
        .batched(|tx| {
            store.thread_write_rollout(tx)?;
            principal.check(tx, person)?;
            super::thread_delivery::check_enrollment(tx, &principal, person, signature)?;
            if let Some((saved_digest, canonical, state)) =
                operation_tx(tx, &operation_id).map_err(internal)?
            {
                owned_creation(tx, thread, person, false)?;
                if state == "conflict" || saved_digest != digest {
                    return Err(FractalError::new(
                        "idempotency-conflict",
                        "the thread start key identifies a different request",
                    ));
                }
                let claim = claim_by_id_tx(tx, &canonical)
                    .map_err(internal)?
                    .ok_or_else(unavailable)?;
                return Ok((claim, false));
            }
            if latest_claim_id_tx(tx, thread).map_err(internal)?.is_some() {
                return Err(FractalError::new(
                    "stale-subject",
                    "the thread already exists",
                ));
            }
            let ask_claim = person_work::request(tx, ask)
                .map_err(internal)?
                .ok_or_else(unavailable)?;
            let assigned = person_work::step(tx, ask)
                .map_err(internal)?
                .and_then(|step| step.assigned_to)
                .ok_or_else(unavailable)?;
            if ask_claim.id != episode
                || ask_claim.actor.as_deref() != Some(agent)
                || identities::resolve_reference_conn(tx, &assigned)?
                    != identities::resolve_reference_conn(tx, person)?
                || !person_work::current(tx, &ask_claim, now_ms()).map_err(internal)?
            {
                return Err(unavailable());
            }
            let mut creation_body = creation_body.clone();
            creation_body["fields"]["person"] =
                json!(identities::resolve_reference_conn(tx, person)?);
            creation_body["fields"]["agent"] =
                json!(identities::resolve_reference_conn(tx, agent)?);
            let created = append_claim_tx(
                tx,
                &store.origin,
                thread,
                "thread.created",
                Some(person),
                &creation_body,
                &[],
                None,
            )
            .map_err(claim_append_error)?;
            if latest_claim_id_tx(tx, &first_message.subject)
                .map_err(internal)?
                .is_some()
            {
                return Err(FractalError::new(
                    "stale-subject",
                    "the first message already exists",
                ));
            }
            if let Some(signature) = encoded_signature.as_deref() {
                tx.execute(
                    "INSERT OR REPLACE INTO expected_claim_signatures(subject,kind,actor,signature) VALUES (?1,'message.sent',?2,?3)",
                    params![first_message.subject, person, signature],
                ).map_err(internal)?;
            }
            // The typed transaction has already authorized this exact ask and sender.
            // Retain the ordinary recipient and provenance checks before the admitted append.
            retirements::admit_recipient(tx, Some(agent))?;
            assistant_write::refuse_unverified(
                "message.sent",
                Some(person),
                Some(person),
                ["sender_class", "assistant_grant_id", "assistant_owner", "assistant_session_actor"]
                    .iter().any(|field| first_message.fields.contains_key(*field)),
            )?;
            let mut message_body = message_body.clone();
            smallclaims::store::idempotency::attach(
                &mut message_body, &format!("thread-start-message:{key}"),
            ).map_err(internal)?;
            let message = append_admitted_claim_tx(
                tx, &store.origin, &first_message.subject, "message.sent", Some(person),
                &message_body, &[], Some(&created.batch_id),
            ).map_err(claim_append_error)?;
            register_operation_tx(tx, &message).map_err(internal)?;
            if encoded_signature.is_some() {
                tx.execute(
                    "DELETE FROM expected_claim_signatures WHERE subject=?1 AND kind='message.sent' AND actor=?2",
                    params![first_message.subject, person],
                ).map_err(internal)?;
            }
            register_operation_tx(tx, &created).map_err(internal)?;
            Ok((created, true))
        })
        .map_err(internal)?
}
