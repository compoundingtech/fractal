//! Graph-side admission and atomic effect receipts for canonical host assertions.
use super::{ExecutionBinding, RunBinding, Store, internal};
use fractal_wire::{Error as FractalError};
use fractal_light_node_core::authority::{local, assertions::{RequestAssertion, RequestIdentity}};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde_json::Value;
use fractal_auth::verify_signature;
#[cfg(test)]
use fractal_auth::MemberKey;

fn refused(reason: &str) -> FractalError {
    FractalError::new("run-request-refused", reason)
}

pub fn initialize_home_authority(connection: &Connection) -> Result<(), FractalError> {
    connection.execute_batch(
        /* sqlite-large-value: allow-for-now lv-h4fd849d8a786 owner=fractal_store::store::run_credentials::assertions TODO=chunk-or-reference-variable-columns */ "CREATE TABLE IF NOT EXISTS home_run_authority_v1 (
             host TEXT NOT NULL, revision TEXT NOT NULL, host_key TEXT NOT NULL,
             mission_revision TEXT NOT NULL, binding_json TEXT NOT NULL,
             epoch INTEGER NOT NULL, revoked INTEGER NOT NULL, PRIMARY KEY(host, revision)
         );
         CREATE TABLE IF NOT EXISTS home_run_epoch_fences_v1 (
             host TEXT PRIMARY KEY, highest_epoch INTEGER NOT NULL, closed INTEGER NOT NULL DEFAULT 0
         );
         CREATE TABLE IF NOT EXISTS home_run_request_receipts_v1 (
             host TEXT NOT NULL, request_id TEXT NOT NULL, identity_hash TEXT NOT NULL,
             outcome_json TEXT NOT NULL, PRIMARY KEY(host, request_id)
         );"
    ).map_err(internal)
}

fn current_revision(tx: &Transaction<'_>, binding: &ExecutionBinding) -> Result<String, FractalError> {
    let run = binding.run.as_ref().ok_or_else(|| refused("run authority is required"))?;
    tx.query_row(
        "SELECT generation.revision FROM step_runs step
         JOIN mission_runs mission ON mission.id=step.run_id
         JOIN run_generations generation ON generation.id=mission.current_generation_id
         WHERE step.subject=?1 AND step.attempt=?2 AND step.run_id=?3
           AND step.status IN ('ready','working','blocked','waiting-person')
           AND step.generation_id=mission.current_generation_id AND mission.phase<>'terminal'",
        params![run.step_run, run.attempt, run.run.trim_start_matches("mission-run/")],
        |row| row.get(0),
    ).optional().map_err(internal)?
        .ok_or_else(|| refused("the run attempt is ended or superseded"))
}

impl Store {
    /// Record authenticated execution provenance, not a start grant. A delayed ended execution
    /// becomes an inactive tombstone so its ordered change can be ACKed without authority.
    /// `host_key` comes from the selected execution grant, never from the assertion.
    pub fn admit_run_execution(&self, binding: &ExecutionBinding, host_key: &str) -> Result<(), FractalError> {
        binding.validate()?;
        if binding.home != self.origin || host_key.is_empty() {
            return Err(refused("the admitted home or host key does not match"));
        }
        let step = &binding.run.as_ref().ok_or_else(|| refused("run authority is required"))?.step_run;
        self.with_run_fence_internal(step, |tx| {
            let encoded = serde_json::to_string(binding).map_err(internal)?;
            let prior: Option<(String, String)> = tx.query_row(
                "SELECT host_key, binding_json FROM home_run_authority_v1 WHERE host=?1 AND revision=?2",
                params![binding.host, binding.authorization_revision], |row| Ok((row.get(0)?, row.get(1)?)),
            ).optional().map_err(internal)?;
            if let Some((key, prior)) = prior {
                if prior != encoded || (!key.is_empty() && key != host_key) {
                    return Err(refused("the admitted revision changed identity or execution key"));
                }
                return Ok(()); // Never reactivate a retained revoked revision.
            }
            let count: usize = tx.query_row("SELECT COUNT(*) FROM home_run_authority_v1", [], |row| row.get(0)).map_err(internal)?;
            if count >= local::MAX_RETAINED_AUTHORITIES {
                return Err(FractalError::new("home-authority-backpressure", "the retained home authority fences are full"));
            }
            let (highest, closed): (u64, bool) = tx.query_row(
                "SELECT highest_epoch, closed FROM home_run_epoch_fences_v1 WHERE host=?1",
                [&binding.host], |row| Ok((row.get(0)?, row.get(1)?)),
            ).optional().map_err(internal)?.ok_or_else(|| refused("the home selection was not installed"))?;
            if binding.epoch > highest {
                return Err(refused("the execution is ahead of the installed home selection"));
            }
            let (revision, mut revoked) = match current_revision(tx, binding) {
                Ok(revision) => (revision, binding.epoch < highest || closed),
                Err(error) if error.code == "run-request-refused" => (String::new(), true),
                Err(error) => return Err(error),
            };
            if !revoked {
                let run = super::super::mission_run_header_tx(tx, binding.run.as_ref().expect("run checked").run.trim_start_matches("mission-run/")).map_err(internal)?;
                revoked = self.mission_run_supersession_pending_tx(tx, &run).map_err(internal)?;
            }
            tx.execute(
                /* sqlite-large-value: allow-for-now lv-h68ade3293619 owner=fractal_store::store::run_credentials::assertions TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO home_run_authority_v1(host, revision, host_key, mission_revision, binding_json, epoch, revoked)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![binding.host, binding.authorization_revision, host_key, revision, encoded, binding.epoch, revoked],
            ).map_err(internal)?;
            Ok(())
        })
    }

    /// Revocation uses the same graph writer as effect admission. A request signed or forwarded
    /// earlier cannot cross this boundary later; an already committed retry can recover its receipt.
    pub fn revoke_run_execution(&self, binding: &ExecutionBinding) -> Result<(), FractalError> {
        binding.validate()?;
        if binding.home != self.origin { return Err(refused("the execution belongs to another home")); }
        let step = &binding.run.as_ref().ok_or_else(|| refused("run authority is required"))?.step_run;
        self.with_run_fence_internal(step, |tx| {
            let encoded = serde_json::to_string(binding).map_err(internal)?;
            let prior: Option<String> = tx.query_row(
                "SELECT binding_json FROM home_run_authority_v1 WHERE host=?1 AND revision=?2",
                params![binding.host, binding.authorization_revision], |row| row.get(0),
            ).optional().map_err(internal)?;
            if let Some(prior) = prior {
                if prior != encoded { return Err(refused("the revoked revision changed identity")); }
                tx.execute(/* sqlite-large-value: allow-for-now lv-hdf1e58f400c7 owner=fractal_store::store::run_credentials::assertions TODO=bound-or-reference-values-and-reclamation */ "UPDATE home_run_authority_v1 SET revoked=1 WHERE host=?1 AND revision=?2", params![binding.host, binding.authorization_revision]).map_err(internal)?;
            } else {
                let count: usize = tx.query_row("SELECT COUNT(*) FROM home_run_authority_v1", [], |row| row.get(0)).map_err(internal)?;
                if count >= local::MAX_RETAINED_AUTHORITIES {
                    return Err(FractalError::new("home-authority-backpressure", "the retained home authority fences are full"));
                }
                tx.execute(
                    /* sqlite-large-value: allow-for-now lv-h32dd16fa1ef9 owner=fractal_store::store::run_credentials::assertions TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO home_run_authority_v1(host, revision, host_key, mission_revision, binding_json, epoch, revoked)
                     VALUES (?1, ?2, '', '', ?3, ?4, 1)",
                    params![binding.host, binding.authorization_revision, encoded, binding.epoch],
                ).map_err(internal)?;
            }
            Ok(())
        })
    }

    /// Install only an authenticated home selection. The monotonic fence and invalidation of
    /// every old execution commit in the same graph writer order as request effects.
    pub fn install_run_epoch(&self, host: &str, epoch: u64) -> Result<(), FractalError> {
        if host.is_empty() || epoch == 0 || epoch > i64::MAX as u64 {
            return Err(refused("the selected host epoch is invalid"));
        }
        self.with_all_runs_fenced_internal(|tx| {
            let highest: Option<(u64, bool)> = tx.query_row(
                "SELECT highest_epoch, closed FROM home_run_epoch_fences_v1 WHERE host=?1", [host],
                |row| Ok((row.get(0)?, row.get(1)?)),
            ).optional().map_err(internal)?;
            if let Some((highest, closed)) = highest {
                if epoch < highest { return Err(refused("the selected home epoch regressed")); }
                if epoch == highest {
                    return if closed { Err(refused("a closed home epoch cannot reopen")) } else { Ok(()) };
                }
            }
            if highest.is_none() {
                let count: usize = tx.query_row("SELECT COUNT(*) FROM home_run_epoch_fences_v1", [], |row| row.get(0)).map_err(internal)?;
                if count >= local::MAX_RETAINED_AUTHORITIES {
                    return Err(FractalError::new("home-authority-backpressure", "the retained home epoch fences are full"));
                }
            }
            tx.execute(/* sqlite-large-value: allow-for-now lv-he1a544e814b9 owner=fractal_store::store::run_credentials::assertions TODO=bound-or-reference-values-and-reclamation */ "UPDATE home_run_authority_v1 SET revoked=1 WHERE host=?1 AND epoch<?2", params![host, epoch]).map_err(internal)?;
            tx.execute(
                /* sqlite-large-value: allow-for-now lv-h9dc209230794 owner=fractal_store::store::run_credentials::assertions TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO home_run_epoch_fences_v1(host, highest_epoch, closed) VALUES (?1, ?2, 0)
                 ON CONFLICT(host) DO UPDATE SET highest_epoch=excluded.highest_epoch, closed=0",
                params![host, epoch],
            ).map_err(internal)?;
            Ok(())
        })
    }

    /// Close effect admission immediately while the old authenticated transport drains receipts
    /// and revocations. Its durable closed fence cannot reopen at the same epoch.
    pub fn close_run_epoch(&self, host: &str, epoch: u64) -> Result<(), FractalError> {
        if host.is_empty() || epoch == 0 || epoch > i64::MAX as u64 {
            return Err(refused("the closing host epoch is invalid"));
        }
        self.with_all_runs_fenced_internal(|tx| {
            let prior: Option<(u64, bool)> = tx.query_row(
                "SELECT highest_epoch, closed FROM home_run_epoch_fences_v1 WHERE host=?1", [host],
                |row| Ok((row.get(0)?, row.get(1)?)),
            ).optional().map_err(internal)?;
            if prior.is_some_and(|(highest, closed)| highest > epoch || (highest == epoch && closed)) {
                return Ok(());
            }
            if prior.is_none() {
                let count: usize = tx.query_row("SELECT COUNT(*) FROM home_run_epoch_fences_v1", [], |row| row.get(0)).map_err(internal)?;
                if count >= local::MAX_RETAINED_AUTHORITIES {
                    return Err(FractalError::new("home-authority-backpressure", "the retained home epoch fences are full"));
                }
            }
            tx.execute(/* sqlite-large-value: allow-for-now lv-he183de98a1a2 owner=fractal_store::store::run_credentials::assertions TODO=bound-or-reference-values-and-reclamation */ "UPDATE home_run_authority_v1 SET revoked=1 WHERE host=?1 AND epoch<=?2", params![host, epoch]).map_err(internal)?;
            tx.execute(
                /* sqlite-large-value: allow-for-now lv-h4404d26934e5 owner=fractal_store::store::run_credentials::assertions TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO home_run_epoch_fences_v1(host, highest_epoch, closed) VALUES (?1, ?2, 1)
                 ON CONFLICT(host) DO UPDATE SET highest_epoch=excluded.highest_epoch, closed=1",
                params![host, epoch],
            ).map_err(internal)?;
            Ok(())
        })
    }

    /// The callback must perform the real graph effect with this transaction (including its
    /// ordinary run/rules checks), not schedule an unauthenticated later write or call a Store
    /// writer recursively. Its result and the effect commit together, so identical retries are
    /// exactly once. A failed callback rolls back both; no success receipt is fabricated.
    pub fn execute_run_request(
        &self, assertion: &RequestAssertion, actual: &RequestIdentity, now_ms: u64,
        effect: impl FnOnce(&Transaction<'_>, &RunBinding) -> Result<Value, FractalError> + Send,
    ) -> Result<Value, FractalError> {
        let admission = RunRequestAdmission { store: self, assertion, actual, now_ms };
        assertion.validate()?;
        let run = assertion.binding.run.as_ref().ok_or_else(|| refused("run authority is required"))?;
        self.with_run_fence_internal(&run.step_run, |tx| {
            if let Some(outcome) = admission.before(tx)? { return Ok(outcome); }
            let outcome = effect(tx, run)?;
            admission.record(tx, &outcome)?;
            Ok(outcome)
        })
    }

    /// Execute the graph-claim operation used by the durable peer session. It enters the same
    /// claim pipeline as the local API, including its schema, rules, CAS and projection gates.
    pub fn execute_asserted_claim(
        &self, assertion: &RequestAssertion, input: &fractal_wire::ClaimInput, now_ms: u64,
    ) -> Result<Value, FractalError> {
        assertion.validate()?;
        let run = assertion.binding.run.as_ref().ok_or_else(|| refused("run authority is required"))?;
        if input.actor.as_deref().is_some_and(|actor| actor != run.run) {
            return Err(refused("a run request cannot impersonate another actor"));
        }
        if input.kind == "message.sent"
            || super::super::local_retention(&input.kind)
            || super::super::latest_retention(&input.kind)
            || super::super::local_code(input)
        {
            return Err(refused("this operation requires its dedicated non-graph effect adapter"));
        }
        let payload = serde_json::to_vec(input).map_err(internal)?;
        let actual = RequestIdentity::new(
            assertion.request.request_id.clone(), "POST".into(), "/v1/claims".into(), &payload,
        );
        let mut input = input.clone();
        input.actor = Some(run.run.clone());
        fractal_schema::registry().validate_public_claim(
            &input.subject, &input.kind, &input.fields, input.actor.as_deref(),
        ).map_err(|error| FractalError::new(error.code, error.message))?;
        self.validate_claim_input(&input)?;
        let admission = RunRequestAdmission { store: self, assertion, actual: &actual, now_ms };
        let subjects = [run.step_run.clone()];
        let (claim, _) = super::super::append_claim_with_commit_context(
            &self.graph, &input, None, None, None, None,
            super::super::ClaimCommitContext {
                run_request: Some(&admission), run_projection_subjects: &subjects,
                ..Default::default()
            },
        )?;
        serde_json::to_value(claim).map_err(internal)
    }
}

pub struct RunRequestAdmission<'a> {
    pub store: &'a Store,
    pub assertion: &'a RequestAssertion,
    pub actual: &'a RequestIdentity,
    pub now_ms: u64,
}

impl RunRequestAdmission<'_> {
    pub(crate) fn before(&self, tx: &Transaction<'_>) -> Result<Option<Value>, FractalError> {
        let store = self.store;
        let assertion = self.assertion;
        let actual = self.actual;
        let now_ms = self.now_ms;
        assertion.validate()?;
        if assertion.binding.home != store.origin || &assertion.request != actual {
            return Err(refused("the home or request contents differ from the signed assertion"));
        }
        let binding = &assertion.binding;
        let run = binding.run.as_ref().ok_or_else(|| refused("run authority is required"))?;
        let (host_key, revision, encoded, revoked): (String, String, String, bool) = tx.query_row(
            "SELECT host_key, mission_revision, binding_json, revoked FROM home_run_authority_v1 WHERE host=?1 AND revision=?2",
            params![binding.host, binding.authorization_revision], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        ).optional().map_err(internal)?.ok_or_else(|| refused("the execution key was not admitted"))?;
        if encoded != serde_json::to_string(binding).map_err(internal)? {
            return Err(refused("the assertion differs from the admitted execution scope"));
        }
        if !verify_signature(&host_key, &assertion.message()?, &assertion.signature) {
            return Err(refused("the host signature is invalid"));
        }
        let identity = assertion.identity_hash()?;
        let prior: Option<(String, String)> = tx.query_row(
            "SELECT identity_hash, outcome_json FROM home_run_request_receipts_v1 WHERE host=?1 AND request_id=?2",
            params![binding.host, assertion.request.request_id], |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional().map_err(internal)?;
        if let Some((prior, outcome)) = prior {
            if prior != identity { return Err(refused("the request ID was reused for different contents or epoch")); }
            return serde_json::from_str(&outcome).map(Some).map_err(internal);
        }
        if now_ms < assertion.issued_at_unix_ms || now_ms >= assertion.expires_at_unix_ms {
            return Err(refused("the assertion is not currently valid"));
        }
        if revoked { return Err(refused("the admitted execution was revoked or ended before admission")); }
        if current_revision(tx, binding)? != revision {
            return Err(refused("the admitted graph authorization revision changed"));
        }
        let header = super::super::mission_run_header_tx(tx, run.run.trim_start_matches("mission-run/")).map_err(internal)?;
        if store.mission_run_supersession_pending_tx(tx, &header).map_err(internal)? {
            return Err(refused("the run awaits revision cutover"));
        }
        Ok(None)
    }

    pub(crate) fn record(&self, tx: &Transaction<'_>, outcome: &Value) -> Result<(), FractalError> {
        let count: usize = tx.query_row("SELECT COUNT(*) FROM home_run_request_receipts_v1", [], |row| row.get(0)).map_err(internal)?;
        if count >= local::MAX_RETAINED_AUTHORITIES {
            return Err(FractalError::new("home-authority-backpressure", "the retained run request receipts are full"));
        }
        let encoded = serde_json::to_string(outcome).map_err(internal)?;
        tx.execute(
            /* sqlite-large-value: allow-for-now lv-h32528dc1bb0d owner=fractal_store::store::run_credentials::assertions TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO home_run_request_receipts_v1(host, request_id, identity_hash, outcome_json) VALUES (?1, ?2, ?3, ?4)",
            params![self.assertion.binding.host, self.assertion.request.request_id, self.assertion.identity_hash()?, encoded],
        ).map_err(internal)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
