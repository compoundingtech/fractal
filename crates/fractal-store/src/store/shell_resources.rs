//! Durable local commands handed to the existing host resource writer.
use super::*;
use super::resource_membership::{Grant, LaneRequest};

pub const PARENT_ENV: &str = "FRACTAL_RESOURCE_PARENT";
/// Cancellation of an as-yet unaccepted request is retained for seven days.
pub const REQUEST_TOMBSTONE_RETENTION_MS: u64 = 7 * 24 * 60 * 60 * 1000;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ParentContext {
    pub grant: Grant,
    pub request: LaneRequest,
    #[serde(default)]
    pub root_exec_id: Option<String>,
}

#[derive(Clone, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Command {
    pub id: String,
    #[serde(default, rename = "request_id", skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    pub lane: LaneRequest,
    pub argv: Vec<String>,
    pub cwd: String,
    pub environment: BTreeMap<String, String>,
    pub timeout_ms: Option<u64>,
    pub parent: Option<Grant>,
}

impl std::fmt::Debug for Command {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Command")
            .field("id", &self.id).field("lane", &self.lane)
            .field("request_id", &self.request_id)
            .field("argv", &self.argv).field("cwd", &self.cwd)
            .field("environment", &"<transient>")
            .field("timeout_ms", &self.timeout_ms).field("parent", &self.parent).finish()
    }
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
    #[serde(default)]
    pub cancellation_started_at: Option<u64>,
    pub timed_out: bool,
    pub head_start: Option<String>,
    pub clean_start: Option<bool>,
    pub result: Option<PayloadResult>,
    /// Host-derived root identity; labels supplied in the payload environment never set it.
    #[serde(default)]
    pub root_exec_id: String,
    #[serde(default)]
    pub receipt_subject: Option<String>,
    /// Digest of the initial semantic submission, before host profile resolution.
    #[serde(default)]
    pub submission_digest: Option<String>,
    #[serde(default)]
    pub refusal_code: Option<resource_membership::RefusalCode>,
    #[serde(default)]
    pub learning_decision: Option<resource_profile_learner::RecordOutcome>,
    /// Frozen host admission evidence, retained after membership retirement and in receipts.
    #[serde(default)]
    pub admission_decision: Option<resource_membership::AdmissionDecision>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestCancellation {
    pub request_id: String,
    pub cancelled: bool,
    pub tombstone: bool,
    pub execution: Option<Execution>,
}

pub(super) fn create_schema(connection: &Connection) -> Result<()> {
    // Transitional debt: command/result bodies and indexed text keys have no byte-size proof.
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-h9574bb92e055 owner=fractal_store::store::shell_resources TODO=chunk-or-reference-variable-columns */ "CREATE TABLE IF NOT EXISTS shell_resources_v1 (id TEXT PRIMARY KEY, host TEXT NOT NULL, state TEXT NOT NULL, body TEXT NOT NULL); CREATE INDEX IF NOT EXISTS shell_resources_host_v1 ON shell_resources_v1(host,state,id);")?;
    // UUID tombstones stay inline for atomic cancellation; retention does not bound cleanup work.
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-h68c595b5ab84 owner=fractal_store::store::shell_resources TODO=chunk-or-reference-variable-columns */ "CREATE TABLE IF NOT EXISTS shell_resource_request_cancellations_v1 (
        id TEXT PRIMARY KEY CHECK(length(id)=36), cancelled_at INTEGER NOT NULL);
        CREATE INDEX IF NOT EXISTS shell_resource_request_cancellation_time_v1
        ON shell_resource_request_cancellations_v1(cancelled_at);")?;
    scrub_historical_environments(connection)?;
    resource_profile_learner::create_schema(connection)?;
    Ok(())
}

/// Keep the completion marker separate from the scrub: a crash or busy checkpoint
/// before physical erasure must repeat the rewrite on the next open.
///
/// The work is bounded by this table and the free list, not the whole store. A full VACUUM
/// at open took over 51 minutes on a 57 GB member and needed the store's size again in free
/// disk, with the daemon unable to serve meanwhile:
/// - Pages freed before secure_delete was on may still hold old bytes on the free list. A
///   store with incremental auto-vacuum (every store created since, or converted) first gives
///   the whole free list back to the filesystem, so nothing below reuses those pages.
/// - The table is copied with every environment emptied, into fresh pages, and the old copy
///   dropped under secure_delete, which zeroes each page it frees: rows, overflow pages and
///   free space. Those zeroed pages go back to the filesystem too.
/// - Other stores consume their old free list with page-sized zero rows before the copy,
///   then drop that scratch table under secure_delete. No startup path runs VACUUM.
///
/// What this can't reach, and a full VACUUM can: bytes in pages that other tables reused
/// before secure_delete was on, in the unused part of those pages.
fn scrub_historical_environments(connection: &Connection) -> Result<()> {
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-hb88f17cedac9 owner=fractal_store::store::shell_resources TODO=chunk-or-reference-variable-columns */ "CREATE TABLE IF NOT EXISTS shell_environment_scrub_v1 (complete INTEGER PRIMARY KEY CHECK(complete=1));")?;
    let complete: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM shell_environment_scrub_v1)", [], |row| row.get(0))?;
    if complete { return Ok(()); }
    let previous: i64 = connection.query_row("PRAGMA secure_delete", [], |row| row.get(0))?;
    connection.pragma_update(None, "secure_delete", 1)?;
    let scrub = scrub_pending_environments(connection);
    // FAST reads back as 2, but numeric assignments are boolean: 2 would enable ON.
    // Restore the keyword before propagating any scrub error.
    let restored = if previous == 2 {
        connection.pragma_update(None, "secure_delete", "FAST")
    } else {
        connection.pragma_update(None, "secure_delete", previous)
    };
    scrub?;
    restored?;
    Ok(())
}

fn scrub_pending_environments(connection: &Connection) -> Result<()> {
    // 2 is INCREMENTAL.
    let incremental = connection.query_row("PRAGMA auto_vacuum", [], |row| row.get::<_, i64>(0))? == 2;
    if incremental {
        give_back_free_list(connection)?;
    } else {
        erase_free_list(connection)?;
    }
    // Historical JSON bodies remain unbounded; scrubbing preserves their other fields.
    // One transaction, rolled back on any error, so a failed rebuild leaves the table as it was.
    let rebuild = rusqlite::Transaction::new_unchecked(connection, rusqlite::TransactionBehavior::Immediate)?;
    rebuild.execute_batch(/* sqlite-large-value: allow-for-now lv-h3fe95fd4aec7 owner=fractal_store::store::shell_resources TODO=chunk-or-reference-variable-columns */ "DROP TABLE IF EXISTS shell_resources_scrub_v1;
        CREATE TABLE shell_resources_scrub_v1 (id TEXT PRIMARY KEY, host TEXT NOT NULL, state TEXT NOT NULL, body TEXT NOT NULL);
        INSERT INTO shell_resources_scrub_v1(id, host, state, body)
            SELECT id, host, state, json_set(body,'$.command.environment',json('{}')) FROM shell_resources_v1;
        DROP TABLE shell_resources_v1;
        ALTER TABLE shell_resources_scrub_v1 RENAME TO shell_resources_v1;
        CREATE INDEX shell_resources_host_v1 ON shell_resources_v1(host,state,id);")?;
    rebuild.commit()?;
    checkpoint_shell_scrub(connection)?;
    if incremental {
        give_back_free_list(connection)?;
    }
    checkpoint_shell_scrub(connection)?;
    connection.execute(/* sqlite-large-value: allow-for-now lv-haceef5fded64 owner=fractal_store::store::shell_resources TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO shell_environment_scrub_v1(complete) VALUES(1)", [])?;
    Ok(())
}

/// Every free page back to the filesystem. Each row the pragma steps gives one back, so it
/// runs to its end.
fn give_back_free_list(connection: &Connection) -> Result<()> {
    let mut statement = connection.prepare("PRAGMA incremental_vacuum")?;
    let mut steps = statement.query([])?;
    while steps.next()?.is_some() {}
    Ok(())
}

/// Overwrite pages freed before secure_delete was enabled, before the table copy can
/// reuse them. One near-page-sized row needs at least one leaf page; allocating the
/// initial free-page count therefore consumes the entire old free list. B-tree
/// bookkeeping can grow the file slightly, but never copies the rest of the store.
/// A crash leaves only zero rows; the next attempt drops them securely and repeats.
fn erase_free_list(connection: &Connection) -> Result<()> {
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-h4929ecffa855 owner=fractal_store::store::shell_resources TODO=chunk-or-reference-variable-columns */ "DROP TABLE IF EXISTS shell_environment_free_scrub_v1;
        CREATE TABLE shell_environment_free_scrub_v1 (body BLOB NOT NULL);")?;
    let free: u64 = connection.query_row("PRAGMA freelist_count", [], |row| row.get(0))?;
    let page_size: u64 = connection.query_row("PRAGMA page_size", [], |row| row.get(0))?;
    let mut remaining = free;
    while remaining > 0 {
        let batch = remaining.min(128);
        let transaction = rusqlite::Transaction::new_unchecked(connection, rusqlite::TransactionBehavior::Immediate)?;
        {
            let mut insert = transaction.prepare(/* sqlite-large-value: allow-for-now lv-hac7744f65d12 owner=fractal_store::store::shell_resources TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO shell_environment_free_scrub_v1(body) VALUES(zeroblob(?1))")?;
            for _ in 0..batch {
                insert.execute(/* sqlite-large-value: allow-for-now lv-ha3413afc70ae owner=fractal_store::store::shell_resources TODO=trace-statement-and-value-representation */ [page_size - 100])?;
            }
        }
        transaction.commit()?;
        checkpoint_shell_scrub(connection)?;
        remaining -= batch;
    }
    let left: u64 = connection.query_row("PRAGMA freelist_count", [], |row| row.get(0))?;
    anyhow::ensure!(left == 0, "shell environment scrub did not overwrite the free list");
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-h7eba39ebc70f owner=fractal_store::store::shell_resources TODO=bound-or-reference-values-and-reclamation */ "DROP TABLE shell_environment_free_scrub_v1;")?;
    checkpoint_shell_scrub(connection)?;
    Ok(())
}

/// How often the scrub tries its checkpoint before it refuses to open the store, and how long
/// each try waits for readers: about 30 s in all, so a reader that a replication worker or a
/// CLI holds at startup ends first, rather than the daemon restarting over and over.
const CHECKPOINT_TRIES: u32 = 5;
const CHECKPOINT_WAIT: std::time::Duration =
    if cfg!(test) { std::time::Duration::from_millis(20) } else { std::time::Duration::from_secs(6) };

fn checkpoint_shell_scrub(connection: &Connection) -> Result<()> {
    // TRUNCATE waits for readers as long as the busy timeout; give each try its own wait.
    let previous: u64 = connection.query_row("PRAGMA busy_timeout", [], |row| row.get(0))?;
    connection.busy_timeout(CHECKPOINT_WAIT)?;
    let mut busy = 1;
    for _ in 0..CHECKPOINT_TRIES {
        busy = connection.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| row.get::<_, i64>(0))?;
        if busy == 0 {
            break;
        }
    }
    connection.busy_timeout(std::time::Duration::from_millis(previous))?;
    anyhow::ensure!(busy == 0, "shell environment scrub checkpoint busy; refusing to open store");
    Ok(())
}

pub fn submission_digest(command: &Command) -> Result<String> {
    use sha2::Digest as _;
    // Ambient environment values are transport-only, never replay identity or audit data.
    let body = if let Some(request_id) = &command.request_id {
        serde_json::to_vec(&("shell-resource-request-submission-v1", &command.id, &command.lane,
            &command.argv, &command.cwd, command.timeout_ms, &command.parent, request_id))?
    } else {
        serde_json::to_vec(&("shell-resource-submission-v1", &command.id, &command.lane,
            &command.argv, &command.cwd, command.timeout_ms, &command.parent))?
    };
    Ok(format!("{:x}", sha2::Sha256::digest(body)))
}
/// Only the authenticated API constructs this scope; environment labels are never inputs.
#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum RequestScope<'a> {
    Local { person: Option<&'a str>, host: &'a str },
    Run { run: &'a str, step: &'a str, attempt: u32 },
    Agent { owner: &'a resource_membership::Owner },
}

impl RequestScope<'_> {
    fn validate_execution(&self, execution: &Execution, request_id: uuid::Uuid) -> Result<()> {
        let owner = &execution.command.lane.owner;
        let owns = match self {
            Self::Local { person, host } => execution.command.lane.host == *host && match person {
                Some(person) => owner.run == *person,
                None => owner.run.strip_prefix("local:") == Some(*host),
            },
            Self::Run { run, step, attempt } => owner.run == *run && owner.step == *step && owner.attempt == *attempt,
            Self::Agent { owner: expected } => owner == *expected,
        };
        if !owns || execution.command.request_id.as_deref().and_then(|id| uuid::Uuid::parse_str(id).ok()) != Some(request_id) {
            return Err(FractalError::new("resource-request-conflict",
                "resource request identity is already bound to different execution intent").into());
        }
        Ok(())
    }
}

fn expire_request_cancellations(connection: &Connection, now: u64) -> Result<()> {
    if let Some(cutoff) = now.checked_sub(REQUEST_TOMBSTONE_RETENTION_MS) {
        // Seven-day retention does not bound the number of expired tombstones deleted per submission.
        connection.execute(/* sqlite-large-value: allow-for-now lv-h3ecd1f97c431 owner=fractal_store::store::shell_resources TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM shell_resource_request_cancellations_v1 WHERE cancelled_at<=?1", [cutoff])?;
    }
    Ok(())
}

fn cancel_command(connection: &Connection, id: &str) -> Result<()> {
    let class: Option<String> = connection.query_row(
        "SELECT json_extract(body,'$.command.lane.class') FROM shell_resources_v1 WHERE id=?1",
        [id], |row| row.get::<_, Option<String>>(0)).optional()?.flatten();
    if let Some(class) = class {
        let class: resource_membership::RequestClass = serde::Deserialize::deserialize(
            serde::de::value::StrDeserializer::<serde::de::value::Error>::new(&class))?;
        class.ensure_supported()?;
    }
    // A scalar cancel flag still rewrites one potentially unbounded JSON body.
    connection.execute(/* sqlite-large-value: allow-for-now lv-hf463932112ca owner=fractal_store::store::shell_resources TODO=bound-or-reference-values-and-reclamation */ "UPDATE shell_resources_v1 SET body=json_set(body,'$.cancel',json('true')) WHERE id=?1", [id])?;
    Ok(())
}

pub fn request_execution_id(scope: &RequestScope<'_>, request_id: uuid::Uuid) -> Result<String> {
    use sha2::Digest as _;
    let body = serde_json::to_vec(&("shell-resource-owner-request-v1", scope, request_id.as_bytes()))?;
    let digest = sha2::Sha256::digest(body);
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    // UUIDv8 custom payload, with RFC variant bits; the domain owns hash semantics.
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Ok(uuid::Uuid::from_bytes(bytes).to_string())
}



impl Store {
    pub fn submit_shell_resource(&self, mut command: Command, now: u64) -> Result<Execution> {
        command.lane.class.ensure_supported()?;
        anyhow::ensure!(!command.id.is_empty() && !command.argv.is_empty(), "shell execution requires identity and argv");
        anyhow::ensure!(command.cwd.starts_with('/'), "shell execution cwd must be absolute");
        anyhow::ensure!(command.argv.iter().all(|argument| !argument.contains('\0')), "resource argv contains NUL");
        let submission_digest = submission_digest(&command)?;
        let root_exec_id = if let Some(parent) = &command.parent {
            let member = self.resource_membership_for_grant(parent)?;
            anyhow::ensure!(member.request == command.lane, "nested command must retain the verified parent request");
            member.request.id.strip_prefix("shell:").context("nested shell parent is not a shell execution")?.to_owned()
        } else { command.id.clone() };
        let environment = std::mem::take(&mut command.environment);
        // Serialize submission with launch lookup; absence after restart refuses launch.
        let mut environments = self.shell_environments.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let execution = Execution { command, revision: 0, queue_wait_ms: 0, state: "waiting".into(), reason: "waiting for host admission".into(), blocked_by: None,
            queued_at: now, started_at: None, ended_at: None, exit_code: None, exit_signal: None,
            stdout: String::new(), stderr: String::new(), cancel: false, cancellation_started_at: None, timed_out: false, head_start: None, clean_start: None, result: None,
            root_exec_id, receipt_subject: None, submission_digest: Some(submission_digest.clone()), refusal_code: None,
            learning_decision: None, admission_decision: None };
        let mut connection = self.connection.write();
        let transaction = connection.transaction()?;
        expire_request_cancellations(&transaction, now)?;
        let cancelled: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM shell_resource_request_cancellations_v1 WHERE id=?1)",
            [&execution.command.id], |row| row.get(0))?;
        if cancelled {
            transaction.commit()?;
            return Err(FractalError::new("resource-request-cancelled",
                "resource request was cancelled before acceptance").into());
        }
        // One row per command does not bound its serialized argv/result bytes.
        let inserted = transaction.execute(/* sqlite-large-value: allow-for-now lv-h319ba7a29fc7 owner=fractal_store::store::shell_resources TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO shell_resources_v1(id,host,state,body) VALUES(?1,?2,?3,?4) ON CONFLICT(id) DO NOTHING",
            rusqlite::params![execution.command.id,execution.command.lane.host,execution.state,serde_json::to_string(&execution)?])?;
        if inserted == 0 {
            let body: String = transaction.query_row("SELECT body FROM shell_resources_v1 WHERE id=?1", [&execution.command.id], |row| row.get(0))?;
            let existing: Execution = serde_json::from_str(&body)?;
            if existing.submission_digest.as_deref() != Some(submission_digest.as_str()) {
                return Err(FractalError::new("resource-request-conflict",
                    "resource request identity is already bound to different execution intent").into());
            }
            transaction.commit()?;
            return Ok(existing);
        }
        transaction.commit()?;
        drop(connection);
        environments.insert(execution.command.id.clone(), environment);
        drop(environments);
        self.resource_admission_elapsed(&format!("shell-wait:{}", execution.command.id), now, now);
        Ok(execution)
    }

    pub fn take_shell_environment(&self, id: &str) -> Option<BTreeMap<String, String>> {
        self.shell_environments.lock().unwrap_or_else(std::sync::PoisonError::into_inner).remove(id)
    }

    pub fn shell_resource(&self, id: &str) -> Result<Execution> {
        let body: String = self.readers.get().query_row("SELECT body FROM shell_resources_v1 WHERE id=?1", [id], |row| row.get(0))?;
        Ok(serde_json::from_str(&body)?)
    }

    pub fn shell_resource_if_exists(&self, id: &str) -> Result<Option<Execution>> {
        let body: Option<String> = self.readers.get().query_row(
            "SELECT body FROM shell_resources_v1 WHERE id=?1", [id], |row| row.get(0)).optional()?;
        body.map(|body| serde_json::from_str(&body).map_err(anyhow::Error::from)).transpose()
    }

    pub fn shell_resource_by_request(&self, scope: &RequestScope<'_>, request_id: uuid::Uuid) -> Result<Option<Execution>> {
        let id = request_execution_id(scope, request_id)?;
        let body: Option<String> = self.readers.get().query_row(
            "SELECT body FROM shell_resources_v1 WHERE id=?1", [&id], |row| row.get(0)).optional()?;
        let execution: Option<Execution> = body.map(|body| serde_json::from_str(&body).map_err(anyhow::Error::from)).transpose()?;
        if let Some(execution) = &execution { scope.validate_execution(execution, request_id)?; }
        Ok(execution)
    }

    pub fn cancel_shell_resource_by_request(&self, scope: &RequestScope<'_>, request_id: uuid::Uuid, now: u64) -> Result<Option<Execution>> {
        let id = request_execution_id(scope, request_id)?;
        let mut connection = self.connection.write();
        let transaction = connection.transaction()?;
        expire_request_cancellations(&transaction, now)?;
        let body: Option<String> = transaction.query_row(
            "SELECT body FROM shell_resources_v1 WHERE id=?1", [&id], |row| row.get(0)).optional()?;
        let mut execution: Option<Execution> = body.map(|body| serde_json::from_str(&body).map_err(anyhow::Error::from)).transpose()?;
        if let Some(execution) = &mut execution {
            scope.validate_execution(execution, request_id)?;
            cancel_command(&transaction, &id)?;
            execution.cancel = true;
        } else {
            // Keep cancellation atomic with request lookup; UUID keys do not prove total storage cost.
            transaction.execute(/* sqlite-large-value: allow-for-now lv-hc433437e984c owner=fractal_store::store::shell_resources TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO shell_resource_request_cancellations_v1(id,cancelled_at)
                VALUES(?1,?2) ON CONFLICT(id) DO NOTHING", rusqlite::params![id, now])?;
        }
        transaction.commit()?;
        Ok(execution)
    }

    pub fn shell_resources(&self, host: &str, active_only: bool) -> Result<Vec<Execution>> {
        let connection = self.readers.get();
        let mut statement = connection.prepare("SELECT body FROM shell_resources_v1 WHERE host=?1 AND (?2=0 OR state IN ('waiting','admitted','running','retiring','finishing','failing','finishing-refused')) ORDER BY id")?;
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
        previous.command.lane.class.ensure_supported()?;
        // Terminal commands and their durable observations are immutable. The claim
        // is appended in this same transaction, so recovery cannot lose or duplicate it.
        if matches!(previous.state.as_str(), "completed" | "refused" | "cancelled") {
            return Ok(false);
        }
        let mut next = execution.clone();
        next.command.environment.clear();
        next.cancel |= previous.cancel;
        next.revision = previous.revision;
        // Admission evidence comes from the host membership, never mutable execution input.
        // Once copied, it remains frozen even after retirement or later host observations.
        next.admission_decision.clone_from(&previous.admission_decision);
        if next.admission_decision.is_none() {
            let decision: Option<String> = transaction.query_row(
                "SELECT json_extract(body,'$.admissionDecision') FROM resource_memberships_v1 WHERE id=?1 AND host=?2",
                rusqlite::params![next.command.lane.id, next.command.lane.host], |row| row.get(0),
            ).optional()?.flatten();
            next.admission_decision = decision.map(|body| serde_json::from_str(&body)).transpose()?;
        }
        if next.root_exec_id.is_empty() { next.root_exec_id.clone_from(&next.command.id); }
        if matches!(next.state.as_str(), "completed" | "refused" | "cancelled") && next.command.parent.is_none() {
            next.receipt_subject = Some(resource_exec_receipts::subject(&self.origin, &next));
            next.learning_decision = resource_profile_learner::evidence::record_execution(&transaction, &next)?;
        }
        if next == previous { return Ok(false); }
        next.revision += 1;
        // Primary-key selection bounds rows, not replacement body bytes or retained rows.
        transaction.execute(/* sqlite-large-value: allow-for-now lv-h0b67351b90df owner=fractal_store::store::shell_resources TODO=bound-or-reference-values-and-reclamation */ "UPDATE shell_resources_v1 SET state=?2,body=?3 WHERE id=?1",
            rusqlite::params![next.command.id,next.state,serde_json::to_string(&next)?])?;
        if matches!(next.state.as_str(), "completed" | "refused" | "cancelled")
            && next.command.parent.is_none()
        {
            resource_exec_receipts::append_tx(&transaction, &self.origin, &next)?;
        }
        transaction.commit()?;
        drop(connection);
        if matches!(next.state.as_str(), "completed" | "refused" | "cancelled") {
            self.take_shell_environment(&next.command.id);
            self.finish_resource_admission_clock(&format!("shell-wait:{}", next.command.id));
        }
        Ok(true)
    }

    pub fn cancel_shell_resource(&self, id: &str) -> Result<()> {
        let mut connection = self.connection.write();
        let transaction = connection.transaction()?;
        cancel_command(&transaction, id)?;
        transaction.commit()?;
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

#[derive(Clone, Debug, Default, Deserialize, Serialize, Eq, PartialEq)]
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
    #[serde(default, flatten)]
    pub evidence: resource_profile_learner::evidence::Evidence,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::resource_membership::{Coverage, Owner, RequestClass};

    fn command(id: &str) -> Command {
        Command { id: id.into(), request_id: None, lane: LaneRequest {
            id: format!("shell:{id}"), name: "checks".into(), host: "host-a".into(), max_members: 1,
            capacity: Some(fractal_runtime::Capacity { cores: 1, memory: 1 << 30 }),
            class: RequestClass::Check, disk: None, io: None, coverage: Coverage::Logical,
            admission_timeout_ms: None,
            profile: None, profile_estimate: None, disk_path: None,
            owner: Owner { run: format!("shell:{id}"), step: format!("shell:{id}"), attempt: 1, invocation: Some(format!("shell:{id}")) },
        }, argv: vec!["true".into()], cwd: "/work".into(), environment: BTreeMap::new(), timeout_ms: None, parent: None }
    }

    #[test]
    fn future_resource_class_rows_remain_readable_but_cannot_be_submitted() {
        let store = Store::open_memory("host-a").unwrap();
        let mut future = store.submit_shell_resource(command("future"), 1).unwrap();
        future.command.lane.class = RequestClass::Unknown("future-class".into());
        future.refusal_code = Some(resource_membership::RefusalCode::RecoveryNotAllowed);
        store.update_shell_resource(&future).unwrap();
        store.submit_shell_resource(command("current"), 2).unwrap();
        let rows = store.shell_resources("host-a", false).unwrap();
        assert_eq!(rows.len(), 2);
        let future = rows.iter().find(|row| row.command.id == "future").unwrap();
        assert_eq!(future.command.lane.class, RequestClass::Unknown("future-class".into()));
        assert_eq!(serde_json::to_value(future).unwrap()["command"]["lane"]["class"], "future-class");
        assert_eq!(store.shell_resource("future").unwrap().command.lane.class, future.command.lane.class);
        assert_eq!(future.refusal_code, Some(resource_membership::RefusalCode::RecoveryNotAllowed));
        assert!(store.update_shell_resource(future).unwrap_err().to_string().contains("unsupported resource class future-class"));
        let mut request = command("unsupported");
        request.lane.class = future.command.lane.class.clone();
        assert!(store.submit_shell_resource(request, 3).unwrap_err().to_string().contains("unsupported resource class future-class"));
        assert!(store.shell_resource_if_exists("unsupported").unwrap().is_none());
    }

    #[test]
    fn future_resource_class_cancellation_is_refused_for_execution_and_request() {
        let store = Store::open_memory("host-a").unwrap();
        let key = uuid::Uuid::now_v7();
        let scope = RequestScope::Local { person: Some("person/operator"), host: "host-a" };
        let id = request_execution_id(&scope, key).unwrap();
        let mut request = command(&id);
        request.lane.owner.run = "person/operator".into();
        request.request_id = Some(key.to_string());
        let mut execution = store.submit_shell_resource(request, 1).unwrap();
        execution.command.lane.class = RequestClass::Unknown("future-class".into());
        store.update_shell_resource(&execution).unwrap();
        let before = store.shell_resource(&id).unwrap();
        assert!(store.cancel_shell_resource(&id).unwrap_err().to_string().contains("unsupported resource class future-class"));
        assert!(!store.shell_resource(&id).unwrap().cancel);
        assert!(store.cancel_shell_resource_by_request(&scope, key, 2).unwrap_err().to_string().contains("unsupported resource class future-class"));
        assert!(!store.shell_resource(&id).unwrap().cancel);
        let after = store.shell_resource(&id).unwrap();
        assert_eq!(after, before);
        let mut supported = store.submit_shell_resource(command("supported"), 3).unwrap();
        store.cancel_shell_resource("supported").unwrap();
        supported.cancel = true;
        assert_eq!(store.shell_resource("supported").unwrap(), supported);
    }

    const ENV_SENTINEL: &str = "shell-environment-security-regression-sentinel";

    fn assert_no_environment_bytes(path: &std::path::Path) {
        for file in [path.to_path_buf(), std::path::PathBuf::from(format!("{}-wal", path.display()))] {
            match std::fs::read(file) {
                Ok(bytes) => assert!(!bytes.windows(ENV_SENTINEL.len()).any(|value| value == ENV_SENTINEL.as_bytes())),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
                Err(error) => panic!("{error}"),
            }
        }
    }

    #[test]
    fn live_environment_reaches_process_but_never_execution_disk_or_receipt() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("store.db");
        let store = Store::open(&path, "host-a").unwrap();
        let mut request = command("secret");
        request.environment = std::env::vars().collect();
        request.environment.insert("SECRET".into(), ENV_SENTINEL.into());
        request.environment.insert("ST3_START_OPERATION".into(), ENV_SENTINEL.into());
        assert!(!format!("{request:?}").contains(ENV_SENTINEL));
        let mut execution = store.submit_shell_resource(request.clone(), 10).unwrap();
        assert!(execution.command.environment.is_empty());
        assert!(store.shell_resource("secret").unwrap().command.environment.is_empty());
        assert!(!serde_json::to_string(&store.shell_resources("host-a", false).unwrap()).unwrap().contains(ENV_SENTINEL));
        request.environment.insert("SECRET".into(), "replacement".into());
        assert_eq!(store.submit_shell_resource(request, 11).unwrap().queued_at, 10);
        let environment = store.take_shell_environment("secret").unwrap();
        let runtime = fractal_runtime::ExecRuntime::new(directory.path().join("exec"), directory.path().join("logs"));
        let generation = runtime.spawn_in_lane_checked("secret",
            &fractal_runtime::Launch::Argv(vec!["/bin/sh".into(), "-c".into(),
                "test \"$SECRET\" = shell-environment-security-regression-sentinel && test \"$ST3_START_OPERATION\" = \"$SECRET\"".into()]),
            directory.path(), &environment,
            &fractal_runtime::FiniteLane { name: "checks".into(), capacity: None, member_capacity: None, memory_high_bytes: None,
                class: RequestClass::Check, require_enforced: false },
            &|| Ok(())).unwrap();
        assert!(generation.start_operation.is_none());
        let exited = runtime.wait_for_exit("secret", &generation.generation_id).unwrap();
        assert_eq!(exited.exit_code, Some(0));
        for entry in std::fs::read_dir(directory.path().join("exec")).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_file() {
                let bytes = std::fs::read(entry.path()).unwrap();
                assert!(!bytes.windows(ENV_SENTINEL.len()).any(|value| value == ENV_SENTINEL.as_bytes()));
            }
        }
        assert!(store.take_shell_environment("secret").is_none());
        execution.state = "completed".into();
        execution.exit_code = Some(0);
        execution.command.environment.insert("SECRET".into(), ENV_SENTINEL.into());
        store.update_shell_resource(&execution).unwrap();
        let page = store.claims_page_matching(crate::store::ClaimsFilter {
            kind: Some("resource.exec-receipt"), ..Default::default()
        }, None, 0, None, false, 100).unwrap();
        assert_eq!(page.claims.len(), 1);
        assert!(!serde_json::to_string(&page.claims).unwrap().contains(ENV_SENTINEL));
        assert!(store.shell_resource("secret").unwrap().command.environment.is_empty());
        assert_no_environment_bytes(&path);
    }

    #[test]
    fn historical_scrub_erases_database_and_wal_without_erasing_execution_facts() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("store.db");
        {
            let store = Store::open(&path, "host-a").unwrap();
            let mut execution = store.submit_shell_resource(command("history"), 10).unwrap();
            execution.state = "completed".into();
            execution.started_at = Some(12);
            execution.ended_at = Some(20);
            execution.exit_code = Some(37);
            execution.result = Some(PayloadResult {
                head_start: None, head_end: None, clean_start: None, clean_end: None,
                exit_code: Some(37), exit_signal: None, wall_duration_ms: 8, memory_peak_bytes: None,
                evidence: Default::default(),
            });
            store.update_shell_resource(&execution).unwrap();
        }
        let legacy = Connection::open(&path).unwrap();
        legacy.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; PRAGMA secure_delete=OFF;
            DELETE FROM shell_environment_scrub_v1;").unwrap();
        legacy.execute("UPDATE shell_resources_v1 SET body=json_set(body,'$.command.environment.SECRET',?1)",
            [ENV_SENTINEL.repeat(256)]).unwrap();
        legacy.execute_batch("PRAGMA wal_checkpoint(FULL)").unwrap();
        let store = Store::open(&path, "host-a").unwrap();
        let execution = store.shell_resource("history").unwrap();
        assert!(execution.command.environment.is_empty());
        assert_eq!(execution.state, "completed");
        assert_eq!(execution.started_at, Some(12));
        assert_eq!(execution.ended_at, Some(20));
        assert_eq!(execution.exit_code, Some(37));
        assert!(store.take_shell_environment("history").is_none());
        assert_no_environment_bytes(&path);
        let expected: i64 = Connection::open_in_memory().unwrap()
            .query_row("PRAGMA secure_delete", [], |row| row.get(0)).unwrap();
        let secure: i64 = store.connection.write().query_row("PRAGMA secure_delete", [], |row| row.get(0)).unwrap();
        assert_eq!(secure, expected);
    }

    #[test]
    fn historical_scrub_restores_all_secure_delete_modes_and_preserves_completed_skip() {
        for (setting, mode) in [("OFF", 0), ("ON", 1), ("FAST", 2)] {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("store.db");
            let connection = Connection::open(&path).unwrap();
            connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;").unwrap();
            create_schema(&connection).unwrap();
            connection.execute("DELETE FROM shell_environment_scrub_v1", []).unwrap();
            connection.execute("INSERT INTO shell_resources_v1 VALUES('old','host-a','waiting',?1)",
                [json!({"command":{"environment":{"SECRET":ENV_SENTINEL}}}).to_string()]).unwrap();
            connection.pragma_update(None, "secure_delete", setting).unwrap();
            assert_eq!(connection.query_row("PRAGMA secure_delete", [], |row| row.get::<_, i64>(0)).unwrap(), mode);
            scrub_historical_environments(&connection).unwrap();
            assert_eq!(connection.query_row("PRAGMA secure_delete", [], |row| row.get::<_, i64>(0)).unwrap(), mode);
            assert_no_environment_bytes(&path);
            // A completed marker must bypass all scrub work and preserve each setting.
            connection.execute_batch("CREATE TRIGGER no_repeat BEFORE UPDATE ON shell_resources_v1
                BEGIN SELECT RAISE(ABORT,'unexpected repeated scrub'); END;").unwrap();
            scrub_historical_environments(&connection).unwrap();
            assert_eq!(connection.query_row("PRAGMA secure_delete", [], |row| row.get::<_, i64>(0)).unwrap(), mode);
        }
    }

    /// A store whose free list holds a secret from before secure_delete, as an old member's
    /// does: `auto_vacuum` is the store's mode, and the scrub must leave no byte of it.
    fn scrub_erases_a_freed_secret(auto_vacuum: &str) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("store.db");
        {
            let store = Store::open(&path, "host-a").unwrap();
            store.submit_shell_resource(command("history"), 10).unwrap();
        }
        let legacy = Connection::open(&path).unwrap();
        legacy.execute_batch(&format!("PRAGMA auto_vacuum={auto_vacuum}; VACUUM; PRAGMA journal_mode=WAL;
            PRAGMA wal_autocheckpoint=0; PRAGMA secure_delete=OFF; DELETE FROM shell_environment_scrub_v1;")).unwrap();
        legacy.execute("UPDATE shell_resources_v1 SET body=json_set(body,'$.command.environment.SECRET',?1)",
            [ENV_SENTINEL.repeat(256)]).unwrap();
        // Rewriting the row again frees the pages that held the secret, without zeroing them.
        legacy.execute("UPDATE shell_resources_v1 SET body=json_set(body,'$.command.environment',json('{}'))", []).unwrap();
        legacy.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)").unwrap();
        let pages: u64 = legacy.query_row("PRAGMA page_count", [], |row| row.get(0)).unwrap();
        let free_before: u64 = legacy.query_row("PRAGMA freelist_count", [], |row| row.get(0)).unwrap();
        assert!(free_before > 0);
        // SQLite implements VACUUM using ATTACH; deny it for the entire scrub.
        legacy.authorizer(Some(|context: rusqlite::hooks::AuthContext<'_>| {
            if matches!(context.action, rusqlite::hooks::AuthAction::Attach { .. }) {
                rusqlite::hooks::Authorization::Deny
            } else {
                rusqlite::hooks::Authorization::Allow
            }
        }));
        scrub_historical_environments(&legacy).unwrap();
        assert_no_environment_bytes(&path);
        let pages_after: u64 = legacy.query_row("PRAGMA page_count", [], |row| row.get(0)).unwrap();
        assert!(pages_after <= pages + free_before + 16, "scrub grew beyond its free-list budget");
        if auto_vacuum == "INCREMENTAL" {
            let free: i64 = legacy.query_row("PRAGMA freelist_count", [], |row| row.get(0)).unwrap();
            assert_eq!(free, 0, "the free list that held the secret is gone");
        }
    }

    #[test]
    fn historical_scrub_gives_back_a_free_list_that_held_a_secret() {
        scrub_erases_a_freed_secret("INCREMENTAL");
    }

    #[test]
    fn historical_scrub_without_incremental_vacuum_erases_without_vacuum() {
        scrub_erases_a_freed_secret("NONE");
    }

    #[test]
    fn a_reader_that_ends_while_the_scrub_retries_lets_it_finish() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("store.db");
        let writer = Connection::open(&path).unwrap();
        writer.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;").unwrap();
        create_schema(&writer).unwrap();
        writer.execute("DELETE FROM shell_environment_scrub_v1", []).unwrap();
        let reader = Connection::open(&path).unwrap();
        reader.execute_batch("BEGIN; SELECT * FROM shell_resources_v1;").unwrap();
        // The reader ends after the first try, before the scrub runs out of them.
        let ending = std::thread::spawn(move || {
            std::thread::sleep(CHECKPOINT_WAIT / 2);
            reader.execute_batch("ROLLBACK").unwrap();
        });
        scrub_historical_environments(&writer).unwrap();
        ending.join().unwrap();
        let complete: bool = writer.query_row("SELECT EXISTS(SELECT 1 FROM shell_environment_scrub_v1)", [], |row| row.get(0)).unwrap();
        assert!(complete);
    }

    #[test]
    fn busy_historical_checkpoint_fails_closed_and_repeats_scrub() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("store.db");
        let writer = Connection::open(&path).unwrap();
        writer.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;").unwrap();
        create_schema(&writer).unwrap();
        writer.execute("DELETE FROM shell_environment_scrub_v1", []).unwrap();
        let reader = Connection::open(&path).unwrap();
        reader.execute_batch("BEGIN; SELECT * FROM shell_resources_v1;").unwrap();
        writer.execute("INSERT INTO shell_resources_v1 VALUES('old','host-a','waiting',?1)",
            [json!({"command":{"environment":{"SECRET":ENV_SENTINEL}}}).to_string()]).unwrap();
        writer.pragma_update(None, "secure_delete", "FAST").unwrap();
        assert_eq!(writer.query_row("PRAGMA secure_delete", [], |row| row.get::<_, i64>(0)).unwrap(), 2);
        assert!(scrub_historical_environments(&writer).is_err());
        assert_eq!(writer.query_row("PRAGMA secure_delete", [], |row| row.get::<_, i64>(0)).unwrap(), 2);
        let complete: bool = writer.query_row("SELECT EXISTS(SELECT 1 FROM shell_environment_scrub_v1)", [], |row| row.get(0)).unwrap();
        assert!(!complete);
        reader.execute_batch("ROLLBACK").unwrap();
        scrub_historical_environments(&writer).unwrap();
        assert_eq!(writer.query_row("PRAGMA secure_delete", [], |row| row.get::<_, i64>(0)).unwrap(), 2);
        assert_no_environment_bytes(&path);
    }

    #[test]
    fn restart_loses_even_empty_launch_context_and_terminal_updates_release_values() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("store.db");
        {
            let store = Store::open(&path, "host-a").unwrap();
            store.submit_shell_resource(command("pending"), 10).unwrap();
            let mut terminal = store.submit_shell_resource(command("cancelled"), 10).unwrap();
            terminal.state = "cancelled".into();
            store.update_shell_resource(&terminal).unwrap();
            assert!(store.take_shell_environment("cancelled").is_none());
        }
        let store = Store::open(&path, "host-a").unwrap();
        assert_eq!(store.shell_resource("pending").unwrap().state, "waiting");
        assert!(store.take_shell_environment("pending").is_none());
    }

    #[test]
    fn stable_submission_id_never_relaunches_or_replaces_semantic_intent() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(&directory.path().join("store.db"), "host-a").unwrap();
        let first = command("stable");
        let accepted = store.submit_shell_resource(first.clone(), 10).unwrap();
        let mut retry = first.clone();
        retry.environment.insert("VOLATILE_SECRET".into(), "not-identity".into());
        let repeated = store.submit_shell_resource(retry, 20).unwrap();
        assert_eq!(repeated.queued_at, accepted.queued_at);
        assert_eq!(repeated.revision, accepted.revision);
        let mut conflicting = first;
        conflicting.argv = vec!["false".into()];
        assert!(store.submit_shell_resource(conflicting, 30).is_err());
    }

    #[test]
    fn owner_scoped_request_identity_is_stable_and_separates_native_owners() {
        let key = uuid::Uuid::parse_str("0192c8d8-97e7-7000-8000-000000000001").unwrap();
        let local = RequestScope::Local { person: Some("person/operator"), host: "host-a" };
        let id = request_execution_id(&local, key).unwrap();
        assert_eq!(request_execution_id(&local, key).unwrap(), id);
        assert!(uuid::Uuid::parse_str(&id).is_ok());
        let owner = resource_membership::Owner { run: "invocation/one".into(), step: "agent/host-a/worker".into(),
            attempt: 1, invocation: Some("invocation/one".into()) };
        let mut next_owner = owner.clone();
        next_owner.invocation = Some("invocation/two".into());
        let scopes = [
            RequestScope::Local { person: Some("person/other"), host: "host-a" },
            RequestScope::Local { person: Some("person/operator"), host: "host-b" },
            RequestScope::Run { run: "run", step: "step", attempt: 1 },
            RequestScope::Run { run: "run", step: "step", attempt: 2 },
            RequestScope::Run { run: "run", step: "other", attempt: 1 },
            RequestScope::Agent { owner: &owner },
            RequestScope::Agent { owner: &next_owner },
        ];
        let mut ids = std::collections::BTreeSet::from([id]);
        for scope in scopes { assert!(ids.insert(request_execution_id(&scope, key).unwrap())); }
    }

    #[test]
    fn request_retry_after_restart_preserves_acceptance_and_never_restores_lost_environment() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("store.db");
        let key = uuid::Uuid::parse_str("0192c8d8-97e7-7000-8000-000000000001").unwrap();
        let scope = RequestScope::Local { person: Some("person/operator"), host: "host-a" };
        let id = request_execution_id(&scope, key).unwrap();
        let mut request = command(&id);
        request.lane.owner.run = "person/operator".into();
        request.request_id = Some(key.to_string());
        request.environment.insert("SECRET".into(), ENV_SENTINEL.into());
        {
            let store = Store::open(&path, "host-a").unwrap();
            assert!(store.shell_resource_by_request(&scope, key).unwrap().is_none());
            let accepted = store.submit_shell_resource(request.clone(), 10).unwrap();
            let body = serde_json::to_value(&accepted.command).unwrap();
            assert_eq!(body["request_id"], key.to_string());
            assert!(body.get("requestId").is_none());
            assert_eq!(store.shell_resource_by_request(&scope, key).unwrap().unwrap(), accepted);
            let mut retry = request.clone();
            retry.environment.insert("SECRET".into(), "rotated-not-identity".into());
            assert_eq!(store.submit_shell_resource(retry, 20).unwrap(), accepted);
            assert_eq!(store.take_shell_environment(&id).unwrap()["SECRET"], ENV_SENTINEL);
        }
        let store = Store::open(&path, "host-a").unwrap();
        let accepted = store.shell_resource_by_request(&scope, key).unwrap().unwrap();
        assert_eq!(accepted.state, "waiting");
        assert_eq!(store.submit_shell_resource(request.clone(), 30).unwrap(), accepted);
        assert!(store.take_shell_environment(&id).is_none(), "retry cannot resurrect a restart-lost launch envelope");
        let mut running = accepted;
        running.state = "running".into(); running.started_at = Some(40);
        store.update_shell_resource(&running).unwrap();
        assert_eq!(store.shell_resource_by_request(&scope, key).unwrap().unwrap().state, "running");
        let mut terminal = store.shell_resource_by_request(&scope, key).unwrap().unwrap();
        terminal.state = "completed".into(); terminal.ended_at = Some(50); terminal.exit_code = Some(0);
        terminal.result = Some(PayloadResult {
            head_start: None, head_end: None, clean_start: None, clean_end: None,
            exit_code: Some(0), exit_signal: None, wall_duration_ms: 10, memory_peak_bytes: None,
            evidence: Default::default(),
        });
        store.update_shell_resource(&terminal).unwrap();
        let terminal = store.shell_resource_by_request(&scope, key).unwrap().unwrap();
        assert_eq!(store.submit_shell_resource(request, 60).unwrap(), terminal);
        assert_eq!(terminal.command.request_id.as_deref(), Some(key.to_string().as_str()));
        assert_no_environment_bytes(&path);
    }

    #[test]
    fn request_identity_conflicts_are_typed_for_every_semantic_intent_change() {
        let store = Store::open_memory("host-a").unwrap();
        let key = uuid::Uuid::parse_str("0192c8d8-97e7-7000-8000-000000000001").unwrap();
        let scope = RequestScope::Local { person: Some("person/operator"), host: "host-a" };
        let id = request_execution_id(&scope, key).unwrap();
        let mut request = command(&id);
        request.request_id = Some(key.to_string());
        request.lane.owner.run = "person/operator".into();
        store.submit_shell_resource(request.clone(), 10).unwrap();
        for field in ["argv", "cwd", "lane", "class", "profile", "capacity", "timeout", "max-members"] {
            let mut changed = request.clone();
            match field {
                "argv" => changed.argv = vec!["different".into()],
                "cwd" => changed.cwd = "/other".into(),
                "lane" => changed.lane.name = "other".into(),
                "class" => changed.lane.class = RequestClass::Landing,
                "profile" => changed.lane.profile = Some("check.other".into()),
                "capacity" => changed.lane.capacity.as_mut().unwrap().memory += 1,
                "timeout" => changed.timeout_ms = Some(1000),
                "max-members" => changed.lane.max_members = 2,
                _ => unreachable!(),
            }
            let error = store.submit_shell_resource(changed, 20).unwrap_err();
            assert_eq!(error.downcast_ref::<FractalError>().unwrap().code, "resource-request-conflict", "{field}");
        }
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
