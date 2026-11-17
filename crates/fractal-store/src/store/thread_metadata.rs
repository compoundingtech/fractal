//! Durable thread metadata. Client adapters authenticate the actor before calling this layer.

use super::*;

#[path = "thread_metadata_snapshot.rs"]
mod metadata_snapshot;
use metadata_snapshot::MetadataSnapshot;

pub(super) fn install_indexes(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch(
         /* sqlite-large-value: allow-for-now lv-h6a3f9c9e7395 owner=fractal_store::store::thread_metadata TODO=chunk-or-reference-variable-columns */ "CREATE INDEX IF NOT EXISTS claims_thread_conversation_index
ON claims(json_extract(body, '$.fields.person'), json_extract(body, '$.fields.agent'), subject)
WHERE kind='thread.created';
CREATE INDEX IF NOT EXISTS claims_thread_message_accepted_index
ON claims(json_extract(body, '$.fields.thread_id'), length(accepted_at_unix_ms), accepted_at_unix_ms)
WHERE kind='message.sent' AND json_extract(body, '$.fields.thread_id') IS NOT NULL;
CREATE INDEX IF NOT EXISTS claims_thread_inbox_origin_index
ON claims(json_extract(body, '$.fields.person'),
          coalesce(json_extract(body, '$.fields.origin_source'),json_extract(body, '$.fields.origin_ask')),
          coalesce(json_extract(body, '$.fields.origin_episode'),json_extract(body, '$.fields.origin_ask_episode')), subject)
WHERE kind='thread.created';
CREATE INDEX IF NOT EXISTS claims_thread_mission_index
ON claims(json_extract(body, '$.fields.person'), json_extract(body, '$.fields.mission'), subject)
WHERE kind='thread.created';
CREATE INDEX IF NOT EXISTS claims_operational_episode_index
ON claims(json_extract(body,'$.fields.episode'))
WHERE kind IN ('operational.failure','operational.recovered');
CREATE INDEX IF NOT EXISTS claims_thread_unread_owner_time_index
ON claims(actor, length(accepted_at_unix_ms) DESC, accepted_at_unix_ms DESC, subject)
WHERE kind='thread.unread-marked';
CREATE INDEX IF NOT EXISTS claims_thread_closed_owner_index
ON claims(actor, store_index DESC)
WHERE kind='thread.state-set' AND json_extract(body, '$.fields.state')='closed';
CREATE INDEX IF NOT EXISTS claims_thread_history_link_index
ON claims(json_extract(body, '$.fields.thread_id'), json_extract(body, '$.fields.ask'), json_extract(body, '$.fields.ask_episode'), subject)
WHERE kind='thread.ask-linked';
",
    )
}


#[path = "thread_inbox_source.rs"]
pub(super) mod inbox_source;
pub use inbox_source::InboxOrigin;

#[path = "thread_message_window.rs"]
mod message_window;
pub use message_window::{ThreadMessage, TimelineAnchor, TimelineAnchorState, TimelineItem, TimelinePage, mark_read, mixed_message_content, mixed_timeline};

#[path = "thread_request_history.rs"]
pub mod request_history;

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
    PairedBrowser(BrowserSendAuthority<'a>),
}

/// A device credential with a trusted browser request as its exact completion predecessor.
pub struct BrowserSendAuthority<'a> {
    grant: &'a str,
    completion: &'a str,
    actor: &'a str,
}

pub fn browser_send_authority<'a>(
    store: &Store,
    person: &str,
    grant: &'a str,
    completion: &'a str,
    actor: &'a str,
) -> Result<PersonAuthority<'a>, FractalError> {
    let authority = PersonAuthority::PairedBrowser(BrowserSendAuthority { grant, completion, actor });
    authority.check(store, &store.readers.get(), person)?;
    Ok(authority)
}

impl PersonAuthority<'_> {
    pub(super) fn check(
        &self, store: &Store, connection: &Connection, person: &str,
    ) -> Result<(), FractalError> {
        self.check_scope(store, connection, person, "control.messages")
    }

    fn check_scope(
        &self, store: &Store, connection: &Connection, person: &str, scope: &str,
    ) -> Result<(), FractalError> {
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
            }
            | Self::PairedBrowser(BrowserSendAuthority { grant, completion, actor }) => {
                let now = now_ms();
                let pair = crate::pairing::latest_completion(connection, grant, None, now)
                    .map_err(internal)?.ok_or_else(forbidden)?;
                let fields = &pair.body["fields"];
                if pair.id != *completion
                    || fields["person_id"].as_str() != Some(person)
                    || fields["session_actor"].as_str() != Some(actor)
                    || crate::pairing::completion_deadline(
                        connection, &pair.id, &pair.origin, pair.actor.as_deref(), fields, now,
                    ).map_err(internal)?.is_none()
                    || !fields["scopes"].as_array().is_some_and(|scopes| {
                        scopes.iter().any(|granted| granted == scope)
                    })
                    || crate::pairing::completion_is_revoked(
                        connection, grant, &pair.id, &pair.origin,
                    ).map_err(internal)?
                {
                    return Err(forbidden());
                }
                if matches!(self, Self::PairedBrowser(_))
                    && (pair.origin != store.origin()
                        || !store.graph.has_local_claim_signer()
                        || !has_browser_pairing_provenance(connection, &pair, person)?)
                {
                    return Err(forbidden());
                }
                Ok(())
            }
        }
    }
}

/// Recheck private thread-read authority on the writer's exact transaction before a
/// surrounding operation may recover a receipt or inspect its extra thread fences.
pub fn authorize_read_fences(
    store: &Store, connection: &Connection, person: &str, authority: &PersonAuthority<'_>,
    expected: &BTreeMap<String, String>,
) -> Result<(), FractalError> {
    if !expected.keys().any(|subject| fractal_schema::threads::is_reserved_subject(subject)) {
        return Ok(());
    }
    authority.check_scope(store, connection, person, "read.projections")?;
    for subject in expected.keys().filter(|subject| fractal_schema::threads::is_reserved_subject(subject)) {
        owned_creation(connection, subject, person, false)?.ok_or_else(unavailable)?;
    }
    Ok(())
}

fn has_browser_pairing_provenance(
    connection: &Connection,
    pair: &ClaimRecord,
    person: &str,
) -> Result<bool, FractalError> {
    let fields = &pair.body["fields"];
    if !crate::pairing::operation_is_trusted(
        connection, &pair.id, &pair.origin, pair.actor.as_deref(), fields,
    ).map_err(internal)? {
        return Ok(false);
    }
    let Some([evidence]) = pair.body["evidence"].as_array().map(Vec::as_slice) else {
        return Ok(false);
    };
    let Some(id) = evidence.as_str() else { return Ok(false) };
    let Some(begun) = claim_by_id_tx(connection, id).map_err(internal)? else { return Ok(false) };
    let request = &begun.body["fields"];
    Ok(begun.kind == "custom.client.pairing-begun"
        && begun.subject == pair.subject
        && begun.origin == pair.origin
        && begun.actor.as_deref() == Some(person)
        && crate::pairing::operation_is_trusted(
            connection, &begun.id, &begun.origin, begun.actor.as_deref(), request,
        ).map_err(internal)?
        && request["pairing_id"].as_str().is_some()
        && request["pairing_id"] == fields["pairing_id"]
        && request["browser_request"] == true
        && request["browser_ask"].as_str().is_some())
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
        principal.check(store, connection, person)?;
        owned_creation(connection, subject, person, revision.is_none()).map(|_| ())
    };

    let subject_snapshot = |connection: &Connection| {
        let snapshot = MetadataSnapshot::read(connection, subject)?;
        if let Some(observed) = input.fields.get("title_revision").and_then(Value::as_str) {
            let title = match snapshot.selected("thread.title-set")? {
                Some(title) => title,
                None => snapshot.selected("thread.created")?
                    .ok_or_else(stale_title)?,
            };
            if title.id != observed {
                return Err(stale_title());
            }
        }
        if let Some(requested) = input.fields.get("state").and_then(Value::as_str) {
            let current_revision = snapshot.revision()?;
            if current_revision.as_deref() == revision {
                let lifecycle = snapshot.selected("thread.state-set")?;
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
        snapshot.commit_snapshot()
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
            subject_snapshot: Some(&subject_snapshot),
            ..Default::default()
        },
    )
}

/// The same semantic version used by typed metadata writes, for other atomic graph fences.
pub(super) fn metadata_revision_at(connection: &Connection, subject: &str) -> Result<Option<String>, FractalError> {
    MetadataSnapshot::read(connection, subject)?.revision()
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

pub(super) fn owned_creation(
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
    principal.check(store, &connection, person)?;
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
    if matches!(kind, "thread.created" | "thread.title-set" | "thread.state-set") {
        return MetadataSnapshot::read(connection, subject)?.selected(kind);
    }
    let selected = connection
        .prepare_cached( /* sqlite-large-value: allow-for-now lv-h4162c9a4ad90 owner=fractal_store::store::thread_metadata TODO=trace-statement-and-value-representation */ &canonical_sql(&format!(
            "SELECT {CLAIM_COLUMNS}, operations.state
             FROM claims INDEXED BY claims_subject_kind_accepted_index
             JOIN operations ON operations.id=json_extract(claims.body, '$._operation.id')
             WHERE claims.subject=?1 AND claims.kind=?2
               AND (operations.state='conflict' OR operations.canonical_claim_id=claims.id)
               AND {UNREPAIRED_CLAIM}
             ORDER BY CANONICAL_DESC(claims) LIMIT 1"
        )))
        .map_err(internal)?
        .query_row( /* sqlite-large-value: allow-for-now lv-hb2b23c870f68 owner=fractal_store::store::thread_metadata TODO=trace-statement-and-value-representation */ params![subject, kind], |row| {
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
    pub message_count: u64,
    pub unread: bool,
    pub unread_reminder: Option<fractal_schema::threads::ThreadUnreadReminder>,
}

/// Read metadata for the authenticated owner. All fields come from one graph snapshot.
pub fn summary(store: &Store, person: &str, subject: &str) -> Result<Summary, FractalError> {
    store
        .graph
        .read_snapshot_unindexed(|| Ok(summary_conn(&store.readers.get(), person, subject, None)))
        .map_err(internal)?
}

/// Select an exact conversation before evaluating lifecycle claims. An ambiguous
/// creation cannot establish membership in that conversation.
pub fn summary_for_conversation(
    store: &Store,
    person: &str,
    agent: &str,
    subject: &str,
) -> Result<Summary, FractalError> {
    store.graph.read_snapshot_unindexed(|| {
        Ok(summary_conn(&store.readers.get(), person, subject, Some(agent)))
    }).map_err(internal)?
}

fn summary_conn(
    connection: &Connection,
    person: &str,
    subject: &str,
    selected_agent: Option<&str>,
) -> Result<Summary, FractalError> {
    let creation = owned_creation(connection, subject, person, false)
        .map_err(|error| {
            if selected_agent.is_some() && error.code == "idempotency-conflict" {
                unavailable()
            } else {
                error
            }
        })?
        .ok_or_else(unavailable)?;
    let agent = creation.body["fields"]["agent"].as_str().ok_or_else(unavailable)?;
    if selected_agent.is_some_and(|selected| selected != agent) {
        return Err(unavailable());
    }
    let snapshot = MetadataSnapshot::read(connection, subject)?;
    let state = match snapshot.selected("thread.state-set")?
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
    let title = snapshot.selected("thread.title-set")?.unwrap_or_else(|| creation.clone());
    let revision = snapshot.revision()?.ok_or_else(unavailable)?;
    let (message_count, unread) = message_window::message_summary(
        connection, person, agent, subject, &creation,
    )?;
    Ok(Summary {
        id: subject.into(),
        agent_id: agent.into(),
        title: title.body["fields"]["title"]
            .as_str()
            .ok_or_else(unavailable)?
            .into(),
        title_revision: title.id,
        revision,
        state,
        message_count,
        unread,
        unread_reminder: super::thread_reading::reminder_conn(connection, person, subject)?,
    })
}

#[derive(serde::Serialize)]
pub struct SummaryPage {
    pub items: Vec<Summary>,
    pub next_cursor: Option<String>,
}

pub enum ThreadContext<'a> {
    Agent(&'a str),
    Inbox { source: &'a str, episode: &'a str },
    Mission(&'a str),
    RecentlyClosed,
    MarkedUnread,
}

/// Read one exact conversation, with cursor scope independent of agent incarnation.
pub fn list_conversation(
    store: &Store,
    person: &str,
    agent: &str,
    cursor: Option<&str>,
    limit: usize,
) -> Result<SummaryPage, FractalError> {
    list_context(store, person, ThreadContext::Agent(agent), cursor, limit)
}

pub fn list_context(
    store: &Store,
    person: &str,
    context: ThreadContext<'_>,
    cursor: Option<&str>,
    limit: usize,
) -> Result<SummaryPage, FractalError> {
    store.graph.read_snapshot_unindexed(|| Ok((|| {
        let connection = store.readers.get();
        let person = identities::resolve_reference_conn(&connection, person)?;
        let limit = limit.clamp(1, 100);
        let (index, fields, key, qualifier) = match context {
            ThreadContext::Agent(agent) => (
                "claims_thread_conversation_index",
                "json_extract(claims.body, '$.fields.agent')=?2",
                identities::resolve_reference_conn(&connection, agent)?,
                None,
            ),
            ThreadContext::Inbox { source, episode } => (
                "claims_thread_inbox_origin_index",
                "coalesce(json_extract(claims.body,'$.fields.origin_source'),json_extract(claims.body,'$.fields.origin_ask'))=?2 AND coalesce(json_extract(claims.body,'$.fields.origin_episode'),json_extract(claims.body,'$.fields.origin_ask_episode'))=?5",
                source.into(),
                Some(episode),
            ),
            ThreadContext::Mission(mission) => (
                "claims_thread_mission_index",
                "json_extract(claims.body, '$.fields.mission')=?2",
                identities::resolve_reference_conn(&connection, mission)?,
                None,
            ),
            ThreadContext::RecentlyClosed => return recently_closed(&connection, &person, cursor, limit),
            ThreadContext::MarkedUnread => return marked_unread(&connection, &person, cursor, limit),
        };
        let scope = if index == "claims_thread_conversation_index" {
            canonical_hash(&("thread.conversation.v1", &person, &key))
        } else {
            canonical_hash(&("thread.context.v1", index, &person, &key, qualifier))
        }.map_err(internal)?;
        let after = scoped_cursor(cursor, &scope)?.unwrap_or_default();
        if !after.is_empty() && (!after.starts_with("thread/") || after.len() > 512) {
            return Err(invalid_context_cursor());
        }
        let sql = format!(
            "SELECT DISTINCT claims.subject
             FROM claims INDEXED BY {index}
             JOIN operations ON operations.canonical_claim_id=claims.id AND operations.state='active'
             WHERE claims.kind='thread.created'
               AND json_extract(claims.body, '$.fields.person')=?1
               AND {fields} AND claims.subject>?3
               AND NOT EXISTS (SELECT 1 FROM projection_digest_repaired_claims WHERE id=claims.id)
             ORDER BY claims.subject LIMIT ?4"
        );
        let mut query = connection.prepare_cached( /* sqlite-large-value: allow-for-now lv-h759e1f16c79a owner=fractal_store::store::thread_metadata TODO=trace-statement-and-value-representation */ &sql).map_err(internal)?;
        let args = [person.as_str(), key.as_str(), after];
        let read = |row: &rusqlite::Row<'_>| row.get::<_, String>(0);
        let mut subjects = match qualifier {
            Some(episode) => query.query_map(params![args[0], args[1], args[2], limit + 1, episode], read),
            None => query.query_map(params![args[0], args[1], args[2], limit + 1], read),
        }.map_err(internal)?.collect::<rusqlite::Result<Vec<_>>>().map_err(internal)?;
        if let Some(episode) = qualifier
            && let Some(thread) = linked_context_thread(&connection, &person, &key, episode, after)?
            && let Err(index) = subjects.binary_search(&thread)
        {
            subjects.insert(index, thread);
        }
        let has_more = subjects.len() > limit;
        subjects.truncate(limit);
        let next_cursor = subjects.last().filter(|_| has_more).map(|subject| format!("{scope}:{subject}"));
        let selected_agent = matches!(context, ThreadContext::Agent(_)).then_some(key.as_str());
        let items = subjects.into_iter().map(|subject| {
            summary_conn(&connection, &person, &subject, selected_agent)
        }).collect::<Result<Vec<_>, FractalError>>()?;
        Ok(SummaryPage { items, next_cursor })
    })())).map_err(internal)?
}

/// Graph changes that can alter an Inbox thread badge, independent of ask publication.
pub fn attention_unread_changed(store: &Store, after: u64, through: u64) -> anyhow::Result<bool> {
    let conn = store.readers.get();
    Ok(conn.prepare_cached(
        "SELECT EXISTS (
            SELECT 1 FROM claims changed
            WHERE changed.store_index>?1 AND changed.store_index<=?2 AND (
                changed.kind IN ('thread.created','thread.ask-linked','agent.renamed','record.repaired')
                OR (changed.kind='message.sent'
                    AND json_extract(changed.body,'$.fields.thread_id') IS NOT NULL)
                OR (changed.kind IN ('message.read','message.closed') AND EXISTS (
                    SELECT 1 FROM claims INDEXED BY claims_subject_kind_index
                    WHERE claims.subject=changed.subject AND claims.kind='message.sent'
                      AND json_extract(claims.body,'$.fields.thread_id') IS NOT NULL
                ))
            )
        )"
    )?.query_row( /* sqlite-large-value: allow-for-now lv-h9a5536c68ae8 owner=fractal_store::store::thread_metadata TODO=trace-statement-and-value-representation */ params![after, through], |row| row.get(0))?)
}

/// Normalize legacy ask origins only at the stored-history boundary.
pub(super) fn origin_pair(creation: &ClaimRecord) -> Option<(&str, &str)> {
    let fields = &creation.body["fields"];
    match (fields["origin_source"].as_str(), fields["origin_episode"].as_str()) {
        (Some(source), Some(episode)) => Some((source, episode)),
        _ => Some((fields["origin_ask"].as_str()?, fields["origin_ask_episode"].as_str()?)),
    }
}

/// Unread incoming thread replies on the caller's already selected Inbox rows.
pub fn unread_inbox_positions(
    store: &Store,
    person: &str,
    asks: &[(&str, &str)],
) -> Result<BTreeSet<usize>, FractalError> {
    if asks.is_empty() {
        return Ok(BTreeSet::new());
    }
    let conn = store.readers.get();
    let person = identities::resolve_reference_conn(&conn, person)?;
    let at = now_ms();
    let source_keys = asks.iter().map(|(source, episode)| request_history::SourceEpisode {
        source_id: (*source).into(), episode: (*episode).into(),
    }).collect::<Vec<_>>();
    // One exact-opening/current-policy lookup for every retained row, not N
    // single-source calls. Live-only families keep their existing admission.
    let retained = discussion_source::qualify_batch(&conn, &person, &source_keys, at)?;
    let keys = asks.iter().map(|(ask, episode)| {
        Ok((ask, episode, thread_links::ask_link_subject(ask, episode)?))
    }).collect::<Result<Vec<_>, FractalError>>()?;
    let keys = serde_json::to_string(&keys).map_err(internal)?;
    let mut query = conn.prepare_cached(
         /* sqlite-large-value: allow-for-now lv-h10da3f869f76 owner=fractal_store::store::thread_metadata TODO=trace-statement-and-value-representation */ "WITH inbox AS (
            SELECT CAST(key AS INTEGER) AS position, json_extract(value,'$[0]') AS ask,
                   json_extract(value,'$[1]') AS episode, json_extract(value,'$[2]') AS link
            FROM json_each(?1)
         )
         SELECT inbox.position, claims.subject, false
         FROM inbox CROSS JOIN claims INDEXED BY claims_thread_inbox_origin_index
         WHERE claims.kind='thread.created' AND json_extract(claims.body,'$.fields.person')=?2
           AND coalesce(json_extract(claims.body,'$.fields.origin_source'),json_extract(claims.body,'$.fields.origin_ask'))=inbox.ask
           AND coalesce(json_extract(claims.body,'$.fields.origin_episode'),json_extract(claims.body,'$.fields.origin_ask_episode'))=inbox.episode
         UNION
         SELECT inbox.position, inbox.link, true
         FROM inbox WHERE EXISTS (
            SELECT 1 FROM claims INDEXED BY claims_subject_kind_index
            WHERE claims.subject=inbox.link AND claims.kind='thread.ask-linked'
         )
         ORDER BY 1"
    ).map_err(internal)?;
    let mut rows = query.query(params![keys, person]).map_err(internal)?;
    let mut unread = BTreeSet::new();
    let mut qualification = None;
    while let Some(row) = rows.next().map_err(internal)? {
        let position: usize = row.get(0).map_err(internal)?;
        let (ask, episode) = asks[position];
        if unread.contains(&position) {
            continue;
        }
        let qualified = match qualification {
            Some((previous, qualified)) if previous == position => qualified,
            _ if retained[position].is_some() => true,
            _ => match inbox_source::resolve(store, &conn, &person, &InboxOrigin {
                source_id: ask.into(), episode: episode.into(),
            }, at) {
                Ok(_) => true,
                Err(error) if error.code == "thread-unavailable" => false,
                Err(error) => return Err(error),
            },
        };
        qualification = Some((position, qualified));
        if !qualified {
            continue;
        }
        let linked: bool = row.get(2).map_err(internal)?;
        let subject: String = row.get(1).map_err(internal)?;
        let thread = if linked {
            let link = match latest_applied_claim(&conn, &subject, "thread.ask-linked") {
                Ok(Some(link)) => link,
                Ok(None) => continue,
                Err(error) if error.code == "idempotency-conflict" => continue,
                Err(error) => return Err(error),
            };
            let fields = &link.body["fields"];
            if fields["ask"].as_str() != Some(ask)
                || fields["ask_episode"].as_str() != Some(episode)
            {
                continue;
            }
            let Some(thread) = fields["thread_id"].as_str() else { continue };
            thread.to_owned()
        } else {
            subject
        };
        let creation = match owned_creation(&conn, &thread, &person, false) {
            Ok(Some(creation)) => creation,
            Ok(None) => continue,
            Err(error) if error.code == "thread-unavailable" => continue,
            Err(error) => return Err(error),
        };
        if !linked && origin_pair(&creation) != Some((ask, episode))
        {
            continue;
        }
        // Both direct origins and later ask links use the thread's frozen recipient.
        let Some(agent) = creation.body["fields"]["agent"].as_str() else { continue };
        if message_window::message_unread(&conn, &person, agent, &thread, &creation)? {
            unread.insert(position);
        }
    }
    Ok(unread)
}

fn owns_ask_episode(
    connection: &Connection,
    person: &str,
    ask: &str,
    episode: &str,
) -> Result<bool, FractalError> {
    let key = request_history::SourceEpisode { source_id: ask.into(), episode: episode.into() };
    if discussion_source::qualify(connection, person, &key, now_ms())?.is_none() {
        return Ok(false);
    }
    // Link discovery remains runtime-ask-only; the shared qualifier has already
    // established the canonical opening and both recipient checks.
    connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM claims WHERE id=?1 AND subject=?2 AND kind='work.person-asked')",
        params![episode, ask], |row| row.get(0),
    ).map_err(internal)
}

/// Qualify the exact ask and person before resolving its private sidecar. Otherwise
/// a conflicted association could reveal the existence of an unrelated ask.
fn linked_context_thread(
    connection: &Connection,
    person: &str,
    ask: &str,
    episode: &str,
    after: &str,
) -> Result<Option<String>, FractalError> {
    if !owns_ask_episode(connection, person, ask, episode)? {
        return Ok(None);
    }
    let link = thread_links::ask_link_subject(ask, episode)?;
    let Some(link) = latest_applied_claim(connection, &link, "thread.ask-linked")? else {
        return Ok(None);
    };
    let fields = &link.body["fields"];
    if fields["ask"].as_str() != Some(ask) || fields["ask_episode"].as_str() != Some(episode) {
        return Ok(None);
    }
    let Some(thread) = fields["thread_id"].as_str().filter(|id| *id > after) else {
        return Ok(None);
    };
    match owned_creation(connection, thread, person, false) {
        Ok(Some(_)) => Ok(Some(thread.into())),
        Ok(None) => Ok(None),
        Err(error) if error.code == "thread-unavailable" => Ok(None),
        Err(error) => Err(error),
    }
}

fn invalid_context_cursor() -> FractalError {
    FractalError::new(
        "invalid-thread-cursor",
        "the cursor belongs to another thread context",
    )
}

fn scoped_cursor<'a>(
    cursor: Option<&'a str>,
    scope: &str,
) -> Result<Option<&'a str>, FractalError> {
    cursor
        .map(|cursor| {
            cursor
                .split_once(':')
                .filter(|(id, _)| *id == scope)
                .map(|(_, marker)| marker)
                .ok_or_else(invalid_context_cursor)
        })
        .transpose()
}

fn marked_candidate_sql(has_cursor: bool) -> String {
    let unrepaired = UNREPAIRED_CLAIM.replace("claims.", "earlier.");
    // JSON=JSON keeps the expression index's affinity, unlike JSON=TEXT-column.
    let base=format!("SELECT claims.subject,claims.accepted_at_unix_ms
        FROM claims INDEXED BY claims_thread_unread_owner_time_index
        JOIN operations ON operations.id=json_extract(claims.body,'$._operation.id')
          AND operations.state='active' AND operations.request_digest=json_extract(claims.body,'$._operation.request_digest')
        WHERE claims.actor=?1 AND claims.kind='thread.unread-marked' AND {UNREPAIRED_CLAIM}
          AND claims.id=(SELECT earlier.id FROM claims earlier INDEXED BY claims_operation_index
            WHERE json_extract(earlier.body,'$._operation.id')=json_extract(claims.body,'$._operation.id')
              AND json_extract(earlier.body,'$._operation.id') IS NOT NULL
              AND json_extract(earlier.body,'$._operation.request_digest')=operations.request_digest
              AND {unrepaired}
            ORDER BY length(earlier.accepted_at_unix_ms),earlier.accepted_at_unix_ms,earlier.id LIMIT 1)");
    if !has_cursor {
        return format!(
            "{base} ORDER BY length(claims.accepted_at_unix_ms) DESC,claims.accepted_at_unix_ms DESC,claims.subject LIMIT ?4"
        );
    }
    // Do not ORDER BY a constant expression prefix: SQLite otherwise sorts the
    // entire qualifying branch instead of stopping at the index's bounded LIMIT.
    let ranges = [
        (
            "length(claims.accepted_at_unix_ms)<length(?2)",
            "length(claims.accepted_at_unix_ms) DESC,claims.accepted_at_unix_ms DESC,claims.subject",
        ),
        (
            "length(claims.accepted_at_unix_ms)=length(?2) AND claims.accepted_at_unix_ms<?2",
            "claims.accepted_at_unix_ms DESC,claims.subject",
        ),
        (
            "length(claims.accepted_at_unix_ms)=length(?2) AND claims.accepted_at_unix_ms=?2 AND claims.subject>?3",
            "claims.subject",
        ),
    ];
    let arms = ranges
        .into_iter()
        .map(|(range, order)| {
            format!("SELECT * FROM ({base} AND {range} ORDER BY {order} LIMIT ?4)")
        })
        .collect::<Vec<_>>()
        .join(" UNION ALL ");
    format!(
        "SELECT * FROM ({arms}) ORDER BY length(accepted_at_unix_ms) DESC,accepted_at_unix_ms DESC,subject LIMIT ?4"
    )
}

/// Scan owner reminder-operation headers, never an ordinary/recent thread page.
/// A bounded candidate page may be empty and still carries its continuation.
fn marked_unread(
    connection: &Connection,
    person: &str,
    cursor: Option<&str>,
    limit: usize,
) -> Result<SummaryPage, FractalError> {
    let scope = canonical_hash(&("thread.marked-unread.v1", person)).map_err(internal)?;
    let boundary = scoped_cursor(cursor, &scope)?
        .map(|marker| {
            let (time, thread) = marker.split_once(':').ok_or_else(invalid_context_cursor)?;
            let time = time.parse::<u128>().map_err(|_| invalid_context_cursor())?;
            if !thread.starts_with("thread/") || thread.len() > 512 {
                return Err(invalid_context_cursor());
            }
            Ok((time.to_string(), thread))
        })
        .transpose()?;
    let sql = marked_candidate_sql(boundary.is_some());
    let scan_limit = limit * 4;
    let mut statement=connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h73dc5806dc06 owner=fractal_store::store::thread_metadata TODO=trace-statement-and-value-representation */ &sql).map_err(internal)?;
    let mut rows = statement
        .query(params![
            person,
            boundary.as_ref().map(|(time, _)| time),
            boundary.as_ref().map(|(_, thread)| thread),
            scan_limit + 1
        ])
        .map_err(internal)?;
    let mut items = Vec::new();
    let mut last = None;
    let mut seen = BTreeSet::new();
    for position in 0..=scan_limit {
        let Some(row) = rows.next().map_err(internal)? else {
            return Ok(SummaryPage {
                items,
                next_cursor: None,
            });
        };
        if position == scan_limit || items.len() == limit {
            break;
        }
        let subject: String = row.get(0).map_err(internal)?;
        let time: String = row.get(1).map_err(internal)?;
        last = Some(format!("{scope}:{time}:{subject}"));
        if seen.contains(&subject) {
            continue;
        }
        let projected =
            match super::thread_reading::reminder_with_time_conn(connection, person, &subject) {
                Ok(value) => value,
                Err(error) if error.code == "thread-unavailable" => continue,
                Err(error) => return Err(error),
            };
        if let Some((_, current_time)) = projected
            && current_time.to_string() == time
        {
            seen.insert(subject.clone());
            items.push(summary_conn(connection, person, &subject, None)?);
        }
    }
    Ok(SummaryPage {
        items,
        next_cursor: last,
    })
}

/// Newest close first. Seek only owner close events and recheck the current state, because an
/// earlier close can have been superseded. Keep the candidate scan bounded even during churn.
fn recently_closed(
    connection: &Connection,
    person: &str,
    cursor: Option<&str>,
    limit: usize,
) -> Result<SummaryPage, FractalError> {
    let scope = canonical_hash(&("thread.recently-closed.v1", person)).map_err(internal)?;
    let before = scoped_cursor(cursor, &scope)?
        .map(|marker| {
            marker
                .parse::<i64>()
                .ok()
                .filter(|index| *index > 0)
                .ok_or_else(invalid_context_cursor)
        })
        .transpose()?
        .unwrap_or(i64::MAX);
    let scan_limit = limit * 4;
    let mut statement = connection
        .prepare_cached(
            "SELECT claims.subject, claims.id, claims.store_index
         FROM claims INDEXED BY claims_thread_closed_owner_index
         JOIN operations ON operations.canonical_claim_id=claims.id AND operations.state='active'
         WHERE claims.actor=?1 AND claims.store_index<?2 AND claims.kind='thread.state-set'
           AND json_extract(claims.body, '$.fields.state')='closed'
           AND NOT EXISTS (SELECT 1 FROM projection_digest_repaired_claims WHERE id=claims.id)
         ORDER BY claims.store_index DESC LIMIT ?3",
        )
        .map_err(internal)?;
    let mut rows = statement
        .query(params![person, before, scan_limit + 1])
        .map_err(internal)?;
    let mut items = Vec::new();
    let mut last = None;
    for index in 0..=scan_limit {
        let Some(row) = rows.next().map_err(internal)? else {
            return Ok(SummaryPage {
                items,
                next_cursor: None,
            });
        };
        if index == scan_limit || items.len() == limit {
            break;
        }
        let subject: String = row.get(0).map_err(internal)?;
        let id: String = row.get(1).map_err(internal)?;
        last = Some(row.get::<_, i64>(2).map_err(internal)?);
        match owned_creation(connection, &subject, person, false) {
            Ok(Some(_)) => {}
            Ok(None) => continue,
            Err(error) if error.code == "thread-unavailable" => continue,
            Err(error) => return Err(error),
        }
        let state = latest_applied_claim(connection, &subject, "thread.state-set")?;
        if state.is_some_and(|current| current.id == id) {
            items.push(summary_conn(connection, person, &subject, None)?);
        }
    }
    Ok(SummaryPage {
        items,
        next_cursor: last.map(|index| format!("{scope}:{index}")),
    })
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
        let mut statement = connection.prepare_cached( /* sqlite-large-value: allow-for-now lv-h5182398e3ef3 owner=fractal_store::store::thread_metadata TODO=trace-statement-and-value-representation */ &sql).map_err(internal)?;
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

#[derive(serde::Serialize)]
pub struct StartContext {
    pub kind: &'static str,
    pub source_id: String,
    pub episode: String,
    pub start_recipient: inbox::StartRecipient,
}

/// Preview one already-known exact opening; this is not retained-source discovery.
pub fn start_context(
    store: &Store, person: &str, origin: &InboxOrigin,
) -> Result<StartContext, FractalError> {
    store.graph.read_snapshot_unindexed(|| Ok((|| {
        let connection = store.readers.get();
        let selected = inbox_source::resolve_discussion(
            store, &connection, person, origin, now_ms(),
        )?;
        let start_recipient = inbox::start_recipient_at(&connection, &selected.item)?
            .ok_or_else(|| FractalError::new(
                "thread-responsibility-unassigned",
                "this Inbox source has no configured responsible agent",
            ))?;
        selected.require_agent(store, &connection, &start_recipient.agent_id)?;
        Ok(StartContext {
            kind: "thread-start-context",
            source_id: origin.source_id.clone(),
            episode: origin.episode.clone(),
            start_recipient,
        })
    })())).map_err(internal)?
}

pub struct StartFromInbox<'a> {
    pub thread: &'a str,
    pub title: &'a str,
    pub person: &'a str,
    pub origin: &'a InboxOrigin,
    pub key: &'a str,
    pub first_message: &'a ClaimInput,
}

/// Start a source-bound Inbox thread as a trusted local person.
pub fn start_from_inbox(
    store: &Store,
    start: StartFromInbox<'_>,
) -> Result<(ClaimRecord, bool), FractalError> {
    start_from_inbox_as(store, start, PersonAuthority::TrustedLocal, None)
}

pub fn start_from_inbox_as(
    store: &Store,
    start: StartFromInbox<'_>,
    principal: PersonAuthority<'_>,
    signature: Option<&smallclaims::principal::ClaimSignature>,
) -> Result<(ClaimRecord, bool), FractalError> {
    let StartFromInbox { thread, title, person, origin, key, first_message } = start;
    let (source, episode) = (origin.source_id.as_str(), origin.episode.as_str());
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
            "thread start requires a plain inline first message to the responsible agent",
        ));
    }
    let mut first_message = first_message.clone();
    thread_delivery::add_reader_marker(&mut first_message);
    if first_message.fields.contains_key("thread_origin_ask") {
        return Err(FractalError::new("invalid-thread-start", "the first message has a different origin type"));
    }
    for (name, value) in [
        ("thread_id", thread), ("thread_origin_source", source), ("thread_origin_episode", episode),
    ] {
        if first_message.fields.get(name).is_some_and(|current| current.as_str() != Some(value)) {
            return Err(FractalError::new(
                "invalid-thread-start", "the first message context differs from the thread origin",
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
        "thread.start.inbox.v1",
        thread, title, person, source, episode, &first_message.subject, &first_message.fields,
    )).map_err(internal)?;
    let mut creation_body = json!({
        "fields": {
            "person": person, "agent": agent, "title": title, "first_message": first_message.subject,
        },
        "evidence": [],
        "_operation": {"id": operation_id, "request_digest": digest},
    });
    creation_body["fields"]["origin_source"] = json!(source);
    creation_body["fields"]["origin_episode"] = json!(episode);
    // This secondary effect has always stored empty evidence and used the
    // thread-start-message local-request operation, not a generic claim key.
    first_message.evidence.clear();
    first_message.idempotency_key = None;
    let message_body = json!({"fields": first_message.fields, "evidence": []});
    store.graph.ensure_principal_key(person)?;
    store
        .connection
        .batched(|tx| {
            store.thread_write_rollout(tx)?;
            principal.check(store, tx, person)?;
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
            let mut creation_body = creation_body.clone();
            creation_body["fields"]["person"] = json!(identities::resolve_reference_conn(tx, person)?);
            creation_body["fields"]["agent"] = json!(identities::resolve_reference_conn(tx, agent)?);
            let selected = inbox_source::resolve_discussion(store, tx, person, origin, now_ms())?;
            selected.require_agent(store, tx, agent)?;
            if latest_claim_id_tx(tx, thread).map_err(internal)?.is_some() {
                return Err(FractalError::new(
                    "stale-subject",
                    "the thread already exists",
                ));
            }
            creation_body["evidence"] = json!(selected.evidence);
            let item = selected.item;
            let mission = if let Some(mission) = item.mission {
                Some(mission)
            } else if let Some(run) = item.mission_run {
                mission_run_header_tx(tx, run.strip_prefix("mission-run/").unwrap_or(&run))
                    .optional().map_err(internal)?.map(|run| run.mission)
            } else {
                None
            };
            if let Some(mission) = mission {
                creation_body["fields"]["mission"] = json!(if mission.starts_with("mission/") {
                    mission
                } else {
                    format!("mission/{mission}")
                });
            }
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
            let mut message_body = message_body.clone();
            smallclaims::store::idempotency::attach(
                &mut message_body, &format!("thread-start-message:{key}"),
            ).map_err(internal)?;
            // Keep the secondary effect's existing operation digest, while sharing
            // typed admission and the caller's atomic thread-creation transaction.
            append_claim_with_commit_context_tx(
                &store.graph, tx, &first_message, Some(&created.batch_id),
                ClaimCommitContext {
                    thread_delivery: true,
                    signature: encoded_signature.as_deref(),
                    effect_operation: smallclaims::store::operation_parts(&message_body),
                    ..Default::default()
                },
            )?;
            register_operation_tx(tx, &created).map_err(internal)?;
            Ok((created, true))
        })
        .map_err(internal)?
}


#[cfg(test)]
mod marked_read_helper_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn read_helpers_marked_cursor_seeks_past_newer_owner_history() {
        let store = Store::open_memory("marked-range").unwrap();
        store.configure_thread_write_rollout([]);
        store.append_claim(&ClaimInput{subject:"daemon/marked-range".into(),kind:"daemon.started".into(),actor:None,
            fields:serde_json::from_value(json!({"status":"running","features":{"thread_read_privacy":fractal_schema::threads::READ_PRIVACY_VERSION}})).unwrap(),evidence:vec![],expected_subject:None,idempotency_key:None}).unwrap();
        let person = "person/example";
        let agent = "agent/example";
        for (thread, message) in [("thread/old", "message/old"), ("thread/new", "message/new")] {
            apply(
                &store,
                person,
                thread,
                thread,
                Edit::Create {
                    agent,
                    title: "Range",
                },
            )
            .unwrap();
            thread_delivery::append_reply(&store,&ClaimInput{subject:message.into(),kind:"message.sent".into(),actor:Some(agent.into()),
                fields:serde_json::from_value(json!({"from":agent,"to":person,"status":"sent","content":"Range","thread_id":thread})).unwrap(),evidence:vec![],expected_subject:None,idempotency_key:Some(message.into())},PersonAuthority::TrustedLocal,None).unwrap();
        }
        let base = store
            .claims_page(None, None, 0, None, false, 100)
            .unwrap()
            .claims
            .iter()
            .map(|claim| claim.accepted_at_unix_ms)
            .max()
            .unwrap()
            + 100;
        store.set_write_clock_at(base + 10).unwrap();
        thread_reading::apply_as(
            &store,
            PersonAuthority::TrustedLocal,
            person,
            "thread/old",
            "old",
            thread_reading::Edit::Mark {
                anchor_message: "message/old",
                observed_marks: &[],
            },
        )
        .unwrap();
        let append_newer = |n: u64| {
            store
                .set_write_clock_at(base + 100 + u128::from(n) * 10)
                .unwrap();
            thread_reading::apply_as(
                &store,
                PersonAuthority::TrustedLocal,
                person,
                "thread/new",
                &format!("new-{n}"),
                thread_reading::Edit::Mark {
                    anchor_message: "message/new",
                    observed_marks: &[],
                },
            )
            .unwrap();
        };
        for n in 0..64 {
            append_newer(n);
        }
        let scope = canonical_hash(&("thread.marked-unread.v1", person)).unwrap();
        let cursor = format!("{scope}:{}:thread/z-boundary", base + 50);
        let measure = || {
            let connection = store.readers.get();
            let steps = Arc::new(AtomicUsize::new(0));
            let count = steps.clone();
            connection.progress_handler(
                1,
                Some(move || {
                    count.fetch_add(1, Ordering::Relaxed);
                    false
                }),
            );
            let page = marked_unread(&connection, person, Some(&cursor), 1).unwrap();
            connection.progress_handler(0, None::<fn() -> bool>);
            assert_eq!(page.items[0].id, "thread/old");
            steps.load(Ordering::Relaxed)
        };
        let before = measure();
        for n in 64..256 {
            append_newer(n);
        }
        let after = measure();
        eprintln!("marked deep-cursor VM steps {before} -> {after}");
        assert!(
            after <= before * 2 + 500,
            "owner-time cursor must seek, not rescan newer history"
        );
        let connection = store.readers.get();
        let plan = connection
            .prepare(&format!(
                "EXPLAIN QUERY PLAN {}",
                marked_candidate_sql(true)
            ))
            .unwrap()
            .query_map(
                params![person, (base + 50).to_string(), "thread/z-boundary", 5],
                |row| row.get::<_, String>(3),
            )
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
            .join("; ");
        eprintln!("marked cursor plan: {plan}");
        assert!(
            plan.contains("claims_thread_unread_owner_time_index") && plan.contains('<'),
            "{plan}"
        );
    }
}
