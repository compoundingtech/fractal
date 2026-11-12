//! Owner-local head reconciliation. The dirty queue is disposable local state; lineage,
//! cancellation, and faults remain ordinary replicated claims.
use super::*;

const HEAD_PAGE: usize = 64;
const FAULT_SCOPE: &str = "mission/supersession";

impl Store {
    /// Drain the finite dirty-head backlog before dispatch, without a writer fence when idle.
    pub fn supersede_mission_publications(&self) -> Result<bool, FractalError> {
        smallclaims::touched::note_read(|| "kind:mission.published".to_owned());
        smallclaims::touched::note_read(|| "kind:owned-set.revised".to_owned());
        let ceiling = {
            let connection = self.readers.get();
            let initialized = supersession_queue_installed(&connection)?;
            if initialized {
                let ceiling: u64 = connection.query_row(
                    "SELECT COALESCE(MAX(seq),0) FROM local_mission_supersession_heads_v1",
                    [], |row| row.get(0),
                ).map_err(internal)?;
                if ceiling == 0 {
                    return Ok(false);
                }
                Some(ceiling)
            } else {
                None
            }
        };
        let mut changed = false;
        let ceiling = match ceiling {
            Some(ceiling) => ceiling,
            None => {
                let (initialized, ceiling) = self.with_all_runs_fenced_internal(|transaction| {
                    let initialized = initialize_supersession_queue(transaction)?;
                    let ceiling: u64 = transaction.query_row(
                        "SELECT COALESCE(MAX(seq),0) FROM local_mission_supersession_heads_v1",
                        [], |row| row.get(0),
                    ).map_err(internal)?;
                    Ok::<_, FractalError>((initialized, ceiling))
                })?;
                changed |= initialized;
                ceiling
            }
        };
        if ceiling == 0 {
            return Ok(changed);
        }
        loop {
            let (page_changed, done) = self.with_all_runs_fenced_internal(|transaction| {
                let page_changed = self.supersede_mission_heads_through_tx(transaction, ceiling)?;
                let done: bool = transaction.query_row(
                    "SELECT NOT EXISTS(SELECT 1 FROM local_mission_supersession_heads_v1 WHERE seq<=?1)",
                    [ceiling], |row| row.get(0),
                ).map_err(internal)?;
                Ok::<_, FractalError>((page_changed, done))
            })?;
            changed |= page_changed;
            if done {
                return Ok(changed);
            }
        }
    }

    pub(super) fn supersede_mission_publications_tx(
        &self,
        transaction: &Transaction<'_>,
    ) -> Result<bool, FractalError> {
        self.supersede_mission_heads_through_tx(transaction, i64::MAX as u64)
    }

    fn supersede_mission_heads_through_tx(
        &self,
        transaction: &Transaction<'_>,
        ceiling: u64,
    ) -> Result<bool, FractalError> {
        let initialized = initialize_supersession_queue(transaction)?;
        let missions = {
            let mut query = transaction.prepare(
                "SELECT mission_id FROM local_mission_supersession_heads_v1
                 WHERE seq<=?1 ORDER BY seq LIMIT ?2",
            ).map_err(internal)?;
            query.query_map(params![ceiling, HEAD_PAGE as u64], |row| row.get::<_, String>(0))
                .map_err(internal)?.collect::<rusqlite::Result<Vec<_>>>().map_err(internal)?
        };
        let changed = self.supersede_published_missions_tx(transaction, &missions)?;
        Ok(initialized || changed)
    }

    /// Apply newly published heads atomically, independent of the dirty-head backlog.
    pub(super) fn supersede_published_missions_tx(
        &self,
        transaction: &Transaction<'_>,
        missions: &[String],
    ) -> Result<bool, FractalError> {
        let mut changed = initialize_supersession_queue(transaction)?;
        for mission in missions {
            // A replayed, older publication never rolls a run back: only the projected head
            // can supply a replacement, even when several publications share this page.
            let head = transaction.query_row(
                "SELECT d.revision, d.claim_id, c.store_index
                 FROM mission_definitions d JOIN claims c ON c.id=d.claim_id
                 WHERE d.mission_id=?1", [mission],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, u64>(2)?)),
            ).optional().map_err(internal)?;
            let Some((revision, publication, publication_index)) = head else { continue };
            let sources = {
                let mut query = transaction.prepare(
                    "SELECT r.id FROM mission_runs r
                     JOIN run_generations g ON g.id=r.current_generation_id
                     WHERE r.mission_id=?1 AND g.revision<>?2
                       AND r.status NOT IN ('completed','failed','cancelled')
                       AND r.phase IN ('normal','supersession-waiting')
                       AND EXISTS(SELECT 1 FROM claims c WHERE c.subject='mission-run/'||r.id
                         AND c.kind='mission-run.created' AND c.origin=?3
                         AND CASE
                           WHEN json_type(c.body,'$.fields.admission_publication') IS NULL
                             THEN c.store_index<?4
                           WHEN json_type(c.body,'$.fields.admission_publication')='null'
                             THEN 1
                           ELSE json_extract(c.body,'$.fields.admission_publication')<>?5 END)
                     ORDER BY r.created_at_unix_ms, r.id",
                ).map_err(internal)?;
                query.query_map(params![mission, revision, self.origin, publication_index, publication],
                    |row| row.get::<_, String>(0)).map_err(internal)?
                    .collect::<rusqlite::Result<Vec<_>>>().map_err(internal)?
            };
            for source_id in sources {
                let source = mission_run_header_tx(transaction, &source_id).map_err(internal)?;
                // Cancelling an earlier source can also cancel one of its descendants.
                if is_terminal_run_state(&source.status)
                    || !matches!(source.phase.as_str(), "normal" | "supersession-waiting")
                {
                    continue;
                }
                if mission_run_lineage_tx(transaction, &source.subject).map_err(internal)?.superseded_by.is_some() {
                    continue;
                }
                // Refusal is an immutable source/publication decision, not a retry queue.
                // Another page (or a rebuilt dirty queue) can encounter the same head again.
                let refused: bool = transaction.query_row(
                    "SELECT EXISTS(SELECT 1 FROM claims
                     WHERE subject=?1 AND kind='reconcile.fault'
                       AND json_extract(body,'$.fields.status')='faulted'
                       AND json_extract(body,'$.fields.scope')=?2
                       AND json_extract(body,'$.fields.publication')=?3)",
                    params![source.subject, FAULT_SCOPE, publication], |row| row.get(0),
                ).map_err(internal)?;
                if refused {
                    continue;
                }
                transaction.execute_batch(/* sqlite-large-value: allow-for-now lv-h880ef8d03231 owner=fractal_store::store::mission_supersession TODO=trace-statement-and-value-representation */ "SAVEPOINT mission_supersession_source").map_err(internal)?;
                let result = self.create_superseding_mission_run_tx(transaction, &source, &publication)
                    .and_then(|replacement| {
                        cancel_mission_run_tx(transaction, &self.origin, &source.subject,
                            &format!("superseded by {} at revision {revision}", replacement.subject),
                            None, now_ms())?;
                        Ok(())
                    });
                match result {
                    Ok(()) => {
                        transaction.execute_batch(/* sqlite-large-value: allow-for-now lv-h7afebadfccf5 owner=fractal_store::store::mission_supersession TODO=trace-statement-and-value-representation */ "RELEASE mission_supersession_source").map_err(internal)?;
                        if reconcile_fault_tx(transaction, &source.subject, FAULT_SCOPE).map_err(internal)?.is_some() {
                            append_claim_tx(transaction, &self.origin, &source.subject, "reconcile.fault", None,
                                &json!({"fields": {"scope": FAULT_SCOPE, "status": "recovered",
                                    "publication": publication}}), &[], None).map_err(internal)?;
                        }
                    }
                    Err(error) => {
                        transaction.execute_batch(/* sqlite-large-value: allow-for-now lv-h6499cb0102a5 owner=fractal_store::store::mission_supersession TODO=trace-statement-and-value-representation */ "ROLLBACK TO mission_supersession_source; RELEASE mission_supersession_source")
                            .map_err(internal)?;
                        append_claim_tx(transaction, &self.origin, &source.subject, "reconcile.fault", None,
                            &json!({"fields": {"scope": FAULT_SCOPE, "status": "faulted",
                                "publication": publication,
                                "reason": format!("cannot supersede revision {} with revision {revision}: {}: {}",
                                    source.revision, error.code, error.message)}}), &[], None).map_err(internal)?;
                    }
                }
                changed = true;
            }
        }
        for mission in missions {
            changed |= transaction.execute(
                /* sqlite-large-value: allow-for-now lv-h71c5f0a9c814 owner=fractal_store::store::mission_supersession TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_mission_supersession_heads_v1 WHERE mission_id=?1",
                [mission],
            ).map_err(internal)? != 0;
        }
        Ok(changed)
    }

    /// A replacement may dispatch only after cleanup of every predecessor has finished.
    pub fn resume_superseding_mission_run(&self, run: &str) -> Result<bool, FractalError> {
        let id = run.strip_prefix("mission-run/").unwrap_or(run);
        if !supersession_ready(&self.readers.get(), id, &self.origin)? {
            return Ok(false);
        }
        self.with_all_runs_fenced_internal(|transaction| {
            if !supersession_ready(transaction, id, &self.origin)? {
                return Ok(false);
            }
            let claim = append_claim_tx(transaction, &self.origin, &format!("mission-run/{id}"),
                "mission-run.state", None,
                &json!({"fields": {"status": "running", "phase": "normal",
                    "previous_phase": "supersession-waiting",
                    "reason": "all superseded predecessors finished cleanup"}}), &[], None).map_err(internal)?;
            project_mission_run_update(transaction, &claim)?;
            Ok(true)
        })
    }

    /// Cancellation must not let an unadmitted replacement run finalizers early.
    pub fn mission_run_supersession_predecessors_terminal(
        &self,
        run: &MissionRunView,
    ) -> Result<bool, FractalError> {
        smallclaims::touched::note_read(|| run.subject.clone());
        supersession_predecessors_terminal(&self.readers.get(), &run.subject)
    }

    /// Follow only explicit supersession edges; ordinary historical run reads stay exact.
    pub fn mission_run_supersession_successor(&self, run: &str) -> Result<String, FractalError> {
        let connection = self.readers.get();
        let mut current = format!("mission-run/{}", run.strip_prefix("mission-run/").unwrap_or(run));
        let mut seen = BTreeSet::new();
        loop {
            if !seen.insert(current.clone()) {
                return Err(supersession_cycle(&current));
            }
            smallclaims::touched::note_read(|| current.clone());
            match mission_run_lineage_tx(&connection, &current).map_err(internal)?.superseded_by {
                Some(next) => current = next,
                None => return Ok(current),
            }
        }
    }
}

fn supersession_ready(connection: &Connection, id: &str, origin: &str) -> Result<bool, FractalError> {
    let subject = format!("mission-run/{id}");
    smallclaims::touched::note_read(|| subject.clone());
    let waiting: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM mission_runs r WHERE r.id=?1
           AND r.phase='supersession-waiting' AND r.status NOT IN ('completed','failed','cancelled')
           AND EXISTS(SELECT 1 FROM claims c WHERE c.subject=?2 AND c.kind='mission-run.created' AND c.origin=?3))",
        params![id, subject, origin], |row| row.get(0),
    ).map_err(internal)?;
    if !waiting { return Ok(false) }
    supersession_predecessors_terminal(connection, &subject)
}

fn supersession_predecessors_terminal(
    connection: &Connection,
    subject: &str,
) -> Result<bool, FractalError> {
    let mut seen = BTreeSet::from([subject.to_owned()]);
    let mut predecessor = mission_run_lineage_tx(connection, subject).map_err(internal)?.supersedes;
    let mut ready = predecessor.is_some();
    while let Some(current) = predecessor {
        if !seen.insert(current.clone()) { return Err(supersession_cycle(&current)) }
        smallclaims::touched::note_read(|| current.clone());
        let terminal: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM mission_runs WHERE id=?1 AND phase='terminal')",
            [current.strip_prefix("mission-run/").unwrap_or(&current)], |row| row.get(0),
        ).map_err(internal)?;
        ready &= terminal;
        // Do not stop at a nonterminal node: each ancestor is a reactive dependency.
        predecessor = mission_run_lineage_tx(connection, &current).map_err(internal)?.supersedes;
    }
    Ok(ready)
}

fn supersession_cycle(subject: &str) -> FractalError {
    FractalError::new("mission-supersession-cycle", format!("mission supersession lineage cycles at `{subject}`"))
}

fn supersession_queue_installed(connection: &Connection) -> Result<bool, FractalError> {
    connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema
         WHERE type='table' AND name='local_mission_supersession_heads_v1')",
        [], |row| row.get(0),
    ).map_err(internal)
}

fn initialize_supersession_queue(transaction: &Transaction<'_>) -> Result<bool, FractalError> {
    if supersession_queue_installed(transaction)? {
        return Ok(false);
    }
    // Bootstrap once, then record only actual projected head changes. In particular,
    // owned-set publications may acquire their head only when a later receipt arrives.
    transaction.execute_batch(
        /* sqlite-large-value: allow-for-now lv-h7d697d7b1345 owner=fractal_store::store::mission_supersession TODO=chunk-or-reference-variable-columns */ "CREATE TABLE local_mission_supersession_heads_v1 (
             seq INTEGER PRIMARY KEY AUTOINCREMENT,
             mission_id TEXT NOT NULL UNIQUE
         );
         CREATE TRIGGER mission_supersession_head_insert_v1
         AFTER INSERT ON mission_definitions BEGIN
             INSERT INTO local_mission_supersession_heads_v1(mission_id)
             VALUES(NEW.mission_id) ON CONFLICT(mission_id) DO NOTHING;
         END;
         CREATE TRIGGER mission_supersession_head_update_v1
         AFTER UPDATE OF claim_id ON mission_definitions
         WHEN OLD.claim_id IS NOT NEW.claim_id BEGIN
             INSERT INTO local_mission_supersession_heads_v1(mission_id)
             VALUES(NEW.mission_id) ON CONFLICT(mission_id) DO NOTHING;
         END;
         CREATE INDEX IF NOT EXISTS claims_mission_supersession_refused_v1
         ON claims(subject, json_extract(body,'$.fields.scope'),
             json_extract(body,'$.fields.publication'))
         WHERE kind='reconcile.fault' AND json_extract(body,'$.fields.status')='faulted';
         INSERT OR IGNORE INTO local_mission_supersession_heads_v1(mission_id)
         SELECT mission_id FROM mission_definitions ORDER BY mission_id;",
    ).map_err(internal)?;
    Ok(true)
}

/// Cleanup waiting does not spend the fresh run's execution budget. An after-run
/// dependency retains its existing, later admission clock instead.
pub(super) fn arm_deadline_tx(
    transaction: &Transaction<'_>,
    run_id: &str,
    admitted_at_unix_ms: u128,
) -> Result<(), FractalError> {
    let timeout: Option<u64> = transaction.query_row(
        "SELECT timeout_ms FROM mission_run_deadlines
         WHERE run_id=?1 AND deadline_at_unix_ms IS NULL
           AND NOT EXISTS(SELECT 1 FROM mission_run_after WHERE run_id=?1)",
        [run_id], |row| row.get(0),
    ).optional().map_err(internal)?;
    if let Some(timeout) = timeout {
        let deadline = admitted_at_unix_ms.saturating_add(u128::from(timeout));
        transaction.execute(
            /* sqlite-large-value: allow-for-now lv-h9b2a4c147025 owner=fractal_store::store::mission_supersession TODO=bound-or-reference-values-and-reclamation */ "UPDATE mission_run_deadlines SET deadline_at_unix_ms=?2
             WHERE run_id=?1 AND deadline_at_unix_ms IS NULL",
            params![run_id, deadline.to_string()],
        ).map_err(internal)?;
    }
    Ok(())
}
