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

/// Count admitted messages and determine whether a person still has an incoming reply to read.
/// The ordinary recipient's message.read/closed claims are durable and apply in both views.
pub(super) fn message_summary(
    connection: &Connection,
    person: &str,
    agent: &str,
    thread: &str,
    creation: &ClaimRecord,
) -> Result<(u64, bool), FractalError> {
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
        )
        SELECT count(*), coalesce(max(recipient_id=?4 AND NOT EXISTS (
            SELECT 1 FROM claims INDEXED BY claims_subject_kind_index
            WHERE claims.subject=participants.subject
              AND ((claims.kind='message.read' AND claims.actor=?4) OR claims.kind='message.closed')
              AND {UNREPAIRED_CLAIM}
        )), 0)
        FROM participants
        WHERE actor_id=sender_id AND ((sender_id=?4 AND recipient_id=?5) OR (sender_id=?5 AND recipient_id=?4))",
    ));
    connection
        .query_row(
            &sql,
            params![thread, first, creation.batch_id, person, agent],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(internal)
}

const ACTIVE_MESSAGE: &str = "EXISTS (SELECT 1 FROM operations
  WHERE operations.id=json_extract(claims.body, '$._operation.id')
    AND operations.canonical_claim_id=claims.id AND operations.state='active')";

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
    let selection = format!("AND claims.store_index <= ?4 AND {ACTIVE_MESSAGE}");
    let after = smallclaims::store::canonical::after_sql("anchor", "claims");
    let cursor_range = format!(
        "AND (?5 IS NULL OR (length(claims.accepted_at_unix_ms), claims.accepted_at_unix_ms) <= (length(?6), ?6))
         AND (?5 IS NULL OR EXISTS (SELECT 1 FROM claims anchor WHERE anchor.id=?5 AND {after}))"
    );
    let sql = canonical_sql(&format!(
        "WITH explicit AS (
            SELECT {CLAIM_COLUMNS}
            FROM claims INDEXED BY claims_thread_message_accepted_index
            WHERE claims.kind='message.sent' AND json_extract(claims.body, '$.fields.thread_id')=?1
              AND json_extract(claims.body, '$.fields.thread_id') IS NOT NULL
              {selection} AND {UNREPAIRED_CLAIM} {cursor_range}
            ORDER BY CANONICAL_DESC(claims) LIMIT ?7
         ), legacy AS (
            SELECT {CLAIM_COLUMNS} FROM claims
            WHERE claims.subject=?2 AND claims.batch_id=?3 AND claims.kind='message.sent'
              AND json_extract(claims.body, '$.fields.thread_id') IS NULL
              {selection} AND {UNREPAIRED_CLAIM} {cursor_range}
         ), lifecycle AS (
            SELECT {CLAIM_COLUMNS} FROM claims INDEXED BY claims_subject_accepted_index
            WHERE claims.subject=?1 AND claims.kind IN ('thread.title-set', 'thread.state-set')
              {selection} AND {UNREPAIRED_CLAIM} {cursor_range}
            ORDER BY CANONICAL_DESC(claims) LIMIT ?7
         )
         SELECT {CLAIM_COLUMNS}
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
            claim_from_row,
        )
        .map_err(internal)?;
    let mut items = Vec::new();
    let mut last = None;
    let mut has_more = false;
    for row in rows {
        let claim = row.map_err(internal)?;
        if items.len() == limit {
            has_more = true;
            break;
        }
        if lifecycle(&claim, thread) {
            last = Some(claim.id.clone());
            items.push(TimelineItem::Lifecycle(
                Event::from_claim(claim).ok_or_else(unavailable)?,
            ));
            continue;
        }
        let actor = message_actor(connection, &claim, &person, &agent)?;
        let fields = &claim.body["fields"];
        let content = fields["content"]
            .as_str()
            .filter(|content| !content.starts_with("doc/") && content.len() <= 8 * 1024)
            .ok_or_else(unavailable)?;
        let quoted = fields["thread_quote_id"].as_str();
        if let Some(quoted) = quoted {
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
            message_actor(connection, &parent, &person, &agent)?;
        }
        last = Some(claim.id);
        items.push(TimelineItem::Message(ThreadMessage {
            kind: "message",
            id: claim.subject,
            actor_id: actor,
            accepted_at_unix_ms: claim.accepted_at_unix_ms,
            content: content.into(),
            thread_quote_id: quoted.map(str::to_owned),
        }));
    }
    let next_cursor = last
        .filter(|_| has_more)
        .map(|last| format!("{scope}:{through}:{last}"));
    Ok(TimelinePage { items, next_cursor })
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
