use super::*;

impl Store {
    pub fn messages(
        &self,
        recipient: Option<&str>,
        include_closed: bool,
    ) -> Result<Vec<MessageView>> {
        self.messages_through(recipient, include_closed, self.index()?)
    }

    /// A pinned reader must page through its SQLite index, which can lead the process atomic
    /// between COMMIT and publication of the committed index.
    pub fn messages_through(
        &self,
        recipient: Option<&str>,
        include_closed: bool,
        through: u64,
    ) -> Result<Vec<MessageView>> {
        self.messages_through_inner(recipient, include_closed, through, false)
    }

    pub fn messages_through_inner(
        &self,
        recipient: Option<&str>,
        include_closed: bool,
        through: u64,
        delivery: bool,
    ) -> Result<Vec<MessageView>> {
        smallclaims::touched::note_read(|| match recipient {
            Some(recipient) => format!("mailbox:{recipient}"),
            None => "kind:message.sent".to_owned(),
        });
        let mut after = None;
        let mut all = Vec::new();
        loop {
            let (items, next) =
                self.messages_page_inner(recipient, include_closed, after, through, 200, delivery)?;
            all.extend(items);
            match next {
                Some(cursor) => after = Some(cursor),
                None => {
                    sort_messages_canonically(&self.readers.get(), &mut all)?;
                    // A message's lifecycle (staged, delivered, read, closed) is written on the
                    // message, not the mailbox.
                    for message in &all {
                        smallclaims::touched::note_read(|| message.subject.clone());
                    }
                    return Ok(all);
                }
            }
        }
    }

    pub fn message(&self, subject: &str) -> Result<Option<MessageView>> {
        let connection = self.readers.get();
        let created_index = connection.query_row(
            "SELECT MIN(store_index) FROM claims WHERE subject=?1 AND subject LIKE 'message/%'",
            [subject],
            |row| row.get::<_, Option<u64>>(0),
        )?;
        created_index
            .map(|index| self.message_view_cached(&connection, subject, index))
            .transpose()
    }

    pub fn message_view_cached(
        &self,
        connection: &Connection,
        subject: &str,
        created_index: u64,
    ) -> Result<MessageView> {
        // A message view depends only on its immutable subject claims and the selected
        // desired row. Unrelated graph writes must not invalidate every native mailbox.
        let mut statement = connection.prepare_cached(
            "SELECT (SELECT COALESCE(MAX(store_index), 0) FROM claims WHERE subject=?1),
                    (SELECT claim_id FROM desired WHERE subject=?1)",
        )?;
        let (latest_claim_index, desired_claim_id) = statement.query_row(/* sqlite-large-value: allow-for-now lv-hc58709d7ae65 owner=fractal_store::store::delivery TODO=trace-statement-and-value-representation */ [subject], |row| {
            Ok((row.get::<_, u64>(0)?, row.get::<_, Option<String>>(1)?))
        })?;
        drop(statement);
        self.message_view_cached_at_version(
            connection,
            subject,
            created_index,
            message_versions::Version {
                latest_claim_index,
                desired_claim_id,
            },
        )
    }

    pub fn message_view_cached_at_version(
        &self,
        connection: &Connection,
        subject: &str,
        created_index: u64,
        version: message_versions::Version,
    ) -> Result<MessageView> {
        let message_versions::Version {
            latest_claim_index,
            desired_claim_id,
        } = version;
        if let Some(view) = self
            .smalltalk
            .message_cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(subject)
            .filter(|entry| {
                entry.latest_claim_index == latest_claim_index
                    && entry.desired_claim_id == desired_claim_id
                    && entry.view.created_index == created_index
            })
            .map(|entry| entry.view.clone())
        {
            return Ok(view);
        }
        let view = message_view_tx(connection, subject, created_index)?;
        let mut cache = self
            .smalltalk
            .message_cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if cache.len() >= MESSAGE_CACHE_LIMIT && !cache.contains_key(subject) {
            cache.clear();
        }
        cache.insert(
            subject.to_owned(),
            MessageCacheEntry {
                latest_claim_index,
                desired_claim_id,
                view: view.clone(),
            },
        );
        Ok(view)
    }

    pub fn messages_page_inner(
        &self,
        recipient: Option<&str>,
        include_closed: bool,
        after: Option<u64>,
        through: u64,
        limit: usize,
        delivery: bool,
    ) -> Result<(Vec<MessageView>, Option<u64>)> {
        let connection = self.readers.get();
        let recipient = recipient.map(|recipient|
            super::identities::resolve_reference_conn(&connection, &normalize_message_party(recipient))
        ).transpose()?;
        // Native harnesses repeatedly ask for their complete durable mailbox so
        // they can reconcile delivery receipts. Start at the indexed recipient
        // claims, not every lifecycle claim for every message in the fleet.
        let fast_recipient = recipient.as_deref();
        let bare_recipient = fast_recipient.map(message_bare_recipient);
        if let (Some(recipient), Some(bare_recipient)) = (fast_recipient, bare_recipient) {
            let mut statement = connection.prepare(/* sqlite-large-value: allow-for-now lv-h06b5186b1e2e owner=fractal_store::store::delivery TODO=trace-statement-and-value-representation */ if delivery {
                concat!("WITH candidates(subject) AS (
                     SELECT subject FROM claims INDEXED BY claims_message_to_index
                     WHERE kind='message.sent'
                       AND ", message_sent_recipient_match!(), "
                     UNION
                     SELECT desired.subject FROM desired,
                            json_each(desired.body, '$.children') child
                     WHERE desired.kind='message'
                       AND ", message_desired_recipient_match!(), "
                 ), created AS (
                     SELECT subject,
                            (SELECT MIN(store_index) FROM claims
                             WHERE claims.subject=candidates.subject) created_index
                     FROM candidates
                 )
                 , ranked AS (
                     SELECT created.*, COALESCE(meta.held,0) held,
                            ROW_NUMBER() OVER (PARTITION BY COALESCE(meta.held,0) ORDER BY created_index DESC,created.subject DESC) held_rank,
                            SUM(COALESCE(meta.held,0)) OVER () held_total
                     FROM created LEFT JOIN coordination_sends meta ON meta.subject=created.subject
                     WHERE created_index<=?4
                 )
                 SELECT subject, created_index, CASE WHEN ?7 THEN MAX(held_total-8,0) ELSE 0 END FROM ranked
                 WHERE created_index>?3 AND created_index<=?4
                   AND (NOT ?7 OR held=0 OR held_rank<=8)
                   AND (?6 OR NOT EXISTS (
                     SELECT 1 FROM claims closed
                     WHERE closed.subject=ranked.subject AND closed.kind='message.closed'
                   ))
                 ORDER BY created_index, subject LIMIT ?5")
            } else {
                concat!("WITH candidates(subject) AS (
                     SELECT subject FROM claims INDEXED BY claims_message_to_index
                     WHERE kind='message.sent'
                       AND ", message_sent_recipient_match!(), "
                     UNION
                     SELECT desired.subject FROM desired,
                            json_each(desired.body, '$.children') child
                     WHERE desired.kind='message'
                       AND ", message_desired_recipient_match!(), "
                 ), created AS (
                     SELECT subject,
                            (SELECT MIN(store_index) FROM claims
                             WHERE claims.subject=candidates.subject) created_index
                     FROM candidates
                 )
                 SELECT subject, created_index, 0 FROM created
                 WHERE (?7 OR NOT ?7) AND created_index>?3 AND created_index<=?4
                   AND (?6 OR NOT EXISTS (
                     SELECT 1 FROM claims closed
                     WHERE closed.subject=created.subject AND closed.kind='message.closed'
                   ))
                 ORDER BY created_index, subject LIMIT ?5")
            })?;
            let mut subjects = statement
                .query_map(
                    params![
                        recipient,
                        bare_recipient,
                        after.unwrap_or(0),
                        through,
                        limit.saturating_add(1),
                        include_closed,
                        delivery,
                    ],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, u64>(1)?,
                            row.get::<_, usize>(2)?,
                        ))
                    },
                )?
                .collect::<Result<Vec<_>, _>>()?;
            let has_more = subjects.len() > limit;
            subjects.truncate(limit);
            let next_after = has_more
                .then(|| subjects.last().map(|(_, index, _)| *index))
                .flatten();
            let mut output = Vec::new();
            for (subject, created_index, remaining) in subjects {
                let mut message = self.message_view_cached(&connection, &subject, created_index)?;
                if message.to == recipient && (include_closed || message.status != "closed") {
                    if remaining > 0 && crate::silent::waits_for_turn(&message) {
                        message
                            .tags
                            .push(format!("{}{remaining}", crate::silent::REMAINING_PREFIX));
                    }
                    output.push(message);
                }
            }
            return Ok((output, next_after));
        }
        // Every recipient took the indexed path above. Without one, the message index lists the
        // open messages, or all of them, without reading their lifecycle claims.
        let mut statement = connection.prepare_cached(
            "SELECT subject, created_index FROM message_index
             WHERE (?1 OR closed=0) AND ?2 IS NULL
               AND created_index>?3 AND created_index<=?4
             ORDER BY created_index, subject LIMIT ?5",
        )?;
        let mut subjects = statement
            .query_map(
                params![
                    include_closed,
                    recipient.as_deref(),
                    after.unwrap_or(0),
                    through,
                    limit.saturating_add(1)
                ],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?)),
            )?
            .collect::<Result<Vec<_>, _>>()?;
        let has_more = subjects.len() > limit;
        subjects.truncate(limit);
        let next_after = has_more
            .then(|| subjects.last().map(|(_, index)| *index))
            .flatten();
        let mut output = Vec::new();
        for (subject, created_index) in subjects {
            let message = self.message_view_cached(&connection, &subject, created_index)?;
            if recipient
                .as_deref()
                .is_none_or(|recipient| recipient == message.to)
                && (include_closed || message.status != "closed")
            {
                output.push(message);
            }
        }
        Ok((output, next_after))
    }

    pub fn subscription_fault_items(&self, as_of: u128) -> Result<Vec<AttentionItemView>> {
        let mut items = Vec::new();
        {
            let connection = self.readers.get();
            let mut statement = connection.prepare(
                /* sqlite-large-value: allow-for-now lv-haaba804a60b4 owner=fractal_store::store::delivery TODO=trace-statement-and-value-representation */ &canonical_sql("SELECT id, store_index, batch_id, subject, kind, origin, actor, body,
                        predecessors, accepted_at_unix_ms
                 FROM claims WHERE kind='subscription.mission-failed' ORDER BY CANONICAL_ASC(claims)"),
            )?;
            let failures = statement
                .query_map([], claim_from_row)?
                .collect::<Result<Vec<_>, _>>()?;
            for failure in failures {
                if failure.accepted_at_unix_ms > as_of
                    || !person_work::declaration_live(&connection, &failure.subject)?
                    || !subscription_failure_is_current_tx(&connection, &failure)?
                {
                    continue;
                }
                let fields = failure.body.get("fields").unwrap_or(&failure.body);
                let request = fields
                    .get("request")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let requester: Option<String> = connection
                    .query_row(
                        "SELECT json_extract(body, '$.fields.requester') FROM claims WHERE id=?1",
                        [request],
                        |row| row.get(0),
                    )
                    .optional()?
                    .flatten();
                let reviewer = requester
                    .filter(|value| value.starts_with("person/"))
                    .unwrap_or_else(|| "person/operator".into());
                let code = fields
                    .get("code")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let reason = fields
                    .get("reason")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                items.push(AttentionItemView {
                    episode: failure.id.clone(),
                    priority: "high".into(),
                    kind: "fault".into(),
                    review_mode: None,
                    subject: failure.subject.clone(),
                    person: reviewer,
                    // st's subscription observer raised it.
                    requester_id: Some(
                        failure
                            .actor
                            .clone()
                            .unwrap_or_else(|| "agent/st3/reconciler".into()),
                    ),
                    launch_id: None,
                    variant_id: None,
                    message_id: None, prompt_call: None, answers: Vec::new(), conversation: None,
                    title: "Subscription mission failed".into(),
                    detail: format!("{code}: {reason}"),
                    mission: None,
                    mission_run: None,
                    step: None,
                    targets: vec![failure.subject.clone()],
                    requested_at_unix_ms: failure.accepted_at_unix_ms,
                    actions: vec![attention_action(
                        "inspect subscription",
                        &["st", "subject", &failure.subject],
                    )],
                    request: None,
                });
            }
        }
        Ok(items)
    }

}

const MESSAGE_CACHE_LIMIT: usize = 4096;

/// A failed subscription request stays a fault while its subscription runs. A failed request is
/// never retried, so only a stop ends it on its own: a stopped subscription starts nothing more.
fn subscription_failure_is_current_tx(
    connection: &Connection,
    failure: &ClaimRecord,
) -> Result<bool> {
    Ok(declaration_stopped_tx(connection, &failure.subject)?.is_none())
}
