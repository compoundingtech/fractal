//! Durable thread metadata. Client adapters authenticate the actor before calling this layer.

use super::*;

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
    fn check(&self, connection: &Connection, person: &str) -> Result<(), FractalError> {
        let forbidden = || {
            FractalError::new(
                "thread-authority-expired",
                "the thread action credential is unavailable",
            )
        };
        match self {
            Self::TrustedLocal => Ok(()),
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
    apply_as(
        store,
        person,
        subject,
        key,
        edit,
        PersonAuthority::TrustedLocal,
    )
}

pub fn apply_as(
    store: &Store,
    person: &str,
    subject: &str,
    key: &str,
    edit: Edit<'_>,
    principal: PersonAuthority<'_>,
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

fn owned_creation(
    connection: &Connection,
    subject: &str,
    person: &str,
    creating: bool,
) -> Result<Option<ClaimRecord>, FractalError> {
    let Some(creator) = latest_applied_claim(connection, subject, "thread.created")? else {
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

fn latest_applied_claim(
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
        .read_snapshot_unindexed(|| {
            Ok((|| {
                let connection = store.readers.get();
                let creation =
                    owned_creation(&connection, subject, person, false)?.ok_or_else(unavailable)?;
                let state = match latest_applied_claim(&connection, subject, "thread.state-set")?
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
                let title = latest_applied_claim(&connection, subject, "thread.title-set")?
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
            })())
        })
        .map_err(internal)?
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
