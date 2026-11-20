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

pub use fractal_runtime::{DiskRequest, IoClass, RequestClass};

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
    #[serde(default)]
    pub class: RequestClass,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disk: Option<DiskRequest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub io: Option<IoClass>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_estimate: Option<fractal_runtime::ResourceProfileEstimate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disk_path: Option<String>,
}

/// Historical tuple policies remain authoritative. Keyed policy identity deliberately excludes
/// the immutable estimate carried by each membership.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(untagged)]
enum InstalledPolicy {
    Keyed {
        version: u32,
        #[serde(rename = "maxMembers")]
        max_members: u32,
        profile: String,
        coverage: Coverage,
    },
    Declared((u32, Option<Capacity>, Coverage)),
}

impl InstalledPolicy {
    fn for_request(request: &LaneRequest) -> Self {
        match &request.profile {
            Some(profile) => Self::Keyed { version: 1, max_members: request.max_members,
                profile: profile.clone(), coverage: request.coverage },
            None => Self::Declared((request.max_members, request.capacity, request.coverage)),
        }
    }
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
pub enum RefusalCode {
    AdmissionExpired,
    MissingProfile,
    UnsupportedCoverage,
    // Historical authoritative refusals remain readable; new records use `impossible`.
    #[serde(alias = "impossible-capacity")]
    Impossible,
    ClockUncertain,
    Cancelled,
    Retired,
    Retiring,
    Uncertain,
    InvalidState,
    RecoveryNotAllowed,
}

impl RefusalCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AdmissionExpired => "admission-expired",
            Self::MissingProfile => "missing-profile",
            Self::UnsupportedCoverage => "unsupported-coverage",
            Self::Impossible => "impossible",
            Self::ClockUncertain => "clock-uncertain",
            Self::Cancelled => "cancelled",
            Self::Retired => "retired",
            Self::Retiring => "retiring",
            Self::Uncertain => "uncertain",
            Self::InvalidState => "invalid-state",
            Self::RecoveryNotAllowed => "recovery-not-allowed",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum BlockedBy { Holder, Hold, Outside, Psi, Reserve, Fifo, Disk, Io }

impl BlockedBy {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Holder => "holder",
            Self::Hold => "hold",
            Self::Outside => "outside",
            Self::Psi => "psi",
            Self::Reserve => "reserve",
            Self::Fifo => "fifo",
            Self::Disk => "disk",
            Self::Io => "io",
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
    /// Reclaim credit was withheld because PSI full is >= 1% or its observation is missing.
    pub psi_blocked: bool,
    pub reserve_bytes: u64,
    pub fifo_predecessor: Option<String>,
    pub disk: Option<DiskFit>,
    /// Fresh mount observations for queued predecessors, keyed by reservation path.
    pub disk_mounts: BTreeMap<String, DiskFit>,
    pub heavy_io_slots: Option<u32>,
    pub pressure_blocked: bool,
    pub recovery_capable: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct DiskFit {
    pub filesystem: u64,
    pub available: u64,
    pub floor: u64,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disk_filesystem: Option<u64>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_by: Option<BlockedBy>,
    #[serde(default)]
    pub admission_decision: Option<AdmissionDecision>,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AdmissionDecision {
    pub class: RequestClass,
    pub at_ms: u64,
    pub exemption: Option<String>,
    pub free_bytes_at_admission: Option<u64>,
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
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-h94d100aa2222 owner=fractal_store::store::resource_membership TODO=chunk-or-reference-variable-columns */ "CREATE TABLE IF NOT EXISTS resource_authorities_v1 (
        host TEXT PRIMARY KEY, epoch TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS resource_lane_policies_v1 (
        host TEXT NOT NULL, lane TEXT NOT NULL, policy TEXT NOT NULL, PRIMARY KEY(host,lane));
        CREATE TABLE IF NOT EXISTS resource_memberships_v1 (
        id TEXT PRIMARY KEY, host TEXT NOT NULL, lane TEXT NOT NULL,
        state TEXT NOT NULL, queued_at INTEGER NOT NULL, memory INTEGER NOT NULL,
        body TEXT NOT NULL);
        CREATE INDEX IF NOT EXISTS resource_memberships_queue_v1
        ON resource_memberships_v1(host,lane,state,queued_at,id);
        CREATE TABLE IF NOT EXISTS resource_admission_pending_v1 (
        host TEXT NOT NULL, membership_id TEXT PRIMARY KEY);
        CREATE INDEX IF NOT EXISTS resource_admission_pending_host_v1 ON resource_admission_pending_v1(host);")
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
    super::resource_admission_objectives::observe(connection, member)?;
    // Durable feed invalidation commits with custody; a diagnostic note alone cannot wake IVM.
    // This observation references custody, never carries a replayable grant.
    insert_local_observation_tx(connection, &member.request.host, &ClaimInput {
        subject: dependency(&member.request.host, &member.request.name),
        kind: "resource-membership.changed".into(), actor: None,
        fields: BTreeMap::from([("membership".into(), json!(member.request.id)), ("state".into(), json!(member.state))]),
        evidence: Vec::new(), idempotency_key: None,
        expected_subject: None,
    }, now_ms())?;
    insert_local_observation_tx(connection, &member.request.host, &ClaimInput {
        subject: format!("resource-membership/{}", member.request.host),
        kind: "resource-membership.changed".into(), actor: None,
        fields: BTreeMap::from([("membership".into(), json!(member.request.id)), ("state".into(), json!(member.state))]),
        evidence: Vec::new(), idempotency_key: None, expected_subject: None,
    }, now_ms())?;
    Ok(())
}

fn authorized(member: &Membership, owner: &Owner) -> Result<()> {
    anyhow::ensure!(&member.request.owner == owner, "resource membership owner mismatch");
    Ok(())
}

fn fenced(connection: &Connection, member: &Membership, grant: &Grant) -> Result<()> {
    member.request.class.ensure_supported()?;
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

/// An aged head reserves by fencing new admissions, not by adding another held footprint.
pub const BACKFILL_AGING_MS: u64 = 15 * 60 * 1000;

fn resource_blocking(connection: &Connection, request: &LaneRequest, fit: &Fit, disk_fit: Option<DiskFit>) -> Result<Option<(BlockedBy, String)>> {
    if fit.pressure_blocked {
        return Ok(Some((BlockedBy::Psi, "host PSI brake: sustained pressure, recovery hysteresis, or unavailable observation".into())));
    }
    if request.class == RequestClass::Recovery {
        let held: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM resource_memberships_v1
            WHERE host=?1 AND id<>?2 AND state IN ('admitted','launched','retiring','uncertain')
            AND json_extract(body,'$.request.class')='recovery')",
            rusqlite::params![request.host,request.id], |row| row.get(0))?;
        if held { return Ok(Some((BlockedBy::Holder, "host recovery slot occupied".into()))); }
    }
    if !super::host_holds::blocking_tx(connection, &request.host, &request.name)?.is_empty() {
        return Ok(Some((BlockedBy::Hold, "host held".into())));
    }
    let held: u32 = connection.query_row("SELECT count(*) FROM resource_memberships_v1
        WHERE host=?1 AND lane=?2 AND state IN ('admitted','launched','retiring','uncertain')",
        rusqlite::params![request.host,request.name], |row| row.get(0))?;
    let unprofiled_held: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM resource_memberships_v1
        WHERE host=?1 AND lane=?2 AND state IN ('admitted','launched','retiring','uncertain')
        AND json_extract(body,'$.request.capacity') IS NULL)",
        rusqlite::params![request.host,request.name], |row| row.get(0))?;
    if unprofiled_held {
        return Ok(Some((BlockedBy::Holder, "existing unprofiled custody must retire before profiled admission".into())));
    }
    if held >= request.max_members { return Ok(Some((BlockedBy::Holder, "lane occupied".into()))); }
    let finite_held: u64 = connection.query_row("SELECT coalesce(sum(memory),0) FROM resource_memberships_v1
        WHERE host=?1 AND id<>?2 AND state IN ('admitted','launched','retiring','uncertain')",
        rusqlite::params![request.host,request.id], |row| row.get(0))?;
    let finite_cores: u64 = connection.query_row("SELECT coalesce(sum(json_extract(body,'$.request.capacity.cores')),0)
        FROM resource_memberships_v1 WHERE host=?1 AND id<>?2 AND state IN ('admitted','launched','retiring','uncertain')",
        rusqlite::params![request.host,request.id], |row| row.get(0))?;
    if request.capacity.is_some_and(|capacity| fit.room.is_none_or(|room|
        finite_cores.saturating_add(u64::from(capacity.cores)) > u64::from(room.cores))) {
        return Ok(Some((BlockedBy::Holder, "held CPU quotas prevent admission".into())));
    }
    let holder_free = fit.room.map_or(0, |room| room.memory.saturating_sub(fit.held_bytes).saturating_sub(finite_held));
    let outside_free = holder_free.saturating_sub(fit.outside_bytes);
    let free = outside_free.saturating_sub(fit.reserve_bytes);
    if request.capacity.is_some_and(|capacity| capacity.memory > holder_free) {
        return Ok(Some((BlockedBy::Holder, "holder footprint prevents admission".into())));
    }
    if request.capacity.is_some_and(|capacity| capacity.memory > outside_free) {
        return Ok(Some(if fit.psi_blocked {
            (BlockedBy::Psi, format!("outside footprint prevents admission: {} bytes; PSI full >= 1% or observation unavailable; reclaim credit withheld", fit.outside_bytes))
        } else {
            (BlockedBy::Outside, format!("outside footprint prevents admission: {} bytes", fit.outside_bytes))
        }));
    }
    if request.capacity.is_some_and(|capacity| capacity.memory > free) {
        return Ok(Some((BlockedBy::Reserve, format!("host reserve prevents admission: {} bytes", fit.reserve_bytes))));
    }
    let mut disk_held = 0_u64;
    let mut io_held = 0_u32;
    if request.disk.is_some() || request.io.is_some() {
        let mut statement = connection.prepare("SELECT body FROM resource_memberships_v1
            WHERE host=?1 AND id<>?2 AND state IN ('admitted','launched','retiring','uncertain')")?;
        for body in statement.query_map(rusqlite::params![request.host,request.id], |row| row.get::<_, String>(0))? {
            let holder: Membership = serde_json::from_str(&body?)?;
            if holder.request.io == Some(IoClass::Heavy) { io_held = io_held.saturating_add(1); }
            if let (Some(disk), Some(mount)) = (&holder.request.disk, disk_fit)
                && holder.disk_filesystem == Some(mount.filesystem)
            { disk_held = disk_held.saturating_add(disk.bytes); }
        }
    }
    if request.class != RequestClass::Recovery && request.disk.as_ref().is_some_and(|disk| disk_fit.is_none_or(|mount|
        u128::from(mount.available) < u128::from(mount.floor) + u128::from(disk_held) + u128::from(disk.bytes))) {
        return Ok(Some((BlockedBy::Disk, "mount free-space floor or peak disk reservations prevent admission".into())));
    }
    if request.io == Some(IoClass::Heavy) && fit.heavy_io_slots.is_some_and(|slots| io_held >= slots) {
        return Ok(Some((BlockedBy::Io, "host heavy IO slots occupied".into())));
    }
    Ok(None)
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
        let aging = (member.state == "queued" && elapsed < BACKFILL_AGING_MS)
            .then(|| BACKFILL_AGING_MS - elapsed);
        Ok(match (member.request.admission_timeout_ms.map(|budget| budget.saturating_sub(elapsed)), aging) {
            (Some(timeout), Some(aging)) => Some(timeout.min(aging)),
            (timeout, aging) => timeout.or(aging),
        })
    }

    pub fn resource_membership_for_grant(&self, grant: &Grant) -> Result<Membership> {
        let connection = self.readers.get();
        let member = read(&connection, &grant.membership_id)?;
        fenced(&connection, &member, grant)?;
        smallclaims::touched::note_read(|| dependency(&member.request.host, &member.request.name));
        Ok(member)
    }
    /// Reconciliation reuses the host's first durable estimate, including while queued.
    pub fn resource_profile_request(&self, id: &str) -> Result<Option<LaneRequest>> {
        let body: Option<String> = self.readers.get().query_row(
            "SELECT body FROM resource_memberships_v1 WHERE id=?1", [id], |row| row.get(0)).optional()?;
        body.map(|body| serde_json::from_str::<Membership>(&body).map(|member| member.request))
            .transpose().map_err(Into::into)
    }

    /// The parent containment covers every live frozen member, not latest-estimate × slots.
    pub fn resource_lane_aggregate(&self, host: &str, lane: &str) -> Result<Option<Capacity>> {
        let connection = self.readers.get();
        let mut statement = connection.prepare("SELECT body FROM resource_memberships_v1
            WHERE host=?1 AND lane=?2 AND state IN ('admitted','launched','retiring','uncertain')")?;
        let rows = statement.query_map(rusqlite::params![host, lane], |row| row.get::<_, String>(0))?;
        let mut aggregate: Option<Capacity> = None;
        for row in rows {
            let member: Membership = serde_json::from_str(&row?)?;
            if let Some(capacity) = member.request.capacity {
                let current = aggregate.get_or_insert(Capacity { cores: 0, memory: 0 });
                current.cores = current.cores.checked_add(capacity.cores).context("aggregate lane cores overflow")?;
                current.memory = current.memory.checked_add(capacity.memory).context("aggregate lane memory overflow")?;
            }
        }
        Ok(aggregate)
    }


    pub fn resource_memberships_for_host(&self, host: &str) -> Result<Vec<Membership>> {
        let connection = self.readers.get();
        let mut statement = connection.prepare("SELECT body FROM resource_memberships_v1 WHERE host=?1
            ORDER BY CASE coalesce(json_extract(body,'$.request.class'),'agent') WHEN 'recovery' THEN 0 WHEN 'landing' THEN 1 WHEN 'check' THEN 2 ELSE 3 END,rowid")?;
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
        let mut connection = self.connection.write();
        let transaction = connection.transaction()?;
        transaction.execute(/* sqlite-large-value: allow-for-now lv-h466c8225b3a9 owner=fractal_store::store::resource_membership TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO resource_authorities_v1 VALUES(?1,?2)
            ON CONFLICT(host) DO UPDATE SET epoch=excluded.epoch", rusqlite::params![host,epoch])?;
        let complete = super::resource_admission_objectives::recover(&transaction, host, epoch)?;
        transaction.commit()?;
        super::resource_admission_objectives::activate(self);
        self.fractal.extensions.get::<super::resource_admission_objectives::Coverage>().0
            .store(complete, std::sync::atomic::Ordering::Release);
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
            if let Err(error) = member.request.class.ensure_supported() {
                tracing::warn!(membership = %member.request.id, %error, "retaining newer-class resource custody until the execution host is upgraded");
                uncertain += 1;
                continue;
            }
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
        request.class.ensure_supported()?;
        anyhow::ensure!(!request.id.is_empty() && !request.name.is_empty() && !request.host.is_empty(), "empty resource identity");
        anyhow::ensure!(request.host == self.origin, "only the execution host may grant resource membership");
        anyhow::ensure!(request.max_members > 0, "maxMembers must be positive");
        anyhow::ensure!(!request.owner.run.is_empty() && !request.owner.step.is_empty() && request.owner.attempt > 0, "resource execution owner required");
        let mut connection = self.connection.write();
        let transaction = connection.transaction()?;
        super::resource_admission_objectives::validate_publication(&transaction, &request.host, &request.class)?;
        let epoch: String = transaction.query_row("SELECT epoch FROM resource_authorities_v1 WHERE host=?1", [&request.host], |r| r.get(0))?;
        if request.capacity.is_some() {
            let expected = InstalledPolicy::for_request(request);
            let policy = serde_json::to_string(&expected)?;
            let installed: Option<String> = transaction.query_row("SELECT policy FROM resource_lane_policies_v1 WHERE host=?1 AND lane=?2",
                rusqlite::params![request.host,request.name], |row| row.get(0)).optional()?;
            if let Some(installed) = installed {
                let previous: InstalledPolicy = serde_json::from_str(&installed)?;
                if previous != expected {
                    anyhow::ensure!(previous == InstalledPolicy::Declared((request.max_members, None, request.coverage)),
                        "resource lane policy differs from host-installed policy");
                    // Only the historical profile-less policy may acquire its first footprint.
                transaction.execute(/* sqlite-large-value: allow-for-now lv-h156314ba7532 owner=fractal_store::store::resource_membership TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM resource_lane_policies_v1 WHERE host=?1 AND lane=?2",
                    rusqlite::params![request.host,request.name])?;
                }
            }
            transaction.execute(/* sqlite-large-value: allow-for-now lv-hacf1fd1c6d10 owner=fractal_store::store::resource_membership TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO resource_lane_policies_v1 VALUES(?1,?2,?3)",
                rusqlite::params![request.host,request.name,policy])?;
        }
        let exists: bool = transaction.query_row("SELECT EXISTS(SELECT 1 FROM resource_memberships_v1 WHERE id=?1)", [&request.id], |r| r.get(0))?;
        let mut member = if exists {
            let member = read(&transaction, &request.id)?;
            anyhow::ensure!(member.request == *request, "resource request identity reused with different execution");
            member
        } else {
            Membership { request: request.clone(), grant: None, state: "queued".into(), reason: "waiting for admission".into(),
                queued_at: now_ms, admitted_at: None, payload_released_at: None, execution_ended_at: None,
                retirement_requested_at: None, retired_at: None, disk_filesystem: None,
                roots: Vec::new(), child_runs: Vec::new(), clock_uncertain: false, clock_epoch: epoch.clone(),
                refusal_code: None, blocked_by: None, admission_decision: None }
        };
        let previous = exists.then(|| member.state.clone());
        if member.state != "queued" {
            if let Some(grant) = &member.grant { fenced(&transaction, &member, grant)?; }
            return Ok(admission(member));
        }
        smallclaims::touched::note_read(|| dependency(&request.host, &request.name));
        smallclaims::touched::note_read(|| format!("resource-membership/{}", request.host));
        let (queue_elapsed, uncertain) = self.resource_admission_elapsed(&request.id, member.queued_at, now_ms);
        member.clock_uncertain |= uncertain || member.clock_epoch != epoch;
        let refused = if request.class == RequestClass::Recovery && !fit.recovery_capable {
            Some((RefusalCode::RecoveryNotAllowed, "profile is not declared recoveryCapable on this host".into()))
        } else if request.class == RequestClass::Recovery && fit.disk.is_none() {
            Some((RefusalCode::UnsupportedCoverage, "recovery admission requires a measured filesystem free-space observation".into()))
        } else if request.capacity.is_none() {
            Some((RefusalCode::MissingProfile, "finite execution requires an explicit resource profile".to_owned()))
        } else if request.coverage == Coverage::Enforced && !cfg!(target_os = "linux") {
            Some((RefusalCode::UnsupportedCoverage, "whole-tree resource enforcement requires Linux".to_owned()))
        } else if now_ms < member.queued_at {
            Some((RefusalCode::ClockUncertain, "queue clock moved backwards; admission requires reconciliation".into()))
        } else if request.admission_timeout_ms.is_some_and(|timeout| queue_elapsed >= timeout) {
            Some((RefusalCode::AdmissionExpired, "execution clock has not started".into()))
        } else if request.io == Some(IoClass::Heavy) && fit.heavy_io_slots == Some(0) {
            Some((RefusalCode::Impossible, "host policy disables heavy IO execution".into()))
        } else if let Some(capacity) = request.capacity {
            match fit.room {
                None => Some((RefusalCode::UnsupportedCoverage, "host lane fit is not configured".into())),
                Some(room) if capacity.cores > room.cores || capacity.memory > room.memory => Some((RefusalCode::Impossible, "requested footprint exceeds host lane room".into())),
                _ => None,
            }
        } else { None };
        member.blocked_by = None;
        if let Some((code, reason)) = refused {
            member.state = "refused".into(); member.reason = reason; member.refusal_code = Some(code);
        } else {
            // Enqueue before inspecting the host-wide head, including first-time requests.
            // The same writer transaction owns ordering and every finite resource reservation.
            if !exists { write(&transaction, &member)?; }
            let mut statement = transaction.prepare("SELECT body FROM resource_memberships_v1
                WHERE host=?1 AND state='queued'
                ORDER BY CASE coalesce(json_extract(body,'$.request.class'),'agent') WHEN 'recovery' THEN 0 WHEN 'landing' THEN 1 WHEN 'check' THEN 2 ELSE 3 END,rowid")?;
            let queued = statement.query_map([&request.host], |row| row.get::<_, String>(0))?;
            let mut predecessor = None;
            for body in queued {
                let earlier: Membership = serde_json::from_str(&body?)?;
                // Preserve unsupported queued records without letting work this binary cannot
                // execute or withdraw become a permanent FIFO predecessor.
                if matches!(earlier.request.class, RequestClass::Unknown(_)) { continue; }
                if earlier.request.id == request.id { break; }
                let (elapsed, uncertain) = self.resource_admission_elapsed(&earlier.request.id, earlier.queued_at, now_ms);
                if earlier.request.class != request.class || uncertain || elapsed >= BACKFILL_AGING_MS {
                    predecessor = Some((BlockedBy::Fifo, format!("class or aged predecessor: {}", earlier.request.id)));
                    break;
                }
                let disk_fit = earlier.request.disk.as_ref().and_then(|disk|
                    fit.disk_mounts.get(&disk.path).copied().or_else(||
                        (request.disk.as_ref().is_some_and(|own| own.path == disk.path)).then_some(fit.disk).flatten()));
                // Without a fresh observation we cannot prove a disk predecessor does not fit.
                let unknown_disk = earlier.request.disk.is_some() && disk_fit.is_none();
                if unknown_disk || resource_blocking(&transaction, &earlier.request, &fit, disk_fit)?.is_none() {
                    predecessor = Some((BlockedBy::Fifo, format!("fitting predecessor: {}", earlier.request.id)));
                    break;
                }
            }
            drop(statement);
            let ordering = predecessor.or_else(|| fit.fifo_predecessor.as_ref()
                .map(|id| (BlockedBy::Fifo, format!("host fit predecessor: {id}"))));
            let blocking = match ordering {
                Some(blocking) => Some(blocking),
                None => resource_blocking(&transaction, request, &fit, fit.disk)?,
            };
            member.reason = if let Some((blocked_by, reason)) = blocking {
                member.blocked_by = Some(blocked_by);
                reason
            } else { String::new() };
            if member.reason.is_empty() {
                member.state = "admitted".into(); member.admitted_at = Some(now_ms);
                member.disk_filesystem = fit.disk.map(|mount| mount.filesystem);
                member.admission_decision = Some(AdmissionDecision {
                    class: request.class.clone(), at_ms: now_ms,
                    exemption: (request.class == RequestClass::Recovery).then(|| "disk-floor".into()),
                    free_bytes_at_admission: fit.disk.map(|mount| mount.available),
                });
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
        member.request.class.ensure_supported()?;
        let previous = member.state.clone();
        if member.state == "queued" {
            member.state = "retired".into(); member.reason = "cancelled before admission".into(); member.retired_at = Some(now_ms);
            member.refusal_code = Some(RefusalCode::Cancelled);
            member.blocked_by = None;
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
            max_members: 1, capacity: Some(Capacity { cores: 1, memory: 256 << 20 }), coverage: Coverage::Logical, admission_timeout_ms: None,
            class: RequestClass::Agent, disk: None, io: None,
            profile: None, profile_estimate: None, disk_path: None,
            owner: Owner { run: format!("mission-run/{id}"), step: format!("step-run/{id}"),
                attempt: 1, invocation: Some(format!("invocation/{id}")) } }
    }
    fn test_fit() -> Fit {
        Fit { room: Some(Capacity { cores: 32, memory: 128 << 30 }), ..Fit::default() }
    }
    fn fixture() -> Store {
        let store = Store::open_memory("host-a").unwrap();
        store.start_resource_authority("host-a", "epoch-a").unwrap();
        store
    }
    fn grant(store: &Store, request: &LaneRequest, now: u64) -> Grant {
        match store.request_resource_membership(request, test_fit(), now).unwrap() {
            Admission::Admitted(grant) => grant,
            other => panic!("expected admission: {other:?}"),
        }
    }
    fn runtime(root: &Path) -> ExecRuntime {
        ExecRuntime::new(root.join("runtime"), root.join("logs"))
    }

    #[test]
    fn future_resource_classes_cannot_acquire_membership() {
        let store = fixture();
        let mut future = request("future");
        future.class = RequestClass::Unknown("future-class".into());
        assert!(store.request_resource_membership(&future, test_fit(), 1).unwrap_err()
            .to_string().contains("unsupported resource class future-class"));
        assert!(store.resource_memberships_for_host("host-a").unwrap().is_empty());
    }

    #[test]
    fn future_resource_class_custody_is_retained_during_authority_recovery() {
        let store = fixture();
        let request = request("future-custody");
        let grant = grant(&store, &request, 1);
        store.update_resource_membership(&grant, |member| {
            member.request.class = RequestClass::Unknown("future-class".into());
            Ok(())
        }).unwrap();
        let before = serde_json::to_value(store.resource_membership_record(&request.id, &request.owner).unwrap()).unwrap();
        store.start_resource_authority("host-a", "epoch-b").unwrap();
        let directory = tempfile::tempdir().unwrap();
        assert_eq!(store.recover_resource_memberships("host-a", &runtime(directory.path()), 2).unwrap(), 1);
        let after = serde_json::to_value(store.resource_membership_record(&request.id, &request.owner).unwrap()).unwrap();
        assert_eq!(after, before);
        let coverage = store.claims_by_kinds_after(&[super::resource_admission_objectives::COVERAGE_KIND], None, 0, 10).unwrap();
        assert_eq!(coverage.claims.last().unwrap().body["fields"]["complete"], false);
        assert!(!store.fractal.extensions.get::<super::resource_admission_objectives::Coverage>().0
            .load(std::sync::atomic::Ordering::Acquire));
        let mut supported = request.clone();
        supported.id = "supported-after-recovery".into();
        supported.name = "supported-lane".into();
        assert!(matches!(store.request_resource_membership(&supported, test_fit(), 3).unwrap(), Admission::Admitted(_)));
    }

    #[test]
    fn unsupported_queued_membership_survives_restart_without_blocking_supported_fifo() {
        for same_lane in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("graph.db");
            let queued = request("future-queued");
            let body;
            {
                let store = Store::open(&path, "host-a").unwrap();
                store.start_resource_authority("host-a", "epoch-a").unwrap();
                let blocked = Fit { fifo_predecessor: Some("seat/predecessor".into()), ..test_fit() };
                assert!(matches!(store.request_resource_membership(&queued, blocked, 1).unwrap(), Admission::Queued { .. }));
                let mut member = store.resource_membership_record(&queued.id, &queued.owner).unwrap();
                member.request.class = RequestClass::Unknown("future-class".into());
                // Seed a newer writer's authoritative representation, including whitespace.
                body = serde_json::to_string_pretty(&member).unwrap();
                store.connection.write().execute("UPDATE resource_memberships_v1 SET body=?2 WHERE id=?1",
                    rusqlite::params![queued.id, body]).unwrap();
            }
            let store = Store::open(&path, "host-a").unwrap();
            store.start_resource_authority("host-a", "epoch-b").unwrap();
            assert_eq!(store.recover_resource_memberships("host-a", &runtime(directory.path()), 2).unwrap(), 0);
            assert_eq!(store.held_resource_membership_bytes("host-a", "").unwrap(), 0);
            assert!(store.withdraw_resource_membership(&queued.id, &queued.owner, 3).is_err());
            let mut supported = request("supported");
            if !same_lane { supported.name = "executor/supported".into(); }
            let admitted = grant(&store, &supported, 4);
            assert_eq!(admitted.authority_epoch, "epoch-b");
            let retained: (String, String, u64, u64) = store.connection.write().query_row(
                "SELECT body,state,queued_at,memory FROM resource_memberships_v1 WHERE id=?1",
                [&queued.id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))).unwrap();
            assert_eq!(retained, (body, "queued".into(), 1, queued.capacity.unwrap().memory),
                "unsupported queued bytes and queue metadata must remain intact");
            assert_eq!(store.held_resource_membership_bytes("host-a", "").unwrap(), supported.capacity.unwrap().memory);
        }
    }

    #[test]
    fn authority_startup_withholds_newer_evidence_without_extending_admission_authority() {
        use super::super::resource_admission_objectives::{Coverage as AdmissionCoverage, COVERAGE_KIND, KIND};
        use std::sync::atomic::Ordering;

        for (class, refusal, complete) in [
            (RequestClass::Unknown("future-class".into()), None, false),
            (RequestClass::Recovery, Some(RefusalCode::RecoveryNotAllowed), false),
            (RequestClass::Agent, Some(RefusalCode::RecoveryNotAllowed), false),
            (RequestClass::Check, None, true),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("graph.db");
            let persisted_request = request("persisted");
            let old_grant;
            let body;
            let evidence_count;
            {
                let store = Store::open(&path, "host-a").unwrap();
                store.start_resource_authority("host-a", "epoch-a").unwrap();
                old_grant = grant(&store, &persisted_request, 1);
                evidence_count = store.claims_by_kinds_after(&[KIND], None, 0, 100).unwrap().claims.len();
                let mut member = store.resource_membership_record(&persisted_request.id, &persisted_request.owner).unwrap();
                member.request.class = class.clone();
                member.refusal_code = refusal;
                // Seed authoritative bytes from a newer writer, without going through this
                // binary's evidence writer. Acquisition must withhold unsupported publication.
                body = serde_json::to_string(&member).unwrap();
                store.connection.write().execute("UPDATE resource_memberships_v1 SET body=?2 WHERE id=?1",
                    rusqlite::params![persisted_request.id, body]).unwrap();
            }
            let store = Store::open(&path, "host-a").unwrap();
            assert!(!store.fractal.extensions.get::<AdmissionCoverage>().0.load(Ordering::Acquire));
            store.start_resource_authority("host-a", "epoch-b").unwrap();
            assert_eq!(store.fractal.extensions.get::<AdmissionCoverage>().0.load(Ordering::Acquire), complete);
            let evidence = store.claims_by_kinds_after(&[KIND], None, 0, 100).unwrap().claims;
            let fields = &evidence.last().unwrap().body["fields"];
            if complete {
                assert_eq!(fields["class"], serde_json::to_value(&class).unwrap());
                assert_eq!(fields["refusal"], serde_json::to_value(refusal).unwrap());
            } else {
                assert_eq!(fields["class"], "agent", "no unsupported observation was republished");
                assert!(fields["refusal"].is_null());
                assert_eq!(evidence.len(), evidence_count);
            }
            let cuts = store.claims_by_kinds_after(&[COVERAGE_KIND], None, 0, 100).unwrap().claims;
            assert_eq!(cuts.last().unwrap().body["fields"]["complete"], json!(complete));
            assert!(store.prepare_resource_membership(&old_grant, "stale-root").is_err());
            if class.ensure_supported().is_err() {
                assert_eq!(store.recover_resource_memberships("host-a", &runtime(directory.path()), 2).unwrap(), 1);
                let retained: String = store.connection.write().query_row(
                    "SELECT body FROM resource_memberships_v1 WHERE id=?1", [&persisted_request.id], |row| row.get(0)).unwrap();
                assert_eq!(retained, body, "unsupported custody must remain byte-for-byte intact");
                let mut unsupported = request("unsupported");
                unsupported.class = class;
                assert!(store.request_resource_membership(&unsupported, test_fit(), 3).is_err());
            }
            // An uncertified SLO source must not disable supported work or ignore held custody.
            let mut supported = request("supported");
            supported.name = "executor/supported".into();
            let admitted = match store.request_resource_membership(&supported, test_fit(), 4).unwrap() {
                Admission::Admitted(grant) => grant,
                other => panic!("supported admission blocked after startup: {other:?}"),
            };
            assert_eq!(admitted.authority_epoch, "epoch-b");
            assert_eq!(store.held_resource_membership_bytes("host-a", "").unwrap(), 512 << 20);
            store.start_resource_authority("host-a", "epoch-c").unwrap();
            assert_eq!(store.fractal.extensions.get::<AdmissionCoverage>().0.load(Ordering::Acquire), complete);
        }
    }

    #[test]
    fn disk_reservations_are_mount_local_and_the_floor_applies_to_zero_byte_requests() {
        let store = fixture();
        let mut first = request("mount-one");
        first.disk = Some(DiskRequest { bytes: 80, path: "/one".into() });
        let fit = Fit { disk: Some(DiskFit { filesystem: 1, available: 100, floor: 20 }), ..test_fit() };
        assert!(matches!(store.request_resource_membership(&first, fit, 1).unwrap(), Admission::Admitted(_)));
        let mut other = request("mount-two");
        other.name = "another-lane".into();
        other.disk = Some(DiskRequest { bytes: 80, path: "/two".into() });
        let fit = Fit { disk: Some(DiskFit { filesystem: 2, available: 100, floor: 20 }), ..test_fit() };
        assert!(matches!(store.request_resource_membership(&other, fit, 2).unwrap(), Admission::Admitted(_)));
        let mut full = request("below-floor");
        full.name = "third-lane".into();
        full.disk = Some(DiskRequest { bytes: 0, path: "/three".into() });
        let fit = Fit { disk: Some(DiskFit { filesystem: 3, available: 19, floor: 20 }), ..test_fit() };
        assert!(matches!(store.request_resource_membership(&full, fit, 3).unwrap(), Admission::Queued { .. }));
        assert_eq!(store.resource_membership_record(&full.id, &full.owner).unwrap().blocked_by, Some(BlockedBy::Disk));
    }

    fn sized_request(id: &str, memory: u64) -> LaneRequest {
        let mut request = request(id);
        request.name = id.into();
        request.capacity = Some(Capacity { cores: 1, memory });
        request
    }

    fn small_room() -> Fit {
        Fit { room: Some(Capacity { cores: 8, memory: 100 }), ..test_fit() }
    }

    #[test]
    fn fitting_backfill_passes_until_the_head_ages_then_capacity_is_reserved_once() {
        let store = fixture();
        let holder = sized_request("holder", 60);
        let head = sized_request("head", 80);
        let later = sized_request("later", 20);
        let last = sized_request("last", 10);
        let Admission::Admitted(holder_grant) = store.request_resource_membership(&holder, small_room(), 1).unwrap()
            else { panic!("holder fits"); };
        assert!(matches!(store.request_resource_membership(&head, small_room(), 2).unwrap(), Admission::Queued { .. }));
        store.advance_resource_admission_clock(&head.id, BACKFILL_AGING_MS / 2);
        assert!(matches!(store.request_resource_membership(&later, small_room(), 3).unwrap(), Admission::Admitted(_)));
        store.advance_resource_admission_clock(&head.id, BACKFILL_AGING_MS / 2);
        assert!(matches!(store.request_resource_membership(&last, small_room(), 4).unwrap(), Admission::Queued { .. }));
        assert_eq!(store.resource_membership_record(&last.id, &last.owner).unwrap().blocked_by, Some(BlockedBy::Fifo));
        assert_eq!(store.held_resource_membership_bytes("host-a", "").unwrap(), 80, "the aged reservation is a fence, not another memory holder");
        let root = tempfile::tempdir().unwrap();
        assert!(store.retire_resource_membership(&holder_grant, &holder.owner, &runtime(root.path()), 5).unwrap());
        assert!(matches!(store.request_resource_membership(&head, small_room(), 6).unwrap(), Admission::Admitted(_)));
        assert_eq!(store.held_resource_membership_bytes("host-a", "").unwrap(), 100);
    }

    #[test]
    fn cancelling_an_aged_head_releases_its_fence_for_the_next_ticket() {
        let store = fixture();
        let head = sized_request("head", 80);
        let later = sized_request("later", 20);
        let blocked = Fit { outside_bytes: 50, ..small_room() };
        assert!(matches!(store.request_resource_membership(&head, blocked.clone(), 1).unwrap(), Admission::Queued { .. }));
        let deadline = store.resource_admission_remaining(&head.id, &head.owner, 1).unwrap().unwrap();
        assert!(deadline <= BACKFILL_AGING_MS && deadline > BACKFILL_AGING_MS / 2);
        store.advance_resource_admission_clock(&head.id, BACKFILL_AGING_MS);
        assert_eq!(store.resource_admission_remaining(&head.id, &head.owner, 2).unwrap(), None, "an aged head does not install a polling deadline");
        assert!(matches!(store.request_resource_membership(&later, blocked.clone(), 2).unwrap(), Admission::Queued { .. }));
        store.withdraw_resource_membership(&head.id, &head.owner, 3).unwrap();
        assert!(matches!(store.request_resource_membership(&later, blocked, 4).unwrap(), Admission::Admitted(_)));
    }

    #[test]
    fn backfill_never_crosses_class_priority() {
        for lower in [RequestClass::Check, RequestClass::Agent] {
            let store = fixture();
            let mut head = sized_request("landing-head", 80);
            head.class = RequestClass::Landing;
            let mut later = sized_request("later", 20);
            later.class = lower;
            let fit = Fit { outside_bytes: 50, ..small_room() };
            assert!(matches!(store.request_resource_membership(&head, fit.clone(), 1).unwrap(), Admission::Queued { .. }));
            assert!(matches!(store.request_resource_membership(&later, fit, 2).unwrap(), Admission::Queued { .. }));
            assert_eq!(store.resource_membership_record(&later.id, &later.owner).unwrap().blocked_by, Some(BlockedBy::Fifo));
        }
    }

    #[test]
    fn missing_profile_is_typed_and_does_not_install_a_lane_default() {
        let store = fixture();
        let mut missing = request("missing");
        missing.capacity = None;
        assert!(matches!(store.request_resource_membership(&missing, test_fit(), 1).unwrap(),
            Admission::Refused { code: RefusalCode::MissingProfile, .. }));
        assert_eq!(serde_json::to_value(RefusalCode::MissingProfile).unwrap(), "missing-profile");
        assert!(matches!(store.request_resource_membership(&request("profiled"), test_fit(), 2).unwrap(), Admission::Admitted(_)));
    }

    #[test]
    fn an_oversize_head_without_holders_is_impossible_and_cannot_wedge_the_queue() {
        let store = fixture();
        let head = sized_request("oversize", 101);
        assert!(matches!(store.request_resource_membership(&head, small_room(), 1).unwrap(),
            Admission::Refused { code: RefusalCode::Impossible, .. }));
        assert_eq!(store.held_resource_membership_bytes("host-a", "").unwrap(), 0);
        assert!(matches!(store.request_resource_membership(&sized_request("next", 20), small_room(), 2).unwrap(), Admission::Admitted(_)));
    }

    #[test]
    fn disk_and_heavy_io_blocked_heads_allow_fitting_backfill_before_aging() {
        for disk_block in [false, true] {
            let store = fixture();
            let mut holder = sized_request("holder", 1);
            holder.io = Some(IoClass::Heavy);
            let mut head = sized_request("head", 1);
            head.io = (!disk_block).then_some(IoClass::Heavy);
            head.disk = disk_block.then(|| DiskRequest { bytes: 90, path: "/work".into() });
            let mut fit = Fit { heavy_io_slots: Some(1), ..small_room() };
            fit.disk_mounts.insert("/work".into(), DiskFit { filesystem: 1, available: 100, floor: 20 });
            assert!(matches!(store.request_resource_membership(&holder, fit.clone(), 1).unwrap(), Admission::Admitted(_)));
            fit.disk = head.disk.as_ref().map(|_| fit.disk_mounts["/work"]);
            assert!(matches!(store.request_resource_membership(&head, fit.clone(), 2).unwrap(), Admission::Queued { .. }));
            fit.disk = None;
            assert!(matches!(store.request_resource_membership(&sized_request("later", 1), fit, 3).unwrap(), Admission::Admitted(_)));
        }
    }

    #[test]
    fn profile_less_installed_policy_upgrades_after_reopen_without_releasing_old_custody() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("store.sqlite");
        let mut historical = request("historical");
        historical.max_members = 2;
        let historical_grant;
        {
            let store = Store::open(&path, "host-a").unwrap();
            store.start_resource_authority("host-a", "epoch-a").unwrap();
            historical_grant = grant(&store, &historical, 1);
            let mut legacy = store.resource_membership_record(&historical.id, &historical.owner).unwrap();
            legacy.request.capacity = None;
            historical.capacity = None;
            // Seed bytes written by the earlier profile-optional authority.
            let connection = store.connection.write();
            connection.execute("UPDATE resource_lane_policies_v1 SET policy=?1",
                [serde_json::to_string(&(2_u32, None::<Capacity>, Coverage::Logical)).unwrap()]).unwrap();
            connection.execute("UPDATE resource_memberships_v1 SET memory=0,body=?1 WHERE id=?2",
                rusqlite::params![serde_json::to_string(&legacy).unwrap(),historical.id]).unwrap();
        }
        let store = Store::open(&path, "host-a").unwrap();
        assert!(matches!(store.request_resource_membership(&historical, test_fit(), 2).unwrap(), Admission::Admitted(_)),
            "an existing grant retains custody despite the new admission requirement");
        let mut profiled = request("profiled");
        profiled.max_members = 2;
        assert!(matches!(store.request_resource_membership(&profiled, test_fit(), 3).unwrap(), Admission::Queued { .. }));
        assert_eq!(store.resource_membership_record(&profiled.id, &profiled.owner).unwrap().blocked_by, Some(BlockedBy::Holder));
        assert_eq!(store.resource_membership_record(&historical.id, &historical.owner).unwrap().grant, Some(historical_grant.clone()));
        assert!(store.retire_resource_membership(&historical_grant, &historical.owner, &runtime(root.path()), 4).unwrap());
        assert!(matches!(store.request_resource_membership(&profiled, test_fit(), 5).unwrap(), Admission::Admitted(_)));
    }

    #[test]
    fn host_classes_are_strict_and_fitting_predecessors_keep_fifo_across_lanes() {
        let store = fixture();
        let mut requests: Vec<_> = [
            ("agent", RequestClass::Agent), ("check-first", RequestClass::Check),
            ("landing-first", RequestClass::Landing), ("landing-next", RequestClass::Landing),
            ("check-next", RequestClass::Check),
        ].into_iter().map(|(id, class)| {
            let mut candidate = request(id);
            candidate.name = id.into();
            candidate.class = class;
            candidate.capacity = Some(Capacity { cores: 1, memory: if candidate.class == RequestClass::Landing { 80 } else { 20 } });
            candidate
        }).collect();
        let blocked = Fit { room: Some(Capacity { cores: 8, memory: 100 }), outside_bytes: 100, ..test_fit() };
        for candidate in &requests {
            assert!(matches!(store.request_resource_membership(candidate, blocked.clone(), 1).unwrap(), Admission::Queued { .. }));
        }
        let order: Vec<_> = store.resource_memberships_for_host("host-a").unwrap().into_iter()
            .map(|member| member.request.id).collect();
        assert_eq!(order, ["landing-first", "landing-next", "check-first", "check-next", "agent"]);
        // A lower-class candidate fits on its own, but cannot bypass the landing head.
        let partial = Fit { room: blocked.room, outside_bytes: 50, ..test_fit() };
        assert!(matches!(store.request_resource_membership(&requests[2], partial.clone(), 2).unwrap(), Admission::Queued { .. }));
        assert_eq!(store.resource_membership_record(&requests[2].id, &requests[2].owner).unwrap().blocked_by, Some(BlockedBy::Outside));
        assert!(matches!(store.request_resource_membership(&requests[1], partial.clone(), 2).unwrap(), Admission::Queued { .. }));
        assert_eq!(store.resource_membership_record(&requests[1].id, &requests[1].owner).unwrap().blocked_by, Some(BlockedBy::Fifo));
        assert!(matches!(store.request_resource_membership(&requests[0], partial, 2).unwrap(), Admission::Queued { .. }));
        let fits = Fit { room: blocked.room, ..test_fit() };
        assert_eq!(store.resource_membership_record(&requests[0].id, &requests[0].owner).unwrap().blocked_by, Some(BlockedBy::Fifo));
        for id in order {
            let index = requests.iter().position(|candidate| candidate.id == id).unwrap();
            let candidate = requests.remove(index);
            let Admission::Admitted(grant) = store.request_resource_membership(&candidate, fits.clone(), 3).unwrap()
                else { panic!("ordered candidate must fit"); };
            let root = tempfile::tempdir().unwrap();
            assert!(store.retire_resource_membership(&grant, &candidate.owner, &runtime(root.path()), 4).unwrap());
        }
    }

    #[test]
    fn disk_mount_floor_and_peak_reservation_survive_cancellation_until_retirement() {
        let store = fixture();
        let mut first = request("disk-first");
        first.disk = Some(DiskRequest { bytes: 60, path: "/work".into() });
        let fit = Fit { disk: Some(DiskFit { filesystem: 7, available: 100, floor: 20 }), ..test_fit() };
        let Admission::Admitted(first_grant) = store.request_resource_membership(&first, fit.clone(), 1).unwrap()
            else { panic!("first disk reservation fits"); };
        let mut next = request("disk-next");
        next.name = "other-lane".into();
        next.disk = Some(DiskRequest { bytes: 21, path: "/other-path-on-same-mount".into() });
        assert!(matches!(store.request_resource_membership(&next, fit.clone(), 2).unwrap(), Admission::Queued { .. }));
        assert_eq!(store.resource_membership_record(&next.id, &next.owner).unwrap().blocked_by, Some(BlockedBy::Disk));
        store.cancel_resource_membership(&first_grant, &first.owner, 3).unwrap();
        assert!(matches!(store.request_resource_membership(&next, fit.clone(), 4).unwrap(), Admission::Queued { .. }));
        let root = tempfile::tempdir().unwrap();
        assert!(store.retire_resource_membership(&first_grant, &first.owner, &runtime(root.path()), 5).unwrap());
        assert!(matches!(store.request_resource_membership(&next, fit, 6).unwrap(), Admission::Admitted(_)));
    }

    #[test]
    fn heavy_io_slots_are_host_policy_and_shared_across_lanes() {
        let store = fixture();
        let mut first = request("heavy-first");
        first.io = Some(IoClass::Heavy);
        let fit = Fit { heavy_io_slots: Some(1), ..test_fit() };
        assert!(matches!(store.request_resource_membership(&first, fit.clone(), 1).unwrap(), Admission::Admitted(_)));
        let mut next = request("heavy-next");
        next.name = "another-lane".into();
        next.io = Some(IoClass::Heavy);
        assert!(matches!(store.request_resource_membership(&next, fit, 2).unwrap(), Admission::Queued { .. }));
        assert_eq!(store.resource_membership_record(&next.id, &next.owner).unwrap().blocked_by, Some(BlockedBy::Io));
        let larger_policy = Fit { heavy_io_slots: Some(2), ..test_fit() };
        assert!(matches!(store.request_resource_membership(&next, larger_policy, 3).unwrap(), Admission::Admitted(_)));
    }

    #[test]
    fn historical_requests_default_to_agent_without_disk_or_io() {
        let mut value = serde_json::to_value(request("old")).unwrap();
        let object = value.as_object_mut().unwrap();
        object.remove("class");
        object.remove("disk");
        object.remove("io");
        let decoded: LaneRequest = serde_json::from_value(value).unwrap();
        assert_eq!(decoded.class, RequestClass::Agent);
        assert_eq!(decoded.disk, None);
        assert_eq!(decoded.io, None);
    }

    #[test]
    fn historical_impossible_refusal_remains_readable_with_canonical_output() {
        let code: RefusalCode = serde_json::from_value(json!("impossible-capacity")).unwrap();
        assert_eq!(code, RefusalCode::Impossible);
        assert_eq!(serde_json::to_value(code).unwrap(), "impossible");
    }

    #[test]
    fn impossible_cores_and_memory_refuse_before_lane_and_host_fifo() {
        for capacity in [Capacity { cores: 5, memory: 64 }, Capacity { cores: 1, memory: 101 }] {
            let store = fixture();
            let mut first = request("predecessor");
            first.capacity = Some(capacity);
            let large_room = Fit { room: Some(Capacity { cores: 8, memory: 200 }), outside_bytes: 200, ..test_fit() };
            assert!(matches!(store.request_resource_membership(&first, large_room, 1).unwrap(), Admission::Queued { .. }));
            let mut next = request("impossible");
            next.capacity = Some(capacity);
            let fit = Fit { room: Some(Capacity { cores: 4, memory: 100 }),
                fifo_predecessor: Some(first.id.clone()), ..test_fit() };
            let result = store.request_resource_membership(&next, fit, 2).unwrap();
            assert!(matches!(result, Admission::Refused { code: RefusalCode::Impossible, .. }));
            let member = store.resource_membership_record(&next.id, &next.owner).unwrap();
            assert_eq!(member.blocked_by, None);
            assert_eq!(serde_json::to_value(member).unwrap()["refusalCode"], "impossible");
            assert_eq!(RefusalCode::Impossible.as_str(), "impossible");
        }
    }

    #[test]
    fn outside_observation_change_admits_without_any_holder_release() {
        let store = fixture();
        let mut request = request("outside");
        request.capacity = Some(Capacity { cores: 1, memory: 60 });
        let mut fit = Fit { room: Some(Capacity { cores: 4, memory: 100 }), outside_bytes: 50, ..test_fit() };
        assert!(matches!(store.request_resource_membership(&request, fit.clone(), 1).unwrap(), Admission::Queued { .. }));
        let member = store.resource_membership_record(&request.id, &request.owner).unwrap();
        assert_eq!(member.blocked_by, Some(BlockedBy::Outside));
        assert!(member.reason.contains("50 bytes"));
        assert_eq!(store.held_resource_membership_bytes("host-a", "").unwrap(), 0);
        fit.outside_bytes = 40;
        assert!(matches!(store.request_resource_membership(&request, fit, 2).unwrap(), Admission::Admitted(_)));
        let member = store.resource_membership_record(&request.id, &request.owner).unwrap();
        assert_eq!(member.blocked_by, None);
        assert_eq!(member.queued_at, 1);
        assert_eq!(member.admitted_at, Some(2));
    }

    #[test]
    fn psi_blocked_observation_then_low_psi_credit_admits_without_holder_release() {
        let store = fixture();
        let mut request = request("psi");
        request.capacity = Some(Capacity { cores: 1, memory: 60 });
        let mut fit = Fit { room: Some(Capacity { cores: 4, memory: 100 }),
            outside_bytes: 50, psi_blocked: true, ..test_fit() };
        assert!(matches!(store.request_resource_membership(&request, fit.clone(), 1).unwrap(), Admission::Queued { .. }));
        let member = store.resource_membership_record(&request.id, &request.owner).unwrap();
        assert_eq!(member.blocked_by, Some(BlockedBy::Psi));
        assert_eq!(serde_json::to_value(&member).unwrap()["blockedBy"], "psi");
        assert!(member.reason.contains("50 bytes"));
        assert!(member.reason.contains("PSI full >= 1%"));
        assert!(member.reason.contains("observation unavailable"));
        assert!(member.reason.contains("reclaim credit withheld"));
        assert_eq!(store.held_resource_membership_bytes("host-a", "").unwrap(), 0);
        // The next low-PSI observation permits reclaim credit in the host accountant.
        fit.psi_blocked = false;
        fit.outside_bytes = 40;
        assert!(matches!(store.request_resource_membership(&request, fit, 2).unwrap(), Admission::Admitted(_)));
        let member = store.resource_membership_record(&request.id, &request.owner).unwrap();
        assert_eq!(member.blocked_by, None);
        assert_eq!(member.queued_at, 1);
        assert_eq!(member.admitted_at, Some(2));
    }

    #[test]
    fn psi_attribution_requires_an_outside_capacity_block() {
        let store = fixture();
        let mut request = request("psi-fit");
        request.capacity = Some(Capacity { cores: 1, memory: 60 });
        let mut fit = Fit { room: Some(Capacity { cores: 4, memory: 100 }),
            held_bytes: 50, outside_bytes: 50, psi_blocked: true, ..test_fit() };
        assert!(matches!(store.request_resource_membership(&request, fit.clone(), 1).unwrap(), Admission::Queued { .. }));
        assert_eq!(store.resource_membership_record(&request.id, &request.owner).unwrap().blocked_by, Some(BlockedBy::Holder));
        fit.held_bytes = 0;
        fit.outside_bytes = 40;
        assert!(matches!(store.request_resource_membership(&request, fit, 2).unwrap(), Admission::Admitted(_)));
        assert_eq!(store.resource_membership_record(&request.id, &request.owner).unwrap().blocked_by, None);
    }

    #[test]
    fn footprint_attribution_uses_first_causal_limit_and_updates_while_queued() {
        let store = fixture();
        let mut request = request("attribution");
        request.capacity = Some(Capacity { cores: 1, memory: 60 });
        let mut fit = Fit { room: Some(Capacity { cores: 4, memory: 100 }),
            held_bytes: 50, outside_bytes: 50, reserve_bytes: 50, ..test_fit() };
        for (now, expected) in [(1, BlockedBy::Holder), (2, BlockedBy::Outside), (3, BlockedBy::Reserve)] {
            assert!(matches!(store.request_resource_membership(&request, fit.clone(), now).unwrap(), Admission::Queued { .. }));
            let member = store.resource_membership_record(&request.id, &request.owner).unwrap();
            assert_eq!(member.blocked_by, Some(expected));
            assert_eq!(serde_json::to_value(&member).unwrap()["blockedBy"], expected.as_str());
            if expected == BlockedBy::Holder { fit.held_bytes = 0; }
            if expected == BlockedBy::Outside { fit.outside_bytes = 0; }
        }
        fit.reserve_bytes = 40;
        assert!(matches!(store.request_resource_membership(&request, fit, 4).unwrap(), Admission::Admitted(_)));
        let member = store.resource_membership_record(&request.id, &request.owner).unwrap();
        assert_eq!(member.blocked_by, None);
        assert!(serde_json::to_value(member).unwrap().get("blockedBy").is_none());
    }

    #[test]
    fn host_and_lane_fifo_are_distinct_from_holder_occupancy() {
        let store = fixture();
        let first = request("first");
        let fit = Fit { fifo_predecessor: Some("seat/predecessor".into()), ..test_fit() };
        assert!(matches!(store.request_resource_membership(&first, fit, 1).unwrap(), Admission::Queued { .. }));
        assert_eq!(store.resource_membership_record(&first.id, &first.owner).unwrap().blocked_by, Some(BlockedBy::Fifo));
        let next = request("next");
        assert!(matches!(store.request_resource_membership(&next, test_fit(), 2).unwrap(), Admission::Queued { .. }));
        assert_eq!(store.resource_membership_record(&next.id, &next.owner).unwrap().blocked_by, Some(BlockedBy::Fifo));
        grant(&store, &first, 3);
        assert!(matches!(store.request_resource_membership(&next, test_fit(), 4).unwrap(), Admission::Queued { .. }));
        assert_eq!(store.resource_membership_record(&next.id, &next.owner).unwrap().blocked_by, Some(BlockedBy::Holder));
        store.withdraw_resource_membership(&next.id, &next.owner, 5).unwrap();
        assert_eq!(store.resource_membership_record(&next.id, &next.owner).unwrap().blocked_by, None);
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
            assert!(matches!(store.request_resource_membership(contender, test_fit(), 2).unwrap(), Admission::Queued { .. }));
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
        assert!(matches!(store.request_resource_membership(&request, test_fit(), 1).unwrap(), Admission::Queued { .. }));
        assert_eq!(store.resource_membership_record(&request.id, &request.owner).unwrap().blocked_by, Some(BlockedBy::Hold));
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
        assert_eq!(record.blocked_by, None);
    }

    #[test]
    fn queue_expiry_and_owner_generation_fences() {
        let store = fixture();
        let holder = request("holder");
        let grant = grant(&store, &holder, 1);
        let mut waiting = request("waiting");
        waiting.admission_timeout_ms = Some(0);
        store.request_resource_membership(&waiting, test_fit(), 2).unwrap();
        assert!(matches!(store.request_resource_membership(&waiting, test_fit(), 12).unwrap(),
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
        let fit = Fit { room: holder.capacity, ..test_fit() };
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
            assert!(matches!(store.request_resource_membership(&request("blocked"), test_fit(), 3).unwrap(), Admission::Queued { .. }));
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
        assert!(matches!(store.request_resource_membership(&successor, test_fit(), 4).unwrap(), Admission::Queued { .. }));
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
        assert!(store.request_resource_membership(&first, test_fit(), 2).is_err());
        first.host = "host-b".into();
        assert!(store.request_resource_membership(&first, test_fit(), 2).is_err());
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
        let policy = fractal_runtime::FiniteLane { name: holder.name.clone(), capacity: holder.capacity,
            member_capacity: holder.capacity, memory_high_bytes: None,
            class: holder.class.clone(), require_enforced: false };
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
            assert!(matches!(store.request_resource_membership(waiter, test_fit(), wall()).unwrap(), Admission::Queued { .. }));
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
        let fit = Fit { room: Some(Capacity { cores: 4, memory: 4 * gib }), ..test_fit() };
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
        assert_eq!(store.resource_membership_record(&second.id, &second.owner).unwrap().blocked_by, Some(BlockedBy::Holder));
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
            Fit { room: Some(Capacity { cores: 2, memory: 4096 }), ..test_fit() }, 100).unwrap() {
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
