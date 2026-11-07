//! Exact thread message reads over the normal message store.

use super::*;

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
}

/// Newest-first page over admitted messages and lifecycle events. The caller authenticates the person.
pub fn mixed_timeline(
    store: &Store,
    person: &str,
    thread: &str,
    cursor: Option<&str>,
    limit: usize,
) -> Result<TimelinePage, FractalError> {
    store
        .graph
        .read_snapshot(|index| {
            let connection = store.readers.get();
            Ok(read(&connection, person, thread, cursor, limit, index))
        })
        .map_err(internal)?
}

fn read(
    connection: &Connection,
    person: &str,
    thread: &str,
    cursor: Option<&str>,
    limit: usize,
    index: u64,
) -> Result<TimelinePage, FractalError> {
    let creation = owned_creation(connection, thread, person, false)?.ok_or_else(unavailable)?;
    let agent = creation.body["fields"]["agent"]
        .as_str()
        .ok_or_else(unavailable)?;
    let person = identities::resolve_reference_conn(connection, person)?;
    let agent = identities::resolve_reference_conn(connection, agent)?;
    let first = creation.body["fields"]["first_message"].as_str();
    let scope =
        canonical_hash(&("thread.mixed-timeline.v1", &person, &agent, thread)).map_err(internal)?;
    let invalid_cursor = || {
        FractalError::new(
            "invalid-thread-cursor",
            "the thread message cursor is unavailable",
        )
    };
    let (through, anchor) = match cursor {
        Some(cursor) => {
            let (cursor_scope, suffix) = cursor.split_once(':').ok_or_else(invalid_cursor)?;
            let (through, anchor) = suffix.split_once(':').ok_or_else(invalid_cursor)?;
            let through = through.parse::<u64>().map_err(|_| invalid_cursor())?;
            if cursor_scope != scope
                || through > index
                || through < creation.store_index
                || anchor.len() > 512
            {
                return Err(invalid_cursor());
            }
            let anchor = claim_by_id_tx(connection, anchor)
                .map_err(internal)?
                .filter(|claim| {
                    claim.store_index <= through
                        && (lifecycle(claim, thread)
                            || member(claim, thread, first, &creation.batch_id, through))
                })
                .ok_or_else(invalid_cursor)?;
            (through, Some(anchor))
        }
        None => (index, None),
    };

    // The explicit membership arm seeks the thread's own message index. The legacy
    // first-message arm reads exactly the atomic start batch and skips a duplicate.
    let selection = "AND claims.store_index <= ?4
        AND EXISTS (SELECT 1 FROM operations
          WHERE operations.id=json_extract(claims.body, '$._operation.id')
            AND operations.canonical_claim_id=claims.id AND operations.state='active')";
    let after = smallclaims::store::canonical::after_sql("anchor", "claims");
    let cursor_range = format!(
        "AND (?5 IS NULL OR (length(claims.accepted_at_unix_ms), claims.accepted_at_unix_ms) <= (length(?6), ?6))
         AND (?5 IS NULL OR EXISTS (SELECT 1 FROM claims anchor WHERE anchor.id=?5 AND {after}))"
    );
    let sql = canonical_sql(&format!(
        "WITH explicit AS (
            SELECT {CLAIM_COLUMNS}, 'active' AS operation_state
            FROM claims INDEXED BY claims_thread_message_accepted_index
            WHERE claims.kind='message.sent' AND json_extract(claims.body, '$.fields.thread_id')=?1
              AND json_extract(claims.body, '$.fields.thread_id') IS NOT NULL
              {selection} AND {UNREPAIRED_CLAIM} {cursor_range}
            ORDER BY CANONICAL_DESC(claims) LIMIT ?7
         ), legacy AS (
            SELECT {CLAIM_COLUMNS}, 'active' AS operation_state FROM claims
            WHERE claims.subject=?2 AND claims.batch_id=?3 AND claims.kind='message.sent'
              AND json_extract(claims.body, '$.fields.thread_id') IS NULL
              {selection} AND {UNREPAIRED_CLAIM} {cursor_range}
         ), lifecycle AS (
            SELECT {CLAIM_COLUMNS}, operations.state AS operation_state
            FROM claims INDEXED BY claims_subject_accepted_index
            JOIN operations ON operations.id=json_extract(claims.body, '$._operation.id')
            WHERE claims.subject=?1 AND claims.kind IN ('thread.title-set', 'thread.state-set')
              AND claims.store_index <= ?4
              AND (operations.state='conflict' OR operations.canonical_claim_id=claims.id)
              AND {UNREPAIRED_CLAIM} {cursor_range}
            ORDER BY CANONICAL_DESC(claims) LIMIT ?7
         )
         SELECT {CLAIM_COLUMNS}, operation_state
         FROM (SELECT * FROM explicit UNION ALL SELECT * FROM legacy UNION ALL SELECT * FROM lifecycle) claims
         ORDER BY CANONICAL_DESC(claims) LIMIT ?7"
    ));
    let limit = limit.clamp(1, 50);
    let mut statement = connection.prepare_cached(&sql).map_err(internal)?;
    let rows = statement
        .query_map(
            params![
                thread,
                first,
                creation.batch_id,
                through,
                anchor.as_ref().map(|claim| &claim.id),
                anchor
                    .as_ref()
                    .map(|claim| claim.accepted_at_unix_ms.to_string()),
                limit + 1,
            ],
            |row| Ok((claim_from_row(row)?, row.get::<_, String>(10)?)),
        )
        .map_err(internal)?;
    let mut items = Vec::new();
    let mut last = None;
    let mut has_more = false;
    for row in rows {
        let (claim, operation_state) = row.map_err(internal)?;
        if items.len() == limit {
            has_more = true;
            break;
        }
        if operation_state == "conflict" {
            return Err(FractalError::new(
                "idempotency-conflict",
                "a thread metadata operation has conflicting requests",
            ));
        }
        if lifecycle(&claim, thread) {
            last = Some(claim.id.clone());
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
        last = Some(claim.id.clone());
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
    let next_cursor = last
        .filter(|_| has_more)
        .map(|last| format!("{scope}:{through}:{last}"));
    Ok(TimelinePage { items, next_cursor })
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
                    .query_row(&sql, [claim_id], claim_from_row)
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
        .query_row(&sql, [quoted], claim_from_row)
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
