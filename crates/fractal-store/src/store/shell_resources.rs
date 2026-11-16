//! Durable local commands handed to the existing host resource writer.
use super::*;
use super::resource_membership::{Grant, LaneRequest};

pub const PARENT_ENV: &str = "FRACTAL_RESOURCE_PARENT";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ParentContext {
    pub grant: Grant,
    pub request: LaneRequest,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Command {
    pub id: String,
    pub lane: LaneRequest,
    pub argv: Vec<String>,
    pub cwd: String,
    pub environment: BTreeMap<String, String>,
    pub timeout_ms: Option<u64>,
    pub parent: Option<Grant>,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Execution {
    pub command: Command,
    pub state: String,
    pub reason: String,
    pub blocked_by: Option<resource_membership::BlockedBy>,
    pub queued_at: u64,
    pub revision: u64,
    pub queue_wait_ms: u64,
    pub started_at: Option<u64>,
    pub ended_at: Option<u64>,
    pub exit_code: Option<i32>,
    pub exit_signal: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub cancel: bool,
    pub timed_out: bool,
    pub head_start: Option<String>,
    pub clean_start: Option<bool>,
    pub result: Option<PayloadResult>,
}

pub(super) fn create_schema(connection: &Connection) -> Result<()> {
    // Transitional debt: command/result bodies and indexed text keys have no byte-size proof.
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-h9574bb92e055 owner=fractal_store::store::shell_resources TODO=chunk-or-reference-variable-columns */ "CREATE TABLE IF NOT EXISTS shell_resources_v1 (id TEXT PRIMARY KEY, host TEXT NOT NULL, state TEXT NOT NULL, body TEXT NOT NULL); CREATE INDEX IF NOT EXISTS shell_resources_host_v1 ON shell_resources_v1(host,state,id);")?;
    Ok(())
}

impl Store {
    pub fn submit_shell_resource(&self, command: Command, now: u64) -> Result<Execution> {
        anyhow::ensure!(!command.id.is_empty() && !command.argv.is_empty(), "shell execution requires identity and argv");
        anyhow::ensure!(command.cwd.starts_with('/'), "shell execution cwd must be absolute");
        let execution = Execution { command, revision: 0, queue_wait_ms: 0, state: "waiting".into(), reason: "waiting for host admission".into(), blocked_by: None,
            queued_at: now, started_at: None, ended_at: None, exit_code: None, exit_signal: None,
            stdout: String::new(), stderr: String::new(), cancel: false, timed_out: false, head_start: None, clean_start: None, result: None };
        // One row per command does not bound its serialized argv/environment/result bytes.
        self.connection.write().execute(/* sqlite-large-value: allow-for-now lv-h379f0157342b owner=fractal_store::store::shell_resources TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO shell_resources_v1(id,host,state,body) VALUES(?1,?2,?3,?4)",
            rusqlite::params![execution.command.id,execution.command.lane.host,execution.state,serde_json::to_string(&execution)?])?;
        self.resource_admission_elapsed(&format!("shell-wait:{}", execution.command.id), now, now);
        Ok(execution)
    }

    pub fn shell_resource(&self, id: &str) -> Result<Execution> {
        let body: String = self.readers.get().query_row("SELECT body FROM shell_resources_v1 WHERE id=?1", [id], |row| row.get(0))?;
        Ok(serde_json::from_str(&body)?)
    }

    pub fn shell_resources(&self, host: &str, active_only: bool) -> Result<Vec<Execution>> {
        let connection = self.readers.get();
        let mut statement = connection.prepare("SELECT body FROM shell_resources_v1 WHERE host=?1 AND (?2=0 OR state IN ('waiting','admitted','running','retiring','finishing')) ORDER BY id")?;
        let mut executions = statement.query_map(rusqlite::params![host,active_only], |row| row.get::<_,String>(0))?
            .map(|body| Ok(serde_json::from_str(&body?)?)).collect::<Result<Vec<Execution>>>()?;
        executions.sort_by(|left, right| left.command.lane.class.cmp(&right.command.lane.class)
            .then_with(|| left.queued_at.cmp(&right.queued_at))
            .then_with(|| left.command.id.cmp(&right.command.id)));
        Ok(executions)
    }

    pub fn update_shell_resource(&self, execution: &Execution) -> Result<bool> {
        let mut connection = self.connection.write();
        let transaction = connection.transaction()?;
        let body: String = transaction.query_row("SELECT body FROM shell_resources_v1 WHERE id=?1", [&execution.command.id], |row| row.get(0))?;
        let previous: Execution = serde_json::from_str(&body)?;
        let mut next = execution.clone();
        next.cancel |= previous.cancel;
        next.revision = previous.revision;
        if next == previous { return Ok(false); }
        next.revision += 1;
        // Primary-key selection bounds rows, not replacement body bytes or retained rows.
        transaction.execute(/* sqlite-large-value: allow-for-now lv-h0b67351b90df owner=fractal_store::store::shell_resources TODO=bound-or-reference-values-and-reclamation */ "UPDATE shell_resources_v1 SET state=?2,body=?3 WHERE id=?1",
            rusqlite::params![next.command.id,next.state,serde_json::to_string(&next)?])?;
        transaction.commit()?;
        if matches!(next.state.as_str(), "completed" | "refused" | "cancelled") {
            self.finish_resource_admission_clock(&format!("shell-wait:{}", next.command.id));
        }
        Ok(true)
    }

    pub fn cancel_shell_resource(&self, id: &str) -> Result<()> {
        // A scalar cancel flag still rewrites one potentially unbounded JSON body.
        self.connection.write().execute(/* sqlite-large-value: allow-for-now lv-hf463932112ca owner=fractal_store::store::shell_resources TODO=bound-or-reference-values-and-reclamation */ "UPDATE shell_resources_v1 SET body=json_set(body,'$.cancel',json('true')) WHERE id=?1", [id])?;
        Ok(())
    }

    pub fn shell_resource_queue_wait_ms(&self, execution: &Execution, now: u64) -> u64 {
        self.resource_admission_elapsed(&format!("shell-wait:{}", execution.command.id), execution.queued_at, now).0
    }

    /// Resolve native invocation evidence, never the caller's environment labels.
    pub fn shell_resource_agent_owner(&self, agent: &str) -> Result<resource_membership::Owner> {
        let harness = self.current_harness(agent)?.context("calling agent has no current harness")?;
        anyhow::ensure!(!matches!(harness.state.as_str(), "ended" | "indeterminate"), "calling agent harness is not live");
        let receipts = self.agent_invocation_receipts(agent)?;
        let receipt = receipts.iter().rev().find(|receipt| receipt.incarnation.as_deref() == Some(harness.incarnation_id.as_str()))
            .context("calling agent has no native invocation receipt for its current incarnation")?;
        let owner = match (&receipt.owner_run, &receipt.owner_step, receipt.owner_attempt) {
            (Some(run), Some(step), Some(attempt)) => resource_membership::Owner {
                run: run.clone(), step: step.clone(), attempt, invocation: Some(receipt.invocation_id.clone()),
            },
            (None, None, None) => resource_membership::Owner {
                run: receipt.invocation_id.clone(), step: receipt.invocation_id.clone(), attempt: 1,
                invocation: Some(receipt.invocation_id.clone()),
            },
            _ => anyhow::bail!("calling invocation has incomplete native step ownership"),
        };
        self.shell_resource_owner_live(&owner)?;
        Ok(owner)
    }

    /// Recheck the real invocation/attempt before admission and physical payload release.
    pub fn shell_resource_owner_live(&self, owner: &resource_membership::Owner) -> Result<()> {
        if let Some(invocation) = owner.invocation.as_deref().filter(|id| id.starts_with("invocation/")) {
            let receipts = self.invocation_receipts(invocation)?;
            use fractal_schema::invocation::InvocationPhase;
            anyhow::ensure!(!receipts.iter().any(|receipt| matches!(receipt.phase,
                InvocationPhase::StartupFailed | InvocationPhase::Revoked | InvocationPhase::Cancelled
                    | InvocationPhase::ProducerClosed | InvocationPhase::Ended)), "shell owner invocation ended");
            let receipt = receipts.iter().rev().find(|receipt| receipt.incarnation.is_some())
                .context("shell owner has no native invocation evidence")?;
            let harness = self.current_harness(&receipt.agent)?.context("shell owner harness disappeared")?;
            anyhow::ensure!(receipt.incarnation.as_deref() == Some(harness.incarnation_id.as_str())
                && !matches!(harness.state.as_str(), "ended" | "indeterminate"), "shell owner invocation is no longer live");
        }
        if owner.run.starts_with("mission-run/") || owner.invocation.as_deref()
            .is_some_and(|invocation| invocation.starts_with("invocation/") && owner.step != invocation) {
            let step = self.step_run(&owner.step)?.context("shell owner step disappeared")?;
            anyhow::ensure!(step.run == owner.run && step.attempt == owner.attempt
                && super::run_credentials::LIVE_STEP_STATES.contains(&step.status.as_str()), "shell owner attempt is no longer live");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PayloadResult {
    pub head_start: Option<String>,
    pub head_end: Option<String>,
    pub clean_start: Option<bool>,
    pub clean_end: Option<bool>,
    pub exit_code: Option<i32>,
    pub exit_signal: Option<i32>,
    pub wall_duration_ms: u64,
    pub memory_peak_bytes: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::resource_membership::{Coverage, Owner, RequestClass};

    fn command(id: &str) -> Command {
        Command { id: id.into(), lane: LaneRequest {
            id: format!("shell:{id}"), name: "checks".into(), host: "host-a".into(), max_members: 1,
            capacity: Some(fractal_runtime::Capacity { cores: 1, memory: 1 << 30 }),
            class: RequestClass::Check, disk: None, io: None, coverage: Coverage::Logical,
            admission_timeout_ms: None,
            owner: Owner { run: format!("shell:{id}"), step: format!("shell:{id}"), attempt: 1, invocation: Some(format!("shell:{id}")) },
        }, argv: vec!["true".into()], cwd: "/work".into(), environment: BTreeMap::new(), timeout_ms: None, parent: None }
    }

    #[test]
    fn durable_commands_are_queryable_without_becoming_second_admission_authority() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("store.db");
        {
            let store = Store::open(&path, "host-a").unwrap();
            store.submit_shell_resource(command("one"), 10).unwrap();
            assert!(store.resource_memberships_for_host("host-a").unwrap().is_empty());
        }
        let store = Store::open(&path, "host-a").unwrap();
        assert_eq!(store.shell_resource("one").unwrap().command.argv, ["true"]);
        assert_eq!(store.shell_resources("host-a", true).unwrap().len(), 1);
        assert!(store.shell_resources("other-host", false).unwrap().is_empty());
        let mut execution = store.shell_resource("one").unwrap();
        execution.state = "completed".into();
        execution.exit_code = Some(37);
        assert!(store.update_shell_resource(&execution).unwrap());
        assert!(!store.update_shell_resource(&execution).unwrap());
        assert_eq!(store.shell_resource("one").unwrap().exit_code, Some(37));
        assert!(store.shell_resources("host-a", true).unwrap().is_empty());
    }

    #[test]
    fn pending_shell_commands_reach_native_admission_in_class_then_fifo_order() {
        let store = Store::open_memory("host-a").unwrap();
        let mut agent = command("agent");
        agent.lane.class = RequestClass::Agent;
        store.submit_shell_resource(agent, 1).unwrap();
        store.submit_shell_resource(command("check-second"), 3).unwrap();
        store.submit_shell_resource(command("check-first"), 2).unwrap();
        let mut landing = command("landing");
        landing.lane.class = RequestClass::Landing;
        store.submit_shell_resource(landing, 4).unwrap();
        let executions = store.shell_resources("host-a", true).unwrap();
        assert_eq!(executions.iter().map(|execution| execution.command.id.as_str()).collect::<Vec<_>>(),
            ["landing", "check-first", "check-second", "agent"]);
    }

    #[test]
    fn cancellation_cannot_be_lost_to_an_in_flight_writer_snapshot() {
        let store = Store::open_memory("host-a").unwrap();
        let mut execution = store.submit_shell_resource(command("one"), 10).unwrap();
        store.cancel_shell_resource("one").unwrap();
        execution.state = "admitted".into();
        store.update_shell_resource(&execution).unwrap();
        assert!(store.shell_resource("one").unwrap().cancel);
        assert_eq!(store.shell_resource("one").unwrap().revision, 1);
    }

    #[test]
    fn agent_owner_is_native_invocation_custody_not_mutable_launch_environment() {
        use crate::store::invocation_receipts::{append_receipt_tx, native_receipt};
        use fractal_schema::invocation::InvocationPhase;
        let store = Store::open_memory("host-a").unwrap();
        let agent = "agent/host-a/worker";
        for (kind, fields) in [
            ("runtime.observed", json!({"status":"running","runtime_id":"worker","incarnation_id":"exact"})),
            ("harness.observed", json!({"state":"ready","driver":"omp","incarnation_id":"exact"})),
        ] {
            store.append_claim(&ClaimInput { subject:agent.into(), kind:kind.into(), actor:None,
                fields:serde_json::from_value(fields).unwrap(), evidence:vec![], idempotency_key:None, expected_subject:None }).unwrap();
        }
        let receipt = native_receipt("owner-operation", agent,
            &json!({"subject":{"kind":"agent"},"launch_member":{"driver":"omp","environment":{}}}),
            InvocationPhase::Started, Some(&json!({"verification":"operation-incarnation","incarnation":"exact"}))).unwrap().unwrap();
        store.connection.batched(|transaction| append_receipt_tx(transaction, store.origin(), &receipt)).unwrap().unwrap();
        let owner = store.shell_resource_agent_owner(agent).unwrap();
        assert_eq!(owner.run,"invocation/owner-operation");
        assert_eq!(owner.invocation.as_deref(),Some("invocation/owner-operation"));
        let mut launch = command("native");
        launch.lane.owner = owner.clone();
        launch.environment.insert("ST_MISSION_RUN".into(),"mission-run/spoofed".into());
        let mut execution = store.submit_shell_resource(launch,10).unwrap();
        execution.command.environment.clear();
        store.update_shell_resource(&execution).unwrap();
        assert_eq!(store.shell_resource("native").unwrap().command.lane.owner,owner);
        let mut ended = receipt.clone(); ended.phase = InvocationPhase::Ended;
        store.connection.batched(|transaction| append_receipt_tx(transaction, store.origin(), &ended)).unwrap().unwrap();
        assert!(store.shell_resource_owner_live(&owner).is_err(), "native invocation end fences even a stale ready harness view");
        store.append_claim(&ClaimInput { subject:agent.into(), kind:"harness.observed".into(), actor:None,
            fields:serde_json::from_value(json!({"state":"ended","driver":"omp","incarnation_id":"exact"})).unwrap(),
            evidence:vec![], idempotency_key:None, expected_subject:None }).unwrap();
        assert!(store.shell_resource_owner_live(&owner).is_err());
        assert!(store.shell_resource_agent_owner("agent/missing").is_err());
    }
}
