//! Immutable, replicated task authorization metadata. Credential values never enter this log.
use super::*;
#[cfg(test)]
use smallclaims::store::checkpoint::claims_of_kind_in_order;
use st3_schema::task_grants::{CreateTaskGrantRequest, TaskGrantReceipt, TaskGrantRecord, TaskGrantState};

pub const CREATED: &str = "task-grant.created";
pub const REVOKED: &str = "task-grant.revoked";

pub fn create_schema(connection: &Connection) -> Result<()> {
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-h085880bb2ea5 owner=st3_store::store::task_grants TODO=chunk-or-reference-variable-columns */ "CREATE TABLE IF NOT EXISTS task_grants (
        grant_id TEXT PRIMARY KEY, created_claim_id TEXT NOT NULL, revoked INTEGER NOT NULL
    );")?;
    Ok(())
}

pub fn open(tx: &Transaction<'_>) -> Result<()> {
    create_schema(tx)
}

fn active_claims(connection: &Connection, subject: &str, kind: &str) -> Result<Vec<ClaimRecord>> {
    connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h921318e7fb55 owner=st3_store::store::task_grants TODO=trace-statement-and-value-representation */ &format!(
        "SELECT {CLAIM_COLUMNS} FROM claims JOIN batches ON batches.id=claims.batch_id
         WHERE claims.subject=?1 AND claims.kind=?2
         AND NOT EXISTS(SELECT 1 FROM replica_records WHERE claim_id=claims.id AND state='repaired')
         ORDER BY {CANONICAL_ORDER}"))?
        .query_map(params![subject, kind], claim_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
}

pub fn repair(tx: &Transaction<'_>, repaired: &str, replacement: &str) -> Result<()> {
    let mut subjects = BTreeSet::new();
    for id in [repaired, replacement] {
        if let Some(claim) = claim_by_id_tx(tx, id)?
            && matches!(claim.kind.as_str(), CREATED | REVOKED) {
            subjects.insert(claim.subject);
        }
    }
    for subject in subjects { project_subject(tx, &subject)?; }
    Ok(())
}

pub fn project(tx: &Transaction<'_>, claim: &ClaimRecord) -> Result<()> {
    project_subject(tx, &claim.subject)
}

fn project_subject(tx: &Transaction<'_>, subject: &str) -> Result<()> {
    let created = active_claims(tx, subject, CREATED)?;
    let Some(first) = created.first() else {
        tx.execute(/* sqlite-large-value: allow-for-now lv-h46440c529308 owner=st3_store::store::task_grants TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM task_grants WHERE grant_id=?1", [subject])?;
        return Ok(());
    };
    // Concurrent conflicting creations are not permission to pick a broader manifest.
    let fields = &first.body["fields"];
    anyhow::ensure!(created.iter().all(|claim| claim.body["fields"] == *fields),
        "conflicting task grant creation records");
    let revoked = !active_claims(tx, subject, REVOKED)?.is_empty();
    tx.execute(/* sqlite-large-value: allow-for-now lv-hf8cd2c3d0a5b owner=st3_store::store::task_grants TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO task_grants(grant_id,created_claim_id,revoked) VALUES(?1,?2,?3)
        ON CONFLICT(grant_id) DO UPDATE SET created_claim_id=excluded.created_claim_id,revoked=excluded.revoked",
        params![subject, first.id, revoked])?;
    Ok(())
}

pub fn rebuild(tx: &Transaction<'_>) -> Result<()> {
    open(tx)?;
    tx.execute(/* sqlite-large-value: allow-for-now lv-he0282d422fbe owner=st3_store::store::task_grants TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM task_grants", [])?;
    let subjects = tx.prepare("SELECT DISTINCT subject FROM claims WHERE kind='task-grant.created'")?
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for subject in subjects { project_subject(tx, &subject)?; }
    Ok(())
}

fn read(connection: &Connection, grant_id: &str, now: chrono::DateTime<chrono::Utc>) -> Result<Option<TaskGrantRecord>, St3Error> {
    let selected: Option<(String, bool)> = connection.query_row(
        "SELECT created_claim_id,revoked FROM task_grants WHERE grant_id=?1", [grant_id],
        |row| Ok((row.get(0)?, row.get(1)?))).optional().map_err(internal)?;
    let Some((claim_id, revoked)) = selected else { return Ok(None); };
    let claim = claim_by_id_tx(connection, &claim_id).map_err(internal)?
        .ok_or_else(|| St3Error::new("internal", "missing task grant creation claim"))?;
    let fields = &claim.body["fields"];
    let manifest: st3_schema::task_grants::TaskManifest = serde_json::from_value(fields["manifest"].clone()).map_err(internal)?;
    let (_, end) = manifest.window.bounds().map_err(internal)?;
    Ok(Some(TaskGrantRecord {
        grant_id: grant_id.into(),
        digest: fields["digest"].as_str().unwrap_or_default().into(),
        op_proxy_case_id: fields["op_proxy_case_id"].as_str().unwrap_or_default().into(),
        window: manifest.window.clone(),
        manifest,
        state: if revoked { TaskGrantState::Revoked } else if now >= end { TaskGrantState::Expired } else { TaskGrantState::Active },
    }))
}

impl Store {
    /// Internal writer seam. The API must authenticate its op-proxy caller before invoking it.
    pub fn create_task_grant(&self, request: &CreateTaskGrantRequest) -> Result<TaskGrantReceipt, St3Error> {
        request.validate().map_err(|error| St3Error::new("invalid-task-grant", error.to_string()))?;
        let grant_id = format!("task-grant/{}", Uuid::now_v7().simple());
        self.connection.batched(|tx| -> Result<TaskGrantReceipt, St3Error> {
            let requester = &request.manifest.requester;
            let fields = json!({"grant_id":grant_id,"manifest":request.manifest,"digest":request.digest,
                "op_proxy_case_id":request.op_proxy_case_id,"requester":requester.principal,
                "run":requester.run,"step":requester.step,"incarnation":requester.incarnation,
                "window":request.manifest.window});
            append_claim_tx(tx, &self.origin, &grant_id, CREATED, None, &json!({"fields":fields}), &[], None).map_err(internal)?;
            Ok(TaskGrantReceipt { grant_id: grant_id.clone(), digest: request.digest.clone(),
                window: request.manifest.window.clone(), state: TaskGrantState::Active })
        }).map_err(|error| St3Error::new("internal", error))?
    }

    pub fn task_grant(&self, grant_id: &str) -> Result<Option<TaskGrantRecord>, St3Error> {
        read(&self.readers.get(), grant_id, chrono::Utc::now())
    }

    /// A second revoke retains the first immutable reason and adds no claim.
    pub fn revoke_task_grant(&self, grant_id: &str, reason: &str) -> Result<Option<TaskGrantRecord>, St3Error> {
        if reason.trim().is_empty() { return Err(St3Error::new("invalid-task-grant", "revocation reason must not be blank")); }
        self.connection.batched(|tx| -> Result<Option<TaskGrantRecord>, St3Error> {
            let Some(record) = read(tx, grant_id, chrono::Utc::now())? else { return Ok(None); };
            if record.state != TaskGrantState::Revoked {
                append_claim_tx(tx, &self.origin, grant_id, REVOKED, None,
                    &json!({"fields":{"grant_id":grant_id,"reason":reason}}), &[], None).map_err(internal)?;
            }
            read(tx, grant_id, chrono::Utc::now())
        }).map_err(|error| St3Error::new("internal", error))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> CreateTaskGrantRequest {
        let now = chrono::Utc::now();
        let manifest = serde_json::from_value(json!({
            "version":1,"title":"Example grant",
            "requester":{"principal":"person/robin","run":"example-run","step":"example-step","incarnation":"example-1"},
            "window":{"notBefore":(now-chrono::Duration::minutes(1)).to_rfc3339(),
                "notAfter":(now+chrono::Duration::hours(1)).to_rfc3339()},
            "provider":{"name":"example","browserProfile":"example","origins":["https://example.com"]},
            "browserSteps":[],"writes":[],"readers":[]
        })).unwrap();
        let mut request = CreateTaskGrantRequest { manifest, op_proxy_case_id:"example-case".into(), digest:String::new() };
        request.digest = request.manifest.digest().unwrap();
        request
    }

    #[test]
    fn creates_reads_and_revokes_without_rewriting_creation() {
        let store = Store::open_memory("example").unwrap();
        let request = request();
        let receipt = store.create_task_grant(&request).unwrap();
        assert_eq!(receipt.state, TaskGrantState::Active);
        let original = store.task_grant(&receipt.grant_id).unwrap().unwrap();
        assert_eq!(original.manifest, request.manifest);
        assert_eq!(original.digest, request.digest);
        assert_eq!(original.op_proxy_case_id, request.op_proxy_case_id);
        assert_eq!(original.state, TaskGrantState::Active);
        let revoked = store.revoke_task_grant(&receipt.grant_id, "example cancellation").unwrap().unwrap();
        assert_eq!(revoked.state, TaskGrantState::Revoked);
        assert_eq!(revoked.manifest, original.manifest);
        assert_eq!(store.revoke_task_grant(&receipt.grant_id, "different reason").unwrap(), Some(revoked));
        let connection = store.readers.get();
        let reasons = claims_of_kind_in_order(&connection, &receipt.grant_id, REVOKED).unwrap();
        assert_eq!(reasons.len(), 1);
        assert_eq!(reasons[0].body["fields"]["reason"], "example cancellation");
        assert_eq!(claims_of_kind_in_order(&connection, &receipt.grant_id, CREATED).unwrap().len(), 1);
        assert!(store.task_grant("task-grant/missing").unwrap().is_none());
        assert!(store.revoke_task_grant("task-grant/missing", "example").unwrap().is_none());
    }

    #[test]
    fn expiry_is_derived_and_revocation_takes_precedence() {
        let store = Store::open_memory("example").unwrap();
        let request = request();
        let receipt = store.create_task_grant(&request).unwrap();
        let (_, end) = request.manifest.window.bounds().unwrap();
        assert_eq!(read(&store.readers.get(), &receipt.grant_id, end).unwrap().unwrap().state, TaskGrantState::Expired);
        store.revoke_task_grant(&receipt.grant_id, "example cancellation").unwrap();
        assert_eq!(read(&store.readers.get(), &receipt.grant_id, end).unwrap().unwrap().state, TaskGrantState::Revoked);
    }

    #[test]
    fn rejects_bad_digest_and_rebuilds_projection_from_durable_claims() {
        let store = Store::open_memory("example").unwrap();
        let mut request = request();
        request.digest = "0".repeat(64);
        assert!(store.create_task_grant(&request).is_err());
        request.digest = request.manifest.digest().unwrap();
        let receipt = store.create_task_grant(&request).unwrap();
        store.revoke_task_grant(&receipt.grant_id, "example cancellation").unwrap();
        let expected = store.task_grant(&receipt.grant_id).unwrap();
        store.connection.batched(|tx| -> Result<()> {
            tx.execute("DELETE FROM task_grants", [])?;
            rebuild(tx)?;
            Ok(())
        }).unwrap().unwrap();
        assert_eq!(store.task_grant(&receipt.grant_id).unwrap(), expected);
    }

    #[test]
    fn fresh_and_existing_current_schema_stores_install_task_grants() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("example.sqlite");
        let schema_version;
        {
            let store = Store::open(&path, "example").unwrap();
            assert!(store.task_grant("task-grant/missing").unwrap().is_none());
            let connection = store.readers.get();
            schema_version = connection.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0)).unwrap();
        }
        // Model a store already at the current version, predating this additive table.
        {
            let connection = Connection::open(&path).unwrap();
            connection.execute_batch("DROP TABLE task_grants").unwrap();
        }
        let store = Store::open(&path, "example").unwrap();
        assert!(store.task_grant("task-grant/missing").unwrap().is_none());
        let version = store.readers.get().query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0)).unwrap();
        assert_eq!(version, schema_version);
        store.create_task_grant(&request()).unwrap();
    }

    #[test]
    fn replicates_normal_claims_and_replays_them() {
        use crate::store::tests_shared::{exchange_from, receive_and_project};
        use crate::model::ReplicationInventory;
        let source = Store::open_memory("example-source").unwrap();
        let target = Store::open_memory("example-target").unwrap();
        let receipt = source.create_task_grant(&request()).unwrap();
        source.revoke_task_grant(&receipt.grant_id, "example cancellation").unwrap();
        receive_and_project(&target, &source.origin, &exchange_from(&source, &ReplicationInventory::default()));
        assert_eq!(target.task_grant(&receipt.grant_id).unwrap(), source.task_grant(&receipt.grant_id).unwrap());
        target.connection.batched(|tx| -> Result<()> {
            replay_graph_from_nothing_tx(tx).map_err(anyhow::Error::new)?;
            Ok(())
        }).unwrap().unwrap();
        assert_eq!(target.task_grant(&receipt.grant_id).unwrap(), source.task_grant(&receipt.grant_id).unwrap());
    }
}
