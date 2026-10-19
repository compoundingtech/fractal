//! The part of `store::mailbox_lease` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::mailbox_lease_shared_seats::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::mailbox_lease_shared_seats::{AFTER_FAULT_ACQUISITION, AFTER_PROMOTION_PRECHECK, BEFORE_FAULT_READER, CARD_FAULT_DEADLINES, DURING_FAULT_READ};
use super::*;
use crate::mailbox::{Authority, Fence};
use crate::model::{LaunchSpec, MemberKind, MemberSpec};
pub use st3_store::store::mailbox_lease::*;
pub use st3_store::store::mailbox_lease::{ChannelLiveness, admit, admit_with_probe, channel_liveness, check_argv_declaration, check_lease_fence, lease, record};

fn capture_mailbox_faults(
    connection: &Connection, subjects: &[String], index: u64,
) -> Result<BTreeMap<String, String>> {
    let work = smallclaims::profile::span("mailbox_faults/work");
    #[cfg(any(test, feature = "test-support"))]
    DURING_FAULT_READ.with(|hook| {
        hook.borrow_mut().take().map_or(Ok(()), |hook| hook(connection))
    })?;
    let query = canonical_sql(
        "WITH selected(subject) AS (SELECT DISTINCT value FROM json_each(?1)),
         failures AS MATERIALIZED (
           SELECT subject, (SELECT id FROM claims INDEXED BY claims_subject_kind_accepted_index
             WHERE claims.subject=selected.subject AND kind='operational.failure'
             AND +store_index<=?2
             AND json_extract(body,'$.fields.condition')='mailbox-channel-lost'
             ORDER BY CANONICAL_DESC(claims) LIMIT 1) AS failure FROM selected
         ), unresolved AS MATERIALIZED (
           SELECT subject,failure FROM failures WHERE failure IS NOT NULL
           AND NOT EXISTS(SELECT 1 FROM claims INDEXED BY claims_subject_kind_index
             WHERE claims.subject=failures.subject AND kind='operational.recovered'
             AND store_index<=?2 AND json_extract(body,'$.fields.failure')=failures.failure)
         )
         SELECT unresolved.subject, fault.body,
           (SELECT body FROM claims INDEXED BY claims_subject_kind_accepted_index
            WHERE claims.subject=unresolved.subject AND kind='runtime.observed'
            AND +store_index<=?2 ORDER BY CANONICAL_DESC(claims) LIMIT 1)
         FROM unresolved JOIN claims fault ON fault.id=unresolved.failure",
    );
    let mut statement = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-hb48185c671e1 owner=st3_store::store::mailbox_lease TODO=trace-statement-and-value-representation */ &query)?;
    let rows = statement.query_map(params![serde_json::to_string(subjects)?, index], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, Option<String>>(2)?))
    })?;
    let mut faults = BTreeMap::new();
    for row in rows {
        smallclaims::read_budget::check()?;
        let (subject, fault, runtime) = row?;
        let body: Value = serde_json::from_str(&fault)?;
        let runtime: Value = runtime.map(|body| serde_json::from_str(&body))
            .transpose()?.unwrap_or(Value::Null);
        let fields = runtime.get("fields").unwrap_or(&runtime);
        // A fault's exact runtime must still be the running one at this same cut.
        if fields["status"] == "running"
            && fields["incarnation_id"] == body["fields"]["incarnation"]
        {
            faults.insert(subject, body["fields"]["reason"].as_str()
                .unwrap_or("mailbox channel lost").into());
        }
    }
    // Finish the successful work check before statement/reader cleanup. A later
    // scheduling pause must not turn a completed read into a local work overrun.
    smallclaims::read_budget::check()?;
    drop(work);
    Ok(faults)
}

/// Whether `promote_current_mailbox_ownership` has nothing to promote for this lease row: there
/// is none, or the caller is not a bootstrap promotion and not a same-session sequence advance.
/// Re-exec can claim a newer sequence for the surviving provider session; only that exact
/// process and current capability may retain its lease, which the caller checks.
fn promotion_is_noop(row: Option<&(Fence, Authority, bool)>, authority: &Authority) -> bool {
    let Some((_, prior, _)) = row else {
        return true;
    };
    let advancing_session = prior.sequence > 0
        && authority.sequence > prior.sequence
        && authority.session == prior.session;
    authority.sequence == 0 || (prior.sequence != 0 && !advancing_session)
}

pub fn refuse_argv_native_adoption(connection: &Connection, fence: &Fence) -> Result<(), St3Error> {
    let retained: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM local_mailbox_argv_bindings WHERE token=?1)",
        [&fence.token], |row| row.get(0),
    ).map_err(internal)?;
    if retained { return Err(refused("an argv capability cannot be reissued or adopted as native custody")); }
    Ok(())
}

pub fn record_argv_binding(connection: &Connection, fence: &Fence, member: &MemberSpec) -> Result<(), St3Error> {
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-h1a5526aa3579 owner=st3_store::store::mailbox_lease TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO local_mailbox_argv_bindings VALUES (?1,?2,?3,?4,?5,?6) ON CONFLICT(token) DO NOTHING",
        params![fence.token,fence.subject,fence.component,fence.incarnation,fence.epoch,serde_json::to_string(member).map_err(internal)?],
    ).map_err(internal)?;
    Ok(())
}

pub fn check_declaration(
    connection: &Connection,
    fence: &Fence,
    authority: &Authority,
    host: &str,
) -> Result<(), St3Error> {
    let declared = current_desired_row(connection, &fence.subject)
        .map_err(internal)?
        .ok_or_else(|| refused("the seat is no longer declared"))?;
    let member: MemberSpec = declared
        .member
        .as_deref()
        .map(serde_json::from_str)
        .transpose()
        .map_err(internal)?
        .ok_or_else(|| refused("the seat is intentionally stopped"))?;
    if declared.kind == "stop"
        || member.driver.as_deref() != Some(&authority.provider)
        || member.host != host
        || member
            .terminal_binding
            .as_ref()
            .is_some_and(|binding| binding.agent_incarnation() != fence.incarnation)
    {
        return Err(refused(
            "the seat is stopped, moved or belongs to another provider",
        ));
    }
    Ok(())
}

pub trait MailboxLeaseStore {
    fn mailbox_failure_episode(
            &self,
            fence: &Fence,
            episode: &str,
        ) -> Result<Option<(String, bool)>, St3Error>;
    /// The caller holds the provider ownership lock. Recheck physical/declaration and
    /// canonical custody inside the same Store writer transaction as fault publication.
    fn append_mailbox_lease_claim(
            &self,
            fence: &Fence,
            owner: &Authority,
            input: &ClaimInput,
            repaired: bool,
        ) -> Result<ClaimRecord, St3Error>;
    /// Promote physical Codex bootstrap custody, or advance a surviving process's
    /// same-session ownership after re-exec, under the caller's provider ownership lock.
    /// Both retain only the still-current capability and exact process generation.
    ///
    /// A reader first reads the lease row, only to skip the writer when this call can promote
    /// nothing: no lease row, or `promotion_is_noop`. That result never authorizes anything.
    /// It is released before the writer is borrowed, and any call that may promote reruns the
    /// whole transaction below on the writer's current state.
    fn promote_current_mailbox_ownership(
            &self,
            fence: &Fence,
            authority: &Authority,
        ) -> Result<(), St3Error>;
    fn mailbox_lease_authority(
            &self,
            fence: &Fence,
        ) -> Result<Option<Authority>, St3Error>;
    /// Compare replay inputs without allocating another decoded lease. This is
    /// not report admission: runtime, custody and binding checks still run there.
    fn mailbox_replay_owner_matches(
            &self,
            fence: &Fence,
            owner: &Authority,
        ) -> Result<bool, St3Error>;
    /// A transient physical query may retry only a still-current canonical capability.
    /// An absent owner row can be repaired; a foreign owner or retired binding is terminal.
    fn check_mailbox_custody_fence(&self, fence: &Fence) -> Result<(), St3Error>;
    fn mailbox_session_active(
            &self,
            fence: &Fence,
            authority: &Authority,
        ) -> Result<(), St3Error>;
    fn has_mailbox_lease(&self, fence: &Fence) -> Result<bool, St3Error>;
    /// Captured public-card fault source, bounded to the selected subjects and cut.
    /// Keep the capture on one blocking worker: its CPU budget belongs to that thread.
    fn mailbox_faults_for(
            &self,
            subjects: &[String],
            index: u64,
        ) -> Result<BTreeMap<String, String>>;
    fn owns_mailbox_lease(
            &self,
            fence: &Fence,
            authority: &Authority,
        ) -> Result<bool, St3Error>;
    /// Repair only the exact canonical lease. Never mint a token or advance an epoch.
    /// Deleting/revoking a binding is terminal; only displacement of its owner is repairable.
    #[cfg(any(test, feature = "test-support"))]
    fn repair_mailbox(
            &self,
            fence: &Fence,
            authority: &Authority,
        ) -> Result<bool, St3Error>;
    fn repair_mailbox_checked(
            &self,
            fence: &Fence,
            authority: &Authority,
            validate: &dyn Fn() -> Result<(), St3Error>,
        ) -> Result<bool, St3Error>;
}

impl MailboxLeaseStore for Store {

    fn mailbox_failure_episode(
            &self,
            fence: &Fence,
            episode: &str,
        ) -> Result<Option<(String, bool)>, St3Error> {
        let connection = self.readers.get();
        connection
            .prepare_cached(/* sqlite-large-value: allow-for-now lv-ha88f631727c8 owner=st3_store::store::mailbox_lease TODO=trace-statement-and-value-representation */ &canonical_sql(
                "SELECT id, NOT EXISTS(
                    SELECT 1 FROM claims AS recovery INDEXED BY claims_subject_kind_index
                    WHERE recovery.subject=claims.subject
                    AND recovery.kind='operational.recovered'
                    AND json_extract(recovery.body,'$.fields.failure')=claims.id)
                 FROM claims INDEXED BY claims_subject_kind_accepted_index
                 WHERE subject=?1 AND kind='operational.failure'
                 AND json_extract(body,'$.fields.episode')=?2
                 ORDER BY CANONICAL_DESC(claims) LIMIT 1",
            ))
            .map_err(internal)?
            .query_row(/* sqlite-large-value: allow-for-now lv-hd0a028b98419 owner=st3_store::store::mailbox_lease TODO=trace-statement-and-value-representation */ params![fence.subject, episode], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .optional()
            .map_err(internal)
    }

    fn append_mailbox_lease_claim(
            &self,
            fence: &Fence,
            owner: &Authority,
            input: &ClaimInput,
            repaired: bool,
        ) -> Result<ClaimRecord, St3Error> {
        let admission = |connection: &Connection| {
            check_mailbox_incarnation(connection, fence)?;
            check_declaration(connection, fence, owner, &self.origin)?;
            let Some((held, authority, revoked)) = lease(connection, fence)? else {
                return Err(refused("fault publication lost canonical custody"));
            };
            if revoked
                || held.token != fence.token
                || held.epoch != fence.epoch
                || held.incarnation != fence.incarnation
                || authority != *owner
            {
                return Err(refused("fault publication belongs to a retired lease"));
            }
            if repaired {
                check_mailbox_fence(connection, fence, &self.origin)?;
            }
            if let Some(revision) = input.fields.get("source_revision").and_then(Value::as_str)
                && current_desired_row(connection, &fence.subject)
                    .map_err(internal)?
                    .is_none_or(|row| row.claim_id != revision)
            {
                return Err(refused("the fault's configured owner declaration changed"));
            }
            Ok(())
        };
        append_claim_with_commit_context(&self.graph, input, None, None, None, None,
            ClaimCommitContext { admission: Some(&admission), ..Default::default() })
            .map(|(claim, _)| claim)
    }

    fn promote_current_mailbox_ownership(
            &self,
            fence: &Fence,
            authority: &Authority,
        ) -> Result<(), St3Error> {
        let observed = lease(&self.readers.get(), fence)?;
        if promotion_is_noop(observed.as_ref(), authority) {
            return Ok(());
        }
        drop(observed);
        #[cfg(any(test, feature = "test-support"))]
        AFTER_PROMOTION_PRECHECK.with(|pause| {
            let pause = pause.borrow_mut().take();
            if let Some(pause) = pause {
                pause();
            }
        });
        let mut connection = self.connection.write();
        let tx = connection.transaction().map_err(internal)?;
        let current = lease(&tx, fence)?;
        if promotion_is_noop(current.as_ref(), authority) {
            return Ok(());
        }
        let Some((held, prior, revoked)) = current else {
            return Ok(());
        };
        if held.incarnation != fence.incarnation
            || held.token != fence.token
            || held.epoch != fence.epoch
        {
            return Ok(());
        }
        if revoked
            || prior.pid != authority.pid
            || prior.process_token != authority.process_token
            || prior.provider != authority.provider
        {
            return Err(refused(
                "current custody belongs to another physical process",
            ));
        }
        check_mailbox_incarnation(&tx, fence)?;
        check_mailbox_fence(&tx, fence, &self.origin)?;
        check_declaration(&tx, fence, authority, &self.origin)?;
        record(&tx, fence, authority)?;
        tx.commit().map_err(internal)
    }

    fn mailbox_lease_authority(
            &self,
            fence: &Fence,
        ) -> Result<Option<Authority>, St3Error> {
        Ok(
            lease(&self.readers.get(), fence)?.and_then(|(held, owner, _)| {
                (held.incarnation == fence.incarnation
                    && held.token == fence.token
                    && held.epoch == fence.epoch)
                    .then_some(owner)
            }),
        )
    }

    fn mailbox_replay_owner_matches(
            &self,
            fence: &Fence,
            owner: &Authority,
        ) -> Result<bool, St3Error> {
        self.readers.get().query_row(
            "SELECT EXISTS(SELECT 1 FROM local_mailbox_leases
             WHERE subject=?1 AND component=?2 AND incarnation=?3
             AND token=?4 AND epoch=?5 AND provider=?6 AND session=?7
             AND sequence=?8 AND pid=?9 AND process_token=CAST(?10 AS TEXT)
             AND revoked=0)",
            params![fence.subject, fence.component, fence.incarnation, fence.token,
                fence.epoch, owner.provider, owner.session, owner.sequence,
                owner.pid, owner.process_token],
            |row| row.get(0),
        ).map_err(internal)
    }

    fn check_mailbox_custody_fence(&self, fence: &Fence) -> Result<(), St3Error> {
        let connection = self.readers.get();
        check_mailbox_incarnation(&connection, fence)?;
        if lease(&connection, fence)?.is_none() {
            return check_mailbox_fence(&connection, fence, &self.origin);
        }
        check_lease_fence(&connection, fence, &self.origin)?;
        let bound: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM local_mailbox_bindings WHERE token=?1 AND subject=?2
             AND component=?3 AND incarnation=?4 AND epoch=?5)",
            params![fence.token, fence.subject, fence.component, fence.incarnation, fence.epoch],
            |row| row.get(0),
        ).map_err(internal)?;
        if !bound { return Err(refused("the binding token was revoked")); }
        let has_owner: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM local_mailbox_owners WHERE subject=?1 AND component=?2)",
            params![fence.subject,fence.component], |row| row.get(0),
        ).map_err(internal)?;
        if has_owner { check_mailbox_fence(&connection, fence, &self.origin)?; }
        Ok(())
    }

    fn mailbox_session_active(
            &self,
            fence: &Fence,
            authority: &Authority,
        ) -> Result<(), St3Error> {
        let connection = self.readers.get();
        check_mailbox_incarnation(&connection, fence)?;
        check_declaration(&connection, fence, authority, &self.origin)
    }

    fn has_mailbox_lease(&self, fence: &Fence) -> Result<bool, St3Error> {
        Ok(lease(&self.readers.get(), fence)?.is_some())
    }

    fn mailbox_faults_for(
            &self,
            subjects: &[String],
            index: u64,
        ) -> Result<BTreeMap<String, String>> {
        if subjects.is_empty() { return Ok(BTreeMap::new()); }
        let _capture = smallclaims::profile::span("mailbox_faults/capture");
        // Admission and result cleanup remain within the enclosing wall deadline.
        // Captures without a request retain their existing finite wall bound.
        let duration = std::time::Duration::from_millis(25);
        #[cfg(any(test, feature = "test-support"))]
        let duration = CARD_FAULT_DEADLINES.with(|remaining| {
            let count = remaining.get();
            remaining.set(count.saturating_sub(1));
            if count == 0 { duration } else { std::time::Duration::ZERO }
        });
        let budget = smallclaims::read_budget::current().unwrap_or_else(
            || smallclaims::read_budget::ReadBudget::new("mailbox/card-faults", duration),
        );
        let checked = budget.clone();
        smallclaims::read_budget::with(Some(budget), || {
            checked.check()?;
            #[cfg(any(test, feature = "test-support"))]
            BEFORE_FAULT_READER.with(|pause| {
                if let Some(pause) = pause.borrow_mut().take() { pause(); }
            });
            let read = || -> Result<BTreeMap<String, String>> {
                let acquisition = smallclaims::profile::span("mailbox_faults/acquisition");
                let connection = self.readers.get();
                drop(acquisition);
                let work = checked.child_work(duration)?;
                let measured = work.clone();
                smallclaims::read_budget::with(Some(work), || {
                    match capture_mailbox_faults(&connection, subjects, index) {
                        Ok(faults) => Ok(faults),
                        Err(error) => {
                            // SQLite can interrupt before the first row. Normalize its
                            // error before leaving the worker's authoritative work scope.
                            measured.check()?;
                            Err(error)
                        }
                    }
                })
            };
            // Keep the authoritative existing snapshot; otherwise acquire cancellable
            // admission before the fallible reader checkout.
            let pinned = self.readers.has_pinned_reader();
            let result = if pinned { read() } else {
                self.readers.request_read(read).and_then(|result| result)
            };
            #[cfg(any(test, feature = "test-support"))]
            AFTER_FAULT_ACQUISITION.with(|pause| {
                if let Some(pause) = pause.borrow_mut().take() { pause(); }
            });
            // Only the enclosing request can expire after successful reader work. Also
            // preserve its typed cancellation when admission or acquisition failed.
            checked.check()?;
            result
        })
    }

    fn owns_mailbox_lease(
            &self,
            fence: &Fence,
            authority: &Authority,
        ) -> Result<bool, St3Error> {
        Ok(
            lease(&self.readers.get(), fence)?.is_some_and(|(held, owner, _)| {
                held.incarnation == fence.incarnation
                    && held.token == fence.token
                    && held.epoch == fence.epoch
                    && owner == *authority
            }),
        )
    }

    #[cfg(any(test, feature = "test-support"))]
    fn repair_mailbox(
            &self,
            fence: &Fence,
            authority: &Authority,
        ) -> Result<bool, St3Error> {
        self.repair_mailbox_checked(fence, authority, &|| Ok(()))
    }

    fn repair_mailbox_checked(
            &self,
            fence: &Fence,
            authority: &Authority,
            validate: &dyn Fn() -> Result<(), St3Error>,
        ) -> Result<bool, St3Error> {
        let mut connection = self.connection.write();
        let tx = connection.transaction().map_err(internal)?;
        check_mailbox_incarnation(&tx, fence)?;
        validate()?;
        check_declaration(&tx, fence, authority, &self.origin)?;
        let Some((owner, held, revoked)) = lease(&tx, fence)? else {
            return Err(refused("this binding has no authenticated recovery lease"));
        };
        if revoked
            || owner.token != fence.token
            || owner.epoch != fence.epoch
            || owner.incarnation != fence.incarnation
            || held != *authority
        {
            return Err(refused(
                "the authenticated mailbox lease is superseded or revoked",
            ));
        }
        let binding: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM local_mailbox_bindings WHERE token=?1 AND
             subject=?2 AND component=?3 AND incarnation=?4 AND epoch=?5)",
                params![
                    fence.token,
                    fence.subject,
                    fence.component,
                    fence.incarnation,
                    fence.epoch
                ],
                |row| row.get(0),
            )
            .map_err(internal)?;
        if !binding {
            return Err(refused("the binding token was revoked"));
        }
        if check_mailbox_fence(&tx, fence, &self.origin).is_ok() {
            return Ok(false);
        }
        let owner_epoch: Option<u64> = tx
            .query_row(
                "SELECT epoch FROM local_mailbox_owners WHERE subject=?1 AND component=?2",
                params![fence.subject, fence.component],
                |row| row.get(0),
            )
            .optional()
            .map_err(internal)?;
        if owner_epoch.is_some() {
            return Err(refused("a successor owner cannot be rewound by recovery"));
        }
        tx.execute(
            /* sqlite-large-value: allow-for-now lv-h9d72a0d5d1ad owner=st3_store::store::mailbox_lease TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO local_mailbox_owners VALUES (?1,?2,?3,?4)
             ON CONFLICT(subject,component) DO UPDATE SET incarnation=excluded.incarnation,epoch=excluded.epoch",
            params![fence.subject,fence.component,fence.incarnation,fence.epoch],
        ).map_err(internal)?;
        check_mailbox_fence(&tx, fence, &self.origin)?;
        tx.commit().map_err(internal)?;
        if let Some(wakes) = self.smalltalk.mailbox_wakes.get() {
            wakes.owner_changed(&fence.subject, &fence.component);
        }
        Ok(true)
    }
}


#[cfg(test)]
mod tests {
    #[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::mailbox_lease_tests_shared_seats::*;
    #[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::mailbox_lease_tests_shared_seats::{authority, change_after_precheck, finished_without_the_writer, precheck_was_reached, promote_elsewhere, request};
    use super::*;

    #[test]
    fn fault_capture_refuses_failed_acquisition_without_waiting_and_preserves_a_pin() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("graph.db");
        let saved = root.path().join("graph.saved");
        let store = Arc::new(Store::open(&path, "node").unwrap());
        let subjects = vec!["agent/absent".into()];
        // A surviving pinned snapshot must not be discarded to open another reader.
        store.read_snapshot(|cut| -> Result<()> {
            let held = std::mem::take(&mut *store.readers.idle.lock().unwrap());
            std::fs::rename(&path, &saved)?;
            let result = store.mailbox_faults_for(&subjects, cut);
            std::fs::rename(&saved, &path)?;
            drop(held);
            assert!(result?.is_empty());
            Ok(())
        }).unwrap();
        // Outside a pin, deliberately make opening impossible with every idle reader
        // retained. Observe a finite result before returning any reader to the pool.
        let held = std::mem::take(&mut *store.readers.idle.lock().unwrap());
        std::fs::rename(&path, &saved).unwrap();
        let worker_store = store.clone();
        let (sent, received) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            assert!(worker_store.mailbox_faults_for(&subjects, 0).is_err());
            // The acquisition has already failed. Cancel exactly at its result boundary,
            // so an early outer `?` would miss the authoritative typed deadline check.
            let reached = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let observed = reached.clone();
            AFTER_FAULT_ACQUISITION.with(|pause| {
                *pause.borrow_mut() = Some(Box::new(move || {
                    observed.store(true, std::sync::atomic::Ordering::SeqCst);
                    smallclaims::read_budget::current().unwrap().cancel();
                }));
            });
            let expired = worker_store.mailbox_faults_for(&subjects, 0).unwrap_err();
            sent.send(reached.load(std::sync::atomic::Ordering::SeqCst)
                && expired.downcast_ref::<St3Error>().is_some_and(|error| error.code == "read-deadline")).unwrap();
        });
        let result = received.recv_timeout(std::time::Duration::from_secs(1));
        std::fs::rename(&saved, &path).unwrap();
        // Release readers even on failure, so the old waiting implementation cannot
        // leave a hung control thread after the finite observation fails.
        store.readers.idle.lock().unwrap().extend(held);
        store.readers.returned.notify_all();
        worker.join().unwrap();
        assert!(result.unwrap(), "failed acquisition must refuse promptly");
    }
}
