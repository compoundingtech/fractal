//! Disposable wake projection and durable acceptance-time wake audit outbox.
//! Audit capture is independent of pending/delivery state and survives projection rebuilds.
use super::*;

const TABLE: &str = "local_wake_demand_v1";
const QUEUE: &str = "local_wake_demand_pending_v1";
const COMPLETE: &str = "local_wake_demand_v1_complete";
const AFTER: &str = "local_wake_demand_v1_after";

pub fn create_schema(connection: &Connection) -> Result<()> {
    create_audit_capture(connection, "NEW.store_index")?;
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-h3eddd3ad87ca owner=fractal_store::store::wake_demand TODO=chunk-or-reference-variable-columns */ "
        CREATE TABLE IF NOT EXISTS local_wake_demand_v1 (
            subject TEXT PRIMARY KEY, recipient TEXT NOT NULL, class INTEGER NOT NULL,
            sent_ms INTEGER NOT NULL, pending INTEGER NOT NULL
        ) WITHOUT ROWID;
        CREATE INDEX IF NOT EXISTS local_wake_demand_seat_v1
            ON local_wake_demand_v1(recipient,class DESC,sent_ms,subject) WHERE pending=1;
        CREATE TABLE IF NOT EXISTS local_wake_demand_pending_v1 (
            subject TEXT PRIMARY KEY
        ) WITHOUT ROWID;
    ")?;
    for (table, subject, condition) in [
        ("claims", "subject", "NEW.kind LIKE 'message.%'"),
        ("local_stream_sent", "subject", "1"),
        ("local_stream_lifecycle", "subject", "1"),
        ("local_message_marks", "message", "1"),
        ("local_message_staged", "message", "1"),
    ] {
        // Receipts clear pending immediately, even before the send reaches this member.
        // Send folds read every receipt source, including receipt-before-send replication.
        let receipt = if table == "local_message_staged" {
            "1".to_owned()
        } else if table == "local_stream_sent" {
            "0".to_owned()
        } else {
            "NEW.kind IN ('message.staged','message.delivered','message.read','message.closed')".to_owned()
        };
        let old_condition = condition.replace("NEW.", "OLD.");
        connection.execute_batch(&format!(/* sqlite-large-value: allow-for-now lv-hc986dc3c5735 owner=fractal_store::store::wake_demand TODO=bound-or-reference-values-and-reclamation */ "
            CREATE TRIGGER IF NOT EXISTS wake_v1_{table}_insert AFTER INSERT ON {table}
            WHEN {condition} BEGIN
                UPDATE {TABLE} SET pending=0 WHERE subject=NEW.{subject} AND {receipt};
                INSERT OR IGNORE INTO {QUEUE} SELECT NEW.{subject} WHERE NOT ({receipt});
            END;
            CREATE TRIGGER IF NOT EXISTS wake_v1_{table}_delete AFTER DELETE ON {table}
            WHEN {old_condition}
            BEGIN INSERT OR IGNORE INTO {QUEUE} VALUES(OLD.{subject}); END;
            CREATE TRIGGER IF NOT EXISTS wake_v1_{table}_update AFTER UPDATE ON {table}
            WHEN {condition} OR {old_condition}
            BEGIN
                INSERT OR IGNORE INTO {QUEUE} VALUES(OLD.{subject});
                INSERT OR IGNORE INTO {QUEUE} VALUES(NEW.{subject});
            END;
        "))?;
    }
    // A fresh version rebuilds from both sources in bounded, restartable transactions.
    let complete: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM meta WHERE key=?1)", [COMPLETE], |row| row.get(0),
    )?;
    if !complete {
        let mut after: String = connection.query_row(
            "SELECT value FROM meta WHERE key=?1", [AFTER], |row| row.get(0),
        ).optional()?.unwrap_or_default();
        loop {
            let subjects = connection.prepare_cached(
                "SELECT subject FROM message_index WHERE subject>?1
                 UNION SELECT subject FROM local_stream_sent WHERE subject>?1
                 ORDER BY subject LIMIT 64",
            )?.query_map([&after], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            if subjects.is_empty() { break; }
            let transaction = connection.unchecked_transaction()?;
            for subject in &subjects { fold(&transaction, subject)?; }
            after = subjects.last().unwrap().clone();
            transaction.execute(/* sqlite-large-value: allow-for-now lv-h7de6575aa7e5 owner=fractal_store::store::wake_demand TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO meta(key,value) VALUES(?1,?2)", params![AFTER, after])?;
            transaction.commit()?;
        }
        let transaction = connection.unchecked_transaction()?;
        transaction.execute(/* sqlite-large-value: allow-for-now lv-h9eed8a526c41 owner=fractal_store::store::wake_demand TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO meta(key,value) VALUES(?1,'1')", [COMPLETE])?;
        transaction.execute(/* sqlite-large-value: allow-for-now lv-hcaf12b1aa1c2 owner=fractal_store::store::wake_demand TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM meta WHERE key=?1", [AFTER])?;
        transaction.commit()?;
    }
    // Also drain work left by an interrupted projection pass.
    let transaction = connection.unchecked_transaction()?;
    flush(&transaction)?;
    transaction.commit()?;
    Ok(())
}

fn fold(connection: &Connection, subject: &str) -> Result<()> {
    // Graph claims win duplicate sends, as in the other mirror consumers. Only metadata is read.
    let sent: Option<(String, i64)> = connection.query_row(
        "SELECT json_object('from',json_extract(body,'$.fields.from'),
                            'to',json_extract(body,'$.fields.to'),
                            'tags',json_extract(body,'$.fields.tags')),
                CAST(accepted_at_unix_ms AS INTEGER)
         FROM claims WHERE subject=?1 AND kind='message.sent'
         ORDER BY CAST(accepted_at_unix_ms AS INTEGER),id LIMIT 1",
        [subject], |row| Ok((row.get(0)?, row.get(1)?)),
    ).optional()?;
    let sent = match sent {
        Some(sent) => Some(sent),
        None => connection.query_row(
            "SELECT fields,sent_ms FROM local_stream_sent WHERE subject=?1", [subject],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional()?,
    };
    connection.execute(/* sqlite-large-value: allow-for-now lv-h7883dab4ee98 owner=fractal_store::store::wake_demand TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_wake_demand_v1 WHERE subject=?1", [subject])?;
    let Some((fields, sent_ms)) = sent else { return Ok(()); };
    let fields: Value = serde_json::from_str(&fields)?;
    let from = fields["from"].as_str().unwrap_or_default();
    let tags: Vec<String> = fields["tags"].as_array().into_iter().flatten()
        .filter_map(Value::as_str).map(str::to_owned).collect();
    let held = tags.iter().any(|tag| crate::silent::is_silent_tag(tag))
        && !crate::silent::always_wakes(from, &tags);
    let class = if held { 0 } else if from.starts_with("person/") || from.starts_with("external/") { 2 } else { 1 };
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-hc920614e4c86 owner=fractal_store::store::wake_demand TODO=prepare-body-and-predecessor-values */ "INSERT INTO local_wake_demand_v1(subject,recipient,class,sent_ms,pending)
         SELECT ?1,?2,?3,?4,NOT (
             EXISTS(SELECT 1 FROM claims WHERE subject=?1 AND kind IN
                 ('message.staged','message.delivered','message.read','message.closed'))
             OR EXISTS(SELECT 1 FROM local_message_marks WHERE message=?1)
             OR EXISTS(SELECT 1 FROM local_message_staged WHERE message=?1)
             OR EXISTS(SELECT 1 FROM local_stream_lifecycle WHERE subject=?1 AND kind IN
                 ('message.staged','message.delivered','message.read','message.closed')))",
        params![subject, fields["to"].as_str().unwrap_or_default(), class, sent_ms],
    )?;
    Ok(())
}

pub fn flush(transaction: &Transaction<'_>) -> Result<()> {
    loop {
        let subjects = transaction.prepare_cached(
            "SELECT subject FROM local_wake_demand_pending_v1 ORDER BY subject LIMIT 64",
        )?.query_map([], |row| row.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        if subjects.is_empty() { return Ok(()); }
        for subject in subjects {
            fold(transaction, &subject)?;
            transaction.execute(/* sqlite-large-value: allow-for-now lv-h58fc3f27c31a owner=fractal_store::store::wake_demand TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_wake_demand_pending_v1 WHERE subject=?1", [&subject])?;
        }
    }
}

/// Independent source positions; graph and shard indexes are never compared to each other.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frontier {
    pub graph: u64,
    pub marks: u64,
    pub mirrored_sent: u64,
    pub mirrored_lifecycle: u64,
    /// None means the shard is unavailable, not empty.
    pub source: Option<(u64, u64)>,
    pub caught_up: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingWake {
    pub subject: String,
    pub sent_ms: i64,
    pub person: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Demand {
    /// Oldest person demand first, otherwise oldest wake; ties use the message subject.
    pub oldest: Option<PendingWake>,
    pub frontier: Frontier,
}

impl Store {
    /// Read-only: never mirrors or populates the projection. A lagging/absent shard cannot
    /// prove absence. The source cut is sampled first; the graph is then read in one snapshot.
    pub fn wake_demand(&self, seat: &str) -> Result<Demand> {
        smallclaims::touched::note_read(|| format!("mailbox:{seat}"));
        let demand = self.wake_demand_untracked(seat)?;
        if let Some(pending) = &demand.oldest {
            smallclaims::touched::note_read(|| pending.subject.clone());
        }
        Ok(demand)
    }

    /// Queue ranking must not subscribe a seat's reactive item to other seats' mailboxes.
    pub(super) fn wake_demand_untracked(&self, seat: &str) -> Result<Demand> {
        let source = messages_shard::shard(self).map(|shard| {
            shard.readers.get().query_row(
                "SELECT COALESCE((SELECT MAX(store_index) FROM stream_messages),0),
                        COALESCE((SELECT MAX(rowid) FROM stream_lifecycle),0)", [],
                |row| Ok((row.get::<_, u64>(0)?, row.get::<_, u64>(1)?)),
            )
        }).transpose()?;
        let connection = self.readers.get();
        let transaction = connection.unchecked_transaction()?;
        let mirrored_sent = stream_mirror::sent_through(&transaction)?;
        let mirrored_lifecycle = stream_mirror::lifecycle_through(&transaction)?;
        let dirty: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM local_wake_demand_pending_v1)", [], |row| row.get(0),
        )?;
        let graph = transaction.query_row("SELECT COALESCE(MAX(store_index),0) FROM claims", [], |row| row.get(0))?;
        let oldest = transaction.query_row(
            "SELECT subject,sent_ms,class FROM local_wake_demand_v1
             WHERE recipient=?1 AND pending=1 AND class>0
             ORDER BY class DESC,sent_ms,subject LIMIT 1", [seat],
            |row| Ok(PendingWake { subject: row.get(0)?, sent_ms: row.get(1)?, person: row.get::<_, i64>(2)? == 2 }),
        ).optional()?;
        Ok(Demand { oldest, frontier: Frontier {
            graph, marks: message_marks::event_position(&transaction)?, mirrored_sent,
            mirrored_lifecycle, source,
            caught_up: !dirty && source.is_some_and(|(sent, lifecycle)| mirrored_sent >= sent && mirrored_lifecycle >= lifecycle),
        } })
    }
}

/// This ledger is not a projection: never rebuild it from old claims. Missing history is
/// unknown, and a restart must not manufacture acceptance-time suspension observations.
pub(super) fn create_audit_capture(connection: &Connection, graph_cut: &str) -> Result<()> {
    connection.execute_batch(&format!("
        CREATE TABLE IF NOT EXISTS local_residency_wake_capture (
            message TEXT PRIMARY KEY, seat TEXT NOT NULL, accepted_at INTEGER NOT NULL,
            received_at INTEGER NOT NULL, graph_cut INTEGER NOT NULL, metadata TEXT NOT NULL,
            processed INTEGER NOT NULL DEFAULT 0
        ) WITHOUT ROWID;
        CREATE INDEX IF NOT EXISTS local_residency_wake_capture_pending
            ON local_residency_wake_capture(processed,message);
        CREATE TRIGGER IF NOT EXISTS residency_wake_capture AFTER INSERT ON claims
        WHEN NEW.kind='message.sent' BEGIN
            INSERT OR IGNORE INTO local_residency_wake_capture
                (message,seat,accepted_at,received_at,graph_cut,metadata)
            VALUES(NEW.subject,COALESCE(json_extract(NEW.body,'$.fields.to'),''),
                CAST(NEW.accepted_at_unix_ms AS INTEGER),
                CAST(unixepoch('subsec')*1000 AS INTEGER),{graph_cut},
                json_object('from',json_extract(NEW.body,'$.fields.from'),
                            'tags',json_extract(NEW.body,'$.fields.tags')));
        END;
    "))?;
    Ok(())
}

type AuditCapture = (String, String, i64, i64, u64, String);

fn audit_captures(source: &Connection) -> Result<Vec<AuditCapture>> {
    Ok(source.prepare_cached(
        "SELECT message,seat,accepted_at,received_at,graph_cut,metadata
         FROM local_residency_wake_capture WHERE processed=0 ORDER BY message LIMIT 64",
    )?.query_map([], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

fn acknowledge_audits(source: &Connection, rows: &[AuditCapture]) -> Result<bool> {
    for row in rows {
        source.execute("UPDATE local_residency_wake_capture SET processed=1 WHERE message=?1", [&row.0])?;
    }
    Ok(!source.query_row("SELECT EXISTS(SELECT 1 FROM local_residency_wake_capture WHERE processed=0)",
        [], |row| row.get::<_, bool>(0))?)
}

impl Store {
    /// Process at most 64 captured messages per database. Returns true only when both
    /// sources are drained; delivery, manual conversion and restart cannot erase a capture.
    pub fn drain_wake_audits(&self) -> Result<bool> {
        let shard = messages_shard::shard(self);
        let mut drained = true;
        // Shard first follows the source-before-graph writer lock order.
        if let Some(shard) = &shard {
            let mut writer = shard.connection.write();
            let source = writer.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            drained &= self.drain_wake_audit_source(&source)?;
            source.commit()?;
        } else {
            drained = false;
        }
        self.connection.batched(|tx| -> Result<bool> {
            // Graph source and destination share one transaction.
            let rows = audit_captures(tx)?;
            if !self.emit_wake_audit_rows(tx, &rows)? { return Ok(false); }
            drained &= acknowledge_audits(tx, &rows)?;
            Ok(drained)
        }).map_err(anyhow::Error::msg)?
    }

    fn drain_wake_audit_source(&self, source: &Transaction<'_>) -> Result<bool> {
        let rows = audit_captures(source)?;
        if !self.connection.batched(|tx| self.emit_wake_audit_rows(tx, &rows))
            .map_err(anyhow::Error::msg)??
        { return Ok(false); }
        // A crash after the graph commit but before this acknowledgement replays immutable
        // evidence under the same key, rather than losing or duplicating the observation.
        acknowledge_audits(source, &rows)
    }

    fn emit_wake_audit_rows(&self, tx: &Transaction<'_>, rows: &[AuditCapture]) -> Result<bool> {
        if projection_work_pending_tx(tx).map_err(anyhow::Error::new)?
            || declared_admission::validate_fleet(tx, &self.origin, &residency_evidence::AUDIT_FEATURE).is_err()
        {
            return Ok(false);
        }
        for (message, seat, accepted, received, cut, metadata) in rows {
            let fields: Value = serde_json::from_str(metadata)?;
            let from = fields["from"].as_str().unwrap_or_default();
            let tags: Vec<String> = fields["tags"].as_array().into_iter().flatten()
                .filter_map(Value::as_str).map(str::to_owned).collect();
            let wakes = !tags.iter().any(|tag| crate::silent::is_silent_tag(tag))
                || crate::silent::always_wakes(from, &tags);
            let desired = desired_row_at(tx, seat, Some(*cut))?;
            let owner = desired.filter(|row| row.kind == "agent")
                .and_then(|row| row.member)
                .map(|member| serde_json::from_str::<MemberSpec>(&member)).transpose()?;
            if wakes && owner.is_some_and(|member| member.host == self.origin)
                && let Some(state) = crate::suspension::current_at_tx(tx, seat, Some(*cut))?
                && state.cause() == "idle" && state.suspended_at_unix_ms.is_some()
                && state.phase != "resumed"
            {
                let suspension = state.suspend_operation_id.as_deref().unwrap_or(&state.operation_id);
                idle_residency::evidence(tx, &self.origin, &seat, "wake-audit",
                    &format!("wake-audit:{}:{message}", self.origin), "residency.wake-audit",
                    json!({"message_id":message,"accepted_at":accepted,"owner_received_at":received,
                        "suspension_id":suspension,"suspension_cause":"idle"}))?;
            }
        }
        Ok(true)
    }
}

/// Inputs to a complete host audit census. Audit output itself does not invalidate the cut.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuditCoverageCut {
    pub graph: u64,
    pub source: Option<(u64, u64)>,
}

#[derive(Default)]
struct CoverageProgress {
    since: Option<u128>,
    through: u128,
    audit_index: u64,
    previous: Option<String>,
    in_flight: bool,
}

#[derive(Default)]
struct CoverageInterval(Mutex<CoverageProgress>);

fn audit_graph_cut(connection: &Connection) -> Result<u64> {
    Ok(connection.query_row(
        "SELECT COALESCE(MAX(store_index),0) FROM claims WHERE kind NOT LIKE 'residency.%'",
        [], |row| row.get(0),
    )?)
}

impl Store {
    pub fn residency_audit_coverage_cut(&self) -> Result<AuditCoverageCut> {
        let interval = self.extension::<CoverageInterval>();
        let mut progress = interval.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        // If the preceding census returned early or failed, its missing interval is unknown.
        if progress.in_flight { *progress = CoverageProgress::default(); }
        progress.in_flight = true;
        drop(progress);
        self.current_residency_audit_coverage_cut()
    }

    fn current_residency_audit_coverage_cut(&self) -> Result<AuditCoverageCut> {
        let source = messages_shard::shard(self).map(|shard| shard.readers.get().query_row(
            "SELECT COALESCE((SELECT MAX(store_index) FROM stream_messages),0),
                    COALESCE((SELECT MAX(rowid) FROM stream_lifecycle),0)",
            [], |row| Ok((row.get(0)?, row.get(1)?)),
        )).transpose()?;
        Ok(AuditCoverageCut { graph: audit_graph_cut(&self.readers.get())?, source })
    }

    /// Called only after a complete resume-history census. A fresh process and every failed
    /// census start a new interval; neither an old frontier nor an unavailable source proves
    /// completeness. Replicas verify the cumulative dependency set before trusting coverage.
    pub fn publish_residency_audit_coverage(
        &self, cut: &AuditCoverageCut, wake_drained: bool, now: u128,
    ) -> Result<bool> {
        let interval = self.extension::<CoverageInterval>();
        let mut progress = interval.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let result = idle_residency::with_source_fence(self, || {
            let current = self.current_residency_audit_coverage_cut()?;
            self.connection.batched(|tx| -> Result<Option<CoverageProgress>> {
                if declared_admission::validate_fleet(tx, &self.origin, &residency_evidence::AUDIT_FEATURE).is_err() {
                    return Ok(None);
                }
                let pending: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM local_residency_wake_capture WHERE processed=0)",
                    [], |row| row.get(0),
                )?;
                let source_pending = messages_shard::shard(self).map(|shard| shard.readers.get().query_row(
                    "SELECT EXISTS(SELECT 1 FROM local_residency_wake_capture WHERE processed=0)",
                    [], |row| row.get::<_, bool>(0),
                )).transpose()?.unwrap_or(true);
                if !wake_drained || cut.source.is_none() || current != *cut || pending || source_pending
                    || progress.since.is_some_and(|since| now < since) || now < progress.through
                    || audit_graph_cut(tx)? != cut.graph
                    || projection_work_pending_tx(tx).map_err(anyhow::Error::new)?
                    || stream_mirror::sent_through(tx)? < cut.source.unwrap().0
                    || stream_mirror::lifecycle_through(tx)? < cut.source.unwrap().1
                {
                    return Ok(None);
                }
                let covered_since = progress.since.unwrap_or(now);
                // Each immutable page binds at most 128 dependencies. A receiver walks the
                // previous_coverage chain; reordered claims cannot certify a missing audit.
                let rows = tx.prepare_cached(
                    "SELECT id,store_index FROM claims WHERE origin=?1 AND store_index>?2
                     AND kind IN ('residency.wake-audit','residency.resume-audit')
                     ORDER BY store_index LIMIT 128",
                )?.query_map(params![self.origin, progress.audit_index],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?)))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                let audit_index = rows.last().map_or(progress.audit_index, |row| row.1);
                let remaining: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM claims WHERE origin=?1 AND store_index>?2
                     AND kind IN ('residency.wake-audit','residency.resume-audit'))",
                    params![self.origin, audit_index], |row| row.get(0),
                )?;
                let mut ids: Vec<_> = rows.into_iter().map(|row| row.0).collect();
                ids.sort();
                let mut digest = Sha256::new();
                for id in &ids { digest.update(id.as_bytes()); digest.update(b"\n"); }
                let through = if remaining { progress.through.max(covered_since) } else { now };
                let key = format!("audit-coverage:{}:{covered_since}:{now}:{audit_index}", self.origin);
                let mut fields = json!({"observed_at":now,"covered_since":covered_since,"through_at":through,
                    "audit_count":ids.len(),"audit_digest":hex::encode(digest.finalize()),
                    "audit_ids":ids,"audit_complete":!remaining});
                if let Some(previous) = &progress.previous {
                    fields["previous_coverage"] = json!(previous);
                }
                idle_residency::evidence(tx, &self.origin, "", "audit-coverage", &key,
                    "residency.audit-coverage", fields)?;
                let previous = tx.query_row(
                    "SELECT canonical_claim_id FROM operations WHERE id=?1 AND state='active'",
                    [operation_id_for_key(&format!("residency-evidence:{key}"))],
                    |row| row.get::<_, String>(0),
                )?;
                Ok(Some(CoverageProgress { since: Some(covered_since), through, audit_index,
                    previous: Some(previous), in_flight: false }))
            }).map_err(anyhow::Error::msg)?
        });
        match result {
            Ok(Some(next)) => { *progress = next; Ok(true) }
            Ok(None) => { *progress = CoverageProgress::default(); Ok(false) }
            Err(error) => { *progress = CoverageProgress::default(); Err(error) }
        }
    }
}
