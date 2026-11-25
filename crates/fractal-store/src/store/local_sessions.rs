//! Executor-local custody. These rows are not graph projections and never authorize a launch.
use super::*;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum SessionCustody { Owned, Borrowed }

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LocalSession {
    pub subject: String,
    pub runtime_id: String,
    /// Logical seat incarnation, including the invocation fence for borrowed terminals.
    pub incarnation: String,
    /// Exact native process/PTY generation, never a PID alone.
    pub native_incarnation: String,
    pub terminal: bool,
    pub driver: Option<String>,
    pub lifecycle: crate::model::MemberLifecycle,
    pub custody: SessionCustody,
    pub start_operation: Option<String>,
    pub invocation: Option<String>,
}

pub fn create_schema(connection: &Connection) -> Result<()> {
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-h75e60616e14f owner=fractal_store::store::local_sessions TODO=chunk-or-reference-variable-columns */ "CREATE TABLE IF NOT EXISTS local_session_custody (
        subject TEXT NOT NULL, incarnation TEXT NOT NULL, body TEXT NOT NULL,
        active INTEGER NOT NULL DEFAULT 1, custody_settled INTEGER NOT NULL DEFAULT 0,
        detached_by TEXT, detach_receipted INTEGER NOT NULL DEFAULT 0,
        PRIMARY KEY(subject,incarnation)
    ) WITHOUT ROWID;
    CREATE UNIQUE INDEX IF NOT EXISTS local_session_current ON local_session_custody(subject) WHERE active=1;")?;
    let has_retirement_proof: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('local_session_custody') WHERE name='native_retirement_verified')",
        [], |row| row.get(0))?;
    if !has_retirement_proof {
        // Existing authoritative bindings survive the additive upgrade without acquiring proof.
        connection.execute(/* sqlite-large-value: allow-for-now lv-h643365c9c213 owner=fractal_store::store::local_sessions TODO=bound-or-reference-values-and-reclamation */ "ALTER TABLE local_session_custody ADD COLUMN native_retirement_verified INTEGER NOT NULL DEFAULT 0", [])?;
    }
    Ok(())
}

/// Existing checkpoint removal may reclaim only closed custody with settled local receipts.
pub fn forget_subject(connection: &Connection, subject: &str) -> Result<()> {
    let installed: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='local_session_custody')", [], |row| row.get(0))?;
    if installed {
        retain_bound_start_reservations(connection)?;
        connection.execute(/* sqlite-large-value: allow-for-now lv-h0de5666a03e4 owner=fractal_store::store::local_sessions TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_session_custody WHERE subject=?1 AND custody_settled=1
            AND (detached_by IS NULL OR detach_receipted=1)
            AND NOT EXISTS(SELECT 1 FROM local_runtime_starts WHERE subject=?1 AND
                (finished=0 OR applied=0 OR receipted=0 OR (revoked_by IS NOT NULL AND cleanup_verified=0)))",
            [subject])?;
    }
    Ok(())
}

#[derive(Debug)]
pub struct SessionCustodyCapacityReached;

impl std::fmt::Display for SessionCustodyCapacityReached {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("executor session custody capacity reached")
    }
}
impl std::error::Error for SessionCustodyCapacityReached {}

/// A bound producer has transferred its reservation into custody permanently,
/// even after that generation is settled and its custody history is reclaimed.
fn retain_bound_start_reservations(connection: &Connection) -> Result<()> {
    connection.execute("UPDATE local_runtime_starts
        SET body=json_set(body,'$.custody_bound',json('true'))
        WHERE id IN (SELECT json_extract(body,'$.start_operation') FROM local_session_custody)
          AND json_extract(body,'$.custody_bound') IS NOT 1
          AND EXISTS(SELECT 1 FROM local_session_custody AS custody
            WHERE custody.subject=local_runtime_starts.subject
              AND json_extract(custody.body,'$.start_operation')=local_runtime_starts.id)", [])?;
    Ok(())
}

/// Admit one unsettled custody slot under the exclusive writer, before native entry.
/// Every start-operation kind must count its unsettled reservations here. Local
/// dispatched producers reserve capacity until binding or proven producer closure.
/// Completed legacy successful producers never reserved custody. Legacy unfinished
/// or unverified producers remain conservative reservations until recovered.
pub fn admit_capacity(connection: &Connection, operation: Option<&str>) -> Result<()> {
    // Backfill retained bindings before reclamation, including pre-marker starts.
    retain_bound_start_reservations(connection)?;
    connection.execute("DELETE FROM local_session_custody
        WHERE active=0 AND custody_settled=1
          AND (detached_by IS NULL OR detach_receipted=1)
          AND NOT EXISTS(SELECT 1 FROM local_runtime_starts AS starts
            WHERE starts.subject=local_session_custody.subject
              AND (?1 IS NULL OR starts.id<>?1) AND
              (finished=0 OR applied=0 OR receipted=0
               OR (revoked_by IS NOT NULL AND cleanup_verified=0)))", [operation])?;
    let count: u64 = connection.query_row(
        "SELECT (SELECT count(*) FROM local_session_custody WHERE custody_settled=0)
          + (SELECT count(*) FROM local_runtime_starts AS starts
             WHERE dispatched=1 AND (?1 IS NULL OR id<>?1)
               AND json_extract(body,'$.custody_bound') IS NOT 1
               AND (json_extract(body,'$.custody_reserved')=1 OR finished=0
                    OR json_extract(result,'$.verification') IS NOT 'operation-incarnation')
               AND (finished=0 OR (cleanup_verified=0
                    AND json_extract(result,'$.producer_closed') IS NOT 1))
               AND NOT EXISTS(SELECT 1 FROM local_session_custody AS custody
                 WHERE json_extract(custody.body,'$.start_operation')=starts.id))",
        [operation], |row| row.get(0))?;
    if count >= 4096 { return Err(SessionCustodyCapacityReached.into()); }
    Ok(())
}

impl Store {
    /// Admission is bounded per executor. Previous generations retain custody and receipt fences.
    pub fn bind_local_session(&self, binding: &LocalSession) -> Result<()> {
        anyhow::ensure!(binding.subject.len() <= 1024 && binding.runtime_id.len() <= 1024
            && binding.incarnation.len() <= 1024 && binding.native_incarnation.len() <= 1024
            && binding.driver.as_ref().is_none_or(|v| v.len() <= 128)
            && binding.start_operation.as_ref().is_none_or(|v| v.len() <= 1024)
            && binding.invocation.as_ref().is_none_or(|v| v.len() <= 1024), "session binding exceeds bounds");
        anyhow::ensure!(!binding.subject.is_empty() && !binding.runtime_id.is_empty()
            && !binding.native_incarnation.is_empty(), "session binding identity is empty");
        if binding.custody == SessionCustody::Borrowed {
            anyhow::ensure!(binding.terminal && binding.lifecycle == crate::model::MemberLifecycle::TerminalBound
                && binding.invocation.as_deref() == Some(binding.incarnation.as_str())
                && binding.incarnation.strip_prefix(binding.native_incarnation.as_str())
                    .is_some_and(|suffix| suffix.starts_with(":bound:") && suffix.len() > 7),
                "borrowed custody requires an exact terminal invocation fence");
        } else {
            anyhow::ensure!(binding.incarnation == binding.native_incarnation && binding.invocation.is_none(),
                "owned custody requires an exact native generation");
        }
        let mut connection = self.connection.write();
        // Custody is executor-owned, not a graph projection. Keep the exclusive writer
        // and atomic generation switch, without preparing graph COMMIT finalizers.
        let tx = Connection::transaction(&mut connection)?;
        let prior: Option<Option<String>> = tx.query_row(
            "SELECT detached_by FROM local_session_custody WHERE subject=?1 AND incarnation=?2",
            params![binding.subject, binding.incarnation], |row| row.get(0)).optional()?;
        if matches!(prior, Some(Some(_))) { return Ok(()); }
        if prior.is_none() {
            admit_capacity(&tx, binding.start_operation.as_deref())?;
        }
        tx.execute(/* sqlite-large-value: allow-for-now lv-hdce8a783bfb6 owner=fractal_store::store::local_sessions TODO=bound-or-reference-values-and-reclamation */ "UPDATE local_session_custody SET active=0 WHERE subject=?1 AND incarnation<>?2 AND active=1",
            params![binding.subject, binding.incarnation])?;
        tx.execute(/* sqlite-large-value: allow-for-now lv-h1298428266a6 owner=fractal_store::store::local_sessions TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO local_session_custody(subject,incarnation,body) VALUES(?1,?2,?3)
            ON CONFLICT(subject,incarnation) DO UPDATE SET body=excluded.body,active=1,native_retirement_verified=0
            WHERE local_session_custody.detached_by IS NULL
              AND (local_session_custody.body<>excluded.body OR local_session_custody.active<>1)",
            params![binding.subject, binding.incarnation, serde_json::to_string(binding)?])?;
        if let Some(operation) = binding.start_operation.as_deref() {
            tx.execute("UPDATE local_runtime_starts
                SET body=json_set(body,'$.custody_bound',json('true'))
                WHERE id=?1 AND subject=?2", params![operation, binding.subject])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Persist only a positively verified whole-scope stop, before reporting native success.
    /// This is not the command completion receipt and does not settle custody.
    /// Exact proof recording converges with a concurrent natural-exit settlement.
    pub fn verify_local_session_retirement(&self, binding: &LocalSession) -> Result<()> {
        anyhow::ensure!(binding.custody == SessionCustody::Owned, "borrowed custody cannot retire its native runtime");
        let changed = self.connection.write().execute(
            /* sqlite-large-value: allow-for-now lv-h31a948e882b8 owner=fractal_store::store::local_sessions TODO=bound-or-reference-values-and-reclamation */ "UPDATE local_session_custody SET native_retirement_verified=1
             WHERE subject=?1 AND incarnation=?2 AND body=?3 AND active=1
               AND detached_by IS NULL",
            params![binding.subject, binding.incarnation, serde_json::to_string(binding)?])?;
        anyhow::ensure!(changed == 1, "native retirement custody changed before its proof was recorded");
        Ok(())
    }

    /// Query exact retained native proof without signalling or consulting graph projections.
    pub fn local_session_retirement_verified(&self, binding: &LocalSession) -> Result<bool> {
        if binding.custody != SessionCustody::Owned { return Ok(false); }
        Ok(self.readers.get().query_row(
            "SELECT EXISTS(SELECT 1 FROM local_session_custody
             WHERE subject=?1 AND incarnation=?2 AND body=?3 AND native_retirement_verified=1)",
            params![binding.subject, binding.incarnation, serde_json::to_string(binding)?], |row| row.get(0))?)
    }

    /// Historical, detached and positively closed rows are retention, not local routing authority.
    pub fn has_current_local_session(&self, subject: &str) -> Result<bool> {
        Ok(self.readers.get().query_row("SELECT EXISTS(SELECT 1 FROM local_session_custody
            WHERE subject=?1 AND active=1 AND custody_settled=0 AND detached_by IS NULL)",
            [subject], |row| row.get(0))?)
    }

    pub fn local_session(&self, subject: &str) -> Result<Option<LocalSession>> {
        let body: Option<String> = self.readers.get().query_row(
            "SELECT body FROM local_session_custody WHERE subject=?1 AND active=1 AND custody_settled=0 AND detached_by IS NULL",
            [subject], |row| row.get(0)).optional()?;
        body.map(|body| serde_json::from_str(&body).map_err(Into::into)).transpose()
    }

    /// Retained provenance for recovery; unlike `local_session`, this grants no live control.
    pub fn local_session_at(&self, subject: &str, incarnation: &str) -> Result<Option<LocalSession>> {
        let body: Option<String> = self.readers.get().query_row(
            "SELECT body FROM local_session_custody WHERE subject=?1 AND incarnation=?2",
            params![subject, incarnation], |row| row.get(0)).optional()?;
        body.map(|body| serde_json::from_str(&body).map_err(Into::into)).transpose()
    }

    pub fn local_session_detached(&self, subject: &str, incarnation: &str) -> Result<bool> {
        Ok(self.readers.get().query_row("SELECT EXISTS(SELECT 1 FROM local_session_custody
            WHERE subject=?1 AND incarnation=?2 AND detached_by IS NOT NULL)",
            params![subject, incarnation], |row| row.get(0))?)
    }

    /// Durable logical detach only: never signal a person's shell or synthesize process exit.
    pub fn detach_local_session(&self, subject: &str, incarnation: &str, operation: &str) -> Result<()> {
        anyhow::ensure!(!operation.is_empty() && operation.len() <= 1024, "invalid detach operation");
        let changed = self.connection.write().execute(/* sqlite-large-value: allow-for-now lv-hbaa88b024680 owner=fractal_store::store::local_sessions TODO=bound-or-reference-values-and-reclamation */ "UPDATE local_session_custody
            SET detached_by=COALESCE(detached_by,?3),custody_settled=1 WHERE subject=?1 AND incarnation=?2
            AND json_extract(body,'$.custody')='Borrowed'",
            params![subject, incarnation, operation])?;
        anyhow::ensure!(changed == 1, "stale or non-borrowed session detach");
        Ok(())
    }

    pub fn pending_local_session_detaches(&self) -> Result<Vec<(LocalSession, String)>> {
        let connection = self.readers.get();
        let mut statement = connection.prepare("SELECT body,detached_by FROM local_session_custody
            WHERE detached_by IS NOT NULL AND detach_receipted=0 ORDER BY subject")?;
        let rows = statement.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter().map(|(body, operation)| Ok((serde_json::from_str(&body)?, operation))).collect()
    }

    pub fn ack_local_session_detach(&self, subject: &str, incarnation: &str, operation: &str) -> Result<()> {
        self.connection.write().execute(/* sqlite-large-value: allow-for-now lv-he5f5cd557854 owner=fractal_store::store::local_sessions TODO=bound-or-reference-values-and-reclamation */ "UPDATE local_session_custody SET detach_receipted=1
            WHERE subject=?1 AND incarnation=?2 AND detached_by=?3", params![subject, incarnation, operation])?;
        Ok(())
    }

    /// Call only after exact native closure and the corresponding observation receipt.
    pub fn settle_local_session_custody(&self, subject: &str, native_incarnation: &str) -> Result<()> {
        self.connection.write().execute(/* sqlite-large-value: allow-for-now lv-h237b1d3d6060 owner=fractal_store::store::local_sessions TODO=bound-or-reference-values-and-reclamation */ "UPDATE local_session_custody SET custody_settled=1
            WHERE subject=?1 AND json_extract(body,'$.native_incarnation')=?2",
            params![subject, native_incarnation])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_inflight_and_unverified_producers_still_reserve_capacity() {
        for finished in [0, 1] {
            let store = Store::open_memory("executor").unwrap();
            store.connection.write().execute(
                "WITH RECURSIVE starts(n) AS (VALUES(1) UNION ALL SELECT n+1 FROM starts WHERE n<4096)
                 INSERT INTO local_runtime_starts(id,subject,owner,body,finished,result,dispatched)
                 SELECT 'inflight-'||n,'agent/inflight/'||n,'old-owner','{}',?1,
                    '{\"verification\":\"launch-accepted-unverified\"}',1 FROM starts",
                [finished]).unwrap();
            let error = admit_capacity(&store.connection.write(), None).unwrap_err();
            assert!(error.downcast_ref::<SessionCustodyCapacityReached>().is_some(),
                "legacy unresolved producers must not lose their reservation on upgrade");
        }
    }

    #[test]
    fn natural_exit_settlement_racing_stop_preserves_exact_retirement_proof() {
        let store = Store::open_memory("executor").unwrap();
        let binding = LocalSession {
            subject: "agent/example/exit-stop".into(), runtime_id: "exit-stop".into(),
            incarnation: "generation".into(), native_incarnation: "generation".into(),
            terminal: true, driver: None,
            lifecycle: crate::model::MemberLifecycle::Service,
            custody: SessionCustody::Owned, start_operation: None, invocation: None,
        };
        store.bind_local_session(&binding).unwrap();
        // The explicit stop already resolved this exact live binding. Hold its
        // native completion until the concurrent exit observer has settled it.
        let resolved = store.local_session(&binding.subject).unwrap().unwrap();
        let settled = std::sync::Barrier::new(2);
        std::thread::scope(|scope| {
            scope.spawn(|| {
                store.settle_local_session_custody(&binding.subject, &binding.native_incarnation).unwrap();
                settled.wait();
            });
            scope.spawn(|| {
                settled.wait();
                store.verify_local_session_retirement(&resolved).unwrap();
            });
        });
        assert!(!store.has_current_local_session(&binding.subject).unwrap());
        assert!(store.local_session_retirement_verified(&resolved).unwrap());
        store.verify_local_session_retirement(&resolved).unwrap();
        store.settle_local_session_custody(&binding.subject, &binding.native_incarnation).unwrap();
        assert!(store.local_session_retirement_verified(&resolved).unwrap());
        let mut stale = resolved.clone();
        stale.runtime_id = "other-runtime".into();
        assert!(store.verify_local_session_retirement(&stale).is_err());
    }

    #[test]
    fn operation_backed_settled_generations_are_reclaimed_without_resurrecting_reservations() {
        let store = Store::open_memory("executor").unwrap();
        let mut binding = LocalSession {
            subject: "agent/example/live".into(), runtime_id: "live".into(),
            incarnation: "0".into(), native_incarnation: "0".into(),
            terminal: true, driver: None,
            lifecycle: crate::model::MemberLifecycle::Service,
            custody: SessionCustody::Owned, start_operation: None, invocation: None,
        };
        for generation in 0..4100 {
            binding.incarnation = generation.to_string();
            binding.native_incarnation = binding.incarnation.clone();
            let operation = format!("start-{generation}");
            store.connection.write().execute(
                "INSERT INTO local_runtime_starts
                    (id,subject,owner,body,finished,result,applied,dispatched,receipted)
                 VALUES(?1,?2,'owner','{\"custody_reserved\":true}',1,'{\"verification\":\"operation-incarnation\"}',1,1,1)",
                params![operation, binding.subject]).unwrap();
            binding.start_operation = Some(operation);
            store.bind_local_session(&binding).unwrap();
            store.settle_local_session_custody(&binding.subject, &binding.native_incarnation).unwrap();
        }
        let retained: u64 = store.readers.get().query_row(
            "SELECT count(*) FROM local_session_custody WHERE subject=?1",
            [&binding.subject], |row| row.get(0)).unwrap();
        assert!(retained <= 2, "settled live-subject history must remain bounded");
        binding.incarnation = "next".into();
        binding.native_incarnation = binding.incarnation.clone();
        store.bind_local_session(&binding).unwrap();
        assert!(store.has_current_local_session(&binding.subject).unwrap());
    }
}
