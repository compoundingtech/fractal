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
                if operation.receipted {
                    return Ok(true);
                }
                let receipts = inputs
                    .iter()
                    .map(|input| -> Result<String> {
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
                            insert_local_observation_tx(tx, self.origin(), input, now_ms())?.0
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
            body: serde_json::json!({"selected_token": token}),
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
