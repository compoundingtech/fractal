//! Host-writer-owned finite execution custody. Replicated observations are never grants.
use super::*;
use fractal_runtime::{Capacity, ExecGeneration, ExecRuntime};

#[path = "resource_membership_observations.rs"]
mod observations;
pub(super) use observations::Observations;

#[path = "resource_membership_clocks.rs"]
mod clocks;
pub(super) use clocks::Clocks;

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Owner {
    pub run: String,
    pub step: String,
    pub attempt: u32,
    pub invocation: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Coverage { Logical, Enforced }

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LaneRequest {
    pub id: String,
    pub name: String,
    pub host: String,
    pub max_members: u32,
    pub capacity: Option<Capacity>,
    pub coverage: Coverage,
    pub admission_timeout_ms: Option<u64>,
    pub owner: Owner,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Grant {
    pub membership_id: String,
    pub authority_epoch: String,
    pub generation: String,
    pub coverage: Coverage,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum Admission { Queued { reason: String }, Admitted(Grant), Refused { code: RefusalCode, reason: String } }

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum RefusalCode { AdmissionExpired, UnsupportedCoverage, ImpossibleCapacity, ClockUncertain, Cancelled, Retired, Retiring, Uncertain, InvalidState }

impl RefusalCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AdmissionExpired => "admission-expired",
            Self::UnsupportedCoverage => "unsupported-coverage",
            Self::ImpossibleCapacity => "impossible-capacity",
            Self::ClockUncertain => "clock-uncertain",
            Self::Cancelled => "cancelled",
            Self::Retired => "retired",
            Self::Retiring => "retiring",
            Self::Uncertain => "uncertain",
            Self::InvalidState => "invalid-state",
        }
    }
}

/// Computed by the existing host lane accountant in its serialized admission pass.
/// `held_bytes` is seats only. Finite custody is added inside the grant transaction, never from
/// a potentially stale caller snapshot. Seat admission reads that same custody inventory.
#[derive(Clone, Debug, Default)]
pub struct Fit {
    pub room: Option<Capacity>,
    pub held_bytes: u64,
    pub outside_bytes: u64,
    pub reserve_bytes: u64,
    pub fifo_predecessor: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Membership {
    pub request: LaneRequest,
    pub grant: Option<Grant>,
    pub state: String,
    pub reason: String,
    pub queued_at: u64,
    pub admitted_at: Option<u64>,
    pub payload_released_at: Option<u64>,
    pub execution_ended_at: Option<u64>,
    #[serde(default)]
    pub retirement_requested_at: Option<u64>,
    pub retired_at: Option<u64>,
    #[serde(default)]
    pub roots: Vec<ExecutionRoot>,
    #[serde(default)]
    pub child_runs: Vec<String>,
    #[serde(default)]
    pub clock_uncertain: bool,
    #[serde(default)]
    pub clock_epoch: String,
    #[serde(default)]
    pub refusal_code: Option<RefusalCode>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionRoot {
    pub runtime_id: String,
    pub execution: Option<ExecutionIdentity>,
    pub payload_released_at: Option<u64>,
    pub execution_ended_at: Option<u64>,
}

/// Reference to the runtime-owned generation record; command outcomes remain runtime facts.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionIdentity {
    pub generation_id: String,
}

impl From<&ExecGeneration> for ExecutionIdentity {
    fn from(execution: &ExecGeneration) -> Self {
        Self { generation_id: execution.generation_id.clone() }
    }
}

pub fn dependency(host: &str, lane: &str) -> String { format!("resource-membership/{host}/{lane}") }

pub(super) fn create_schema(connection: &Connection) -> Result<(), FractalError> {
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-hb7bab5eb8be2 owner=fractal_store::store::resource_membership TODO=chunk-or-reference-variable-columns */ "CREATE TABLE IF NOT EXISTS resource_authorities_v1 (
        host TEXT PRIMARY KEY, epoch TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS resource_lane_policies_v1 (
        host TEXT NOT NULL, lane TEXT NOT NULL, policy TEXT NOT NULL, PRIMARY KEY(host,lane));
        CREATE TABLE IF NOT EXISTS resource_memberships_v1 (
        id TEXT PRIMARY KEY, host TEXT NOT NULL, lane TEXT NOT NULL,
        state TEXT NOT NULL, queued_at INTEGER NOT NULL, memory INTEGER NOT NULL,
        body TEXT NOT NULL);
        CREATE INDEX IF NOT EXISTS resource_memberships_queue_v1
        ON resource_memberships_v1(host,lane,state,queued_at,id);")
        .map_err(internal)
}

fn read(connection: &Connection, id: &str) -> Result<Membership> {
    let body: String = connection.query_row("SELECT body FROM resource_memberships_v1 WHERE id=?1", [id], |row| row.get(0))?;
    Ok(serde_json::from_str(&body)?)
}

fn write(connection: &Transaction<'_>, member: &Membership) -> Result<()> {
    let changed = connection.execute(/* sqlite-large-value: allow-for-now lv-h15191adeac97 owner=fractal_store::store::resource_membership TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO resource_memberships_v1(id,host,lane,state,queued_at,memory,body)
        VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(id) DO UPDATE SET state=excluded.state,body=excluded.body
        WHERE resource_memberships_v1.body<>excluded.body",
        rusqlite::params![member.request.id,member.request.host,member.request.name,member.state,member.queued_at,
            member.request.capacity.map_or(0, |c| c.memory),serde_json::to_string(member)?])?;
    if changed == 0 { return Ok(()); }
    // Durable feed invalidation commits with custody; a diagnostic note alone cannot wake IVM.
    // This observation references custody, never carries a replayable grant.
    insert_local_observation_tx(connection, &member.request.host, &ClaimInput {
        subject: dependency(&member.request.host, &member.request.name),
        kind: "resource-membership.changed".into(), actor: None,
        fields: BTreeMap::from([("membership".into(), json!(member.request.id)), ("state".into(), json!(member.state))]),
        evidence: Vec::new(), idempotency_key: None,
        expected_subject: None,
    }, now_ms())?;
    Ok(())
}

fn authorized(member: &Membership, owner: &Owner) -> Result<()> {
    anyhow::ensure!(&member.request.owner == owner, "resource membership owner mismatch");
    Ok(())
}

fn fenced(connection: &Connection, member: &Membership, grant: &Grant) -> Result<()> {
    anyhow::ensure!(member.grant.as_ref() == Some(grant), "stale resource generation");
    let epoch: String = connection.query_row("SELECT epoch FROM resource_authorities_v1 WHERE host=?1", [&member.request.host], |r| r.get(0))?;
    anyhow::ensure!(epoch == grant.authority_epoch, "stale resource authority");
    Ok(())
}

fn admission(member: Membership) -> Admission {
    match member.state.as_str() {
        "admitted" | "launched" => member.grant.map_or_else(
            || Admission::Refused { code: RefusalCode::InvalidState, reason: "missing durable grant".into() }, Admission::Admitted),
        "queued" => Admission::Queued { reason: member.reason },
        _ => Admission::Refused { code: member.refusal_code.unwrap_or(match member.state.as_str() {
            "retired" => RefusalCode::Retired, "retiring" => RefusalCode::Retiring,
            "uncertain" => RefusalCode::Uncertain, _ => RefusalCode::InvalidState,
        }), reason: member.reason },
    }
}

impl Store {
    pub fn resource_membership_record(&self, id: &str, owner: &Owner) -> Result<Membership> {
        let member = read(&self.readers.get(), id)?;
        authorized(&member, owner)?;
        smallclaims::touched::note_read(|| dependency(&member.request.host, &member.request.name));
        Ok(member)
    }

    pub fn resource_admission_remaining(&self, id: &str, owner: &Owner, now_ms: u64) -> Result<Option<u64>> {
        let member = self.resource_membership_record(id, owner)?;
        if !matches!(member.state.as_str(), "queued" | "admitted") || member.payload_released_at.is_some() { return Ok(None); }
        let (elapsed, _) = self.resource_admission_elapsed(id, member.queued_at, now_ms);
        Ok(member.request.admission_timeout_ms.map(|budget| budget.saturating_sub(elapsed)))
    }

    pub fn resource_membership_for_grant(&self, grant: &Grant) -> Result<Membership> {
        let connection = self.readers.get();
        let member = read(&connection, &grant.membership_id)?;
        fenced(&connection, &member, grant)?;
        smallclaims::touched::note_read(|| dependency(&member.request.host, &member.request.name));
        Ok(member)
    }

    pub fn resource_memberships_for_host(&self, host: &str) -> Result<Vec<Membership>> {
        let connection = self.readers.get();
        let mut statement = connection.prepare("SELECT body FROM resource_memberships_v1 WHERE host=?1 ORDER BY queued_at,id")?;
        let bodies = statement.query_map([host], |row| row.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        bodies.into_iter().map(|body| Ok(serde_json::from_str(&body)?)).collect()
    }

    pub fn attach_resource_child(&self, grant: &Grant, owner: &Owner, child: &str) -> Result<()> {
        let mut connection = self.connection.write();
        let transaction = connection.transaction()?;
        let mut member = read(&transaction, &grant.membership_id)?;
        authorized(&member, owner)?;
        fenced(&transaction, &member, grant)?;
        anyhow::ensure!(matches!(member.state.as_str(), "admitted" | "launched"), "resource scope is closing");
        if member.child_runs.iter().any(|run| run == child) { return Ok(()); }
        let exists: bool = transaction.query_row("SELECT EXISTS(SELECT 1 FROM mission_runs WHERE id=?1)",
            [child.strip_prefix("mission-run/").unwrap_or(child)], |row| row.get(0))?;
        anyhow::ensure!(!exists, "resource context must be attached before a child becomes runnable");
        member.child_runs.push(child.into());
        // Child context is metadata, not an admission/fit transition. Its publication/state
        // events wake readers; emitting a lane event here self-wakes a capacity-refused parent.
        transaction.execute(/* sqlite-large-value: allow-for-now lv-h32081bb17f3d owner=fractal_store::store::resource_membership TODO=bound-or-reference-values-and-reclamation */ "UPDATE resource_memberships_v1 SET body=?2 WHERE id=?1",
            rusqlite::params![member.request.id,serde_json::to_string(&member)?])?;
        transaction.commit()?;
        Ok(())
    }

    pub fn reject_resource_child(&self, grant: &Grant, owner: &Owner, child: &str, refusal: &FractalError) -> Result<()> {
        anyhow::ensure!(refusal.code != "internal" && !refusal.is_sqlite_contention(),
            "uncertain child creation cannot release resource context");
        let mut connection = self.connection.write();
        let transaction = connection.transaction()?;
        let mut member = read(&transaction, &grant.membership_id)?;
        authorized(&member, owner)?;
        fenced(&transaction, &member, grant)?;
        let exists: bool = transaction.query_row("SELECT EXISTS(SELECT 1 FROM mission_runs WHERE id=?1)",
            [child.strip_prefix("mission-run/").unwrap_or(child)], |row| row.get(0))?;
        anyhow::ensure!(!exists, "a created child retains its resource context");
        member.child_runs.retain(|run| run != child);
        transaction.execute(/* sqlite-large-value: allow-for-now lv-hbb24831194f3 owner=fractal_store::store::resource_membership TODO=bound-or-reference-values-and-reclamation */ "UPDATE resource_memberships_v1 SET body=?2 WHERE id=?1",
            rusqlite::params![member.request.id,serde_json::to_string(&member)?])?;
        transaction.commit()?;
        Ok(())
    }

    pub fn resource_context_for_run(&self, child: &str) -> Result<Option<Membership>> {
        let connection = self.readers.get();
        let mut statement = connection.prepare("SELECT body FROM resource_memberships_v1 WHERE state IN ('admitted','launched','retiring','uncertain')")?;
        for body in statement.query_map([], |row| row.get::<_, String>(0))? {
            let member: Membership = serde_json::from_str(&body?)?;
            if member.child_runs.iter().any(|run| run == child) {
                smallclaims::touched::note_read(|| dependency(&member.request.host, &member.request.name));
                if let Some(grant) = &member.grant { fenced(&connection, &member, grant)?; }
                return Ok(Some(member));
            }
        }
        Ok(None)
    }
    /// Called once by the locally exclusive host daemon after obtaining its existing writer lock.
    /// Changing epochs fences launch; surviving execution custody remains charged until recovery.
    pub fn start_resource_authority(&self, host: &str, epoch: &str) -> Result<()> {
        anyhow::ensure!(!host.is_empty() && !epoch.is_empty(), "empty resource authority identity");
        anyhow::ensure!(host == self.origin, "only the execution host may own resource authority");
        let connection = self.connection.write();
        connection.execute(/* sqlite-large-value: allow-for-now lv-h466c8225b3a9 owner=fractal_store::store::resource_membership TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO resource_authorities_v1 VALUES(?1,?2)
            ON CONFLICT(host) DO UPDATE SET epoch=excluded.epoch", rusqlite::params![host,epoch])?;
        Ok(())
    }

    /// Replacement authority retires old custody, never replays a payload launch.
    /// Unknown preparation outcomes remain occupied and visible for explicit recovery.
    pub fn recover_resource_memberships(&self, host: &str, runtime: &ExecRuntime, now_ms: u64) -> Result<usize> {
        anyhow::ensure!(host == self.origin, "cannot recover another host's resource custody");
        let epoch: String = self.readers.get().query_row(
            "SELECT epoch FROM resource_authorities_v1 WHERE host=?1", [host], |row| row.get(0))?;
        self.observe_recovered_resource_memberships(host, now_ms)?;
        let mut uncertain = 0;
        for mut member in self.resource_memberships_for_host(host)? {
            if !matches!(member.state.as_str(), "admitted" | "launched" | "retiring" | "uncertain") { continue; }
            let Some(mut grant) = member.grant.take() else { anyhow::bail!("active resource custody has no durable grant"); };
            if grant.authority_epoch == epoch { continue; }
            for root in &mut member.roots {
                if root.execution.is_some() { continue; }
                let observed = runtime.observe(&root.runtime_id)?;
                let execution = match observed {
                    Some(fractal_runtime::ExecObservation::Running(execution) | fractal_runtime::ExecObservation::Exited(execution)) => Some(execution),
                    Some(fractal_runtime::ExecObservation::Indeterminate(reason)) => {
                        member.reason = format!("prepared generation indeterminate: {reason}");
                        None
                    }
                    None => None,
                };
                if let Some(execution) = execution {
                    if execution.resource_membership.as_ref().is_some_and(|binding|
                        binding.membership_id == grant.membership_id && binding.generation == grant.generation
                            && binding.authority_epoch == member.clock_epoch)
                    {
                        root.payload_released_at = execution.payload_released_at_unix_ms;
                        member.payload_released_at = member.payload_released_at.or(root.payload_released_at);
                        root.execution = Some(ExecutionIdentity::from(&execution));
                    } else {
                        member.reason = "prepared runtime generation has no matching resource authority binding".into();
                    }
                }
            }
            // The daemon's existing exclusive writer lock fences the predecessor authority.
            // Rotate only the capability epoch, never the physical execution generation.
            grant.authority_epoch.clone_from(&epoch);
            {
                let mut connection = self.connection.write();
                let transaction = connection.transaction()?;
                let mut current = read(&transaction, &grant.membership_id)?;
                current.grant = Some(grant.clone());
                current.state = "retiring".into();
                current.retirement_requested_at.get_or_insert(now_ms);
                current.clock_uncertain = true;
                current.roots = member.roots.clone();
                current.payload_released_at = member.payload_released_at;
                current.reason = format!("authority restart: reconciling exact execution custody; {}", member.reason);
                write(&transaction, &current)?;
                transaction.commit()?;
            }
            match self.retire_resource_membership(&grant, &member.request.owner, runtime, now_ms) {
                Ok(true) => {}
                Ok(false) => uncertain += 1,
                Err(error) => {
                    self.update_resource_membership(&grant, |current| {
                        current.state = "uncertain".into();
                        current.reason = format!("restart retirement unconfirmed: {error:#}");
                        Ok(())
                    })?;
                    uncertain += 1;
                }
            }
        }
        self.observe_recovered_resource_memberships(host, now_ms)?;
        Ok(uncertain)
    }

    pub fn request_resource_membership(&self, request: &LaneRequest, fit: Fit, now_ms: u64) -> Result<Admission> {
        anyhow::ensure!(!request.id.is_empty() && !request.name.is_empty() && !request.host.is_empty(), "empty resource identity");
        anyhow::ensure!(request.host == self.origin, "only the execution host may grant resource membership");
        anyhow::ensure!(request.max_members > 0, "maxMembers must be positive");
        anyhow::ensure!(!request.owner.run.is_empty() && !request.owner.step.is_empty() && request.owner.attempt > 0, "resource execution owner required");
        let mut connection = self.connection.write();
        let transaction = connection.transaction()?;
        let epoch: String = transaction.query_row("SELECT epoch FROM resource_authorities_v1 WHERE host=?1", [&request.host], |r| r.get(0))?;
        let policy = serde_json::to_string(&(request.max_members, request.capacity, request.coverage))?;
        transaction.execute(/* sqlite-large-value: allow-for-now lv-hacf1fd1c6d10 owner=fractal_store::store::resource_membership TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO resource_lane_policies_v1 VALUES(?1,?2,?3)",
            rusqlite::params![request.host,request.name,policy])?;
        let installed: String = transaction.query_row("SELECT policy FROM resource_lane_policies_v1 WHERE host=?1 AND lane=?2",
            rusqlite::params![request.host,request.name], |row| row.get(0))?;
        anyhow::ensure!(installed == policy, "resource lane policy differs from host-installed policy");
        let exists: bool = transaction.query_row("SELECT EXISTS(SELECT 1 FROM resource_memberships_v1 WHERE id=?1)", [&request.id], |r| r.get(0))?;
        let mut member = if exists {
            let member = read(&transaction, &request.id)?;
            anyhow::ensure!(member.request == *request, "resource request identity reused with different execution");
            member
        } else {
            Membership { request: request.clone(), grant: None, state: "queued".into(), reason: "waiting for admission".into(),
                queued_at: now_ms, admitted_at: None, payload_released_at: None, execution_ended_at: None,
                retirement_requested_at: None, retired_at: None,
                roots: Vec::new(), child_runs: Vec::new(), clock_uncertain: false, clock_epoch: epoch.clone(), refusal_code: None }
        };
        let previous = exists.then(|| member.state.clone());
        if member.state != "queued" {
            if let Some(grant) = &member.grant { fenced(&transaction, &member, grant)?; }
            return Ok(admission(member));
        }
        smallclaims::touched::note_read(|| dependency(&request.host, &request.name));
        let (queue_elapsed, uncertain) = self.resource_admission_elapsed(&request.id, member.queued_at, now_ms);
        member.clock_uncertain |= uncertain || member.clock_epoch != epoch;
        let refused = if request.coverage == Coverage::Enforced && !cfg!(target_os = "linux") {
            Some((RefusalCode::UnsupportedCoverage, "whole-tree resource enforcement requires Linux".to_owned()))
        } else if now_ms < member.queued_at {
            Some((RefusalCode::ClockUncertain, "queue clock moved backwards; admission requires reconciliation".into()))
        } else if request.admission_timeout_ms.is_some_and(|timeout| queue_elapsed >= timeout) {
            Some((RefusalCode::AdmissionExpired, "execution clock has not started".into()))
        } else if let Some(capacity) = request.capacity {
            match fit.room {
                None => Some((RefusalCode::UnsupportedCoverage, "host lane fit is not configured".into())),
                Some(room) if capacity.cores > room.cores || capacity.memory > room.memory => Some((RefusalCode::ImpossibleCapacity, "requested footprint exceeds host lane room".into())),
                _ => None,
            }
        } else { None };
        if let Some((code, reason)) = refused {
            member.state = "refused".into(); member.reason = reason; member.refusal_code = Some(code);
        } else {
            // Same writer transaction as hold placement/resolution: no snapshot/subscription gap.
            let holds = super::host_holds::blocking_tx(&transaction, &request.host, &request.name)?;
            let held: u32 = transaction.query_row("SELECT count(*) FROM resource_memberships_v1
                WHERE host=?1 AND lane=?2 AND state IN ('admitted','launched','retiring','uncertain')", rusqlite::params![request.host,request.name], |r| r.get(0))?;
            let head: Option<String> = transaction.query_row("SELECT id FROM resource_memberships_v1
                WHERE host=?1 AND lane=?2 AND state='queued' ORDER BY rowid LIMIT 1", rusqlite::params![request.host,request.name], |r| r.get(0)).optional()?;
            let finite_held: u64 = transaction.query_row(
                "SELECT coalesce(sum(memory),0) FROM resource_memberships_v1
                 WHERE host=?1 AND id<>?2 AND state IN ('admitted','launched','retiring','uncertain')",
                rusqlite::params![request.host,request.id], |row| row.get(0))?;
            let free = fit.room.map_or(0, |room| room.memory.saturating_sub(fit.held_bytes).saturating_sub(finite_held)
                .saturating_sub(fit.outside_bytes).saturating_sub(fit.reserve_bytes));
            member.reason = if !holds.is_empty() { "host held".into() }
                else if held >= request.max_members { "lane occupied".into() }
                else if head.as_deref().is_some_and(|head| head != request.id) { "FIFO predecessor".into() }
                else if let Some(predecessor) = &fit.fifo_predecessor { format!("host fit FIFO predecessor: {predecessor}") }
                else if request.capacity.is_some_and(|capacity| capacity.memory > free) { "host footprint does not fit".into() }
                else { String::new() };
            if member.reason.is_empty() {
                member.state = "admitted".into(); member.admitted_at = Some(now_ms);
                member.grant = Some(Grant { membership_id: request.id.clone(), authority_epoch: epoch,
                    generation: uuid::Uuid::new_v4().to_string(), coverage: request.coverage });
            }
        }
        write(&transaction, &member)?;
        transaction.commit()?;
        if matches!(member.state.as_str(), "retired" | "refused") { self.finish_resource_admission_clock(&request.id); }
        self.observe_resource_transition(&connection, &member, previous.as_deref(), now_ms, false);
        Ok(admission(member))
    }

    pub fn resource_membership(&self, id: &str, owner: &Owner, _now_ms: u64) -> Result<Admission> {
        let connection = self.readers.get();
        let member = read(&connection, id)?;
        authorized(&member, owner)?;
        smallclaims::touched::note_read(|| dependency(&member.request.host, &member.request.name));
        if let Some(grant) = &member.grant { fenced(&connection, &member, grant)?; }
        Ok(admission(member))
    }

    pub fn held_resource_membership_bytes(&self, host: &str, excluding_id: &str) -> Result<u64> {
        smallclaims::touched::note_read(|| "seat-admission".into());
        Ok(self.readers.get().query_row("SELECT coalesce(sum(memory),0) FROM resource_memberships_v1
            WHERE host=?1 AND id<>?2 AND state IN ('admitted','launched','retiring','uncertain')", rusqlite::params![host,excluding_id], |r| r.get(0))?)
    }

    pub fn withdraw_resource_membership(&self, id: &str, owner: &Owner, now_ms: u64) -> Result<()> {
        let mut connection = self.connection.write(); let tx = connection.transaction()?;
        let mut member = read(&tx, id)?; authorized(&member, owner)?;
        let previous = member.state.clone();
        if member.state == "queued" {
            member.state = "retired".into(); member.reason = "cancelled before admission".into(); member.retired_at = Some(now_ms);
            member.refusal_code = Some(RefusalCode::Cancelled);
        } else if matches!(member.state.as_str(), "admitted" | "launched") {
            member.state = "retiring".into(); member.reason = "cancellation requested".into();
            member.retirement_requested_at.get_or_insert(now_ms);
        }
        write(&tx, &member)?; tx.commit()?;
        self.observe_resource_transition(&connection, &member, Some(&previous), now_ms, false);
        Ok(())
    }

    pub fn cancel_resource_membership(&self, grant: &Grant, owner: &Owner, now_ms: u64) -> Result<()> {
        self.update_resource_membership(grant, |member| {
            authorized(member, owner)?;
            if member.state != "retired" {
                member.state = "retiring".into(); member.reason = format!("cancellation requested at {now_ms}");
                member.retirement_requested_at.get_or_insert(now_ms);
            }
            Ok(())
        })
    }

    /// Preparation intent is durable before spawning. A missing execution record after this point
    /// is uncertain, never evidence that launch did not happen.
    pub fn prepare_resource_membership(&self, grant: &Grant, runtime_id: &str) -> Result<()> {
        self.update_resource_membership(grant, |member| {
            anyhow::ensure!(matches!(member.state.as_str(), "admitted" | "launched"), "membership is not launchable");
            anyhow::ensure!(!member.roots.iter().any(|root| root.runtime_id == runtime_id), "execution root already prepared");
            member.roots.push(ExecutionRoot { runtime_id: runtime_id.into(), execution: None,
                payload_released_at: None, execution_ended_at: None });
            Ok(())
        })
    }

    /// Only positive producer rejection may erase a preparation intent. Absence of a record
    /// or a timeout cannot stand in for this evidence.
    pub fn reject_resource_preparation(&self, grant: &Grant, runtime_id: &str, _proof: &fractal_runtime::StartRejected) -> Result<()> {
        self.update_resource_membership(grant, |member| {
            let root = member.roots.iter().find(|root| root.runtime_id == runtime_id)
                .context("preparation intent missing")?;
            anyhow::ensure!(root.execution.is_none(), "cannot reject a launched execution");
            member.roots.retain(|root| root.runtime_id != runtime_id);
            Ok(())
        })
    }

    /// Serialize launch authorization with cancellation through the physical payload gate.
    /// Bind the exact already-gated producer durably before the irreversible release action.
    pub fn release_resource_payload(&self, grant: &Grant, runtime_id: &str, execution: &ExecGeneration,
        release: &mut dyn FnMut() -> Result<u64>) -> Result<()>
    {
        let mut connection = self.connection.write();
        let mut member;
        let mut admission_failure = None;
        {
            let transaction = connection.transaction()?;
            member = read(&transaction, &grant.membership_id)?;
            fenced(&transaction, &member, grant)?;
            anyhow::ensure!(matches!(member.state.as_str(), "admitted" | "launched"), "resource scope cancelled before payload release");
            let root = member.roots.iter_mut().find(|root| root.runtime_id == runtime_id)
                .context("payload release has no durable preparation")?;
            anyhow::ensure!(root.execution.is_none(), "payload generation already authorized");
            root.execution = Some(ExecutionIdentity::from(execution));
            // Admission covers preparation and scope creation, not merely waiting for a seat.
            // Bind the gated producer even on refusal so cleanup must prove its retirement
            // before the charged membership can return capacity.
            if member.payload_released_at.is_none() {
                let now = u64::try_from(now_ms()).unwrap_or(u64::MAX);
                let (elapsed, uncertain) = self.resource_admission_elapsed(&member.request.id, member.queued_at, now);
                if member.clock_uncertain || uncertain {
                    admission_failure = Some("admission clock uncertain before payload release");
                } else if member.request.admission_timeout_ms.is_some_and(|limit| elapsed >= limit) {
                    admission_failure = Some("admission-expired before payload release");
                }
                if let Some(reason) = admission_failure {
                    member.state = "retiring".into();
                    member.reason = reason.into();
                    member.retirement_requested_at.get_or_insert(now);
                }
            }
            write(&transaction, &member)?;
            transaction.commit()?;
        }
        if let Some(reason) = admission_failure {
            // The runtime closes the gate and kills the producer when authorization fails.
            // Its exact identity remains charged here until whole-tree retirement succeeds.
            anyhow::bail!(reason);
        }
        // Keep the writer guard, not a transaction, across the OS action. A crash can leave
        // a known gated/released generation; neither cancellation nor a replacement can race it.
        let released_at = match release() {
            Ok(at) => at,
            Err(error) => {
                member.state = "uncertain".into();
                member.reason = format!("payload release outcome requires exact-generation cleanup: {error:#}");
                let transaction = connection.transaction()?;
                write(&transaction, &member)?;
                transaction.commit()?;
                self.observe_resource_transition(&connection, &member, Some("admitted"),
                    u64::try_from(now_ms()).unwrap_or(u64::MAX), false);
                return Err(error);
            }
        };
        let root = member.roots.iter_mut().find(|root| root.runtime_id == runtime_id)
            .context("authorized execution root disappeared")?;
        root.payload_released_at = Some(released_at);
        member.state = "launched".into();
        member.payload_released_at.get_or_insert(released_at);
        member.execution_ended_at = None;
        let transaction = connection.transaction()?;
        write(&transaction, &member)?;
        transaction.commit()?;
        self.observe_resource_transition(&connection, &member, Some("admitted"), released_at, false);
        Ok(())
    }

    pub fn launch_resource_membership(&self, grant: &Grant, runtime_id: &str, execution: &ExecGeneration, _now_ms: u64) -> Result<()> {
        let released_at = execution.payload_released_at_unix_ms.context("finite execution has no confirmed payload-release receipt")?;
        self.update_resource_membership(grant, |member| {
            anyhow::ensure!(matches!(member.state.as_str(), "admitted" | "launched"), "membership cancelled before launch");
            let root = member.roots.iter_mut().find(|root| root.runtime_id == runtime_id)
                .context("launch intent mismatch")?;
            anyhow::ensure!(root.execution.as_ref().is_none_or(|bound| bound.generation_id == execution.generation_id),
                "a different execution generation is already attached");
            root.execution = Some(ExecutionIdentity::from(execution));
            root.payload_released_at = Some(released_at);
            member.state = "launched".into();
            member.payload_released_at.get_or_insert(released_at);
            member.execution_ended_at = None;
            Ok(())
        })
    }

    pub fn end_resource_membership(&self, grant: &Grant, now_ms: u64) -> Result<()> {
        self.update_resource_membership(grant, |member| { member.execution_ended_at.get_or_insert(now_ms); Ok(()) })
    }

    pub fn end_resource_execution(&self, grant: &Grant, runtime_id: &str, now_ms: u64) -> Result<()> {
        self.update_resource_membership(grant, |member| {
            let root = member.roots.iter_mut().find(|root| root.runtime_id == runtime_id)
                .context("execution root missing")?;
            anyhow::ensure!(root.execution.is_some(), "payload release was not confirmed");
            root.execution_ended_at.get_or_insert(now_ms);
            if member.roots.iter().all(|root| root.execution_ended_at.is_some()) {
                member.execution_ended_at = Some(now_ms);
            }
            Ok(())
        })
    }

    pub fn retire_resource_membership(&self, grant: &Grant, owner: &Owner, runtime: &ExecRuntime, now_ms: u64) -> Result<bool> {
        // Do not hold the SQLite writer while asking the OS to terminate a tree.
        let member = read(&self.readers.get(), &grant.membership_id)?;
        authorized(&member, owner)?;
        fenced(&self.readers.get(), &member, grant)?;
        if member.state == "retired" { return Ok(true); }
        self.update_resource_membership(grant, |member| {
            authorized(member, owner)?;
            member.state = "retiring".into();
            member.reason = "execution retirement requested".into();
            member.retirement_requested_at.get_or_insert(now_ms);
            Ok(())
        })?;
        // Closing the state first prevents any new preparation; read roots after that commit.
        let member = read(&self.readers.get(), &grant.membership_id)?;
        let mut empty = true;
        let mut uncertain = false;
        let mut retirement_errors = Vec::new();
        for child in &member.child_runs {
            smallclaims::touched::note_read(|| child.clone());
            let status = self.readers.get().query_row("SELECT status FROM mission_runs WHERE id=?1",
                [child.strip_prefix("mission-run/").unwrap_or(child)], |row| row.get::<_, String>(0)).optional();
            match status {
                Ok(Some(status)) if matches!(status.as_str(), "completed" | "failed" | "cancelled") => {}
                Ok(Some(_)) => empty = false,
                Ok(None) => { empty = false; uncertain = true; }
                Err(error) => {
                    empty = false;
                    uncertain = true;
                    retirement_errors.push(format!("{child}: {error:#}"));
                }
            }
        }
        for root in &member.roots {
            if let Some(execution) = &root.execution {
                if let Err(error) = self.retire_resource_root(grant, runtime, &root.runtime_id, &execution.generation_id) {
                    empty = false;
                    uncertain = true;
                    retirement_errors.push(format!("{}: {error:#}", root.runtime_id));
                }
            } else {
                empty = false; // A prepared generation with unknown launch outcome retains custody.
                uncertain = true;
            }
        }
        let teardown_error = (!retirement_errors.is_empty()).then(|| retirement_errors.join("; "));
        self.update_resource_membership(grant, |member| {
            if empty { member.state = "retired".into(); member.retired_at = Some(now_ms); member.reason = "execution tree confirmed empty".into(); }
            else if uncertain { member.state = "uncertain".into(); member.reason = teardown_error.clone()
                .unwrap_or_else(|| "execution retirement unconfirmed; custody retained".into()); }
            else { member.state = "retiring".into(); member.reason = "awaiting linked child termination; custody retained".into(); }
            Ok(())
        })?;
        if let Some(error) = teardown_error { anyhow::bail!("resource tree retirement failed: {error}"); }
        Ok(empty)
    }

    fn retire_resource_root(&self, grant: &Grant, runtime: &ExecRuntime, id: &str, generation: &str) -> Result<()> {
        match runtime.retire_if(id, generation) {
            Ok(_) => Ok(()),
            Err(error) => {
                self.update_resource_membership(grant, |member| {
                    member.state = "uncertain".into();
                    member.reason = format!("execution tree retirement unconfirmed: {error:#}");
                    Ok(())
                })?;
                Err(error)
            }
        }
    }

    fn update_resource_membership(&self, grant: &Grant, update: impl FnOnce(&mut Membership) -> Result<()>) -> Result<()> {
        let mut connection = self.connection.write(); let tx = connection.transaction()?;
        let mut member = read(&tx, &grant.membership_id)?; fenced(&tx, &member, grant)?;
        let previous = member.state.clone();
        update(&mut member)?; write(&tx, &member)?; tx.commit()?;
        let now_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX));
        self.observe_resource_transition(&connection, &member, Some(&previous), now_ms, false);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::host_holds::{PlaceHostHold, ReleaseHostHold};

    fn request(id: &str) -> LaneRequest {
        LaneRequest { id: id.into(), name: "executor/activation".into(), host: "host-a".into(),
            max_members: 1, capacity: None, coverage: Coverage::Logical, admission_timeout_ms: None,
            owner: Owner { run: format!("mission-run/{id}"), step: format!("step-run/{id}"),
                attempt: 1, invocation: Some(format!("invocation/{id}")) } }
    }
    fn fixture() -> Store {
        let store = Store::open_memory("host-a").unwrap();
        store.start_resource_authority("host-a", "epoch-a").unwrap();
        store
    }
    fn grant(store: &Store, request: &LaneRequest, now: u64) -> Grant {
        match store.request_resource_membership(request, Fit::default(), now).unwrap() {
            Admission::Admitted(grant) => grant,
            other => panic!("expected admission: {other:?}"),
        }
    }
    fn runtime(root: &Path) -> ExecRuntime {
        ExecRuntime::new(root.join("runtime"), root.join("logs"))
    }

    #[test]
    fn contention_fifo_exact_withdrawal_and_handoff() {
        let store = fixture();
        let root = tempfile::tempdir().unwrap();
        let runtime = runtime(root.path());
        let holder = request("holder");
        let first = grant(&store, &holder, 1);
        let contenders = (0..16).rev().map(|i| request(&format!("waiter-{i:02}"))).collect::<Vec<_>>();
        for contender in &contenders {
            assert!(matches!(store.request_resource_membership(contender, Fit::default(), 2).unwrap(), Admission::Queued { .. }));
        }
        store.withdraw_resource_membership(&contenders[3].id, &contenders[3].owner, 3).unwrap();
        assert!(store.retire_resource_membership(&first, &holder.owner, &runtime, 4).unwrap());
        let started = std::time::Instant::now();
        for (index, contender) in contenders.iter().enumerate() {
            if index == 3 { continue; }
            let next = grant(&store, contender, 5);
            assert!(store.retire_resource_membership(&next, &contender.owner, &runtime, 6).unwrap());
        }
        eprintln!("membership-only FIFO benchmark: 15 handoffs in {:?}; no payload/OS timing claim", started.elapsed());
        assert_eq!(store.held_resource_membership_bytes("host-a", "").unwrap(), 0);
    }

    #[test]
    fn hold_resolution_wakes_same_request_without_occupancy() {
        let store = fixture();
        let hold = store.place_host_hold("person/operator", &PlaceHostHold {
            actor: "person/operator".into(), host: "host-a".into(), reason: "investigate".into(),
            operations: vec!["executor/activation".into()], originating_run: Some("mission-run/old".into()),
            receipt: Some("receipt/failure".into()),
        }).unwrap();
        let request = request("blocked");
        assert!(matches!(store.request_resource_membership(&request, Fit::default(), 1).unwrap(), Admission::Queued { .. }));
        assert_eq!(store.held_resource_membership_bytes("host-a", "").unwrap(), 0);
        store.release_host_hold("person/operator", &ReleaseHostHold {
            actor: "person/operator".into(), host: "host-a".into(), id: hold.id,
            expected_revision: 1, resolution: "repaired".into(),
        }).unwrap();
        grant(&store, &request, 2);
        let record = store.resource_membership_record(&request.id, &request.owner).unwrap();
        assert_eq!(record.queued_at, 1);
        assert_eq!(record.admitted_at, Some(2));
        assert_eq!(record.payload_released_at, None);
    }

    #[test]
    fn queue_expiry_and_owner_generation_fences() {
        let store = fixture();
        let holder = request("holder");
        let grant = grant(&store, &holder, 1);
        let mut waiting = request("waiting");
        waiting.admission_timeout_ms = Some(0);
        store.request_resource_membership(&waiting, Fit::default(), 2).unwrap();
        assert!(matches!(store.request_resource_membership(&waiting, Fit::default(), 12).unwrap(),
            Admission::Refused { code: RefusalCode::AdmissionExpired, .. }));
        assert!(store.cancel_resource_membership(&grant, &waiting.owner, 13).is_err());
        let mut stale = grant.clone(); stale.generation = "stale".into();
        assert!(store.cancel_resource_membership(&stale, &holder.owner, 13).is_err());
        assert!(store.prepare_resource_membership(&grant, "runtime-a").is_ok());
        store.withdraw_resource_membership(&holder.id, &holder.owner, 14).unwrap();
        assert!(store.prepare_resource_membership(&grant, "runtime-b").is_err());
    }

    #[test]
    fn linked_child_retirement_waits_with_custody_and_tracks_terminal_dependency() {
        let store = fixture();
        let root = tempfile::tempdir().unwrap();
        let runtime = runtime(root.path());
        let mut holder = request("child-holder");
        holder.capacity = Some(Capacity { cores: 1, memory: 1024 });
        let fit = Fit { room: holder.capacity, ..Fit::default() };
        let Admission::Admitted(current) = store.request_resource_membership(&holder, fit.clone(), 1).unwrap() else {
            panic!("holder must acquire physical custody");
        };
        let child = "mission-run/linked-child";
        store.attach_resource_child(&current, &holder.owner, child).unwrap();
        store.connection.write().execute(
            "INSERT INTO mission_runs(
                id,mission_id,initial_revision,current_generation_id,root_revision,root_run_id,
                workspace,requester,inputs,mode,status,phase,created_at_unix_ms,updated_at_unix_ms
             ) VALUES(
                'linked-child','resource-child','revision','generation','revision','linked-child',
                ?1,'person/operator','{}','run','running','working','1','1'
             )", [root.path().to_string_lossy().as_ref()],
        ).unwrap();
        let (retired, reads) = smallclaims::touched::record(|| {
            store.retire_resource_membership(&current, &holder.owner, &runtime, 2).unwrap()
        });
        assert!(!retired);
        assert!(reads.contains(child), "release must subscribe to the child's terminal transition");
        let pending = store.resource_membership_for_grant(&current).unwrap();
        assert_eq!(pending.state, "retiring");
        assert_eq!(pending.retirement_requested_at, Some(2));
        assert_eq!(pending.retired_at, None);
        assert_eq!(store.held_resource_membership_bytes("host-a", "").unwrap(), 1024);
        assert!(store.prepare_resource_membership(&current, "late-root").is_err());
        let mut waiter = request("child-waiter");
        waiter.capacity = holder.capacity;
        assert!(matches!(store.request_resource_membership(&waiter, fit.clone(), 3).unwrap(), Admission::Queued { .. }));
        store.connection.write().execute("UPDATE mission_runs SET status='cancelled',phase='terminal' WHERE id='linked-child'", []).unwrap();
        assert!(store.retire_resource_membership(&current, &holder.owner, &runtime, 4).unwrap());
        let retired = store.resource_membership_for_grant(&current).unwrap();
        assert_eq!(retired.state, "retired");
        assert_eq!(retired.retired_at, Some(4));
        assert_eq!(store.held_resource_membership_bytes("host-a", "").unwrap(), 0);
        assert!(matches!(store.request_resource_membership(&waiter, fit, 5).unwrap(), Admission::Admitted(_)));
    }

    #[test]
    fn missing_child_or_unknown_preparation_retains_uncertain_custody() {
        for preparation in [false, true] {
            let store = fixture();
            let root = tempfile::tempdir().unwrap();
            let holder = request("unknown-holder");
            let current = grant(&store, &holder, 1);
            if preparation {
                store.prepare_resource_membership(&current, "unknown-root").unwrap();
            } else {
                store.attach_resource_child(&current, &holder.owner, "mission-run/missing-child").unwrap();
            }
            assert!(!store.retire_resource_membership(&current, &holder.owner, &runtime(root.path()), 2).unwrap());
            let record = store.resource_membership_for_grant(&current).unwrap();
            assert_eq!(record.state, "uncertain");
            assert_eq!(record.retired_at, None);
            assert!(matches!(store.request_resource_membership(&request("blocked"), Fit::default(), 3).unwrap(), Admission::Queued { .. }));
        }
    }

    #[test]
    fn restart_retains_unknown_preparation_and_fences_prior_grant() {
        let store = fixture();
        let root = tempfile::tempdir().unwrap();
        let runtime = runtime(root.path());
        let unknown = request("unknown");
        let old = grant(&store, &unknown, 1);
        store.prepare_resource_membership(&old, "runtime-unknown").unwrap();
        store.start_resource_authority("host-a", "epoch-b").unwrap();
        assert!(store.cancel_resource_membership(&old, &unknown.owner, 2).is_err());
        assert_eq!(store.recover_resource_memberships("host-a", &runtime, 3).unwrap(), 1);
        let successor = request("successor");
        assert!(matches!(store.request_resource_membership(&successor, Fit::default(), 4).unwrap(), Admission::Queued { .. }));
        let recovered = store.resource_membership_record(&unknown.id, &unknown.owner).unwrap();
        assert_eq!(recovered.state, "uncertain");
        assert_eq!(recovered.grant.unwrap().authority_epoch, "epoch-b");
        assert_eq!(recovered.payload_released_at, None);
    }

    #[test]
    fn host_authority_and_policy_are_not_requester_overrides() {
        let store = fixture();
        let mut first = request("first");
        grant(&store, &first, 1);
        first.id = "different".into(); first.max_members = 2;
        assert!(store.request_resource_membership(&first, Fit::default(), 2).is_err());
        first.host = "host-b".into();
        assert!(store.request_resource_membership(&first, Fit::default(), 2).is_err());
        assert!(store.start_resource_authority("host-b", "epoch").is_err());
    }

    #[test]
    fn durable_restart_recovers_unlaunched_permit_without_reexecuting() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("graph.db");
        let request = request("restart");
        let old = {
            let store = Store::open(&path, "host-a").unwrap();
            store.start_resource_authority("host-a", "epoch-a").unwrap();
            grant(&store, &request, 1)
        };
        let reopened = Store::open(&path, "host-a").unwrap();
        reopened.start_resource_authority("host-a", "epoch-b").unwrap();
        assert!(reopened.resource_membership(&request.id, &request.owner, 2).is_err());
        assert_eq!(reopened.recover_resource_memberships("host-a", &runtime(root.path()), 3).unwrap(), 0);
        assert_eq!(reopened.resource_membership_record(&old.membership_id, &request.owner).unwrap().state, "retired");
    }

    #[test]
    fn execution_clocks_begin_at_payload_receipt_and_track_each_root() {
        let store = fixture();
        let request = request("clocks");
        let grant = grant(&store, &request, 1);
        for (index, released_at) in [500_u64, 700].into_iter().enumerate() {
            let id = format!("runtime-{index}");
            store.prepare_resource_membership(&grant, &id).unwrap();
            let execution: ExecGeneration = serde_json::from_value(json!({
                "schema": "st.runtime.exec-generation.v1", "pid": 1, "createdAtUnixMs": 100,
                "startToken": 1, "generationId": format!("generation-{index}"),
                "payloadReleasedAtUnixMs": released_at
            })).unwrap();
            store.launch_resource_membership(&grant, &id, &execution, 10_000).unwrap();
        }
        let record = store.resource_membership_record(&request.id, &request.owner).unwrap();
        assert_eq!(record.queued_at, 1);
        assert_eq!(record.admitted_at, Some(1));
        assert_eq!(record.payload_released_at, Some(500));
        assert_eq!(record.roots[1].payload_released_at, Some(700));
        store.end_resource_execution(&grant, "runtime-0", 800).unwrap();
        assert_eq!(store.resource_membership_record(&request.id, &request.owner).unwrap().execution_ended_at, None);
        store.end_resource_execution(&grant, "runtime-1", 900).unwrap();
        assert_eq!(store.resource_membership_record(&request.id, &request.owner).unwrap().execution_ended_at, Some(900));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn native_contention_cancel_and_eight_payload_handoffs() {
        if fractal_runtime::isolation_mode() != fractal_runtime::Isolation::Scope {
            eprintln!("native resource benchmark requires a systemd user manager");
            return;
        }
        let store = fixture();
        let root = tempfile::tempdir().unwrap();
        let runtime = runtime(root.path());
        let wall = || u64::try_from(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis()).unwrap();
        let mut holder = request("native-holder");
        holder.name = format!("resource-test-{}", uuid::Uuid::new_v4().simple());
        let current = grant(&store, &holder, wall());
        let policy = fractal_runtime::FiniteLane { name: holder.name.clone(), capacity: None, require_enforced: false };
        store.prepare_resource_membership(&current, "holder").unwrap();
        let binding = fractal_runtime::ResourceExecutionBinding {
            membership_id: current.membership_id.clone(), generation: current.generation.clone(),
            authority_epoch: current.authority_epoch.clone(),
        };
        let generation = runtime.spawn_in_lane_authorized("holder", &fractal_runtime::Launch::Argv(vec!["sleep".into(), "60".into()]),
            root.path(), &std::env::vars().collect(), fractal_runtime::FiniteExecutionAuthorization {
                lane: &policy, binding: &binding, guard: &|| Ok(()),
                authorize: &|execution, release| store.release_resource_payload(&current, "holder", execution, release),
            }).unwrap();
        store.launch_resource_membership(&current, "holder", &generation, wall()).unwrap();
        let waiters = (0..8).map(|index| {
            let mut waiter = request(&format!("native-{index}"));
            waiter.name.clone_from(&holder.name);
            waiter
        }).collect::<Vec<_>>();
        let mut queued_at = Vec::new();
        for waiter in &waiters {
            queued_at.push(std::time::Instant::now());
            assert!(matches!(store.request_resource_membership(waiter, Fit::default(), wall()).unwrap(), Admission::Queued { .. }));
        }
        let cancel_started = std::time::Instant::now();
        store.cancel_resource_membership(&current, &holder.owner, wall()).unwrap();
        assert!(store.retire_resource_membership(&current, &holder.owner, &runtime, wall()).unwrap());
        let cancelled_after = cancel_started.elapsed();
        assert!(runtime.retired_if("holder", &generation.generation_id).unwrap());
        let mut handoffs = Vec::new();
        let mut queue_waits = Vec::new();
        let mut execution_observed = Vec::new();
        let mut retirements = Vec::new();
        let mut max_holders = 0;
        for (index, waiter) in waiters.iter().enumerate() {
            let started = std::time::Instant::now();
            let next = grant(&store, waiter, wall());
            queue_waits.push(queued_at[index].elapsed().as_secs_f64() * 1000.0);
            let holders = store.resource_memberships_for_host(&holder.host).unwrap().into_iter()
                .filter(|member| matches!(member.state.as_str(), "admitted" | "launched" | "retiring" | "uncertain")).count();
            max_holders = max_holders.max(holders);
            assert_eq!(holders, 1, "capacity-one admission must retain exclusive custody");
            let id = format!("payload-{index}");
            store.prepare_resource_membership(&next, &id).unwrap();
            let binding = fractal_runtime::ResourceExecutionBinding {
                membership_id: next.membership_id.clone(), generation: next.generation.clone(),
                authority_epoch: next.authority_epoch.clone(),
            };
            let generation = runtime.spawn_in_lane_authorized(&id, &fractal_runtime::Launch::Argv(vec!["true".into()]),
                root.path(), &std::env::vars().collect(), fractal_runtime::FiniteExecutionAuthorization {
                    lane: &policy, binding: &binding, guard: &|| Ok(()),
                    authorize: &|execution, release| store.release_resource_payload(&next, &id, execution, release),
                }).unwrap();
            store.launch_resource_membership(&next, &id, &generation, wall()).unwrap();
            handoffs.push(started.elapsed().as_secs_f64() * 1000.0);
            let ended = runtime.wait_for_exit(&id, &generation.generation_id).unwrap();
            assert_eq!(ended.exit_code, Some(0));
            execution_observed.push(std::time::Duration::from_millis(wall().saturating_sub(
                generation.payload_released_at_unix_ms.unwrap())).as_secs_f64() * 1000.0);
            store.end_resource_execution(&next, &id, wall()).unwrap();
            let retiring = std::time::Instant::now();
            assert!(store.retire_resource_membership(&next, &waiter.owner, &runtime, wall()).unwrap());
            retirements.push(retiring.elapsed().as_secs_f64() * 1000.0);
            assert!(runtime.retired_if(&id, &generation.generation_id).unwrap());
        }
        assert_eq!(max_holders, 1);
        let records = store.resource_memberships_for_host(&holder.host).unwrap();
        assert_eq!(records.len(), 9);
        assert!(records.iter().all(|member| member.state == "retired"), "all holder and waiter custody must retire");
        queue_waits.sort_by(f64::total_cmp);
        execution_observed.sort_by(f64::total_cmp);
        retirements.sort_by(f64::total_cmp);
        handoffs.sort_by(f64::total_cmp);
        eprintln!("native resource benchmark: N=8, capacity=1, completed=8, max_holders={max_holders}, final_active=0, cancel_to_empty_ms={:.3}, admission_to_payload_median_ms={:.3}, max_ms={:.3}, queue_wait_median_ms={:.3}, queue_wait_max_ms={:.3}, execution_observed_median_ms={:.3}, execution_observed_max_ms={:.3}, retirement_median_ms={:.3}, retirement_max_ms={:.3}",
            cancelled_after.as_secs_f64() * 1000.0, handoffs[4], handoffs[7], queue_waits[4], queue_waits[7],
            execution_observed[4], execution_observed[7], retirements[4], retirements[7]);
    }

    #[test]
    fn common_physical_fit_and_durable_release_feed_wake_all_affected_readers() {
        let store = fixture();
        let incremental = crate::incremental::Incremental::default();
        incremental.observe(&store).unwrap();
        let gib = 1_u64 << 30;
        let fit = Fit { room: Some(Capacity { cores: 4, memory: 4 * gib }), ..Fit::default() };
        let mut first = request("physical-first");
        first.capacity = Some(Capacity { cores: 2, memory: 3 * gib });
        let first_grant = match store.request_resource_membership(&first, fit.clone(), 1).unwrap() {
            Admission::Admitted(grant) => grant,
            other => panic!("{other:?}"),
        };
        let mut second = request("physical-second");
        second.name = "independent/check".into();
        second.capacity = first.capacity;
        let (waiting, reads) = smallclaims::touched::record(|| {
            store.held_resource_membership_bytes("host-a", &second.id).unwrap();
            store.request_resource_membership(&second, fit.clone(), 2)
        });
        assert!(matches!(waiting.unwrap(), Admission::Queued { .. }), "different named lanes share one physical inventory");
        incremental.observe(&store).unwrap();
        incremental.evaluated("finite-waiter", reads, None);
        incremental.evaluated("seat-waiter", BTreeSet::from(["seat-admission".into()]), None);
        assert!(!incremental.needs("finite-waiter", now_ms()));
        let root = tempfile::tempdir().unwrap();
        store.retire_resource_membership(&first_grant, &first.owner, &runtime(root.path()), 3).unwrap();
        incremental.observe(&store).unwrap();
        assert!(incremental.needs("finite-waiter", now_ms()));
        assert!(incremental.needs("seat-waiter", now_ms()));
        assert!(matches!(store.request_resource_membership(&second, fit, 4).unwrap(), Admission::Admitted(_)));
    }

    #[test]
    fn admission_budget_covers_preparation_until_first_payload_release() {
        let store = fixture();
        let mut request = request("slow-preparation");
        request.admission_timeout_ms = Some(60_000);
        request.capacity = Some(Capacity { cores: 1, memory: 1024 });
        let grant = match store.request_resource_membership(&request,
            Fit { room: Some(Capacity { cores: 2, memory: 4096 }), ..Fit::default() }, 100).unwrap() {
            Admission::Admitted(grant) => grant,
            other => panic!("{other:?}"),
        };
        store.prepare_resource_membership(&grant, "gated").unwrap();
        // Advance monotonic age, not wall time, while OS preparation is in progress.
        store.advance_resource_admission_clock(&request.id, 60_000);
        let execution: ExecGeneration = serde_json::from_value(json!({
            "schema": "st.runtime.exec-generation.v1", "pid": 1, "createdAtUnixMs": 100,
            "startToken": 1, "generationId": "gated-generation"
        })).unwrap();
        let mut released = false;
        let error = store.release_resource_payload(&grant, "gated", &execution,
            &mut || { released = true; Ok(200) }).unwrap_err();
        assert!(error.to_string().contains("admission-expired"));
        assert!(!released);
        let member = store.resource_membership_for_grant(&grant).unwrap();
        assert_eq!(member.state, "retiring");
        assert_eq!(member.roots[0].execution.as_ref().unwrap().generation_id, "gated-generation");
        assert_eq!(member.payload_released_at, None);
        assert_eq!(store.held_resource_membership_bytes("host-a", "").unwrap(), 1024,
            "expiry cannot return capacity before the gated producer retires");
    }

    #[test]
    fn payload_release_and_cancellation_share_one_authority_serializer() {
        use std::sync::{Arc, atomic::{AtomicUsize, Ordering}, mpsc};
        let store = Arc::new(fixture());
        let request = request("atomic-gate");
        let grant = grant(&store, &request, 1);
        store.prepare_resource_membership(&grant, "atomic-root").unwrap();
        let execution: ExecGeneration = serde_json::from_value(json!({
            "schema": "st.runtime.exec-generation.v1", "pid": 1, "createdAtUnixMs": 100,
            "startToken": 1, "generationId": "atomic-generation"
        })).unwrap();
        let cancelled_execution = execution.clone();
        let sequence = Arc::new(AtomicUsize::new(0));
        let (entered_tx, entered_rx) = mpsc::sync_channel(0);
        let (release_tx, release_rx) = mpsc::sync_channel(0);
        let releasing_store = store.clone();
        let releasing_grant = grant.clone();
        let release_sequence = sequence.clone();
        let releasing = std::thread::spawn(move || {
            releasing_store.release_resource_payload(&releasing_grant, "atomic-root", &execution, &mut || {
                let bound = releasing_store.resource_membership_for_grant(&releasing_grant).unwrap();
                assert_eq!(bound.roots[0].execution.as_ref().unwrap().generation_id, "atomic-generation",
                    "exact producer identity must commit before the irreversible gate action");
                entered_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                assert_eq!(release_sequence.fetch_add(1, Ordering::SeqCst), 0);
                Ok(500)
            }).unwrap();
        });
        entered_rx.recv().unwrap();
        let cancelling_store = store.clone();
        let owner = request.owner.clone();
        let cancelling_grant = grant.clone();
        let cancel_sequence = sequence.clone();
        let cancelling = std::thread::spawn(move || {
            cancelling_store.cancel_resource_membership(&cancelling_grant, &owner, 600).unwrap();
            assert_eq!(cancel_sequence.fetch_add(1, Ordering::SeqCst), 1);
        });
        release_tx.send(()).unwrap();
        releasing.join().unwrap();
        cancelling.join().unwrap();
        let record = store.resource_membership_for_grant(&grant).unwrap();
        assert_eq!(record.state, "retiring");
        assert_eq!(record.roots[0].payload_released_at, Some(500));
        let mut released = false;
        assert!(store.release_resource_payload(&grant, "atomic-root",
            &cancelled_execution, &mut || { released = true; Ok(700) }).is_err());
        assert!(!released, "a cancellation that wins serialization must prevent gate release");
    }
}
