//! The part of `store::delivery` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use crate::message_desired_recipient_match;
use crate::message_sent_recipient_match;
use super::*;
pub use fractal_store::store::delivery::*;

fn subscription_request_view(
    request: &ClaimRecord,
    decisions: &[ClaimRecord],
) -> SubscriptionRequestView {
    let fields = request.body.get("fields").unwrap_or(&request.body);
    let field = |name: &str| {
        fields
            .get(name)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let decision = |kind: &str| {
        decisions.iter().find(|claim| {
            claim.kind == kind
                && claim
                    .body
                    .pointer("/fields/request")
                    .and_then(Value::as_str)
                    == Some(request.id.as_str())
        })
    };
    let started = decision("subscription.mission-started");
    let status = if started.is_some() {
        "started"
    } else if decision("subscription.mission-request-cancelled").is_some() {
        "cancelled"
    } else if decision("subscription.mission-failed").is_some() {
        "failed"
    } else if fields.get("held").and_then(Value::as_bool) == Some(true)
        && decision("subscription.mission-request-released").is_none()
    {
        "held"
    } else {
        "pending"
    };
    SubscriptionRequestView {
        request: request.id.clone(),
        subscription: request.subject.clone(),
        resource: field("resource"),
        mission: field("mission"),
        status: status.into(),
        mission_run: started
            .and_then(|claim| claim.body.pointer("/fields/mission_run"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        requested_at_unix_ms: request.accepted_at_unix_ms,
    }
}

/// The seat channel's current owner, and whether `fence`'s binding still exists.
fn mailbox_owner_key(
    connection: &Connection,
    fence: &crate::mailbox::Fence,
) -> Result<Option<(String, i64, bool)>, St3Error> {
    connection
        .prepare_cached(
            "SELECT owner.incarnation, owner.epoch,
                    EXISTS(SELECT 1 FROM local_mailbox_bindings WHERE token=?3)
             FROM local_mailbox_owners owner WHERE owner.subject=?1 AND owner.component=?2",
        )
        .map_err(internal)?
        .query_row(
            /* sqlite-large-value: allow-for-now lv-h25f8cba63deb owner=fractal_delivery::store::delivery TODO=trace-statement-and-value-representation */ params![fence.subject, fence.component, fence.token],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(internal)
}

pub trait DeliveryStore {
    fn create_subscription_mission_run(&self, request: &MissionRunRequest,
            parent: Option<&MissionRunView>, subscription: &str, resource: &str, discovery: &str,
        ) -> Result<MissionRunView, St3Error>;
    fn next_subscription_mission_retry_deadline(&self) -> Result<Option<u128>>;
    fn populate_work_wake_for_reconcile(
            &self,
            view: &mut StepRunView,
            snapshot_unix_ms: u128,
        ) -> Result<()>;
    /// The API has verified this device signature. Stage, admit and consume it in the
    /// same writer savepoint as the message; retries keep the first committed signature.
    fn append_signed_message(
            &self,
            input: &ClaimInput,
            signature: &smallclaims::principal::ClaimSignature,
        ) -> Result<(ClaimRecord, bool), St3Error>;
    #[cfg(any(test, feature = "test-support"))]
    fn append_mailbox_receipt(
            &self,
            input: &ClaimInput,
            fence: &crate::mailbox::Fence,
        ) -> Result<ClaimRecord, St3Error>;
    /// A fenced receipt, and whether it appended a claim rather than repeating or settling one.
    fn append_mailbox_receipt_outcome(
            &self,
            input: &ClaimInput,
            fence: &crate::mailbox::Fence,
        ) -> Result<(ClaimRecord, bool), St3Error>;
    /// Called only inside the native lease/authority guard. Metadata commits with
    /// the first new read receipt; replay/settlement never adds or changes a binding.
    fn append_native_mailbox_receipt_outcome(
            &self,
            input: &ClaimInput,
            fence: &crate::mailbox::Fence,
            native: Option<&crate::conversation_delivery::NativeReceipt>,
        ) -> Result<(ClaimRecord, bool), St3Error>;
    /// Search refreshes only a person's private message texts, using the same endpoint
    /// indexes as the mailbox. Other fleet writes do not invalidate this source.
    fn conversation_search_mail_stamp(&self, person: &str, through: u64) -> Result<String>;
    fn conversation_search_timeline_stamp(&self, agent: &str, incarnation: &str, through: u64) -> Result<String>;
    fn conversation_search_messages(&self, person: &str, through: u64) -> Result<Vec<MessageView>>;
    fn messages_for_delivery_through(
            &self,
            recipient: &str,
            through: u64,
        ) -> Result<Vec<MessageView>>;
    /// The reconciler needs historical work-wake attempts, including closed
    /// ones, but not an agent's unrelated lifetime of conversation messages.
    /// First select a superset from the indexed recipient's sent claims; the
    /// final tag check keeps this equivalent to filtering the normal mailbox.
    fn work_wake_messages_for_reconcile(&self, agent: &str) -> Result<Vec<MessageView>>;
    /// Provider reply watches read only new native message events through the existing
    /// sender/store-index range. They never rebuild a recipient's historical mailbox.
    fn adapter_delivery_page(
            &self,
            sender: &str,
            recipients: &[String],
            after: u64,
            through: u64,
            limit: usize,
        ) -> Result<(Vec<MessageView>, Option<u64>)>;
    fn messages_page(
            &self,
            recipient: Option<&str>,
            include_closed: bool,
            after: Option<u64>,
            through: u64,
            limit: usize,
        ) -> Result<(Vec<MessageView>, Option<u64>)>;
    /// Filter the authoritative message view before applying the page size. The cursor is
    /// a local creation-order position, not a replicated incremental-change watermark.
    #[allow(clippy::too_many_arguments)]
    fn messages_page_filtered(
            &self,
            recipient: Option<&str>,
            sender: Option<&str>,
            subject: Option<&str>,
            subject_prefix: Option<&str>,
            include_closed: bool,
            after: Option<u64>,
            through: u64,
            limit: usize,
        ) -> Result<(Vec<MessageView>, Option<u64>)>;
    fn messages_page_for_delivery(
            &self,
            recipient: &str,
            after: Option<u64>,
            through: u64,
            limit: usize,
        ) -> Result<(Vec<MessageView>, Option<u64>)>;
    fn operational_messages(
            &self,
            recipient: Option<&str>,
            include_history: bool,
        ) -> Result<Vec<MessageView>>;
    /// Every mission request that a subscription recorded, oldest first, with its disposition.
    fn subscription_requests(
            &self,
            subscription: &str,
        ) -> Result<Vec<SubscriptionRequestView>>;
    /// Release a held subscription request, or cancel a pending or held one, as a person or an
    /// agent.
    fn decide_subscription_request(
            &self,
            request: &str,
            decision: &str,
            input: &SubscriptionRequestDecision,
        ) -> Result<SubscriptionRequestView, St3Error>;
    fn pending_subscription_mission_requests(&self, subject: &str) -> Result<Vec<ClaimRecord>>;
    /// Read only one pending request's retry history. The request index covers the count and
    /// locates its newest deadline without materializing every deferral on the subscription.
    fn subscription_mission_deferral(
            &self,
            subject: &str,
            request: &str,
        ) -> Result<Option<(u128, u32)>>;
    fn record_subscription_mission_deferral(
            &self,
            subject: &str,
            request: &str,
            deadline: u128,
            attempts: u32,
        ) -> Result<()>;
    /// Local subscription ownership survives a daemon outage; an older channel cannot retake it.
    fn bind_mailbox(
            &self,
            request: &crate::mailbox::Fence,
        ) -> Result<crate::mailbox::Fence, St3Error>;
    fn bind_mailbox_with_lease(
            &self,
            request: &crate::mailbox::Fence,
            authority: Option<&crate::mailbox::Authority>,
        ) -> Result<crate::mailbox::Fence, St3Error>;
    /// Recheck captured physical custody under the writer, before any capability mutation.
    fn bind_mailbox_with_lease_checked(
            &self,
            request: &crate::mailbox::Fence,
            authority: Option<&crate::mailbox::Authority>,
            validate: &dyn Fn() -> Result<(), St3Error>,
        ) -> Result<crate::mailbox::Fence, St3Error>;
    /// Explicit argv-only transports retain their legacy protocol, never provider custody.
    fn bind_argv_mailbox_checked(
            &self,
            request: &crate::mailbox::Fence,
            member: &crate::model::MemberSpec,
            validate: &dyn Fn() -> Result<(), St3Error>,
        ) -> Result<crate::mailbox::Fence, St3Error>;
    fn bind_mailbox_with_admission_checked(
            &self,
            request: &crate::mailbox::Fence,
            authority: Option<&crate::mailbox::Authority>,
            argv_member: Option<&crate::model::MemberSpec>,
            validate: &dyn Fn() -> Result<(), St3Error>,
        ) -> Result<crate::mailbox::Fence, St3Error>;
    fn check_mailbox(&self, fence: &crate::mailbox::Fence) -> Result<(), St3Error>;
    /// Durable cursors and the selected declaration/owner in one SQLite read snapshot.
    /// Streams advance this watermark only after capturing their corresponding delta.
    fn mailbox_watermark(
            &self,
            fence: &crate::mailbox::Fence,
        ) -> Result<MailboxWatermark, St3Error>;
    /// Whether anything a mailbox stream's snapshot reads changed after `mark`: a claim or local
    /// observation of the seat (its declaration, runtime and harness), its channel ownership, a
    /// message sent to it or newly declared, or a claim of a message in its last snapshot. The
    /// local dispatcher targets dependent streams; this durable check still filters coalesced
    /// wakes, without trusting the ephemeral routing index for correctness.
    #[cfg(any(test, feature = "test-support"))]
    fn mailbox_changed_since(
            &self,
            fence: &crate::mailbox::Fence,
            mark: &MailboxWatermark,
            messages: &[String],
        ) -> Result<bool, St3Error>;
    /// True only when at least one new claim was projected and every new claim
    /// is a usage sample, a lease renewal, or a lifecycle claim of a message that is not a
    /// work wake (the reconciler reads no other message). Other projection changes must
    /// still run the reconciler, including a recovered stale projection.
    fn claims_since_only_quiet_notifications(&self, after_index: u64) -> Result<bool>;
}

impl DeliveryStore for Store {

    fn create_subscription_mission_run(&self, request: &MissionRunRequest,
            parent: Option<&MissionRunView>, subscription: &str, resource: &str, discovery: &str,
        ) -> Result<MissionRunView, St3Error> {
        let child = parent.map(|parent| ChildMissionContext {
            root_revision: parent.root_revision.clone(),
            root_run_id: parent.root_mission_run.trim_start_matches("mission-run/").into(),
            parent_step_run: normalize_step_run(subscription), default_selector: None,
        });
        self.create_mission_run_inner(request, child, None, Some((resource, discovery)))
    }

    fn next_subscription_mission_retry_deadline(&self) -> Result<Option<u128>> {
        let connection = self.readers.get();
        let now = now_ms() as i64;
        let legacy: Option<i64> = connection.query_row(
            "SELECT MIN(CAST(json_extract(body, '$.fields.not_before_unix_ms') AS INTEGER))
             FROM claims INDEXED BY claims_subscription_deferred_deadline_index
             WHERE kind='subscription.mission-deferred'
               AND CAST(json_extract(body, '$.fields.not_before_unix_ms') AS INTEGER)>?1",
            [now],
            |row| row.get(0),
        )?;
        let local: Option<i64> = connection.query_row(
            "SELECT MIN(not_before_unix_ms) FROM local_subscription_mission_deferrals
             WHERE not_before_unix_ms>?1",
            [now],
            |row| row.get(0),
        )?;
        Ok(local
            .into_iter()
            .chain(legacy)
            .min()
            .map(|value| value as u128))
    }

    fn populate_work_wake_for_reconcile(
            &self,
            view: &mut StepRunView,
            snapshot_unix_ms: u128,
        ) -> Result<()> {
        let connection = self.readers.get();
        enrich_step_wake_at(&connection, view, snapshot_unix_ms)?;
        Ok(())
    }

    fn append_signed_message(
            &self,
            input: &ClaimInput,
            signature: &smallclaims::principal::ClaimSignature,
        ) -> Result<(ClaimRecord, bool), St3Error> {
        assistant_write::refuse_unverified(&input.kind, input.actor.as_deref(),
            input.fields.get("from").and_then(Value::as_str),
            input.fields.contains_key("sender_class") || input.fields.contains_key("assistant_grant_id"))?;
        if input.kind != "message.sent" || input.actor.is_none() {
            return Err(St3Error::new("invalid-signature", "a signed message names its actor"));
        }
        self.graph.ensure_principal_key(input.actor.as_deref().unwrap())?;
        let signature = serde_json::to_string(signature).map_err(internal)?;
        append_claim_with_commit_context(&self.graph, input, None, None, None, None,
            ClaimCommitContext { signature: Some(&signature), ..Default::default() })
    }

    #[cfg(any(test, feature = "test-support"))]
    fn append_mailbox_receipt(
            &self,
            input: &ClaimInput,
            fence: &crate::mailbox::Fence,
        ) -> Result<ClaimRecord, St3Error> {
        self.append_mailbox_receipt_outcome(input, fence)
            .map(|(claim, _)| claim)
    }

    fn append_mailbox_receipt_outcome(
            &self,
            input: &ClaimInput,
            fence: &crate::mailbox::Fence,
        ) -> Result<(ClaimRecord, bool), St3Error> {
        self.append_claim_fenced_outcome(input, Some(fence))
    }

    fn append_native_mailbox_receipt_outcome(
            &self,
            input: &ClaimInput,
            fence: &crate::mailbox::Fence,
            native: Option<&crate::conversation_delivery::NativeReceipt>,
        ) -> Result<(ClaimRecord, bool), St3Error> {
        if native.is_some_and(|native| !native.valid() || input.kind != "message.read") {
            return Err(St3Error::new(
                "invalid-native-receipt",
                "native proof needs a read receipt",
            ));
        }
        append_claim_with_commit_context(
            &self.graph,
            input,
            Some(fence),
            None,
            None,
            None,
            ClaimCommitContext {
                native_receipt: native,
                ..Default::default()
            },
        )
    }

    fn conversation_search_mail_stamp(&self, person: &str, through: u64) -> Result<String> {
        let connection = self.readers.get();
        let (count, newest): (u64, u64) = connection.query_row(
            /* sqlite-large-value: allow-for-now lv-hc7848faab778 owner=fractal_delivery::store::delivery TODO=trace-statement-and-value-representation */ "WITH sent AS (
                SELECT store_index FROM claims INDEXED BY claims_message_to_index
                WHERE kind='message.sent' AND json_extract(body,'$.fields.to')=?1 AND store_index<=?2
                UNION SELECT store_index FROM claims INDEXED BY claims_message_from_index
                WHERE kind='message.sent' AND json_extract(body,'$.fields.from')=?1 AND store_index<=?2
            ) SELECT COUNT(*), COALESCE(MAX(store_index),0) FROM sent",
            params![person,through],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        Ok(format!("{count}:{newest}"))
    }

    fn conversation_search_timeline_stamp(&self, agent: &str, incarnation: &str, through: u64) -> Result<String> {
        let connection = self.readers.get();
        let agent = fractal_store::store::identities::resolve_reference_conn(&connection, agent)?;
        let agent = agent.as_str();
        let (local_count, local_newest): (u64,u64) = connection.query_row(
            "SELECT COUNT(*),COALESCE(MAX(id),0) FROM local_observations
             WHERE subject=?1 AND kind='harness.timeline'
               AND json_extract(body,'$.fields.incarnation_id')=?2 AND after_store_index<=?3", params![agent,incarnation,through],
            |row| Ok((row.get(0)?,row.get(1)?)))?;
        let (legacy_count, legacy_newest): (u64,u64) = connection.query_row(
            "SELECT COUNT(*),COALESCE(MAX(store_index),0) FROM claims
             WHERE subject=?1 AND kind='harness.timeline'
               AND json_extract(body,'$.fields.incarnation_id')=?2 AND store_index<=?3", params![agent,incarnation,through],
            |row| Ok((row.get(0)?,row.get(1)?)))?;
        Ok(format!("{local_count}:{local_newest}:{legacy_count}:{legacy_newest}"))
    }

    fn conversation_search_messages(&self, person: &str, through: u64) -> Result<Vec<MessageView>> {
        let connection = self.readers.get();
        let mut statement = connection.prepare(/* sqlite-large-value: allow-for-now lv-hbb8f1fc66365 owner=fractal_delivery::store::delivery TODO=trace-statement-and-value-representation */ &canonical_sql(
            "WITH candidates AS (
                SELECT id FROM claims INDEXED BY claims_message_to_index
                WHERE kind='message.sent' AND json_extract(body,'$.fields.to')=?1 AND store_index<=?2
                UNION SELECT id FROM claims INDEXED BY claims_message_from_index
                WHERE kind='message.sent' AND json_extract(body,'$.fields.from')=?1 AND store_index<=?2
            ), ranked AS (
                SELECT claims.id,claims.batch_id,claims.subject,claims.store_index,claims.accepted_at_unix_ms,
                    MIN(claims.store_index) OVER (PARTITION BY claims.subject) AS created_index,
                    ROW_NUMBER() OVER (PARTITION BY claims.subject ORDER BY CANONICAL_ASC(claims)) AS ordinal
                FROM claims JOIN candidates ON candidates.id=claims.id
            ) SELECT sent.subject,sent.created_index FROM ranked AS sent
              WHERE ordinal=1 ORDER BY CANONICAL_DESC(sent) LIMIT 50001"))?;
        let rows = statement.query_map(params![person,through], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
        })?;
        let mut messages = Vec::new();
        for row in rows {
            let (subject, index) = row?;
            messages.push(self.message_view_cached(&connection, &subject, index)?);
        }
        Ok(messages)
    }

    fn messages_for_delivery_through(
            &self,
            recipient: &str,
            through: u64,
        ) -> Result<Vec<MessageView>> {
        self.messages_through_inner(Some(recipient), false, through, true)
    }

    fn work_wake_messages_for_reconcile(&self, agent: &str) -> Result<Vec<MessageView>> {
        let connection = self.readers.get();
        let recipient = fractal_store::store::identities::resolve_reference_conn(
            &connection, &normalize_message_party(agent),
        )?;
        let bare_recipient = recipient
            .strip_prefix("agent/")
            .filter(|suffix| !suffix.contains('/'))
            .unwrap_or(&recipient);
        let mut statement = connection.prepare_cached(
            "SELECT DISTINCT sent.subject,
                    (SELECT MIN(created.store_index) FROM claims created
                     WHERE created.subject=sent.subject) AS created_index
             FROM claims sent INDEXED BY claims_message_to_index
             WHERE sent.kind='message.sent'
               AND json_extract(sent.body, '$.fields.to') IN (SELECT name FROM identity_names WHERE identity=?1 UNION SELECT ?1 UNION SELECT ?2)
               AND instr(sent.body, 'st3-work:')>0
             ORDER BY created_index, sent.subject",
        )?;
        let subjects = statement
            .query_map(params![recipient, bare_recipient], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        drop(statement);
        let mut messages = Vec::with_capacity(subjects.len());
        for (subject, created_index) in subjects {
            let message = self.message_view_cached(&connection, &subject, created_index)?;
            if message.to == recipient
                && message.tags.iter().any(|tag| tag.starts_with("st3-work:"))
            {
                messages.push(message);
            }
        }
        sort_messages_canonically(&connection, &mut messages)?;
        smallclaims::touched::note_read(|| format!("mailbox:{recipient}"));
        smallclaims::touched::note_read(|| format!("mailbox:{bare_recipient}"));
        for message in &messages {
            smallclaims::touched::note_read(|| message.subject.clone());
        }
        Ok(messages)
    }

    fn adapter_delivery_page(
            &self,
            sender: &str,
            recipients: &[String],
            after: u64,
            through: u64,
            limit: usize,
        ) -> Result<(Vec<MessageView>, Option<u64>)> {
        let connection = self.readers.get();
        let sender = fractal_store::store::identities::resolve_reference_conn(&connection, sender)?;
        let sender = sender.as_str();
        let recipients = recipients.iter().map(|recipient|
            fractal_store::store::identities::resolve_reference_conn(&connection, recipient)
        ).collect::<Result<Vec<_>, _>>()?;
        let mut statement = connection.prepare_cached(
            "SELECT subject, store_index FROM claims INDEXED BY claims_message_from_index
             WHERE kind='message.sent' AND json_extract(body, '$.fields.from') IN (SELECT name FROM identity_names WHERE identity=?1 UNION SELECT ?1)
               AND store_index>?2 AND store_index<=?3
               AND json_extract(body, '$.fields.to') IN (SELECT value FROM json_each(?4) UNION SELECT name FROM identity_names WHERE identity IN (SELECT value FROM json_each(?4)))
             ORDER BY store_index LIMIT ?5",
        )?;
        let mut subjects = statement.query_map(
            params![sender, after, through, serde_json::to_string(&recipients)?, limit.saturating_add(1)],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?)),
        )?.collect::<Result<Vec<_>, _>>()?;
        let more = subjects.len() > limit;
        subjects.truncate(limit);
        let next = more.then(|| subjects.last().map(|(_, index)| *index)).flatten();
        let mut output = Vec::new();
        for (subject, index) in subjects {
            let message = self.message_view_cached(&connection, &subject, index)?;
            if message.from == sender && recipients.contains(&message.to) && message.in_reply_to.is_some() {
                output.push(message);
            }
        }
        Ok((output, next))
    }

    fn messages_page(
            &self,
            recipient: Option<&str>,
            include_closed: bool,
            after: Option<u64>,
            through: u64,
            limit: usize,
        ) -> Result<(Vec<MessageView>, Option<u64>)> {
        self.messages_page_inner(recipient, include_closed, after, through, limit, false)
    }

    #[allow(clippy::too_many_arguments)]
    fn messages_page_filtered(
            &self,
            recipient: Option<&str>,
            sender: Option<&str>,
            subject: Option<&str>,
            subject_prefix: Option<&str>,
            include_closed: bool,
            after: Option<u64>,
            through: u64,
            limit: usize,
        ) -> Result<(Vec<MessageView>, Option<u64>)> {
        let connection = self.readers.get();
        let recipient = recipient.map(|party| fractal_store::store::identities::resolve_reference_conn(
            &connection, &normalize_message_party(party),
        )).transpose()?;
        let sender = sender.map(|party| fractal_store::store::identities::resolve_reference_conn(
            &connection, &normalize_message_party(party),
        )).transpose()?;
        anyhow::ensure!(
            sender.is_some() || (subject.is_none() && subject_prefix.is_none()),
            "subject filters need a sender filter"
        );
        // Sender selection starts at the existing expression indexes, not the recipient's
        // entire mailbox. Include legacy root fields and declarations, whose defaults and
        // canonical party spelling are resolved by message_view_cached below.
        let sql = if sender.is_some() {
            "WITH candidates(subject) AS (
                SELECT subject FROM claims INDEXED BY claims_message_from_index
                WHERE kind='message.sent' AND json_extract(body,'$.fields.from') IN (SELECT name FROM identity_names WHERE identity=?1 UNION SELECT ?1 UNION SELECT ?2)
                UNION
                SELECT subject FROM claims INDEXED BY claims_message_legacy_from_index
                WHERE kind='message.sent' AND json_type(body,'$.fields') IS NULL
                  AND json_extract(body,'$.from') IN (SELECT name FROM identity_names WHERE identity=?1 UNION SELECT ?1 UNION SELECT ?2)
                UNION
                SELECT desired.subject FROM desired
                WHERE desired.kind='message' AND (
                    EXISTS (SELECT 1 FROM json_each(desired.body,'$.children') child
                            WHERE json_extract(child.value,'$.name')='from'
                              AND json_extract(child.value,'$.arguments[0]') IN (SELECT name FROM identity_names WHERE identity=?1 UNION SELECT ?1 UNION SELECT ?2))
                    OR (?1='requester' AND NOT EXISTS (
                        SELECT 1 FROM json_each(desired.body,'$.children') child
                        WHERE json_extract(child.value,'$.name')='from')))
             )
             SELECT message_index.subject, created_index FROM candidates
             JOIN message_index ON message_index.subject=candidates.subject
             WHERE created_index>?3 AND created_index<=?4
             ORDER BY created_index, message_index.subject"
        } else {
            "SELECT subject, created_index FROM message_index
             WHERE (?1 IS NULL AND ?2 IS NULL) AND created_index>?3 AND created_index<=?4
             ORDER BY created_index, subject"
        };
        let mut statement = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h9c2f69ab3fea owner=fractal_delivery::store::delivery TODO=trace-statement-and-value-representation */ sql)?;
        let bare_sender = sender.as_deref().map(message_bare_recipient);
        let mut rows = statement.query(params![
            sender.as_deref(), bare_sender, after.unwrap_or(0), through,
        ])?;
        let mut recipient_discovery = connection.prepare_cached(concat!(
            "SELECT EXISTS (
                SELECT 1 FROM claims
                WHERE subject=?3 AND kind='message.sent'
                  AND ", message_sent_recipient_match!(), "
             ) OR EXISTS (
                SELECT 1 FROM desired, json_each(desired.body, '$.children') child
                WHERE desired.subject=?3 AND desired.kind='message'
                  AND ", message_desired_recipient_match!(), "
             )"
        ))?;
        let mut output: Vec<MessageView> = Vec::new();
        while let Some(row) = rows.next()? {
            let resource: String = row.get(0)?;
            let created_index = row.get(1)?;
            // Match the unfiltered recipient listing's discovery domain, not just the
            // hydrated recipient. In particular, a root-body legacy send alone is not
            // discovered by that endpoint; a matching declaration/native send can be.
            if let Some(recipient) = recipient.as_deref() {
                let discovered: bool = recipient_discovery.query_row(
                    /* sqlite-large-value: allow-for-now lv-hba917bc3669d owner=fractal_delivery::store::delivery TODO=trace-statement-and-value-representation */ params![recipient, message_bare_recipient(recipient), &resource],
                    |row| row.get(0),
                )?;
                if !discovered {
                    continue;
                }
            }
            let message = self.message_view_cached(&connection, &resource, created_index)?;
            // Hydration selects canonical actual state and desired fallbacks. Filtering raw
            // historical claim fields here would misclassify edited/declarative messages.
            if recipient.as_deref().is_none_or(|to| message.to == to)
                && sender.as_deref().is_none_or(|from| message.from == from)
                && subject.is_none_or(|title| message.title.as_deref() == Some(title))
                && subject_prefix.is_none_or(|prefix| {
                    message.title.as_deref().is_some_and(|title| title.starts_with(prefix))
                })
                && (include_closed || message.status != "closed")
            {
                if output.len() == limit {
                    let next = output.last().map(|message| message.created_index);
                    return Ok((output, next));
                }
                output.push(message);
            }
        }
        Ok((output, None))
    }

    fn messages_page_for_delivery(
            &self,
            recipient: &str,
            after: Option<u64>,
            through: u64,
            limit: usize,
        ) -> Result<(Vec<MessageView>, Option<u64>)> {
        self.messages_page_inner(Some(recipient), false, after, through, limit, true)
    }

    fn operational_messages(
            &self,
            recipient: Option<&str>,
            include_history: bool,
        ) -> Result<Vec<MessageView>> {
        let messages = self.messages(recipient, include_history)?;
        Ok(if include_history {
            messages
        } else {
            selected_actionable_messages(&self.readers.get(), messages)?
        })
    }

    fn subscription_requests(
            &self,
            subscription: &str,
        ) -> Result<Vec<SubscriptionRequestView>> {
        let mut decisions = self.claims_for(subscription, Some("subscription.mission-started"))?;
        for kind in [
            "subscription.mission-request-cancelled",
            "subscription.mission-request-released",
            "subscription.mission-failed",
        ] {
            decisions.extend(self.claims_for(subscription, Some(kind))?);
        }
        Ok(self
            .claims_for(subscription, Some("subscription.mission-requested"))?
            .into_iter()
            .map(|request| subscription_request_view(&request, &decisions))
            .collect())
    }

    fn decide_subscription_request(
            &self,
            request: &str,
            decision: &str,
            input: &SubscriptionRequestDecision,
        ) -> Result<SubscriptionRequestView, St3Error> {
        if !(input.actor.starts_with("person/") || input.actor.starts_with("agent/")) {
            return Err(St3Error::new(
                "invalid-subscription-request-actor",
                "a subscription request decision needs a person or agent actor",
            ));
        }
        if input.reason.trim().is_empty() {
            return Err(St3Error::new(
                "missing-subscription-request-reason",
                "a subscription request decision needs a reason",
            ));
        }
        let claim = self
            .claim_by_id(request)
            .map_err(internal)?
            .filter(|claim| claim.kind == "subscription.mission-requested")
            .ok_or_else(|| {
                St3Error::new(
                    "missing-subscription-request",
                    format!("subscription request `{request}` does not exist"),
                )
            })?;
        let view = |store: &Self| -> Result<SubscriptionRequestView, St3Error> {
            store
                .subscription_requests(&claim.subject)
                .map_err(internal)?
                .into_iter()
                .find(|view| view.request == claim.id)
                .ok_or_else(|| St3Error::new("internal", "the subscription request vanished"))
        };
        if self
            .operation_claim(&input.idempotency_key)
            .map_err(internal)?
            .is_some()
        {
            return view(self);
        }
        let current = view(self)?;
        let kind = match (decision, current.status.as_str()) {
            ("cancel", "pending" | "held") => "subscription.mission-request-cancelled",
            ("release", "held") => "subscription.mission-request-released",
            ("cancel" | "release", status) => {
                return Err(St3Error::new(
                    "subscription-request-not-open",
                    format!("subscription request `{request}` is {status}"),
                ));
            }
            (decision, _) => {
                return Err(St3Error::new(
                    "invalid-subscription-request-decision",
                    format!("`{decision}` is not a subscription request decision"),
                ));
            }
        };
        self.append_claim(&ClaimInput {
            subject: claim.subject.clone(),
            kind: kind.into(),
            actor: Some(input.actor.clone()),
            fields: BTreeMap::from([
                ("request".into(), Value::String(claim.id.clone())),
                ("reason".into(), Value::String(input.reason.clone())),
            ]),
            evidence: vec![claim.id.clone()],
            expected_subject: None,
            idempotency_key: Some(input.idempotency_key.clone()),
        })?;
        view(self)
    }

    fn pending_subscription_mission_requests(&self, subject: &str) -> Result<Vec<ClaimRecord>> {
        smallclaims::touched::note_read(|| subject.to_owned());
        let connection = self.readers.get();
        // Every pass asks this for every subscription. The finished requests are listed once, not
        // scanned for each request: a correlated NOT EXISTS read every finished claim once per
        // request, 0.4 s a pass for a subscription with hundreds of runs.
        let mut statement = connection.prepare(/* sqlite-large-value: allow-for-now lv-hfa785c4cf4bd owner=fractal_delivery::store::delivery TODO=trace-statement-and-value-representation */ &canonical_sql(
            "SELECT request.id, request.store_index, request.batch_id, request.subject,
                    request.kind, request.origin, request.actor, request.body,
                    request.predecessors, request.accepted_at_unix_ms
             FROM claims AS request
             WHERE request.subject=?1 AND request.kind='subscription.mission-requested'
               AND request.id NOT IN (
                 SELECT json_extract(finished.body, '$.fields.request') FROM claims AS finished
                 WHERE finished.subject=?1
                   AND finished.kind IN (
                     'subscription.mission-started',
                     'subscription.mission-failed',
                     'subscription.mission-request-cancelled'
                   )
                   AND json_extract(finished.body, '$.fields.request') IS NOT NULL
               )
             ORDER BY CANONICAL_ASC(request)",
        ))?;
        statement
            .query_map([subject], claim_from_row)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    fn subscription_mission_deferral(
            &self,
            subject: &str,
            request: &str,
        ) -> Result<Option<(u128, u32)>> {
        let connection = self.readers.get();
        let local: Option<(i64, u32)> = connection
            .query_row(
                "SELECT not_before_unix_ms, attempts
                 FROM local_subscription_mission_deferrals
                 WHERE subject=?1 AND request=?2",
                params![subject, request],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if let Some((deadline, attempts)) = local {
            return Ok(Some((deadline as u128, attempts)));
        }
        let count: u32 = connection.query_row(
            "SELECT COUNT(*) FROM claims
             WHERE subject=?1 AND kind='subscription.mission-deferred'
               AND json_extract(body, '$.fields.request')=?2",
            params![subject, request],
            |row| row.get(0),
        )?;
        if count == 0 {
            return Ok(None);
        }
        let deadline: Option<u64> = connection.query_row(
            /* sqlite-large-value: allow-for-now lv-h7dd4dec29fb6 owner=fractal_delivery::store::delivery TODO=trace-statement-and-value-representation */ &canonical_sql(
                "SELECT CAST(json_extract(body, '$.fields.not_before_unix_ms') AS INTEGER)
             FROM claims
             WHERE subject=?1 AND kind='subscription.mission-deferred'
               AND json_extract(body, '$.fields.request')=?2
             ORDER BY CANONICAL_DESC(claims) LIMIT 1",
            ),
            params![subject, request],
            |row| row.get(0),
        )?;
        Ok(deadline.map(|deadline| (u128::from(deadline), count)))
    }

    fn record_subscription_mission_deferral(
            &self,
            subject: &str,
            request: &str,
            deadline: u128,
            attempts: u32,
        ) -> Result<()> {
        let deadline = i64::try_from(deadline)?;
        self.connection
            .batched(|transaction| {
                transaction.execute(
                    /* sqlite-large-value: allow-for-now lv-hc27f5736969e owner=fractal_delivery::store::delivery TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO local_subscription_mission_deferrals
                         (subject, request, not_before_unix_ms, attempts)
                     VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT(subject, request) DO UPDATE SET
                         not_before_unix_ms=excluded.not_before_unix_ms,
                         attempts=excluded.attempts",
                    params![subject, request, deadline, attempts],
                )
            })
            .map_err(anyhow::Error::msg)??;
        Ok(())
    }

    fn bind_mailbox(
            &self,
            request: &crate::mailbox::Fence,
        ) -> Result<crate::mailbox::Fence, St3Error> {
        self.bind_mailbox_with_lease(request, None)
    }

    fn bind_mailbox_with_lease(
            &self,
            request: &crate::mailbox::Fence,
            authority: Option<&crate::mailbox::Authority>,
        ) -> Result<crate::mailbox::Fence, St3Error> {
        self.bind_mailbox_with_lease_checked(request, authority, &|| Ok(()))
    }

    fn bind_mailbox_with_lease_checked(
            &self,
            request: &crate::mailbox::Fence,
            authority: Option<&crate::mailbox::Authority>,
            validate: &dyn Fn() -> Result<(), St3Error>,
        ) -> Result<crate::mailbox::Fence, St3Error> {
        self.bind_mailbox_with_admission_checked(request, authority, None, validate)
    }

    fn bind_argv_mailbox_checked(
            &self,
            request: &crate::mailbox::Fence,
            member: &crate::model::MemberSpec,
            validate: &dyn Fn() -> Result<(), St3Error>,
        ) -> Result<crate::mailbox::Fence, St3Error> {
        self.bind_mailbox_with_admission_checked(request, None, Some(member), validate)
    }

    fn bind_mailbox_with_admission_checked(
            &self,
            request: &crate::mailbox::Fence,
            authority: Option<&crate::mailbox::Authority>,
            argv_member: Option<&crate::model::MemberSpec>,
            validate: &dyn Fn() -> Result<(), St3Error>,
        ) -> Result<crate::mailbox::Fence, St3Error> {
        let mut connection = self.connection.write();
        let tx = connection.transaction().map_err(internal)?;
        let mut stable_request;
        let identity = fractal_store::store::identities::resolve_reference_conn(&tx, &request.subject)?;
        let request = if identity == request.subject { request } else {
            stable_request = request.clone();
            stable_request.subject = identity;
            &stable_request
        };
        check_mailbox_incarnation(&tx, request)?;
        if let Some(member) = argv_member {
            mailbox_lease::check_argv_declaration(&tx, request, member, &self.origin)?;
        }
        validate()?;
        if let Some(authority) = authority {
            mailbox_lease::refuse_argv_native_adoption(&tx, request)?;
            mailbox_lease::check_declaration(&tx, request, authority, &self.origin)?;
            mailbox_lease::admit(&tx, request, authority)?;
        }
        if request.epoch != 0 {
            check_mailbox_fence(&tx, request, &self.origin)?;
            if let Some(authority) = authority {
                mailbox_lease::record(&tx, request, authority)?;
            }
            if let Some(member) = argv_member {
                mailbox_lease::record_argv_binding(&tx, request, member)?;
            }
            if authority.is_some() || argv_member.is_some() {
                tx.commit().map_err(internal)?;
            }
            return Ok(request.clone());
        }
        if request.token.is_empty() || request.token.len() > 128 {
            return Err(St3Error::new(
                "invalid-mailbox-token",
                "binding requires a stable request token",
            ));
        }
        let prior: Option<(String, String, String, u64)> = tx.query_row(
            "SELECT subject, component, incarnation, epoch FROM local_mailbox_bindings WHERE token=?1",
            [&request.token], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
        ).optional().map_err(internal)?;
        let mut bound = request.clone();
        if let Some((subject, component, incarnation, epoch)) = prior {
            if (subject, component, incarnation)
                != (
                    request.subject.clone(),
                    request.component.clone(),
                    request.incarnation.clone(),
                )
            {
                return Err(St3Error::new(
                    "invalid-mailbox-token",
                    "binding token belongs to another session",
                ));
            }
            bound.epoch = epoch;
            // Retired tokens cannot allocate another epoch and retake their successor's mailbox.
            check_mailbox_fence(&tx, &bound, &self.origin)?;
            if let Some(authority) = authority {
                mailbox_lease::record(&tx, &bound, authority)?;
            }
            if let Some(member) = argv_member {
                mailbox_lease::record_argv_binding(&tx, &bound, member)?;
            }
            if authority.is_some() || argv_member.is_some() {
                tx.commit().map_err(internal)?;
            }
            return Ok(bound);
        }
        // A revoked argv capability must not be recreated even if its binding row
        // was removed; its durable authenticated designation remains terminal.
        mailbox_lease::refuse_argv_native_adoption(&tx, request)?;
        let previous: Option<u64> = tx
            .query_row(
                "SELECT epoch FROM local_mailbox_owners WHERE subject=?1 AND component=?2",
                params![request.subject, request.component],
                |r| r.get(0),
            )
            .optional()
            .map_err(internal)?;
        bound.epoch = previous
            .unwrap_or(0)
            .checked_add(1)
            .filter(|epoch| *epoch <= i64::MAX as u64)
            .ok_or_else(|| St3Error::new("internal", "mailbox epoch exhausted"))?;
        tx.execute(
            /* sqlite-large-value: allow-for-now lv-he3ce29256ad4 owner=fractal_delivery::store::delivery TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO local_mailbox_bindings VALUES (?1,?2,?3,?4,?5)",
            params![
                bound.token,
                bound.subject,
                bound.component,
                bound.incarnation,
                bound.epoch
            ],
        )
        .map_err(internal)?;
        tx.execute(/* sqlite-large-value: allow-for-now lv-he60cb7a02517 owner=fractal_delivery::store::delivery TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO local_mailbox_owners VALUES (?1,?2,?3,?4)
            ON CONFLICT(subject,component) DO UPDATE SET incarnation=excluded.incarnation, epoch=excluded.epoch",
            params![bound.subject, bound.component, bound.incarnation, bound.epoch]).map_err(internal)?;
        if let Some(authority) = authority {
            mailbox_lease::record(&tx, &bound, authority)?;
        }
        if let Some(member) = argv_member {
            mailbox_lease::record_argv_binding(&tx, &bound, member)?;
        }
        tx.commit().map_err(internal)?;
        if let Some(wakes) = self.smalltalk.mailbox_wakes.get() {
            wakes.owner_changed(&bound.subject, &bound.component);
        }
        Ok(bound)
    }

    fn check_mailbox(&self, fence: &crate::mailbox::Fence) -> Result<(), St3Error> {
        check_mailbox_fence(&self.readers.get(), fence, &self.origin)
    }

    fn mailbox_watermark(
            &self,
            fence: &crate::mailbox::Fence,
        ) -> Result<MailboxWatermark, St3Error> {
        let connection = self.readers.get();
        let mut stable_fence;
        let identity = fractal_store::store::identities::resolve_reference_conn(&connection, &fence.subject)?;
        let fence = if identity == fence.subject { fence } else {
            stable_fence = fence.clone();
            stable_fence.subject = identity;
            &stable_fence
        };
        // The process-wide committed index may advance beyond a pinned reader. Capture both
        // cursors from its SQLite snapshot, or a concurrent commit could be skipped forever.
        let (index, local) = connection
            .prepare_cached("SELECT COALESCE((SELECT MAX(store_index) FROM claims),0),
                                    COALESCE((SELECT MAX(id) FROM local_observations),0)")
            .map_err(internal)?
            .query_row(/* sqlite-large-value: allow-for-now lv-h3cd58499d880 owner=fractal_delivery::store::delivery TODO=trace-statement-and-value-representation */ [], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(internal)?;
        Ok(MailboxWatermark {
            index,
            local,
            owner: mailbox_owner_key(&connection, fence)?,
            desired: connection.prepare_cached("SELECT claim_id FROM desired WHERE subject=?1")
                .map_err(internal)?.query_row(/* sqlite-large-value: allow-for-now lv-hea5ac5c547c1 owner=fractal_delivery::store::delivery TODO=trace-statement-and-value-representation */ [&fence.subject], |row| row.get(0))
                .optional().map_err(internal)?,
        })
    }

    #[cfg(any(test, feature = "test-support"))]
    fn mailbox_changed_since(
            &self,
            fence: &crate::mailbox::Fence,
            mark: &MailboxWatermark,
            messages: &[String],
        ) -> Result<bool, St3Error> {
        let connection = self.readers.get();
        let recipient = normalize_message_party(&fence.subject);
        let bare_recipient = recipient
            .strip_prefix("agent/")
            .filter(|suffix| !suffix.contains('/'))
            .unwrap_or(&recipient)
            .to_owned();
        let changed: i64 = connection
            .prepare_cached(
                "SELECT EXISTS(SELECT 1 FROM claims WHERE subject=?1 AND store_index>?2)
                     OR EXISTS(SELECT 1 FROM claims INDEXED BY claims_message_to_order_index
                               WHERE kind='message.sent'
                                 AND json_extract(body, '$.fields.to') IN (?3, ?4)
                                 AND store_index>?2)
                     OR EXISTS(SELECT 1 FROM claims
                               WHERE subject IN (SELECT value FROM json_each(?5))
                                 AND store_index>?2)
                     OR EXISTS(SELECT 1 FROM claims
                               WHERE kind='intent.desired' AND store_index>?2
                                 AND subject GLOB 'message/*')
                     OR EXISTS(SELECT 1 FROM local_observations WHERE subject=?1 AND id>?6)",
            )
            .map_err(internal)?
            .query_row(
                /* sqlite-large-value: allow-for-now lv-h8ca5a305a1ac owner=fractal_delivery::store::delivery TODO=trace-statement-and-value-representation */ params![
                    fence.subject,
                    mark.index,
                    recipient,
                    bare_recipient,
                    serde_json::to_string(messages).map_err(internal)?,
                    mark.local,
                ],
                |row| row.get(0),
            )
            .map_err(internal)?;
        Ok(changed != 0 || mailbox_owner_key(&connection, fence)? != mark.owner)
    }

    fn claims_since_only_quiet_notifications(&self, after_index: u64) -> Result<bool> {
        let connection = self.readers.get();
        let (total, other): (u64, u64) = connection.query_row(
            "SELECT COUNT(*), COUNT(*) FILTER (
                 WHERE kind NOT IN ('harness.usage', 'work.renewed', 'subagent.renewed',
                                    'sekret.called', 'sekret.exited', 'sekret.refused',
                                    'sekret.changed')
                   AND NOT (
                     kind IN ('message.sent', 'message.staged', 'message.delivered',
                              'message.read', 'message.closed')
                     AND EXISTS (
                       SELECT 1 FROM claims sent
                       WHERE sent.subject=claims.subject AND sent.kind='message.sent'
                     )
                     AND NOT EXISTS (
                       SELECT 1 FROM claims sent
                       WHERE sent.subject=claims.subject AND sent.kind='message.sent'
                         AND instr(sent.body, 'st3-work:')>0
                     )
                   )
             )
             FROM claims WHERE store_index > ?1",
            [after_index],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        Ok(total != 0 && other == 0)
    }
}

