//! Exact thread message reads over the normal message store.

use super::*;

#[cfg(test)]
thread_local! { static WINDOW_BODY_FETCHES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

#[derive(serde::Serialize)]
pub struct ThreadMessage {
    pub kind: &'static str,
    pub id: String,
    pub actor_id: String,
    pub accepted_at_unix_ms: u128,
    pub content: String,
    pub thread_quote_id: Option<String>,
    #[serde(skip)]
    pub claim_id: String,
}

#[derive(serde::Serialize)]
#[serde(untagged)]
pub enum TimelineItem {
    Message(ThreadMessage),
    Lifecycle(Event),
}

#[derive(serde::Serialize)]
pub struct TimelinePage {
    pub items: Vec<TimelineItem>,
    pub next_cursor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_cursor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor: Option<TimelineAnchor>,
}

#[derive(Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TimelineAnchorState {
    Current,
    Unavailable,
    Pending,
}

#[derive(Debug, PartialEq, Eq, serde::Serialize)]
pub struct TimelineAnchor {
    pub message_id: String,
    pub state: TimelineAnchorState,
}

/// Count admitted messages and determine whether a person still has an incoming reply to read.
/// The ordinary recipient's message.read/closed claims are durable and apply in both views.
pub(super) fn message_summary(
    connection: &Connection,
    person: &str,
    agent: &str,
    thread: &str,
    creation: &ClaimRecord,
) -> Result<(u64, bool), FractalError> {
    thread_message_activity(
        connection,
        person,
        agent,
        thread,
        creation,
        "SELECT count(*), coalesce(max(unread), 0) FROM qualified",
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
}

/// Same message membership and receipts as the thread summary. An Inbox badge
/// can stop at the first unread reply without counting the rest of the history.
pub(super) fn message_unread(
    connection: &Connection,
    person: &str,
    agent: &str,
    thread: &str,
    creation: &ClaimRecord,
) -> Result<bool, FractalError> {
    thread_message_activity(
        connection,
        person,
        agent,
        thread,
        creation,
        "SELECT EXISTS(SELECT 1 FROM qualified WHERE unread)",
        |row| row.get(0),
    )
}

fn thread_message_activity<T>(
    connection: &Connection,
    person: &str,
    agent: &str,
    thread: &str,
    creation: &ClaimRecord,
    select: &str,
    decode: impl FnOnce(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
) -> Result<T, FractalError> {
    let person = identities::resolve_reference_conn(connection, person)?;
    let agent = identities::resolve_reference_conn(connection, agent)?;
    let first = creation.body["fields"]["first_message"].as_str();
    let sql = canonical_sql(&format!(
        "WITH explicit AS (
            SELECT claims.subject, claims.actor, claims.body
            FROM claims INDEXED BY claims_thread_message_accepted_index
            WHERE claims.kind='message.sent' AND json_extract(claims.body, '$.fields.thread_id')=?1
              AND json_extract(claims.body, '$.fields.thread_id') IS NOT NULL
              AND {ACTIVE_MESSAGE} AND {UNREPAIRED_CLAIM}
        ), legacy AS (
            SELECT claims.subject, claims.actor, claims.body FROM claims
            WHERE claims.subject=?2 AND claims.batch_id=?3 AND claims.kind='message.sent'
              AND json_extract(claims.body, '$.fields.thread_id') IS NULL
              AND {ACTIVE_MESSAGE} AND {UNREPAIRED_CLAIM}
        ), participants AS (
            SELECT messages.subject,
                   COALESCE(actor.identity, messages.actor) AS actor_id,
                   COALESCE(sender.identity, json_extract(messages.body, '$.fields.from')) AS sender_id,
                   COALESCE(recipient.identity, json_extract(messages.body, '$.fields.to')) AS recipient_id
            FROM (SELECT * FROM explicit UNION ALL SELECT * FROM legacy) messages
            LEFT JOIN identity_names actor ON actor.name=messages.actor
            LEFT JOIN identity_names sender ON sender.name=json_extract(messages.body, '$.fields.from')
            LEFT JOIN identity_names recipient ON recipient.name=json_extract(messages.body, '$.fields.to')
        ), qualified AS (
            SELECT recipient_id=?4 AND NOT EXISTS (
                SELECT 1 FROM claims INDEXED BY claims_subject_kind_index
                WHERE claims.subject=participants.subject
                  AND ((claims.kind='message.read' AND claims.actor=?4) OR claims.kind='message.closed')
                  AND {UNREPAIRED_CLAIM}
            ) AS unread
            FROM participants
            WHERE actor_id=sender_id AND ((sender_id=?4 AND recipient_id=?5) OR (sender_id=?5 AND recipient_id=?4))
        )
        {select}",
    ));
    connection
        .query_row(
            /* sqlite-large-value: allow-for-now lv-h772975c067c2 owner=fractal_store::store::thread_message_window TODO=trace-statement-and-value-representation */ &sql,
            params![thread, first, creation.batch_id, person, agent],
            decode,
        )
        .map_err(internal)
}

const ACTIVE_MESSAGE: &str = "EXISTS (SELECT 1 FROM operations
  WHERE operations.id=json_extract(claims.body, '$._operation.id')
    AND operations.canonical_claim_id=claims.id AND operations.state='active')";

/// Acknowledge only exact rendered message identities. No time or replication frontier
/// is advanced, so a later arrival stays unread even if it sorts before the anchor.
pub fn mark_read(
    store: &Store,
    authority: PersonAuthority<'_>,
    person: &str,
    thread: &str,
    through: Option<&str>,
    seen: &[String],
) -> Result<(Vec<String>, bool), FractalError> {
    let invalid = || {
        FractalError::new(
            "invalid-thread-read",
            "expected up to 50 distinct message IDs and their newest anchor",
        )
    };
    let valid_id = |id: &str| id.starts_with("message/") && (9..=512).contains(&id.len());
    if seen.len() > 50 {
        return Err(invalid());
    }
    let unique = seen.iter().map(String::as_str).collect::<BTreeSet<_>>();
    if unique.len() != seen.len()
        || !unique.iter().all(|id| valid_id(id))
        || through.map_or(!seen.is_empty(), |anchor| !unique.contains(anchor))
    {
        return Err(invalid());
    }

    if !seen.is_empty() {
        store.graph.ensure_principal_key(person)?;
    }
    store.connection.batched(|tx| {
        authority.check(store, tx, person)?;
        store.thread_write_rollout(tx)?;
        let creation = owned_creation(tx, thread, person, false)
            .map_err(|_| unavailable())?.ok_or_else(unavailable)?;
        let agent = creation.body["fields"]["agent"].as_str().ok_or_else(unavailable)?;
        let person = identities::resolve_reference_conn(tx, person)?;
        let agent = identities::resolve_reference_conn(tx, agent)?;
        let first = creation.body["fields"]["first_message"].as_str();
        let through_index = current_index(tx).map_err(internal)?;
        let sql = canonical_sql(&format!(
            "SELECT {CLAIM_COLUMNS} FROM claims
             WHERE claims.subject=?1 AND claims.kind='message.sent'
               AND {ACTIVE_MESSAGE} AND {UNREPAIRED_CLAIM}
             ORDER BY CANONICAL_DESC(claims) LIMIT 1"
        ));
        let mut select = tx.prepare_cached(/* sqlite-large-value: allow-for-now lv-h83e8088e51d8 owner=fractal_store::store::thread_message_window TODO=trace-statement-and-value-representation */ &sql).map_err(internal)?;
        let mut newest = None;
        let mut incoming = Vec::new();
        for subject in seen {
            let claim = select.query_row(/* sqlite-large-value: allow-for-now lv-h181b1773755f owner=fractal_store::store::thread_message_window TODO=trace-statement-and-value-representation */ [subject], claim_from_row).optional().map_err(internal)?
                .filter(|claim| member(claim, thread, first, &creation.batch_id, through_index))
                .ok_or_else(unavailable)?;
            let actor = message_actor(tx, &claim, &person, &agent)?;
            let key = smallclaims::store::canonical::claim_key(tx, &claim.id).map_err(internal)?;
            if newest.as_ref().is_none_or(|(newest, _)| &key > newest) {
                newest = Some((key, subject.as_str()));
            }
            if actor == agent {
                incoming.push(subject.clone());
            }
        }
        if newest.map(|(_, id)| id) != through {
            return Err(invalid());
        }

        // Every member was authorized above. A fixed person-and-message operation key
        // makes a retried or concurrent acknowledgement converge across serving members.
        let mut batch = None;
        for subject in &incoming {
            let settled: bool = tx.query_row(
                /* sqlite-large-value: allow-for-now lv-hbc8c4357028b owner=fractal_store::store::thread_message_window TODO=trace-statement-and-value-representation */ &format!("SELECT EXISTS(SELECT 1 FROM claims INDEXED BY claims_subject_kind_index
                 WHERE claims.subject=?1
                   AND ((claims.kind='message.read' AND claims.actor=?2) OR claims.kind='message.closed')
                   AND {UNREPAIRED_CLAIM})"),
                params![subject, person], |row| row.get(0),
            ).map_err(internal)?;
            if settled {
                continue;
            }
            let mut body = json!({"fields":{"status":"read"},"evidence":[]});
            let key = canonical_hash(&("thread-read-v1", &person, subject)).map_err(internal)?;
            smallclaims::store::idempotency::attach(&mut body, &key).map_err(internal)?;
            let mark = append_claim_tx(
                tx, &store.origin, subject, "message.read", Some(&person), &body, &[], batch.as_deref(),
            ).map_err(claim_append_error)?;
            register_operation_tx(tx, &mark).map_err(internal)?;
            batch = Some(mark.batch_id);
        }
        Ok((incoming, batch.is_some()))
    }).map_err(internal)?
}

/// Newest-first page over admitted messages and lifecycle events. The caller authenticates the person.
pub fn mixed_timeline(
    store: &Store,
    person: &str,
    thread: &str,
    cursor: Option<&str>,
    around: Option<&str>,
    limit: usize,
) -> Result<TimelinePage, FractalError> {
    store
        .graph
        .read_snapshot(|index| {
            Ok(read(
                &store.readers.get(),
                person,
                thread,
                cursor,
                around,
                limit,
                index,
            ))
        })
        .map_err(internal)?
}

fn read(
    connection: &Connection,
    person: &str,
    thread: &str,
    cursor: Option<&str>,
    around: Option<&str>,
    limit: usize,
    index: u64,
) -> Result<TimelinePage, FractalError> {
    let invalid_cursor = || {
        FractalError::new(
            "invalid-thread-cursor",
            "the thread message cursor is unavailable",
        )
    };
    if cursor.is_some() && around.is_some() {
        return Err(invalid_cursor());
    }
    let creation = owned_creation(connection, thread, person, false)?.ok_or_else(unavailable)?;
    let person = identities::resolve_reference_conn(connection, person)?;
    let agent = identities::resolve_reference_conn(
        connection,
        creation.body["fields"]["agent"]
            .as_str()
            .ok_or_else(unavailable)?,
    )?;
    let first = creation.body["fields"]["first_message"].as_str();
    let scope =
        canonical_hash(&("thread.mixed-timeline.v1", &person, &agent, thread)).map_err(internal)?;
    let (through, boundary, newer) = match cursor {
        Some(cursor) => {
            let (cursor_scope, suffix) = cursor.split_once(':').ok_or_else(invalid_cursor)?;
            let (through, marker) = suffix.split_once(':').ok_or_else(invalid_cursor)?;
            let through = through.parse::<u64>().map_err(|_| invalid_cursor())?;
            let (newer, id) = marker
                .strip_prefix("newer/")
                .map_or((false, marker), |id| (true, id));
            if cursor_scope != scope
                || through > index
                || through < creation.store_index
                || id.len() > 512
            {
                return Err(invalid_cursor());
            }
            let claim = claim_by_id_tx(connection, id)
                .map_err(internal)?
                .filter(|claim| {
                    claim.store_index <= through
                        && (lifecycle(claim, thread)
                            || member(claim, thread, first, &creation.batch_id, through))
                })
                .ok_or_else(invalid_cursor)?;
            if claim.kind == "message.sent" {
                message_actor(connection, &claim, &person, &agent).map_err(|_| invalid_cursor())?;
            }
            (through, Some(claim), newer)
        }
        None => (index, None, false),
    };
    let limit = limit.clamp(1, 50);
    let mut anchor_state = None;
    let mut selected = if let Some(subject) = around {
        if !subject.starts_with("message/") || subject.len() > 512 {
            return Err(unavailable());
        }
        let sql = canonical_sql(&format!(
            "SELECT {CLAIM_COLUMNS} FROM claims INDEXED BY claims_subject_kind_index
            WHERE claims.subject=?1 AND claims.kind='message.sent' AND claims.store_index<=?2
              AND {ACTIVE_MESSAGE} AND {UNREPAIRED_CLAIM} ORDER BY CANONICAL_DESC(claims) LIMIT 1"
        ));
        #[cfg(test)]
        WINDOW_BODY_FETCHES.with(|count| count.set(count.get() + 1));
        let anchor = connection
            .query_row(/* sqlite-large-value: allow-for-now lv-hbc0b493bd9e2 owner=fractal_store::store::thread_message_window TODO=trace-statement-and-value-representation */ &sql, params![subject, through], claim_from_row)
            .optional()
            .map_err(internal)?
            .filter(|claim| member(claim, thread, first, &creation.batch_id, through));
        let anchor = match anchor {
            Some(anchor) if message_actor(connection, &anchor, &person, &agent).is_ok() => anchor,
            _ => {
                if !super::super::thread_reading::private_anchor_intent_conn(
                    connection, &person, thread, subject,
                )? {
                    return Err(unavailable());
                }
                let known:bool=connection.query_row("SELECT EXISTS(SELECT 1 FROM claims INDEXED BY claims_subject_kind_index WHERE subject=?1 AND kind='message.sent')",[subject],|row|row.get(0)).map_err(internal)?;
                return Ok(TimelinePage {
                    items: vec![],
                    next_cursor: None,
                    previous_cursor: None,
                    anchor: Some(TimelineAnchor {
                        message_id: subject.into(),
                        state: if known {
                            TimelineAnchorState::Unavailable
                        } else {
                            TimelineAnchorState::Pending
                        },
                    }),
                });
            }
        };
        let mut newer = query_window_headers(
            connection,
            thread,
            &creation,
            through,
            Some(&anchor),
            true,
            limit,
        )?;
        let older = query_window_headers(
            connection,
            thread,
            &creation,
            through,
            Some(&anchor),
            false,
            limit,
        )?;
        let mut newer_count = newer.len().min(limit / 2);
        let older_count = older.len().min(limit - 1 - newer_count);
        newer_count = newer.len().min(limit - 1 - older_count);
        newer.truncate(newer_count);
        newer.reverse();
        let mut newer = materialize_window(connection, newer)?;
        newer.push((anchor, "active".into()));
        newer.extend(materialize_window(
            connection,
            older.into_iter().take(older_count).collect(),
        )?);
        anchor_state = Some(TimelineAnchor {
            message_id: subject.into(),
            state: TimelineAnchorState::Current,
        });
        newer
    } else {
        let mut selected = query_window(
            connection,
            thread,
            &creation,
            through,
            boundary.as_ref(),
            newer,
            limit,
        )?;
        selected.truncate(limit);
        if newer {
            selected.reverse();
        }
        selected
    };
    let first_claim = selected.first().map(|(claim, _)| claim);
    let last_claim = selected.last().map(|(claim, _)| claim);
    let previous_cursor = match first_claim {
        Some(first)
            if !query_window_headers(
                connection,
                thread,
                &creation,
                through,
                Some(first),
                true,
                1,
            )?
            .is_empty() =>
        {
            Some(format!("{scope}:{through}:newer/{}", first.id))
        }
        _ => None,
    };
    let next_cursor = match last_claim {
        Some(last)
            if !query_window_headers(
                connection,
                thread,
                &creation,
                through,
                Some(last),
                false,
                1,
            )?
            .is_empty() =>
        {
            Some(format!("{scope}:{through}:{}", last.id))
        }
        _ => None,
    };
    let mut items = Vec::with_capacity(selected.len());
    for (claim, operation_state) in selected.drain(..) {
        if operation_state == "conflict" {
            return Err(FractalError::new(
                "idempotency-conflict",
                "a thread metadata operation has conflicting requests",
            ));
        }
        if lifecycle(&claim, thread) {
            items.push(TimelineItem::Lifecycle(
                Event::from_claim(claim).ok_or_else(unavailable)?,
            ));
            continue;
        }
        let actor = message_actor(connection, &claim, &person, &agent)?;
        let content = message_content(&claim)?.to_owned();
        let quoted = message_quote(
            connection, &claim, &creation, thread, &person, &agent, through,
        )?
        .map(str::to_owned);
        items.push(TimelineItem::Message(ThreadMessage {
            kind: "message",
            id: claim.subject,
            actor_id: actor,
            accepted_at_unix_ms: claim.accepted_at_unix_ms,
            content,
            thread_quote_id: quoted,
            claim_id: claim.id,
        }));
    }
    Ok(TimelinePage {
        items,
        next_cursor,
        previous_cursor,
        anchor: anchor_state,
    })
}

/// Both directions seek this thread's routing index before decoding only the bounded window.
fn materialize_window(
    connection: &Connection,
    headers: Vec<(String, String)>,
) -> Result<Vec<(ClaimRecord, String)>, FractalError> {
    headers
        .into_iter()
        .map(|(id, state)| {
            #[cfg(test)]
            WINDOW_BODY_FETCHES.with(|count| count.set(count.get() + 1));
            Ok((
                claim_by_id_tx(connection, &id)
                    .map_err(internal)?
                    .ok_or_else(unavailable)?,
                state,
            ))
        })
        .collect()
}
fn query_window(
    connection: &Connection,
    thread: &str,
    creation: &ClaimRecord,
    through: u64,
    anchor: Option<&ClaimRecord>,
    newer: bool,
    limit: usize,
) -> Result<Vec<(ClaimRecord, String)>, FractalError> {
    materialize_window(
        connection,
        query_window_headers(connection, thread, creation, through, anchor, newer, limit)?,
    )
}

/// Decimal u128 time is indexed as (length,text). SQLite does not range-seek an
/// expression row-value tuple, so each varying component gets its own bounded arm.
/// The remaining canonical tie keys stay exactly those of canonical::components.
fn query_window_headers(
    connection: &Connection,
    thread: &str,
    creation: &ClaimRecord,
    through: u64,
    anchor: Option<&ClaimRecord>,
    newer: bool,
    limit: usize,
) -> Result<Vec<(String, String)>, FractalError> {
    let direction = if newer { "ASC" } else { "DESC" };
    let comparison = if newer { ">" } else { "<" };
    let after = if newer {
        canonical::after_sql("claims", "anchor")
    } else {
        canonical::after_sql("anchor", "claims")
    };
    let strict = if anchor.is_some() {
        format!("AND EXISTS(SELECT 1 FROM claims anchor WHERE anchor.id=?5 AND {after})")
    } else {
        String::new()
    };
    let headers = "claims.id,claims.batch_id,claims.store_index,claims.accepted_at_unix_ms";
    let arm =
        |from: &str, condition: &str, state: &str, range: &str, constant_components: usize| {
            let order = canonical::components("claims")
                .into_iter()
                .skip(constant_components)
                .map(|key| format!("{key} {direction}"))
                .collect::<Vec<_>>()
                .join(",");
            format!(
                "SELECT {headers},{state} AS operation_state FROM {from} WHERE {condition}
          AND claims.store_index<=?4 AND {UNREPAIRED_CLAIM} {range} {strict}
          ORDER BY {order} LIMIT ?7"
            )
        };
    let slices = |from: &str, condition: &str, state: &str| {
        let arms = if anchor.is_some() {
            vec![
                arm(
                    from,
                    condition,
                    state,
                    &format!("AND length(claims.accepted_at_unix_ms){comparison}length(?6)"),
                    0,
                ),
                arm(
                    from,
                    condition,
                    state,
                    &format!(
                        "AND length(claims.accepted_at_unix_ms)=length(?6) AND claims.accepted_at_unix_ms{comparison}?6"
                    ),
                    1,
                ),
                arm(
                    from,
                    condition,
                    state,
                    "AND length(claims.accepted_at_unix_ms)=length(?6) AND claims.accepted_at_unix_ms=?6",
                    2,
                ),
            ]
        } else {
            vec![arm(from, condition, state, "", 0)]
        };
        arms.into_iter()
            .map(|sql| format!("SELECT * FROM ({sql})"))
            .collect::<Vec<_>>()
            .join(" UNION ALL ")
    };
    let active = "JOIN operations active_operation ON active_operation.canonical_claim_id=claims.id AND active_operation.state='active'";
    let explicit = slices(
        &format!("claims INDEXED BY claims_thread_message_accepted_index {active}"),
        "claims.kind='message.sent' AND json_extract(claims.body,'$.fields.thread_id')=?1 AND json_extract(claims.body,'$.fields.thread_id') IS NOT NULL",
        "'active'",
    );
    // The legacy arm reads only the exact first-message subject/batch, never thread history.
    let legacy = arm(
        &format!("claims {active}"),
        "claims.subject=?2 AND claims.batch_id=?3 AND claims.kind='message.sent' AND json_extract(claims.body,'$.fields.thread_id') IS NULL",
        "'active'",
        "",
        0,
    );
    let lifecycle = slices(
        "claims INDEXED BY claims_subject_accepted_index JOIN operations ON operations.id=json_extract(claims.body,'$._operation.id')",
        "claims.subject=?1 AND claims.kind IN ('thread.title-set','thread.state-set') AND (operations.state='conflict' OR operations.canonical_claim_id=claims.id)",
        "operations.state",
    );
    let order = canonical::order_sql("claims", !newer);
    let sql=format!("WITH explicit AS ({explicit}),legacy AS ({legacy}),lifecycle AS ({lifecycle})
        SELECT claims.id,operation_state FROM (SELECT * FROM explicit UNION ALL SELECT * FROM legacy UNION ALL SELECT * FROM lifecycle) claims
        ORDER BY {order} LIMIT ?7");
    let mut statement=connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-he5a2848d26a3 owner=fractal_store::store::thread_message_window TODO=trace-statement-and-value-representation */ &sql).map_err(internal)?;
    let rows = statement
        .query_map(
            params![
                thread,
                creation.body["fields"]["first_message"].as_str(),
                creation.batch_id,
                through,
                anchor.map(|claim| &claim.id),
                anchor.map(|claim| claim.accepted_at_unix_ms.to_string()),
                limit + 1
            ],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .map_err(internal)?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(internal)
}

/// Resolve the same canonical admitted message from a previously issued content handle.
pub fn mixed_message_content(
    store: &Store,
    person: &str,
    thread: &str,
    claim_id: &str,
) -> Result<String, FractalError> {
    store
        .graph
        .read_snapshot(|index| {
            let connection = store.readers.get();
            let read = || {
                let creation =
                    owned_creation(&connection, thread, person, false)?.ok_or_else(unavailable)?;
                let agent = creation.body["fields"]["agent"]
                    .as_str()
                    .ok_or_else(unavailable)?;
                let first = creation.body["fields"]["first_message"].as_str();
                let sql = canonical_sql(&format!(
                    "SELECT {CLAIM_COLUMNS} FROM claims WHERE id=?1
                  AND {UNREPAIRED_CLAIM}
                  AND EXISTS (SELECT 1 FROM operations
                    WHERE operations.id=json_extract(claims.body, '$._operation.id')
                      AND operations.canonical_claim_id=claims.id AND operations.state='active')"
                ));
                let claim = connection
                    .query_row(/* sqlite-large-value: allow-for-now lv-h8da1ab4af0ab owner=fractal_store::store::thread_message_window TODO=trace-statement-and-value-representation */ &sql, [claim_id], claim_from_row)
                    .optional()
                    .map_err(internal)?
                    .filter(|claim| member(claim, thread, first, &creation.batch_id, index))
                    .ok_or_else(unavailable)?;
                let person = identities::resolve_reference_conn(&connection, person)?;
                let agent = identities::resolve_reference_conn(&connection, agent)?;
                message_actor(&connection, &claim, &person, &agent)?;
                message_quote(
                    &connection,
                    &claim,
                    &creation,
                    thread,
                    &person,
                    &agent,
                    index,
                )?;
                message_content(&claim).map(str::to_owned)
            };
            Ok(read())
        })
        .map_err(internal)?
}

fn message_quote<'a>(
    connection: &Connection,
    claim: &'a ClaimRecord,
    creation: &ClaimRecord,
    thread: &str,
    person: &str,
    agent: &str,
    through: u64,
) -> Result<Option<&'a str>, FractalError> {
    let Some(quoted) = claim.body["fields"]["thread_quote_id"].as_str() else {
        return Ok(None);
    };
    let first = creation.body["fields"]["first_message"].as_str();
    let sql = canonical_sql(&format!(
        "SELECT {CLAIM_COLUMNS} FROM claims WHERE subject=?1 AND kind='message.sent'
          AND {UNREPAIRED_CLAIM}
          AND EXISTS (SELECT 1 FROM operations
            WHERE operations.id=json_extract(claims.body, '$._operation.id')
              AND operations.canonical_claim_id=claims.id AND operations.state='active')
          ORDER BY CANONICAL_DESC(claims) LIMIT 1"
    ));
    let parent = connection
        .query_row(/* sqlite-large-value: allow-for-now lv-h223e94a92d75 owner=fractal_store::store::thread_message_window TODO=trace-statement-and-value-representation */ &sql, [quoted], claim_from_row)
        .optional()
        .map_err(internal)?
        .filter(|parent| {
            parent.subject != claim.subject
                && member(parent, thread, first, &creation.batch_id, through)
        })
        .ok_or_else(unavailable)?;
    message_actor(connection, &parent, person, agent)?;
    Ok(Some(quoted))
}

fn message_content(claim: &ClaimRecord) -> Result<&str, FractalError> {
    claim.body["fields"]["content"]
        .as_str()
        .filter(|content| !content.starts_with("doc/") && content.len() <= 8 * 1024)
        .ok_or_else(unavailable)
}

fn lifecycle(claim: &ClaimRecord, thread: &str) -> bool {
    claim.subject == thread
        && matches!(claim.kind.as_str(), "thread.title-set" | "thread.state-set")
}

fn member(
    claim: &ClaimRecord,
    thread: &str,
    first: Option<&str>,
    batch: &str,
    through: u64,
) -> bool {
    claim.kind == "message.sent"
        && claim.store_index <= through
        && match claim.body["fields"]["thread_id"].as_str() {
            Some(id) => id == thread,
            None => first == Some(claim.subject.as_str()) && claim.batch_id == batch,
        }
}

fn message_actor(
    connection: &Connection,
    claim: &ClaimRecord,
    person: &str,
    agent: &str,
) -> Result<String, FractalError> {
    let resolve = |value: Option<&str>| {
        identities::resolve_reference_conn(connection, value.ok_or_else(unavailable)?)
    };
    let fields = &claim.body["fields"];
    let actor = resolve(claim.actor.as_deref())?;
    let from = resolve(fields["from"].as_str())?;
    let to = resolve(fields["to"].as_str())?;
    if actor == from && ((from == person && to == agent) || (from == agent && to == person)) {
        Ok(actor)
    } else {
        Err(unavailable())
    }
}

#[cfg(test)]
mod read_progress_tests {
    use super::*;

    #[test]
    fn exact_thread_reads_replicate_without_clearing_late_older_mail() {
        let left = Store::open_memory("thread-read-left").unwrap();
        let right = Store::open_memory("thread-read-right").unwrap();
        for store in [&left, &right] {
            store.configure_thread_write_rollout([]);
            store.append_claim(&ClaimInput {
                subject: format!("daemon/{}", store.origin()), kind: "daemon.started".into(),
                actor: None, fields: serde_json::from_value(json!({
                    "status":"running", "features":{"thread_read_privacy":fractal_schema::threads::READ_PRIVACY_VERSION}
                })).unwrap(), evidence: vec![], expected_subject: None, idempotency_key: None,
            }).unwrap();
        }
        let owner = "person/example";
        let agent = "agent/example";
        let thread = "thread/read-test";
        let (_created, _) = apply(
            &left,
            owner,
            thread,
            "create-read-test",
            Edit::Create {
                agent,
                title: "Review",
            },
        )
        .unwrap();
        let replicate = |source: &Store, target: &Store| {
            target
                .import_replication(source.origin(), &source.export_replication(0).unwrap())
                .unwrap();
        };
        replicate(&left, &right);
        let message = |store: &Store, id: &str| {
            super::super::thread_delivery::append_reply(store, &ClaimInput {
                subject:id.into(), kind:"message.sent".into(), actor:Some(agent.into()),
                fields:serde_json::from_value(json!({
                    "from":agent,"to":owner,"content":"Reply","status":"sent","thread_id":thread,
                })).unwrap(), evidence:vec![], expected_subject:None, idempotency_key:Some(id.into()),
            }, PersonAuthority::TrustedLocal, None).unwrap().0
        };
        let late = message(&right, "message/older-late");
        std::thread::sleep(std::time::Duration::from_millis(3));
        let anchor = message(&left, "message/shown-anchor");
        assert!(late.accepted_at_unix_ms < anchor.accepted_at_unix_ms);
        let (_closed, _) = apply(
            &left,
            owner,
            thread,
            "close-read-test",
            Edit::Close {
                revision: &crate::store::thread_metadata::summary(&left, owner, thread)
                    .unwrap()
                    .revision,
            },
        )
        .unwrap();
        let closed_revision = summary(&left, owner, thread).unwrap().revision;
        let seen = vec![anchor.subject.clone()];
        assert_eq!(
            mark_read(
                &left,
                PersonAuthority::TrustedLocal,
                owner,
                thread,
                None,
                &[]
            )
            .unwrap(),
            (vec![], false)
        );
        assert_eq!(
            mark_read(
                &left,
                PersonAuthority::TrustedLocal,
                "person/other",
                thread,
                None,
                &[]
            )
            .unwrap_err()
            .code,
            "thread-unavailable"
        );
        let invalid = vec![anchor.subject.clone(), "message/unknown".into()];
        assert_eq!(
            mark_read(
                &left,
                PersonAuthority::TrustedLocal,
                owner,
                thread,
                Some(&anchor.subject),
                &invalid
            )
            .unwrap_err()
            .code,
            "thread-unavailable"
        );
        assert!(summary(&left, owner, thread).unwrap().unread);
        assert_eq!(
            mark_read(
                &left,
                PersonAuthority::TrustedLocal,
                owner,
                thread,
                Some(&anchor.subject),
                &seen
            )
            .unwrap(),
            (seen.clone(), true)
        );
        assert_eq!(
            mark_read(
                &left,
                PersonAuthority::TrustedLocal,
                owner,
                thread,
                Some(&anchor.subject),
                &seen
            )
            .unwrap(),
            (seen, false)
        );
        assert!(!summary(&left, owner, thread).unwrap().unread);
        replicate(&right, &left);
        replicate(&left, &right);
        for store in [&left, &right] {
            let summary = summary(store, owner, thread).unwrap();
            assert!(summary.unread);
            assert_eq!(summary.state, "closed");
            assert_eq!(summary.revision, closed_revision);
            assert_eq!(
                store
                    .claims_for(&anchor.subject, Some("message.read"))
                    .unwrap()
                    .len(),
                1
            );
            assert!(
                store
                    .claims_for(&late.subject, Some("message.read"))
                    .unwrap()
                    .is_empty()
            );
        }
    }

    #[test]
    fn read_helpers_anchor_seek_work_is_bounded_as_history_grows() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let store = Store::open_memory("seek-cost").unwrap();
        store.configure_thread_write_rollout([]);
        store.append_claim(&ClaimInput{subject:"daemon/seek-cost".into(),kind:"daemon.started".into(),actor:None,
            fields:serde_json::from_value(json!({"status":"running","features":{"thread_read_privacy":fractal_schema::threads::READ_PRIVACY_VERSION}})).unwrap(),evidence:vec![],expected_subject:None,idempotency_key:None}).unwrap();
        let person = "person/example";
        let agent = "agent/example";
        let thread = "thread/seek-cost";
        apply(
            &store,
            person,
            thread,
            "create",
            Edit::Create {
                agent,
                title: "Seek",
            },
        )
        .unwrap();
        let base = store
            .claims_page(None, None, 0, None, false, 100)
            .unwrap()
            .claims
            .iter()
            .map(|claim| claim.accepted_at_unix_ms)
            .max()
            .unwrap()
            + 100;
        let send = |n: u64, time: u128| {
            store.set_write_clock_at(time).unwrap();
            let receipt=super::super::thread_delivery::append_reply(&store,&ClaimInput{subject:format!("message/cost-{n}"),kind:"message.sent".into(),actor:Some(agent.into()),
                fields:serde_json::from_value(json!({"from":agent,"to":person,"status":"sent","content":"Bounded body","thread_id":thread})).unwrap(),evidence:vec![],expected_subject:None,idempotency_key:Some(format!("cost-{n}"))},PersonAuthority::TrustedLocal,None).unwrap().0;
            assert_eq!(
                receipt.accepted_at_unix_ms, time,
                "fixture must use actual distinct or tied signed times"
            );
        };
        let measure = |anchor: &str| {
            let connection = store.readers.get();
            let steps = Arc::new(AtomicUsize::new(0));
            let count = steps.clone();
            WINDOW_BODY_FETCHES.with(|count| count.set(0));
            connection.progress_handler(
                1,
                Some(move || {
                    count.fetch_add(1, Ordering::Relaxed);
                    false
                }),
            );
            let page = read(
                &connection,
                person,
                thread,
                None,
                Some(anchor),
                1,
                current_index(&connection).unwrap(),
            )
            .unwrap();
            connection.progress_handler(0, None::<fn() -> bool>);
            assert_eq!(page.items.len(), 1);
            (
                steps.load(Ordering::Relaxed),
                WINDOW_BODY_FETCHES.with(|count| count.get()),
            )
        };
        for n in 0..64 {
            send(n, base + u128::from(n) * 10);
        }
        let before = measure("message/cost-0");
        for n in 64..512 {
            send(n, base + u128::from(n) * 10);
        }
        let after = measure("message/cost-0");
        eprintln!("varied-time seek: VM/body-fetches {before:?} -> {after:?}");
        assert!(
            after.0 <= before.0 * 2 + 500,
            "varied-time seek VM steps {before:?} -> {after:?}"
        );
        assert!(
            before.1 > 0 && after.1 > 0,
            "body-fetch trace must be active"
        );
        assert!(
            before.1 <= 8 && after.1 <= 8,
            "bounded bodies plus creator/anchor point reads"
        );
        let tied = base + 10_000;
        for n in 1024..1088 {
            send(n, tied);
        }
        let ties_before = measure("message/cost-1024");
        for n in 1088..1536 {
            send(n, tied);
        }
        let ties_after = measure("message/cost-1024");
        eprintln!(
            "equal-time canonical header bucket: VM/body-fetches {ties_before:?} -> {ties_after:?}"
        );
        // Existing canonical tie keys live in batches/claim_positions. Header-only
        // bucket sorting can grow; no broad canonical projection is invented here.
        assert!(
            ties_before.1 <= 8 && ties_after.1 <= 8,
            "tied buckets must not hydrate history bodies"
        );
        assert_eq!(ties_before.1, ties_after.1);
    }
}
