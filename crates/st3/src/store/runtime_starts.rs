//! Owner-local launch admission and revocation. No replicated schema or receipt vocabulary.
use super::*;
use serde::Deserialize;
use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;

pub(super) fn create_schema(connection: &Connection) -> Result<()> {
    let transaction = connection.unchecked_transaction()?;
    transaction.execute_batch(r#"
CREATE TABLE IF NOT EXISTS local_runtime_starts (
    id TEXT PRIMARY KEY,
    subject TEXT NOT NULL,
    owner TEXT NOT NULL,
    body TEXT NOT NULL,
    revoked_by TEXT,
    finished INTEGER NOT NULL DEFAULT 0,
    result TEXT,
    applied INTEGER NOT NULL DEFAULT 0,
    cleanup_verified INTEGER NOT NULL DEFAULT 0,
    dispatched INTEGER NOT NULL DEFAULT 0,
    receipted INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS local_runtime_starts_subject ON local_runtime_starts(subject,id DESC);
CREATE INDEX IF NOT EXISTS local_runtime_starts_pending ON local_runtime_starts(id)
WHERE finished=0 OR applied=0 OR (revoked_by IS NOT NULL AND cleanup_verified=0)
 OR (json_extract(result,'$.error') IS NOT NULL AND cleanup_verified=0);
CREATE UNIQUE INDEX IF NOT EXISTS local_runtime_starts_active_subject ON local_runtime_starts(subject)
WHERE finished=0 OR applied=0 OR (revoked_by IS NOT NULL AND cleanup_verified=0)
 OR (json_extract(result,'$.error') IS NOT NULL AND cleanup_verified=0);
CREATE TABLE IF NOT EXISTS local_runtime_start_evidence (
    operation_id TEXT NOT NULL,
    observation_id INTEGER NOT NULL,
    PRIMARY KEY(operation_id, observation_id)
);
CREATE INDEX IF NOT EXISTS local_runtime_start_evidence_observation ON local_runtime_start_evidence(observation_id);
"#)?;
    // Capture the format boundary before admission can write any operation-bound
    // request. Never infer a later boundary from an already-used ledger.
    let has_cuts: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM meta WHERE key='publication-operation-format-shared-cut')
         AND EXISTS(SELECT 1 FROM meta WHERE key='publication-operation-format-local-cut')",
        [],
        |row| row.get(0),
    )?;
    if !has_cuts {
        let has_operations: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM local_runtime_starts)",
            [],
            |row| row.get(0),
        )?;
        anyhow::ensure!(
            !has_operations,
            "runtime-start ledger has operations but lacks immutable publication format cuts; preserve this store and roll forward with its original format-boundary evidence"
        );
        transaction.execute_batch(
            "INSERT OR IGNORE INTO meta(key,value)
             SELECT 'publication-operation-format-shared-cut', CAST(COALESCE(MAX(store_index),0) AS TEXT) FROM claims;
             INSERT OR IGNORE INTO meta(key,value)
             SELECT 'publication-operation-format-local-cut', CAST(COALESCE(MAX(id),0) AS TEXT) FROM local_observations;",
        )?;
    }
    transaction.commit()?;
    Ok(())
}

const PENDING: &str = "finished=0 OR applied=0 OR (revoked_by IS NOT NULL AND cleanup_verified=0) OR (json_extract(result,'$.error') IS NOT NULL AND cleanup_verified=0)";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeStartOperation {
    pub id: String,
    pub subject: String,
    pub owner: String,
    pub body: Value,
    pub revoked_by: Option<String>,
    pub finished: bool,
    pub result: Option<Value>,
    pub applied: bool,
    pub cleanup_verified: bool,
    pub dispatched: bool,
    pub receipted: bool,
}

/// Nonblocking process-death-safe admission. Controls never acquire this lock.
pub struct RuntimeStartLock(File);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeStartAdmission {
    Admitted,
    SeatBusy,
    CapacityBusy,
    Superseded,
}

fn read(row: &rusqlite::Row<'_>) -> rusqlite::Result<RuntimeStartOperation> {
    let decode = |column| -> rusqlite::Result<Value> {
        let text: String = row.get(column)?;
        serde_json::from_str(&text).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                column,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })
    };
    let result: Option<String> = row.get(6)?;
    Ok(RuntimeStartOperation {
        id: row.get(0)?,
        subject: row.get(1)?,
        owner: row.get(2)?,
        body: decode(3)?,
        revoked_by: row.get(4)?,
        finished: row.get(5)?,
        result: result
            .map(|text| serde_json::from_str(&text))
            .transpose()
            .map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    6,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?,
        applied: row.get(7)?,
        cleanup_verified: row.get(8)?,
        dispatched: row.get(9)?,
        receipted: row.get(10)?,
    })
}

fn pin_evidence(tx: &Transaction<'_>, operation: &str, origin: &str, evidence: &str) -> Result<()> {
    if let Some(position) =
        evidence.strip_prefix(&format!("{LOCAL_OBSERVATION_ID_PREFIX}{origin}/"))
    {
        let position: i64 = position.parse()?;
        tx.execute("INSERT OR IGNORE INTO local_runtime_start_evidence(operation_id,observation_id) VALUES (?1,?2)",
            params![operation, position])?;
    }
    Ok(())
}

fn release_evidence(tx: &Transaction<'_>, operation: &str) -> rusqlite::Result<()> {
    tx.execute(
        "DELETE FROM local_runtime_start_evidence WHERE operation_id=?1 AND EXISTS (
        SELECT 1 FROM local_runtime_starts WHERE id=?1 AND finished=1 AND applied=1
          AND (revoked_by IS NULL OR cleanup_verified=1)
          AND (json_extract(result,'$.error') IS NULL OR cleanup_verified=1))",
        [operation],
    )?;
    Ok(())
}

impl Store {
    /// Revocation, declaration selection and all launch receipts linearize in one short
    /// writer transaction. A stale completion cannot leave a success/starting receipt.
    pub(crate) fn apply_runtime_start_result(
        &self,
        id: &str,
        inputs: &[ClaimInput],
    ) -> Result<bool> {
        for input in inputs {
            anyhow::ensure!(
                local_retention(&input.kind)
                    || (input.actor.is_none()
                        && (system_local_retention(&input.kind)
                            || input.kind == "runtime.observed")),
                "launch completion uses local action receipts and the existing runtime observation"
            );
            validate_local_observation(input)?;
        }
        self.connection
            .batched(|tx| -> Result<bool> {
                let operation =
                    tx.query_row("SELECT * FROM local_runtime_starts WHERE id=?1", [id], read)?;
                if operation.revoked_by.is_some() || operation.applied || !operation.finished {
                    return Ok(false);
                }
                if operation.result.as_ref().is_some_and(|result| {
                    !result["error"].is_string()
                        && result["verification"] != "operation-incarnation"
                }) {
                    return Ok(false);
                }
                let selected =
                    current_desired_row_tx(tx, &operation.subject)?.map(|row| row.claim_id);
                let expected = operation.body["selected_token"].as_str().map(str::to_owned);
                if selected != expected {
                    return Ok(false);
                }
                let result = operation
                    .result
                    .as_ref()
                    .context("finished start has no result")?;
                for input in inputs {
                    if input.subject != operation.subject {
                        return Ok(false);
                    }
                    if input
                        .fields
                        .get("action")
                        .is_some_and(|action| action == "start")
                    {
                        if input
                            .fields
                            .get("operation")
                            .is_some_and(|bound| bound != id)
                            || input.fields.get("runtime_id").is_some_and(|bound| {
                                bound != &operation.body["member"]["runtime_id"]
                            })
                            || input
                                .fields
                                .get("terminal")
                                .is_some_and(|bound| bound != &operation.body["member"]["terminal"])
                            || input
                                .fields
                                .get("desired_token")
                                .is_some_and(|bound| bound != &operation.body["desired_token"])
                        {
                            return Ok(false);
                        }
                        if input.kind == "runtime.action.succeeded"
                            && (result["error"].is_string()
                                || result["verification"] != "operation-incarnation"
                                || !result["incarnation"].is_string()
                                || !operation.body["member"]["runtime_id"].is_string()
                                || !operation.body["member"]["terminal"].is_boolean()
                                || !operation.body["desired_token"].is_string()
                                || input
                                    .fields
                                    .get("incarnation_id")
                                    .is_some_and(|bound| bound != &result["incarnation"]))
                        {
                            return Ok(false);
                        }
                        if input.kind == "runtime.action.failed" && !result["error"].is_string() {
                            return Ok(false);
                        }
                    }
                }
                if operation.receipted {
                    return Ok(true);
                }
                let receipts = inputs
                    .iter()
                    .map(|input| -> Result<String> {
                        let mut input = input.clone();
                        if input
                            .fields
                            .get("action")
                            .is_some_and(|action| action == "start")
                        {
                            input.fields.insert("operation".into(), Value::from(id));
                            if let Some(runtime) = operation.body["member"]["runtime_id"].as_str() {
                                input
                                    .fields
                                    .insert("runtime_id".into(), Value::from(runtime));
                            }
                            if let Some(terminal) = operation.body["member"]["terminal"].as_bool() {
                                input
                                    .fields
                                    .insert("terminal".into(), Value::from(terminal));
                            }
                            if let Some(token) = operation.body["desired_token"].as_str() {
                                input
                                    .fields
                                    .insert("desired_token".into(), Value::from(token));
                            }
                            input.fields.remove("operation_status");
                            if input.kind == "runtime.action.succeeded" {
                                input
                                    .fields
                                    .insert("operation_status".into(), Value::from("verified"));
                                input
                                    .fields
                                    .insert("incarnation_id".into(), result["incarnation"].clone());
                            } else if input.kind == "runtime.action.failed"
                                && result["verification"] == "producer-closed"
                                && result["producer_closed"] == true
                            {
                                input.fields.insert(
                                    "operation_status".into(),
                                    Value::from("producer-closed"),
                                );
                            }
                            let request = operation.body["start_request"]
                                .as_str()
                                .context("start result has no original request")?;
                            if !input.evidence.iter().any(|evidence| evidence == request) {
                                input.evidence.push(request.into());
                            }
                            validate_local_observation(&input)?;
                        }
                        let claim = if input.kind == "runtime.observed" {
                            let mut predecessors = latest_claim_id_tx(tx, &input.subject)?
                                .into_iter()
                                .collect::<Vec<_>>();
                            for evidence in &input.evidence {
                                if claim_by_id_tx(tx, evidence)?
                                    .is_some_and(|claim| claim.subject == input.subject)
                                {
                                    predecessors.push(evidence.clone());
                                }
                            }
                            predecessors.sort();
                            predecessors.dedup();
                            append_receipt_claim_tx(
                                tx,
                                self.origin(),
                                &input.subject,
                                &input.kind,
                                input.actor.as_deref(),
                                &json!({"fields": input.fields, "evidence": input.evidence}),
                                &predecessors,
                                None,
                                input
                                    .idempotency_key
                                    .as_deref()
                                    .context("runtime start observation needs its operation key")?,
                            )?
                        } else {
                            insert_local_observation_tx(tx, self.origin(), &input, now_ms())?.0
                        };
                        pin_evidence(tx, id, self.origin(), &claim.id)?;
                        Ok(claim.id)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let mut result = operation.result.context("finished start has no result")?;
                result["result_receipts"] = serde_json::to_value(receipts)?;
                tx.execute(
                    "UPDATE local_runtime_starts SET receipted=1,result=?2 WHERE id=?1",
                    params![id, serde_json::to_string(&result)?],
                )?;
                Ok(true)
            })
            .map_err(|error| anyhow::anyhow!("{error}"))?
    }
    pub fn try_runtime_start_lock(&self, subject: &str) -> Result<Option<RuntimeStartLock>> {
        let directory = if self.shared_memory {
            std::env::temp_dir().join("st3-test-start-locks").join(
                smallclaims::hash::canonical_hash(&self.path.to_string_lossy())?,
            )
        } else {
            self.path.with_extension("runtime-start-locks")
        };
        std::fs::create_dir_all(&directory)?;
        let path = directory.join(smallclaims::hash::canonical_hash(&subject)?);
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(path)?;
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            return Ok(Some(RuntimeStartLock(file)));
        }
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::WouldBlock {
            return Ok(None);
        }
        Err(error.into())
    }

    pub fn runtime_start_for(&self, subject: &str) -> Result<Option<RuntimeStartOperation>> {
        smallclaims::touched::note_read(|| format!("local-runtime-start:{subject}"));
        self.readers.get().query_row(
            &format!("SELECT * FROM local_runtime_starts WHERE subject=?1 AND ({PENDING}) ORDER BY id DESC LIMIT 1"),
            [subject], read,
        ).optional().map_err(Into::into)
    }

    pub fn pending_runtime_starts(&self) -> Result<Vec<RuntimeStartOperation>> {
        let connection = self.readers.get();
        let mut statement = connection.prepare_cached(&format!(
            "SELECT * FROM local_runtime_starts WHERE {PENDING} ORDER BY id"
        ))?;
        statement
            .query_map([], read)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn admit_runtime_start(
        &self,
        operation: &mut RuntimeStartOperation,
        limit: usize,
    ) -> Result<RuntimeStartAdmission> {
        self.connection.batched(|tx| -> Result<RuntimeStartAdmission> {
            let selected = current_desired_row_tx(tx, &operation.subject)?.map(|row| row.claim_id);
            if selected.as_deref() != operation.body["selected_token"].as_str() {
                return Ok(RuntimeStartAdmission::Superseded);
            }
            let busy: bool = tx.query_row(
                &format!("SELECT EXISTS(SELECT 1 FROM local_runtime_starts WHERE subject=?1 AND ({PENDING}))"),
                [&operation.subject], |row| row.get(0))?;
            if busy { return Ok(RuntimeStartAdmission::SeatBusy); }
            let mut fields = BTreeMap::from([
                ("action".into(), Value::from("start")),
                ("operation".into(), Value::from(operation.id.clone())),
            ]);
            if let Some(runtime) = operation.body.pointer("/member/runtime_id").and_then(Value::as_str) {
                fields.insert("runtime_id".into(), Value::from(runtime));
            }
            if let Some(terminal) = operation.body.pointer("/member/terminal").and_then(Value::as_bool) {
                fields.insert("terminal".into(), Value::from(terminal));
            }
            if let Some(incarnation) = operation.body["expected_incarnation"].as_str() {
                fields.insert("incarnation_id".into(), Value::from(incarnation));
            }
            let request = ClaimInput { subject: operation.subject.clone(), kind: "runtime.action.requested".into(), actor: None,
                fields, evidence: selected.into_iter().collect(), expected_subject: None,
                idempotency_key: Some(format!("start-request:{}", operation.id)) };
            validate_local_observation(&request)?;
            let (request, _) = insert_local_observation_tx(tx, self.origin(), &request, now_ms())?;
            operation.body["start_request"] = Value::from(request.id);
            let body = serde_json::to_string(&operation.body)?;
            tx.execute("INSERT INTO local_runtime_starts(id,subject,owner,body) VALUES (?1,?2,?3,?4)",
                params![operation.id, operation.subject, operation.owner, body])?;
            if let Some(request) = operation.body["start_request"].as_str() {
                pin_evidence(tx, &operation.id, self.origin(), request)?;
            }
            let running: usize = tx.query_row("SELECT COUNT(*) FROM local_runtime_starts WHERE finished=0 AND dispatched=1", [], |row| row.get(0))?;
            if running >= limit { return Ok(RuntimeStartAdmission::CapacityBusy); }
            Ok(RuntimeStartAdmission::Admitted)
        }).map_err(|error| anyhow::anyhow!("{error}"))?
    }

    pub fn dispatch_runtime_start(&self, id: &str, limit: usize) -> Result<bool> {
        self.connection.batched(|tx| -> Result<bool> {
            let running: usize = tx.query_row("SELECT COUNT(*) FROM local_runtime_starts WHERE finished=0 AND dispatched=1", [], |row| row.get(0))?;
            if running >= limit { return Ok(false); }
            Ok(tx.execute("UPDATE local_runtime_starts SET dispatched=1 WHERE id=?1 AND dispatched=0 AND finished=0 AND revoked_by IS NULL", [id])? == 1)
        }).map_err(|error| anyhow::anyhow!("{error}"))?
    }

    pub fn runtime_start_failure(&self, subject: &str) -> Result<Option<String>> {
        smallclaims::touched::note_read(|| format!("local-runtime-start:{subject}"));
        let operation = self
            .readers
            .get()
            .query_row(
                "SELECT * FROM local_runtime_starts WHERE subject=?1 ORDER BY id DESC LIMIT 1",
                [subject],
                read,
            )
            .optional()?;
        let Some(operation) = operation else {
            return Ok(None);
        };
        if !operation.finished
            || operation.revoked_by.is_some()
            || operation.body["selected_token"].as_str()
                != self.selected_desired_token(subject)?.as_deref()
        {
            return Ok(None);
        }
        Ok(operation
            .result
            .and_then(|body| body["error"].as_str().map(str::to_owned)))
    }

    pub fn runtime_start_counts(&self) -> Result<(u64, u64)> {
        self.readers.get().query_row(
            "SELECT COUNT(*), COALESCE(SUM(revoked_by IS NOT NULL),0) FROM local_runtime_starts WHERE finished=0 AND dispatched=1",
            [], |row| Ok((row.get(0)?, row.get(1)?))).map_err(Into::into)
    }

    pub fn unresolved_runtime_start_counts(&self) -> Result<(u64, u64)> {
        self.readers.get().query_row(
            &format!("SELECT COUNT(*), COALESCE(SUM(revoked_by IS NOT NULL),0) FROM local_runtime_starts WHERE {PENDING}"),
            [], |row| Ok((row.get(0)?, row.get(1)?)),
        ).map_err(Into::into)
    }

    pub fn runtime_start_revoked(&self, id: &str) -> Result<bool> {
        self.readers
            .get()
            .query_row(
                "SELECT revoked_by IS NOT NULL FROM local_runtime_starts WHERE id=?1",
                [id],
                |row| row.get(0),
            )
            .map_err(Into::into)
    }

    pub fn revoke_runtime_start(&self, id: &str, request: &str) -> Result<()> {
        self.connection.batched(|tx| -> Result<()> {
            tx.execute("UPDATE local_runtime_starts SET revoked_by=COALESCE(revoked_by,?2) WHERE id=?1", params![id, request])?;
            pin_evidence(tx, id, self.origin(), request)?;
            Ok(())
        })
            .map_err(|error| anyhow::anyhow!("{error}"))??;
        Ok(())
    }

    pub fn finish_runtime_start(&self, id: &str, result: &Value) -> Result<()> {
        let result = serde_json::to_string(result)?;
        self.connection.batched(|tx| tx.execute(
            "UPDATE local_runtime_starts SET finished=1,result=?2,
             cleanup_verified=CASE WHEN json_extract(?2,'$.error') IS NOT NULL AND json_extract(?2,'$.producer_closed')=1 THEN 1 ELSE cleanup_verified END
             WHERE id=?1 AND finished=0", params![id, result]))
            .map_err(|error| anyhow::anyhow!("{error}"))??;
        Ok(())
    }

    pub(crate) fn verify_runtime_start(&self, id: &str, incarnation: &str) -> Result<bool> {
        self.connection.batched(|tx| -> Result<bool> {
            Ok(tx.execute(
                "UPDATE local_runtime_starts SET result=json_set(result,
                    '$.incarnation',?2,'$.verification','operation-incarnation','$.phase','completed')
                 WHERE id=?1 AND finished=1 AND applied=0 AND receipted=0
                    AND revoked_by IS NULL AND json_extract(result,'$.error') IS NULL",
                params![id, incarnation])? == 1)
        }).map_err(|error| anyhow::anyhow!("{error}"))?
    }

    pub(crate) fn close_unverified_runtime_start(&self, id: &str) -> Result<bool> {
        self.connection.batched(|tx| -> Result<bool> {
            Ok(tx.execute(
                "UPDATE local_runtime_starts SET cleanup_verified=1, result=json_set(result,
                    '$.error','producer closed without a verified launch',
                    '$.producer_closed',json('true'),'$.verification','producer-closed','$.phase','failed')
                 WHERE id=?1 AND finished=1 AND applied=0 AND receipted=0
                    AND revoked_by IS NULL AND json_extract(result,'$.error') IS NULL",
                [id])? == 1)
        }).map_err(|error| anyhow::anyhow!("{error}"))?
    }

    pub fn runtime_start_applied(&self, id: &str) -> Result<()> {
        self.connection
            .batched(|tx| {
                tx.execute(
                    "UPDATE local_runtime_starts SET applied=1 WHERE id=?1 AND finished=1
                        AND (revoked_by IS NOT NULL OR json_extract(result,'$.error') IS NOT NULL
                            OR json_extract(result,'$.verification')='operation-incarnation')",
                    [id],
                )?;
                release_evidence(tx, id)
            })
            .map_err(|error| anyhow::anyhow!("{error}"))??;
        Ok(())
    }

    /// Commit a positive, exact-incarnation physical retirement receipt and its
    /// ledger proof together. This is never called for a signal-only stop.
    pub(crate) fn apply_runtime_start_retirement(
        &self,
        id: &str,
        incarnation: &str,
        request: &str,
    ) -> Result<bool> {
        self.connection.batched(|tx| -> Result<bool> {
            let operation = tx.query_row("SELECT * FROM local_runtime_starts WHERE id=?1", [id], read)?;
            if !operation.finished || operation.revoked_by.is_none() || operation.cleanup_verified {
                return Ok(false);
            }
            let Some(position) = request.strip_prefix(&format!("{LOCAL_OBSERVATION_ID_PREFIX}{}/", self.origin())) else {
                return Ok(false);
            };
            let requested = tx.query_row(
                "SELECT id,after_store_index,subject,kind,actor,body,observed_at_unix_ms FROM local_observations WHERE id=?1",
                [position], |row| local_observation_from_row(self.origin(), row),
            ).optional()?;
            let Some(requested) = requested else { return Ok(false) };
            let member = &operation.body["member"];
            if requested.id != request || requested.subject != operation.subject || requested.kind != "runtime.action.requested"
                || requested.body["fields"]["action"] != "terminate"
                || requested.body["fields"]["runtime_id"] != member["runtime_id"]
                || requested.body["fields"]["terminal"] != member["terminal"]
                || requested.body["fields"]["incarnation_id"] != incarnation
                || requested.body["fields"]["operation"].as_str().is_some_and(|bound| bound != id) {
                return Ok(false);
            }
            let mut input = ClaimInput {
                subject: operation.subject.clone(), kind: "runtime.action.succeeded".into(), actor: None,
                fields: BTreeMap::from([
                    ("action".into(), Value::from("terminate")),
                    ("operation".into(), Value::from(id)),
                    ("runtime_id".into(), member["runtime_id"].clone()),
                    ("terminal".into(), member["terminal"].clone()),
                    ("incarnation_id".into(), Value::from(incarnation)),
                    ("operation_status".into(), Value::from("retired")),
                ]),
                evidence: vec![request.into(), operation.body["start_request"].as_str().context("retirement has no launch request")?.into()],
                expected_subject: None, idempotency_key: Some(format!("start-retirement:{id}:{incarnation}")),
            };
            if let Some(token) = operation.body["desired_token"].as_str() {
                input.fields.insert("desired_token".into(), Value::from(token));
            }
            validate_local_observation(&input)?;
            let receipt = insert_local_observation_tx(tx, self.origin(), &input, now_ms())?.0;
            let mut result = operation.result.context("retired operation has no launch result")?;
            result["incarnation"] = Value::from(incarnation);
            result["verification"] = Value::from("operation-retired");
            result["phase"] = Value::from("retired");
            result["retirement_request"] = Value::from(request);
            result["retirement_result"] = Value::from(receipt.id);
            tx.execute("UPDATE local_runtime_starts SET cleanup_verified=1,result=?2 WHERE id=?1",
                params![id, serde_json::to_string(&result)?])?;
            release_evidence(tx, id)?;
            Ok(true)
        }).map_err(|error| anyhow::anyhow!("{error}"))?
    }

    pub fn runtime_start_cleanup_verified(&self, id: &str) -> Result<()> {
        self.connection
            .batched(|tx| {
                tx.execute(
                    "UPDATE local_runtime_starts SET cleanup_verified=1 WHERE id=?1",
                    [id],
                )?;
                release_evidence(tx, id)
            })
            .map_err(|error| anyhow::anyhow!("{error}"))??;
        Ok(())
    }
}

impl Drop for RuntimeStartLock {
    fn drop(&mut self) {
        // Explicit unlock: forked native subprocesses must not retain admission after
        // the owner's job exits. The descriptor itself is opened CLOEXEC by Rust.
        unsafe {
            libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn operation(id: &str, subject: &str, token: Option<String>) -> RuntimeStartOperation {
        RuntimeStartOperation {
            id: id.into(),
            subject: subject.into(),
            owner: "owner".into(),
            body: serde_json::json!({"selected_token": token, "desired_token":"desired-test", "member":{"runtime_id":subject,"terminal":true}}),
            revoked_by: None,
            finished: false,
            result: None,
            applied: false,
            cleanup_verified: false,
            dispatched: false,
            receipted: false,
        }
    }

    fn receipt(subject: &str) -> ClaimInput {
        ClaimInput {
            subject: subject.into(),
            kind: "runtime.action.succeeded".into(),
            actor: None,
            fields: BTreeMap::from([("action".into(), Value::from("start"))]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: Some(format!("test-start:{subject}")),
        }
    }

    fn publish(store: &Store, source: &str, key: &str) {
        let intent = crate::graph::parse_test_intent(source, "node").unwrap();
        let preview = store
            .mission(
                &intent,
                crate::model::IntentInput {
                    kdl: source.into(),
                    source_name: None,
                },
            )
            .unwrap();
        store.apply(&intent, &preview.subject_tokens, key).unwrap();
    }

    #[test]
    fn retirement_proof_requires_the_exact_subject_operation_request_and_incarnation() {
        let store = Store::open_memory("node").unwrap();
        let subject = "agent/example/worker";
        let mut admitted = operation("operation-a", subject, None);
        admitted.body["member"] = json!({"runtime_id":"example.worker", "terminal":true});
        assert_eq!(
            store.admit_runtime_start(&mut admitted, 2).unwrap(),
            RuntimeStartAdmission::Admitted
        );
        store
            .finish_runtime_start(
                &admitted.id,
                &json!({"error":"producer returned", "producer_closed":false}),
            )
            .unwrap();
        let request = |seat: &str, operation: &str, incarnation: &str, key: &str| {
            store
                .append_claim(&ClaimInput {
                    subject: seat.into(),
                    kind: "runtime.action.requested".into(),
                    actor: None,
                    fields: BTreeMap::from([
                        ("action".into(), Value::from("terminate")),
                        ("operation".into(), Value::from(operation)),
                        ("runtime_id".into(), Value::from("example.worker")),
                        ("terminal".into(), Value::from(true)),
                        ("incarnation_id".into(), Value::from(incarnation)),
                    ]),
                    evidence: vec![],
                    expected_subject: None,
                    idempotency_key: Some(key.into()),
                })
                .unwrap()
        };
        let correct = request(subject, &admitted.id, "incarnation-a", "correct-retirement");
        store
            .revoke_runtime_start(&admitted.id, &correct.id)
            .unwrap();
        store.runtime_start_applied(&admitted.id).unwrap();
        let foreign_seat = request(
            "agent/example/other",
            &admitted.id,
            "incarnation-a",
            "foreign-seat",
        );
        let foreign_operation =
            request(subject, "operation-b", "incarnation-a", "foreign-operation");
        assert!(
            !store
                .apply_runtime_start_retirement(&admitted.id, "incarnation-a", &foreign_seat.id)
                .unwrap()
        );
        assert!(
            !store
                .apply_runtime_start_retirement(
                    &admitted.id,
                    "incarnation-a",
                    &foreign_operation.id
                )
                .unwrap()
        );
        assert!(
            !store
                .apply_runtime_start_retirement(&admitted.id, "incarnation-b", &correct.id)
                .unwrap()
        );
        assert_eq!(store.unresolved_runtime_start_counts().unwrap(), (1, 1));
        assert!(
            store
                .observations_for(subject, "runtime.action.succeeded")
                .unwrap()
                .is_empty()
        );
        assert!(
            store
                .apply_runtime_start_retirement(&admitted.id, "incarnation-a", &correct.id)
                .unwrap()
        );
        assert_eq!(store.unresolved_runtime_start_counts().unwrap(), (0, 0));
        let result: String = store
            .readers
            .get()
            .query_row(
                "SELECT result FROM local_runtime_starts WHERE id=?1",
                [&admitted.id],
                |row| row.get(0),
            )
            .unwrap();
        let result: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(result["incarnation"], "incarnation-a");
        assert_eq!(result["verification"], "operation-retired");
        assert_eq!(result["retirement_request"], correct.id);
        assert_eq!(
            store
                .observations_for(subject, "runtime.action.succeeded")
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn verified_start_receipts_bind_writer_proof_and_refuse_conflicting_inputs() {
        let store = Store::open_memory("node").unwrap();
        let subject = "agent/example/worker";
        let mut start = operation("operation-a", subject, None);
        start.body["expected_incarnation"] = Value::from("prior-incarnation");
        store.admit_runtime_start(&mut start, 2).unwrap();
        store.finish_runtime_start(&start.id, &json!({"incarnation":"actual-incarnation", "verification":"operation-incarnation"})).unwrap();
        for (field, value) in [
            ("operation", "another-operation"),
            ("incarnation_id", "another-incarnation"),
            ("runtime_id", "another-runtime"),
            ("desired_token", "another-declaration"),
        ] {
            let mut wrong = receipt(subject);
            wrong.fields.insert(field.into(), Value::from(value));
            assert!(
                !store
                    .apply_runtime_start_result(&start.id, &[wrong])
                    .unwrap()
            );
        }
        assert!(
            !store
                .apply_runtime_start_result(&start.id, &[receipt("agent/example/other")])
                .unwrap()
        );
        assert!(
            store
                .observations_for(subject, "runtime.action.succeeded")
                .unwrap()
                .is_empty()
        );
        assert!(
            store
                .apply_runtime_start_result(&start.id, &[receipt(subject)])
                .unwrap()
        );
        let completed = store
            .observations_for(subject, "runtime.action.succeeded")
            .unwrap()
            .remove(0);
        assert_eq!(completed.body["fields"]["operation_status"], "verified");
        assert_eq!(completed.body["fields"]["operation"], start.id);
        assert_eq!(completed.body["fields"]["runtime_id"], subject);
        assert_eq!(completed.body["fields"]["terminal"], true);
        assert_eq!(
            completed.body["fields"]["incarnation_id"],
            "actual-incarnation"
        );
        assert_eq!(completed.body["fields"]["desired_token"], "desired-test");
        let requested = store
            .observations_for(subject, "runtime.action.requested")
            .unwrap()
            .remove(0);
        assert_eq!(requested.body["fields"]["terminal"], true);
        assert_eq!(
            requested.body["fields"]["incarnation_id"],
            "prior-incarnation"
        );
        assert!(
            completed.body["evidence"]
                .as_array()
                .unwrap()
                .contains(&Value::from(requested.id))
        );
    }

    #[test]
    fn ordinary_failed_starts_cannot_publish_success_or_forge_closure_status() {
        for closed in [false, true] {
            let store = Store::open_memory("node").unwrap();
            let subject = "agent/example/worker";
            let mut start = operation("operation-a", subject, None);
            store.admit_runtime_start(&mut start, 2).unwrap();
            store.finish_runtime_start(&start.id, &json!({"error":"producer returned", "producer_closed":closed, "verification":if closed {"producer-closed"} else {"producer-open-unverified"}})).unwrap();
            assert!(
                !store
                    .apply_runtime_start_result(&start.id, &[receipt(subject)])
                    .unwrap()
            );
            let mut failed = receipt(subject);
            failed.kind = "runtime.action.failed".into();
            failed
                .fields
                .insert("operation_status".into(), Value::from("producer-closed"));
            assert!(
                store
                    .apply_runtime_start_result(&start.id, &[failed])
                    .unwrap()
            );
            let completed = store
                .observations_for(subject, "runtime.action.failed")
                .unwrap()
                .remove(0);
            assert_eq!(
                completed.body["fields"]["operation_status"],
                if closed {
                    Value::from("producer-closed")
                } else {
                    Value::Null
                }
            );
            assert_eq!(completed.body["fields"]["operation"], start.id);
            assert_eq!(completed.body["fields"]["runtime_id"], subject);
            assert!(
                store
                    .observations_for(subject, "runtime.action.succeeded")
                    .unwrap()
                    .is_empty()
            );
        }
    }

    #[test]
    fn declaration_and_revocation_fence_admission_and_completion_atomically() {
        let store = Store::open_memory("node").unwrap();
        let subject = "agent/example/worker";
        publish(
            &store,
            "version 2\nagent \"example/worker\" { workspace \"/tmp\"; command \"true\" }",
            "initial",
        );
        let token = store.selected_desired_token(subject).unwrap();
        let first = operation("one", subject, token.clone());
        assert_eq!(
            store.admit_runtime_start(&mut first.clone(), 2).unwrap(),
            RuntimeStartAdmission::Admitted
        );
        assert!(store.dispatch_runtime_start("one", 2).unwrap());
        store
            .finish_runtime_start("one", &serde_json::json!({"incarnation": "one"}))
            .unwrap();
        store
            .revoke_runtime_start("one", "durable-stop-request")
            .unwrap();
        assert!(
            !store
                .apply_runtime_start_result("one", &[receipt(subject)])
                .unwrap()
        );
        assert!(
            store
                .observations_for(subject, "runtime.action.succeeded")
                .unwrap()
                .is_empty()
        );
        store.runtime_start_applied("one").unwrap();
        store.runtime_start_cleanup_verified("one").unwrap();
        let second = operation("two", subject, token.clone());
        assert_eq!(
            store.admit_runtime_start(&mut second.clone(), 2).unwrap(),
            RuntimeStartAdmission::Admitted
        );
        store
            .finish_runtime_start("two", &serde_json::json!({"incarnation": "two"}))
            .unwrap();
        publish(&store, "version 2\nstop \"agent/example/worker\"", "stop");
        assert!(
            !store
                .apply_runtime_start_result("two", &[receipt(subject)])
                .unwrap()
        );
        assert_eq!(
            store
                .admit_runtime_start(&mut operation("three", "agent/example/new", token), 2)
                .unwrap(),
            RuntimeStartAdmission::Superseded
        );
        assert!(
            store
                .observations_for(subject, "runtime.action.succeeded")
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn accepted_completion_receipts_are_idempotent_before_context_is_applied() {
        let store = Store::open_memory("node").unwrap();
        let subject = "agent/example/worker";
        let start = operation("one", subject, None);
        store.admit_runtime_start(&mut start.clone(), 2).unwrap();
        store
            .finish_runtime_start("one", &serde_json::json!({"incarnation": null, "verification": "launch-accepted-unverified"}))
            .unwrap();
        assert!(
            !store
                .apply_runtime_start_result("one", &[receipt(subject)])
                .unwrap()
        );
        assert!(store.runtime_start_for(subject).unwrap().is_some());
        assert!(store.verify_runtime_start("one", "one").unwrap());
        assert!(
            store
                .apply_runtime_start_result("one", &[receipt(subject)])
                .unwrap()
        );
        assert!(
            store
                .apply_runtime_start_result("one", &[receipt(subject)])
                .unwrap()
        );
        assert_eq!(
            store
                .observations_for(subject, "runtime.action.succeeded")
                .unwrap()
                .len(),
            1
        );
        assert!(store.runtime_start_for(subject).unwrap().is_some());
        store.runtime_start_applied("one").unwrap();
        assert!(store.runtime_start_for(subject).unwrap().is_none());
    }

    #[test]
    fn an_ordinary_backend_error_does_not_release_launch_admission() {
        let store = Store::open_memory("node").unwrap();
        let subject = "agent/example/worker";
        let mut start = operation("one", subject, None);
        store.admit_runtime_start(&mut start, 2).unwrap();
        store
            .finish_runtime_start(
                "one",
                &json!({"error":"producer unverified", "producer_closed":false}),
            )
            .unwrap();
        store.runtime_start_applied("one").unwrap();
        assert!(store.runtime_start_for(subject).unwrap().is_some());
        assert_eq!(store.unresolved_runtime_start_counts().unwrap(), (1, 0));
        assert_eq!(
            store
                .admit_runtime_start(&mut operation("two", subject, None), 2)
                .unwrap(),
            RuntimeStartAdmission::SeatBusy
        );
        store.runtime_start_cleanup_verified("one").unwrap();
        assert!(store.runtime_start_for(subject).unwrap().is_none());
        assert_eq!(store.unresolved_runtime_start_counts().unwrap(), (0, 0));
    }

    #[test]
    fn the_kernel_releases_nonblocking_admission_when_its_owner_process_dies() {
        use std::os::unix::ffi::OsStrExt;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("claims.db");
        let store = Store::open(&path, "node").unwrap();
        let subject = "agent/example/worker";
        drop(store.try_runtime_start_lock(subject).unwrap().unwrap());
        let lock_path = std::ffi::CString::new(
            path.with_extension("runtime-start-locks")
                .join(smallclaims::hash::canonical_hash(&subject).unwrap())
                .as_os_str()
                .as_bytes(),
        )
        .unwrap();
        let mut pipe = [0; 2];
        assert_eq!(unsafe { libc::pipe(pipe.as_mut_ptr()) }, 0);
        let pid = unsafe { libc::fork() };
        assert!(pid >= 0);
        if pid == 0 {
            // Only async-signal-safe syscalls after fork of the threaded Store owner.
            unsafe {
                libc::close(pipe[0]);
                let fd = libc::open(lock_path.as_ptr(), libc::O_RDWR | libc::O_CLOEXEC);
                let acquired =
                    u8::from(fd >= 0 && libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) == 0);
                libc::write(pipe[1], (&acquired as *const u8).cast(), 1);
                libc::close(pipe[1]);
                loop {
                    libc::pause();
                }
            }
        }
        struct Child(libc::pid_t);
        impl Drop for Child {
            fn drop(&mut self) {
                unsafe {
                    libc::kill(self.0, libc::SIGKILL);
                    libc::waitpid(self.0, std::ptr::null_mut(), 0);
                }
            }
        }
        let child = Child(pid);
        unsafe {
            libc::close(pipe[1]);
        }
        let mut acquired = 0u8;
        assert_eq!(
            unsafe { libc::read(pipe[0], (&mut acquired as *mut u8).cast(), 1) },
            1
        );
        unsafe {
            libc::close(pipe[0]);
        }
        assert_eq!(acquired, 1);
        assert!(store.try_runtime_start_lock(subject).unwrap().is_none());
        drop(child);
        assert!(store.try_runtime_start_lock(subject).unwrap().is_some());
    }

    #[test]
    fn trimming_preserves_unresolved_start_and_revocation_evidence() {
        let store = Store::open_memory("node").unwrap();
        let subject = "agent/example/worker";
        let request = |action: &str| {
            store
                .append_claim(&ClaimInput {
                    subject: subject.into(),
                    kind: "runtime.action.requested".into(),
                    actor: None,
                    fields: BTreeMap::from([("action".into(), Value::from(action))]),
                    evidence: Vec::new(),
                    expected_subject: None,
                    idempotency_key: None,
                })
                .unwrap()
        };
        let mut prepared = operation("one", subject, None);
        store.admit_runtime_start(&mut prepared, 2).unwrap();
        let start = prepared.body["start_request"].as_str().unwrap().to_owned();
        let revoke = request("terminate");
        store.revoke_runtime_start("one", &revoke.id).unwrap();
        for _ in 0..8 {
            request("start");
        }
        store
            .finish_runtime_start("one", &serde_json::json!({"error": "producer unverified"}))
            .unwrap();
        store.runtime_start_applied("one").unwrap();
        store.trim_local_observations(now_ms() + 1, 1, 2).unwrap();
        let retained = store
            .observations_for(subject, "runtime.action.requested")
            .unwrap();
        assert!(retained.iter().any(|r| r.id == start));
        assert!(retained.iter().any(|r| r.id == revoke.id));
        store.runtime_start_cleanup_verified("one").unwrap();
        store.trim_local_observations(now_ms() + 1, 1, 2).unwrap();
        let retained = store
            .observations_for(subject, "runtime.action.requested")
            .unwrap();
        assert!(!retained.iter().any(|r| r.id == start || r.id == revoke.id));
        let resolved = store
            .readers
            .get()
            .query_row(
                "SELECT * FROM local_runtime_starts WHERE id='one'",
                [],
                read,
            )
            .unwrap();
        assert!(resolved.finished && resolved.applied && resolved.cleanup_verified);
        assert_eq!(resolved.body["start_request"], start);
        assert_eq!(resolved.revoked_by.as_deref(), Some(revoke.id.as_str()));
        assert_eq!(resolved.result.unwrap()["error"], "producer unverified");
    }

    fn format_cuts(connection: &Connection) -> (i64, i64) {
        let read_cut = |key| {
            connection
                .query_row(
                    "SELECT CAST(value AS INTEGER) FROM meta WHERE key=?1",
                    [key],
                    |row| row.get(0),
                )
                .unwrap()
        };
        (
            read_cut("publication-operation-format-shared-cut"),
            read_cut("publication-operation-format-local-cut"),
        )
    }

    #[test]
    fn format_cuts_precede_bound_requests_and_survive_replay_forget_and_reopen() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("claims.db");
        let store = Store::open(&path, "node").unwrap();
        assert_eq!(format_cuts(&store.readers.get()), (0, 0));
        publish(
            &store,
            "version 2\nagent \"example/worker\" { workspace \"/tmp\"; command \"true\" }",
            "legacy",
        );
        store
            .append_claim(&receipt("agent/example/worker"))
            .unwrap();
        drop(store);
        let legacy = Connection::open(&path).unwrap();
        let shared: i64 = legacy
            .query_row("SELECT MAX(store_index) FROM claims", [], |row| row.get(0))
            .unwrap();
        let local: i64 = legacy
            .query_row("SELECT MAX(id) FROM local_observations", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert!(shared > 0 && local > 0);
        legacy.execute_batch(
            "DROP TABLE local_runtime_start_evidence; DROP TABLE local_runtime_starts;
             DELETE FROM meta WHERE key IN ('publication-operation-format-shared-cut','publication-operation-format-local-cut');
             PRAGMA user_version=18;",
        ).unwrap();
        drop(legacy);
        let store = Store::open(&path, "node").unwrap();
        assert_eq!(format_cuts(&store.readers.get()), (shared, local));
        let subject = "agent/example/worker";
        let token = store.selected_desired_token(subject).unwrap();
        let mut start = operation("modern-operation", subject, token);
        assert_eq!(
            store.admit_runtime_start(&mut start, 2).unwrap(),
            RuntimeStartAdmission::Admitted
        );
        let request_position: i64 = store
            .readers
            .get()
            .query_row("SELECT MAX(id) FROM local_observations", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert!(request_position > local);
        publish(
            &store,
            "version 2\nstop \"agent/example/worker\"",
            "modern-stop",
        );
        {
            let mut writer = store.connection.write();
            let transaction = writer.transaction().unwrap();
            replay_graph_from_nothing_tx(&transaction).unwrap();
            assert_eq!(format_cuts(&transaction), (shared, local));
            transaction.commit().unwrap();
        }
        store.forget_current_views();
        assert_eq!(format_cuts(&store.readers.get()), (shared, local));
        drop(store);
        let reopened = Store::open(&path, "node").unwrap();
        assert_eq!(format_cuts(&reopened.readers.get()), (shared, local));
        assert_eq!(
            reopened.runtime_start_for(subject).unwrap().unwrap().body["start_request"],
            start.body["start_request"]
        );
    }

    #[test]
    fn missing_format_cut_with_existing_operations_refuses_a_later_boundary() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("claims.db");
        let store = Store::open(&path, "node").unwrap();
        store
            .admit_runtime_start(&mut operation("one", "agent/example/worker", None), 2)
            .unwrap();
        drop(store);
        let connection = Connection::open(&path).unwrap();
        connection
            .execute(
                "DELETE FROM meta WHERE key='publication-operation-format-local-cut'",
                [],
            )
            .unwrap();
        drop(connection);
        let error = match Store::open(&path, "node") {
            Ok(_) => panic!("must not capture a boundary after bound requests exist"),
            Err(error) => error,
        };
        assert!(format!("{error:#}").contains("lacks immutable publication format cuts"));
        let connection = Connection::open(&path).unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM meta WHERE key='publication-operation-format-local-cut'",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM local_runtime_starts", [], |row| row
                    .get::<_, i64>(
                    0
                ))
                .unwrap(),
            1
        );
    }

    #[test]
    fn schema_18_upgrade_and_reopen_preserve_unresolved_revocation() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("claims.db");
        drop(Store::open(&path, "node").unwrap());
        let old = Connection::open(&path).unwrap();
        old.execute_batch("DROP TABLE local_runtime_starts; PRAGMA user_version=18;")
            .unwrap();
        drop(old);
        let store = Store::open(&path, "node").unwrap();
        let subject = "agent/example/worker";
        store
            .admit_runtime_start(&mut operation("one", subject, None), 2)
            .unwrap();
        store.dispatch_runtime_start("one", 2).unwrap();
        store
            .revoke_runtime_start("one", "durable-stop-request")
            .unwrap();
        drop(store);
        let reopened = Store::open(&path, "node").unwrap();
        let operation = reopened.runtime_start_for(subject).unwrap().unwrap();
        assert_eq!(
            operation.revoked_by.as_deref(),
            Some("durable-stop-request")
        );
        assert!(operation.dispatched && !operation.finished);
        assert_eq!(reopened.runtime_start_counts().unwrap(), (1, 1));
        let version: u32 = reopened
            .readers
            .get()
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, smallclaims::store::CURRENT_SCHEMA_VERSION);
        assert_eq!(version, st3_schema::STORAGE_VERSION);
    }
}
