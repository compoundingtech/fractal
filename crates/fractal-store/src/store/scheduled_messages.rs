//! Absolute-time seat mail. Claims are the durable queue and immutable key tombstones;
//! partial indexes seek pending requests without scanning ordinary message history.
//! Delivery and cancellation share the graph writer transaction. Mail is deliberately
//! inserted through the native graph path so its receipt commits with the insertion.
use super::*;
use crate::model::ScheduledMessageView;

pub const REQUESTED: &str = "message-schedule.requested";
pub const CANCELLED: &str = "message-schedule.cancelled";
pub const DELIVERED: &str = "message-schedule.delivered";
const RUNTIME: &str = "daemon/runtime";

pub(super) fn validate_replicated(claim: &ClaimRecord) -> Result<(), FractalError> {
    if claim.kind != REQUESTED { return Ok(()); }
    let fields = &claim.body["fields"];
    let sender = fields["from"].as_str().unwrap_or("");
    let target = fields["to"].as_str().unwrap_or("");
    fractal_schema::scheduled_messages::validate_request_actor(sender, claim.actor.as_deref())
        .map_err(|error| FractalError::new(error.code, error.message))?;
    let id = format!("message-schedule/{:x}", Sha256::digest(fields["key"].as_str().unwrap_or("").as_bytes()));
    if claim.subject != id || !sender.starts_with("agent/") || !target.starts_with("agent/")
        || fields["message"]["from"] != sender || fields["message"]["to"] != target
        || fields["at_unix_ms"].as_u64().is_none_or(|at| i64::try_from(at).is_err())
    {
        return Err(FractalError::new("invalid-scheduled-message", "request identity and native message bindings must agree"));
    }
    let message = serde_json::from_value::<BTreeMap<String,Value>>(fields["message"].clone()).map_err(internal)?;
    fractal_schema::registry().validate_claim(&message_id(&id), "message.sent", &message)
        .map_err(|error| FractalError::new(error.code, error.message))?;
    Ok(())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScheduledMessagePass {
    pub changed: bool,
    pub next_deadline: Option<u128>,
}

pub(super) fn create_schema(connection: &Connection) -> Result<()> {
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-h16d5941a8dee owner=fractal_store::store::scheduled_messages TODO=chunk-or-reference-variable-columns */ "CREATE INDEX IF NOT EXISTS claims_message_schedule_due
        ON claims(CAST(json_extract(body,'$.fields.at_unix_ms') AS INTEGER),subject)
        WHERE kind='message-schedule.requested';
        CREATE TABLE IF NOT EXISTS local_scheduled_message_pending(subject TEXT PRIMARY KEY,at INTEGER NOT NULL);
        CREATE INDEX IF NOT EXISTS local_scheduled_message_due ON local_scheduled_message_pending(at,subject);
        CREATE TRIGGER IF NOT EXISTS scheduled_message_pending_insert AFTER INSERT ON claims
        WHEN NEW.kind='message-schedule.requested' BEGIN
            INSERT OR IGNORE INTO local_scheduled_message_pending VALUES(NEW.subject,CAST(json_extract(NEW.body,'$.fields.at_unix_ms') AS INTEGER));
        END;
        CREATE TRIGGER IF NOT EXISTS scheduled_message_pending_delete AFTER DELETE ON claims
        WHEN OLD.kind IN ('message-schedule.requested','message-schedule.cancelled','message-schedule.delivered') BEGIN
            DELETE FROM local_scheduled_message_pending WHERE subject=OLD.subject;
            INSERT OR IGNORE INTO local_scheduled_message_pending
            SELECT subject,CAST(json_extract(body,'$.fields.at_unix_ms') AS INTEGER) FROM claims
            WHERE subject=OLD.subject AND kind='message-schedule.requested';
        END;
        INSERT OR IGNORE INTO local_scheduled_message_pending
        SELECT subject,CAST(json_extract(body,'$.fields.at_unix_ms') AS INTEGER) FROM claims
        WHERE kind='message-schedule.requested' AND NOT EXISTS(SELECT 1 FROM meta WHERE key='scheduled-message-index-v1');
        INSERT OR IGNORE INTO meta VALUES('scheduled-message-index-v1','1');")?;
    Ok(())
}

pub(super) fn reserved(kind: &str) -> Result<(), FractalError> {
    if matches!(kind, REQUESTED | CANCELLED | DELIVERED) {
        return Err(FractalError::new("scheduled-message-writer-required",
            "scheduled deliveries must use the scheduled-message writer"));
    }
    Ok(())
}

pub(super) fn refuse_immediate_key(connection: &Connection, input: &ClaimInput) -> Result<(), FractalError> {
    if input.kind == "message.sent" && let Some(key) = input.idempotency_key.as_deref() {
        let id = format!("message-schedule/{:x}", Sha256::digest(key.as_bytes()));
        let exists: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM claims
            WHERE kind='message-schedule.requested' AND subject=?1)",
            [&id], |row| row.get(0)).map_err(internal)?;
        if exists { return Err(FractalError::new("idempotency-mismatch",
            "this key already identifies a scheduled delivery")); }
    }
    Ok(())
}

fn requested(connection: &Connection, id: &str) -> Result<Option<ClaimRecord>> {
    connection.query_row(/* sqlite-large-value: allow-for-now lv-h3a21258307de owner=fractal_store::store::scheduled_messages TODO=trace-statement-and-value-representation */ &canonical_sql(&format!("SELECT {CLAIM_COLUMNS} FROM claims
        WHERE subject=?1 AND kind='message-schedule.requested'
          AND NOT EXISTS(SELECT 1 FROM replica_records WHERE claim_id=claims.id AND state='repaired')
        ORDER BY CANONICAL_ASC(claims) LIMIT 1")), [id], claim_from_row).optional().map_err(Into::into)
}

fn field<'a>(request: &'a ClaimRecord, name: &str) -> Result<&'a str> {
    request.body["fields"][name].as_str().with_context(|| format!("scheduled message missing {name}"))
}

fn due(request: &ClaimRecord) -> Result<u128> {
    Ok(u128::from(request.body["fields"]["at_unix_ms"].as_u64().context("scheduled message missing due time")?))
}

fn owner(connection: &Connection, origin: &str) -> Result<String, FractalError> {
    let (owner, _) = notice_authority::owner(connection)?;
    if owner.member != origin {
        return Err(FractalError::new("scheduled-message-owner-pending",
            format!("delivery belongs to member {}", owner.member)));
    }
    current_desired_row_tx(connection, crate::notice_owner::SUBJECT).map_err(internal)?
        .map(|row| row.claim_id).ok_or_else(|| FractalError::new("scheduled-message-owner-pending", "delivery owner unavailable"))
}

fn owner_writer(tx: &Transaction<'_>, origin: &str) -> Result<String, FractalError> {
    let pin = owner(tx, origin)?;
    notice_authority::validate_owner_writer(tx, RUNTIME,
        Some(notice_authority::Position::next(tx, origin).map_err(internal)?)).map_err(internal)?;
    Ok(pin)
}

// Validate terminal facts against the request, not arrival order. An unrelated replicated
// cancellation never changes the state. Alternative identical replica request IDs adopt the
// same immutable schedule subject and deterministic mailbox identity.
fn terminal(connection: &Connection, request: &ClaimRecord) -> Result<Option<ClaimRecord>> {
    let mut statement = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h9ee4cb9d5884 owner=fractal_store::store::scheduled_messages TODO=trace-statement-and-value-representation */ &canonical_sql(&format!(
        "SELECT {CLAIM_COLUMNS} FROM claims WHERE subject=?1
         AND kind IN ('message-schedule.delivered','message-schedule.cancelled')
         AND NOT EXISTS(SELECT 1 FROM replica_records WHERE claim_id=claims.id AND state='repaired')
         ORDER BY CANONICAL_ASC(claims)")))?;
    let records = statement.query_map([&request.subject], claim_from_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
    for record in records {
        let Some(reference) = record.body["fields"]["request"].as_str() else { continue };
        let Some(bound) = claim_by_id_tx(connection, reference)? else { continue };
        if bound.subject != request.subject || bound.kind != REQUESTED || bound.body["fields"] != request.body["fields"] { continue; }
        let actor = record.actor.as_deref();
        let authorized_cancel = record.kind == CANCELLED
            && (actor == Some(field(request,"from")?) || actor == Some(field(request,"to")?));
        if !authorized_cancel && actor != Some(RUNTIME) { continue; }
        let Some(pin) = record.body["fields"]["owner_claim"].as_str() else { continue };
        if notice_authority::validate_owner_receipt(connection, RUNTIME, pin,
            Some(notice_authority::Position::written(connection, &record)?)).is_err() { continue; }
        if record.kind == DELIVERED {
            if record.body["fields"]["message"].as_str() != Some(message_id(&request.subject).as_str()) { continue; }
        } else if !authorized_cancel && (record.body["fields"]["reason"] != "recipient-retired" || retirements::record(connection, field(request,"to")?, None)?.is_none()) {
            continue;
        }
        return Ok(Some(record));
    }
    Ok(None)
}

fn message_id(id: &str) -> String {
    format!("message/scheduled-{}", id.strip_prefix("message-schedule/").unwrap_or(id))
}

fn conflicting_request(connection: &Connection, request: &ClaimRecord) -> Result<bool> {
    Ok(connection.query_row("SELECT EXISTS(SELECT 1 FROM claims WHERE subject=?1
        AND kind='message-schedule.requested' AND json_extract(body,'$.fields')<>?2
        AND NOT EXISTS(SELECT 1 FROM replica_records WHERE claim_id=claims.id AND state='repaired'))",
        params![request.subject, serde_json::to_string(&request.body["fields"])?], |row| row.get(0))?)
}

fn view(connection: &Connection, request: &ClaimRecord, origin: &str) -> Result<ScheduledMessageView> {
    let terminal = terminal(connection, request)?;
    let state = match terminal.as_ref().map(|claim| claim.kind.as_str()) {
        Some(DELIVERED) => "delivered", Some(CANCELLED) => "cancelled", _ => "pending",
    };
    let blocker = if terminal.is_some() { None } else if conflicting_request(connection, request)? {
        Some("conflicting immutable scheduled delivery payloads".into())
    } else if current_desired_row_tx(connection, field(request,"to")?)?.is_none() {
        Some("recipient declaration unavailable".into())
    } else {
        owner(connection, origin).err().map(|error| error.message).or_else(|| {
            retirements::admit_recipient(connection, request.body["fields"]["to"].as_str()).err().map(|error| error.message)
        })
    };
    let at_unix_ms = due(request)?;
    Ok(ScheduledMessageView {
        id: request.subject.clone(), from: field(request,"from")?.into(), to: field(request,"to")?.into(),
        at: crate::external_sessions::timestamp(at_unix_ms), at_unix_ms,
        key: field(request,"key")?.into(), state: state.into(), blocker,
        content: request.body["fields"]["message"]["content"].as_str().unwrap_or("").to_owned(),
        owner: notice_authority::owner(connection).ok().map(|(owner,_)| owner.member),
        message_id: terminal.as_ref().filter(|claim| claim.kind == DELIVERED).map(|_| message_id(&request.subject)),
        cancel_reason: terminal.as_ref().filter(|claim| claim.kind == CANCELLED)
            .and_then(|claim| claim.body["fields"]["reason"].as_str().map(str::to_owned)),
    })
}

fn append(tx: &Transaction<'_>, origin: &str, subject: &str, kind: &str, actor: &str, fields: Value, evidence: &[String]) -> Result<ClaimRecord> {
    append_admitted_claim_tx(tx, origin, subject, kind, Some(actor),
        &json!({"fields":fields,"evidence":evidence}), &[], None, false)
}

impl Store {
    pub fn schedule_message(&self, input: &ClaimInput, at_unix_ms: u128, key: &str) -> Result<ScheduledMessageView, FractalError> {
        if input.kind != "message.sent" || key.trim().is_empty() || i64::try_from(at_unix_ms).is_err() {
            return Err(FractalError::new("invalid-scheduled-message", "a message, nonempty key and representable UTC instant are required"));
        }
        validate_claim_input(input)?;
        let from = input.fields.get("from").and_then(Value::as_str).unwrap_or("");
        let to = input.fields.get("to").and_then(Value::as_str).unwrap_or("");
        if input.actor.as_deref() != Some(from) || !from.starts_with("agent/") || !to.starts_with("agent/") {
            return Err(FractalError::new("invalid-scheduled-message", "scheduled delivery requires its seat sender and a seat recipient"));
        }
        let id = format!("message-schedule/{:x}", Sha256::digest(key.as_bytes()));
        message_sent::gated_by_key(self, &id, Some(key), || self.connection.batched(|tx| -> Result<ScheduledMessageView, FractalError> {
            let from = identities::resolve_reference_conn(tx, from).map_err(internal)?;
            let to = identities::resolve_reference_conn(tx, to).map_err(internal)?;
            let mut message = input.fields.clone();
            message.insert("from".into(), json!(from));
            message.insert("to".into(), json!(to));
            if let Some(prior) = requested(tx, &id).map_err(internal)? {
                let fields = &prior.body["fields"];
                if fields["message"] != json!(message) || fields["evidence"] != json!(input.evidence)
                    || fields["at_unix_ms"] != json!(at_unix_ms) || fields["from"] != from || fields["to"] != to {
                    return Err(FractalError::new("idempotency-mismatch", "scheduled delivery key already has a different payload"));
                }
                return view(tx, &prior, &self.origin).map_err(internal);
            }
            let operation = operation_id_for_key(key);
            let exists: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM operations WHERE id=?1)", [&operation], |row| row.get(0)).map_err(internal)?;
            if exists { return Err(FractalError::new("idempotency-mismatch", "key already identifies an immediate operation")); }
            if message_sent::sent_to_stream(self, input)?.is_some() {
                return Err(FractalError::new("idempotency-mismatch", "key already identifies an immediate message"));
            }
            retirements::admit_recipient(tx, Some(&to))?;
            let declaration = current_desired_row_tx(tx, &to).map_err(internal)?
                .ok_or_else(|| FractalError::new("unknown-recipient", "scheduled recipient must have a declaration"))?;
            let request = append(tx, &self.origin, &id, REQUESTED, &from, json!({
                "from":from,"to":to,"at_unix_ms":at_unix_ms,"key":key,"message":message,
                "evidence":input.evidence,"recipient_declaration":declaration.claim_id,
            }), &[declaration.claim_id]).map_err(internal)?;
            view(tx, &request, &self.origin).map_err(internal)
        }).map_err(internal)?)
    }

    pub fn scheduled_messages(&self) -> Result<Vec<ScheduledMessageView>> {
        let connection = self.readers.get();
        let ids = connection.prepare_cached("SELECT DISTINCT subject FROM claims INDEXED BY claims_message_schedule_due
            WHERE kind='message-schedule.requested' ORDER BY CAST(json_extract(body,'$.fields.at_unix_ms') AS INTEGER),subject")?
            .query_map([], |row| row.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        ids.into_iter().map(|id| {
            let request = requested(&connection, &id)?.context("scheduled request disappeared")?;
            view(&connection, &request, &self.origin)
        }).collect()
    }

    pub fn scheduled_message(&self, id: &str) -> Result<Option<ScheduledMessageView>> {
        let connection = self.readers.get();
        requested(&connection, id)?.map(|request| view(&connection, &request, &self.origin)).transpose()
    }

    pub fn cancel_scheduled_message(&self, id: &str, actor: &str, reason: &str) -> Result<ScheduledMessageView, FractalError> {
        self.connection.batched(|tx| -> Result<ScheduledMessageView, FractalError> {
            let request = requested(tx, id).map_err(internal)?.ok_or_else(|| FractalError::new("not-found", "scheduled delivery not found"))?;
            let actor = identities::resolve_reference_conn(tx, actor).map_err(internal)?;
            if actor != field(&request,"from").map_err(internal)? && actor != field(&request,"to").map_err(internal)? {
                return Err(FractalError::new("scheduled-message-forbidden", "only the sender or target can cancel this delivery"));
            }
            if let Some(terminal) = terminal(tx, &request).map_err(internal)? {
                if terminal.kind == DELIVERED { return Err(FractalError::new("already-delivered", "message already inserted; cancellation cannot retract it")); }
                return view(tx, &request, &self.origin).map_err(internal);
            }
            let owner_claim = owner_writer(tx, &self.origin)?;
            append(tx, &self.origin, id, CANCELLED, &actor,
                json!({"request":request.id,"reason":reason,"owner_claim":owner_claim}),
                &[request.id.clone(),owner_claim.clone()]).map_err(internal)?;
            tx.execute(/* sqlite-large-value: allow-for-now lv-h3c75a80d12d3 owner=fractal_store::store::scheduled_messages TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_scheduled_message_pending WHERE subject=?1", [id]).map_err(internal)?;
            view(tx, &request, &self.origin).map_err(internal)
        }).map_err(internal)?
    }

    pub fn reconcile_scheduled_messages_at(&self, now: u128) -> Result<ScheduledMessagePass> {
        self.connection.batched(|tx| -> Result<ScheduledMessagePass> {
            let mut pass = ScheduledMessagePass::default();
            let clock = i64::try_from(now).unwrap_or(i64::MAX);
            pass.next_deadline = tx.query_row("SELECT MIN(at) FROM local_scheduled_message_pending WHERE at>?1",
                [clock], |row| row.get::<_,Option<u64>>(0))?.map(u128::from);
            let ids = tx.prepare_cached("SELECT subject FROM local_scheduled_message_pending WHERE at<=?1 ORDER BY at,subject")?
                .query_map([clock], |row| row.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            for id in ids {
                let request = requested(tx, &id)?.context("scheduled request disappeared")?;
                if terminal(tx, &request)?.is_some() {
                    tx.execute(/* sqlite-large-value: allow-for-now lv-h97c8c83ad46e owner=fractal_store::store::scheduled_messages TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_scheduled_message_pending WHERE subject=?1", [&id])?;
                    continue;
                }
                let at = due(&request)?;
                if at > now {
                    pass.next_deadline = Some(pass.next_deadline.map_or(at, |next| next.min(at)));
                    continue;
                }
                if conflicting_request(tx, &request)? { continue; }
                let Ok(owner_claim) = owner_writer(tx, &self.origin) else { continue };
                let to = field(&request,"to")?;
                if retirements::record(tx, to, None)?.is_some() {
                    append(tx, &self.origin, &id, CANCELLED, RUNTIME,
                        json!({"request":request.id,"reason":"recipient-retired","owner_claim":owner_claim}), &[request.id.clone(),owner_claim.clone()])?;
                    tx.execute(/* sqlite-large-value: allow-for-now lv-he85a4a926d8f owner=fractal_store::store::scheduled_messages TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_scheduled_message_pending WHERE subject=?1", [&id])?;
                    pass.changed = true;
                    continue;
                }
                if retirements::admit_recipient(tx, Some(to)).is_err() { continue; }
                if current_desired_row_tx(tx, to)?.is_none() { continue; }
                let fields = serde_json::from_value(request.body["fields"]["message"].clone())?;
                let input = ClaimInput {
                    subject: message_id(&id), kind: "message.sent".into(), actor: Some(field(&request,"from")?.into()),
                    fields, evidence: serde_json::from_value(request.body["fields"]["evidence"].clone())?,
                    expected_subject: None, idempotency_key: None,
                };
                // Graph-native insertion is intentional: a cross-file shard write cannot
                // atomically serialize with cancel and the delivered receipt.
                if !message_sent::message_sent_tx(tx, &input.subject)? {
                    message_routing::send_message_tx(tx, &self.origin, &input, Some("scheduled-atomic"))?;
                }
                append(tx, &self.origin, &id, DELIVERED, RUNTIME,
                    json!({"request":request.id,"message":input.subject,"owner_claim":owner_claim}),
                    &[request.id.clone(),owner_claim.clone()])?;
                tx.execute(/* sqlite-large-value: allow-for-now lv-h07d713865c5e owner=fractal_store::store::scheduled_messages TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_scheduled_message_pending WHERE subject=?1", [&id])?;
                pass.changed = true;
            }
            Ok(pass)
        }).map_err(anyhow::Error::msg)?
    }
}
