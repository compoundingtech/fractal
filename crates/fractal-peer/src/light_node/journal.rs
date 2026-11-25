//! Authoritative light-seat command custody. This database is not a graph replica.
use std::{path::Path, sync::{Arc, Mutex, MutexGuard}};

use anyhow::{Context, Result, bail, ensure};
use rusqlite::{Connection, OptionalExtension as _, Transaction, params};
use serde::{Deserialize, Serialize};
use tokio::sync::Notify;

use crate::store::run_credentials::{ExecutionBinding, local};
pub use crate::config::{LightNodeGrant as Selection, LightNodeScope as Scope, LightNodeOperation as OperationKind};
pub use super::start::{StartAuthorization, StartCheckout, StartContinuation, StartCredentials, StartHelperScope, StartRender, StartRolloutScope, StartWorkspaceAdmission};

pub const VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    Start { authorization: Box<StartAuthorization> },
    Observe,
    Stop,
}
impl Operation {
    pub fn kind(&self) -> OperationKind {
        match self { Self::Start { .. } => OperationKind::Start, Self::Observe => OperationKind::Observe, Self::Stop => OperationKind::Stop }
    }

    fn require_supported(&self)->Result<()> {
        if let Self::Start { authorization } = self { authorization.verify()?; }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Command {
    pub version: u32,
    pub sequence: u64,
    pub execution_id: String,
    pub decision_id: String,
    pub binding: ExecutionBinding,
    pub operation: Operation,
}

pub struct CommandDraft {
    pub decision_id: String,
    pub binding: ExecutionBinding,
    pub operation: Operation,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum Outcome {
    Started { incarnation: String },
    Observed { incarnation: String, state: String },
    Stopped { incarnation: String },
    Refused { reason: String },
    Cancelled { reason: String },
    Detached { incarnation: String },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "phase", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReceiptState { Accepted, Settled { outcome: Outcome } }

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub version: u32,
    pub receipt_sequence: u64,
    pub epoch: u64,
    pub command_sequence: u64,
    pub execution_id: String,
    pub home: String,
    pub home_key: String,
    pub host: String,
    pub host_key: String,
    pub state: ReceiptState,
}

#[derive(Clone, Debug)]
pub struct Limits {
    pub max_payload_bytes: usize,
    pub max_commands: usize,
    pub max_identities: usize,
}
impl Default for Limits {
    fn default() -> Self { Self { max_payload_bytes: 48 * 1024, max_commands: 1024, max_identities: 65536 } }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Admission { Accepted(Receipt), Duplicate(Receipt), Replay { next_sequence: u64 }, ResultExpired }

#[derive(Clone, Debug)]
pub struct Recovery {
    pub command: Command,
    /// Missing proof means entry may have happened before the process died. Reconcile by
    /// immutable execution/incarnation identity; absence is NEVER permission to relaunch.
    pub custody_proof: Option<String>,
    pub cancellation_reason: Option<String>,
}

#[derive(Clone)]
pub struct Journal {
    connection: Arc<Mutex<Connection>>,
    pub changed: Arc<Notify>,
    pub authority_changed: Arc<Notify>,
    limits: Limits,
}

const SCHEMA: &str = /* sqlite-large-value: allow-for-now lv-hdeb9b3a5a824 owner=fractal_peer::light_node::journal TODO=chunk-or-reference-variable-columns */ "
CREATE TABLE epochs(epoch INTEGER PRIMARY KEY, selection TEXT NOT NULL, closed INTEGER NOT NULL DEFAULT 0,
 home_floor INTEGER NOT NULL DEFAULT 0, seat_floor INTEGER NOT NULL DEFAULT 0,
 peer_drained INTEGER NOT NULL DEFAULT 0, endpoint TEXT);
CREATE TABLE commands(side TEXT NOT NULL, epoch INTEGER NOT NULL, sequence INTEGER NOT NULL,
 execution TEXT NOT NULL, decision TEXT NOT NULL, payload TEXT NOT NULL, state TEXT NOT NULL,
 proof TEXT, cancel_reason TEXT, cancel_applied INTEGER NOT NULL DEFAULT 0,
 local_finalization INTEGER NOT NULL DEFAULT 0,
 PRIMARY KEY(side,epoch,sequence), UNIQUE(side,execution), UNIQUE(side,decision));
CREATE TABLE identities(side TEXT NOT NULL, decision TEXT NOT NULL, execution TEXT NOT NULL,
 digest TEXT NOT NULL, epoch INTEGER NOT NULL, sequence INTEGER NOT NULL,
 PRIMARY KEY(side,decision), UNIQUE(side,execution));
CREATE TABLE receipts(sequence INTEGER NOT NULL, epoch INTEGER NOT NULL,
 command_sequence INTEGER NOT NULL, payload TEXT NOT NULL, acknowledged INTEGER NOT NULL DEFAULT 0,
 PRIMARY KEY(epoch,sequence));
CREATE TABLE received(sequence INTEGER NOT NULL, digest TEXT NOT NULL, epoch INTEGER NOT NULL,
 command_sequence INTEGER NOT NULL, payload TEXT, PRIMARY KEY(epoch,sequence));
CREATE TABLE counters(name TEXT PRIMARY KEY, value INTEGER NOT NULL);
INSERT INTO counters VALUES('received',0),('acknowledged',0),('issued',0);
CREATE TABLE effects(request_id TEXT PRIMARY KEY, digest TEXT NOT NULL, payload TEXT,
 result TEXT, retired INTEGER NOT NULL DEFAULT 0);
CREATE TABLE authority_received(epoch INTEGER NOT NULL, sequence INTEGER NOT NULL, digest TEXT NOT NULL,
 PRIMARY KEY(epoch,sequence));
CREATE TABLE reported(epoch INTEGER NOT NULL, revision TEXT NOT NULL, binding TEXT NOT NULL,
 revoked INTEGER NOT NULL, PRIMARY KEY(epoch,revision));
PRAGMA user_version=1;";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunRequest {
    pub assertion: crate::store::run_credentials::assertions::RequestAssertion,
    pub claim: crate::model::ClaimInput,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RunRefusal { pub code: String, pub message: String }

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RunResult {
    pub request_id: String,
    pub result: std::result::Result<serde_json::Value, RunRefusal>,
}

impl Journal {
    pub fn open(path: &Path, limits: Limits) -> Result<Self> {
        ensure!(limits.max_payload_bytes > 0 && limits.max_commands > 0 && limits.max_identities > 0, "invalid journal limits");
        let mut connection = Connection::open(path)?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON;")?;
        let transaction = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let version: u32 = transaction.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        match version {
            0 => {
                let tables: u64 = transaction.query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'", [], |row| row.get(0))?;
                ensure!(tables == 0, "unversioned journal contains authoritative data; refusing replacement");
                transaction.execute_batch(/* sqlite-large-value: allow-for-now lv-h938942f23651 owner=fractal_peer::light_node::journal TODO=trace-statement-and-value-representation */ SCHEMA)?;
            }
            VERSION => {}
            _ => bail!("unsupported light-node journal version {version}"),
        }
        local::initialize_local_authority(&transaction)?;
        transaction.execute_batch(/* sqlite-large-value: allow-for-now lv-h17d2cf9e3f86 owner=fractal_peer::light_node::journal TODO=trace-statement-and-value-representation */ START_SCHEMA)?;
        transaction.commit()?;
        Ok(Self { connection: Arc::new(Mutex::new(connection)), changed: Arc::new(Notify::new()), authority_changed: Arc::new(Notify::new()), limits })
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>> { self.connection.lock().map_err(|_| anyhow::anyhow!("journal custody barrier poisoned")) }

    pub fn selection(&self) -> Result<Selection> { current(&*self.lock()?) }

    pub fn configured_selection(&self) -> Result<Option<Selection>> {
        let connection=self.lock()?;
        let payload:Option<String>=connection.query_row("SELECT selection FROM epochs WHERE closed<>1",[],|row|row.get(0)).optional()?;
        payload.map(|payload|Ok(serde_json::from_str(&payload)?)).transpose()
    }

    pub fn with_authority<T>(&self, action: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        action(&*self.lock()?)
    }

    pub fn with_authority_transaction<T>(&self, action: impl FnOnce(&Transaction<'_>) -> Result<T>) -> Result<T> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let before=transaction.total_changes();
        let result = action(&transaction)?;
        let changed=transaction.total_changes()!=before;
        transaction.commit()?;
        if changed { self.authority_changed.notify_waiters(); self.changed.notify_waiters(); }
        Ok(result)
    }

    /// Prepare both sides of retirement, then advance only after durable peer drain proof.
    /// The old selection remains connectable while receipts, revocations and native work drain.
    pub fn install_epoch(&self, selection: &Selection) -> Result<EpochInstall> {
        selection.validate()?;
        let mut connection = self.lock()?;
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let previous: Option<(u64, String)> = tx.query_row("SELECT epoch,selection FROM epochs WHERE closed<>1", [], |row| Ok((row.get(0)?, row.get(1)?))).optional()?;
        if let Some((epoch, payload)) = previous {
            if epoch == selection.epoch {
                ensure!(serde_json::from_str::<Selection>(&payload)? == *selection, "conflicting epoch selection");
                return Ok(if is_closing(&tx)? { EpochInstall::Draining } else { EpochInstall::Installed });
            }
            ensure!(selection.epoch > epoch, "closed epoch is fenced");
            let before=tx.total_changes();
            prepare_closing(&tx,&self.limits)?;
            let peer:bool=tx.query_row("SELECT peer_drained FROM epochs WHERE epoch=?1",[epoch],|row|row.get(0))?;
            let evidence:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM identities) OR EXISTS(SELECT 1 FROM authority_received) OR EXISTS(SELECT 1 FROM effects)",[],|row|row.get(0))?;
            let authority_history=local::acknowledged_change_high_water(&tx,epoch)?>0 || !local::pending_changes(&tx,epoch)?.is_empty();
            if !drain_ready(&tx)? || (!peer && (evidence || authority_history)) {
                let changed=tx.total_changes()!=before;
                tx.commit()?;
                if changed { self.authority_changed.notify_waiters(); self.changed.notify_waiters(); }
                return Ok(EpochInstall::Draining);
            }
            tx.execute(/* sqlite-large-value: allow-for-now lv-h4a870d7bef9c owner=fractal_peer::light_node::journal TODO=bound-or-reference-values-and-reclamation */ "UPDATE epochs SET closed=1 WHERE closed=2", [])?;
        }
        let maximum: u64 = tx.query_row("SELECT COALESCE(MAX(epoch),0) FROM epochs", [], |row| row.get(0))?;
        ensure!(selection.epoch > maximum, "retired epoch is fenced");
        let count:usize=tx.query_row("SELECT COUNT(*) FROM epochs",[],|row|row.get(0))?;
        ensure!(count<self.limits.max_identities,"backpressure: retained epoch fences");
        tx.execute(/* sqlite-large-value: allow-for-now lv-h102052ca35fc owner=fractal_peer::light_node::journal TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO epochs(epoch,selection) VALUES(?1,?2)", params![selection.epoch, serde_json::to_string(selection)?])?;
        tx.execute(/* sqlite-large-value: allow-for-now lv-he362e06c6cb9 owner=fractal_peer::light_node::journal TODO=bound-or-reference-values-and-reclamation */ "UPDATE counters SET value=0 WHERE name IN ('received','acknowledged','issued')", [])?;
        tx.commit()?;
        self.authority_changed.notify_waiters(); self.changed.notify_waiters();
        Ok(EpochInstall::Installed)
    }

    pub fn begin_closing(&self)->Result<()> {
        let mut connection=self.lock()?;
        let tx=connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let before=tx.total_changes();
        prepare_closing(&tx,&self.limits)?;
        let changed=tx.total_changes()!=before;
        tx.commit()?;
        if changed { self.authority_changed.notify_waiters(); self.changed.notify_waiters(); }
        Ok(())
    }

    pub fn drained(&self)->Result<bool> { drain_ready(&*self.lock()?) }

    pub fn mark_peer_drained(&self,epoch:u64)->Result<()> {
        let connection=self.lock()?;
        let changed=connection.execute(/* sqlite-large-value: allow-for-now lv-h7726667ff8cd owner=fractal_peer::light_node::journal TODO=bound-or-reference-values-and-reclamation */ "UPDATE epochs SET peer_drained=1 WHERE epoch=?1 AND peer_drained=0",[epoch])?;
        if changed>0 { self.changed.notify_waiters(); }
        Ok(())
    }

    pub fn epoch_progress(&self,epoch:u64)->Result<(u64,u64)> {
        let connection=self.lock()?;
        let received=connection.query_row("SELECT COALESCE(MAX(sequence),0) FROM received WHERE epoch=?1",[epoch],|row|row.get(0))?;
        let authority=connection.query_row("SELECT COALESCE(MAX(sequence),0) FROM authority_received WHERE epoch=?1",[epoch],|row|row.get(0))?;
        Ok((received,authority))
    }

    pub fn selection_for_epoch(&self,epoch:u64)->Result<Option<Selection>> {
        let connection=self.lock()?;
        let payload:Option<String>=connection.query_row("SELECT selection FROM epochs WHERE epoch=?1",[epoch],|row|row.get(0)).optional()?;
        payload.map(|payload|Ok(serde_json::from_str(&payload)?)).transpose()
    }

    pub fn retired(&self,epoch:u64)->Result<bool> {
        Ok(self.lock()?.query_row("SELECT closed=1 FROM epochs WHERE epoch=?1",[epoch],|row|row.get(0))?)
    }

    pub fn remember_endpoint(&self,selection:&Selection,endpoint:&str)->Result<()> {
        crate::config::LightNodeSessionConfig::Agent {home_endpoint:endpoint.into(),grant:selection.clone()}.validate_for(&selection.host)?;
        let connection=self.lock()?;
        let tx=connection.unchecked_transaction()?;
        ensure!(epoch_selection(&tx,selection.epoch)?==*selection,"endpoint selection differs");
        let previous:Option<String>=tx.query_row("SELECT endpoint FROM epochs WHERE epoch=?1",[selection.epoch],|row|row.get(0))?;
        if let Some(previous)=previous { ensure!(previous==endpoint,"epoch endpoint is immutable"); }
        else {tx.execute(/* sqlite-large-value: allow-for-now lv-h920263c26897 owner=fractal_peer::light_node::journal TODO=bound-or-reference-values-and-reclamation */ "UPDATE epochs SET endpoint=?2 WHERE epoch=?1",params![selection.epoch,endpoint])?;}
        tx.commit()?;
        Ok(())
    }

    pub fn endpoint(&self,epoch:u64)->Result<Option<String>> {
        Ok(self.lock()?.query_row("SELECT endpoint FROM epochs WHERE epoch=?1",[epoch],|row|row.get(0))?)
    }

    pub fn enqueue(&self, draft: CommandDraft) -> Result<Command> {
        draft.operation.require_supported()?;
        let mut connection = self.lock()?;
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let selection = current(&tx)?;
        ensure!(!is_closing(&tx)?, "epoch is closing; new planning is fenced");
        let previous = identity(&tx, "home", &draft.decision_id)?;
        if let Some((execution, _, epoch, sequence)) = previous {
            let command = load_command(&tx, "home", epoch, sequence)?.context("result-expired: planning decision already retired")?;
            let candidate = Command { version: VERSION, sequence, execution_id: execution, decision_id: draft.decision_id, binding: draft.binding, operation: draft.operation };
            ensure!(same(&command, &candidate)?, "conflicting planning decision reuse");
            return Ok(command);
        }
        // Evidence retained across reconnect/epoch changes drains before new planning.
        let old_receipts: u64 = tx.query_row("SELECT COUNT(*) FROM receipts WHERE acknowledged=0 AND epoch<>?1", [selection.epoch], |row| row.get(0))?;
        ensure!(old_receipts == 0, "backpressure: retained epoch receipts must drain first");
        let sequence: u64 = tx.query_row("SELECT MAX(COALESCE((SELECT MAX(sequence) FROM commands WHERE side='home' AND epoch=?1),0),(SELECT home_floor FROM epochs WHERE epoch=?1))+1", [selection.epoch], |row| row.get(0))?;
        let execution_id = format!("execution/{}", uuid::Uuid::new_v4());
        let command = Command { version: VERSION, sequence, execution_id, decision_id: draft.decision_id, binding: draft.binding, operation: draft.operation };
        validate(&selection, &command)?;
        install_start(&tx, "home", &command, &self.limits)?;
        self.insert(&tx, "home", &command, "queued")?;
        tx.commit()?;
        self.changed.notify_waiters();
        Ok(command)
    }

    pub fn pending_commands(&self) -> Result<Vec<Command>> { commands(&*self.lock()?, "home", None) }

    /// Snapshot retained home launch custody for event-driven graph-fact withdrawal.
    /// Includes acknowledged Started results until exact Stop/withdrawal closure.
    /// The caller receives owned values and must not hold this barrier while reading Store.
    pub fn home_active_starts(&self) -> Result<Vec<Command>> {
        let connection = self.lock()?;
        let mut query = connection.prepare(
            "SELECT c.payload FROM commands c JOIN start_authorizations a ON a.side=c.side AND a.execution=c.execution
             JOIN epochs e ON e.epoch=c.epoch WHERE c.side='home' AND a.closed=0 AND e.closed=0
             ORDER BY c.epoch,c.sequence")?;
        query.query_map([], |row| row.get::<_,String>(0))?
            .map(|row| Ok(serde_json::from_str(&row?)?)).collect()
    }

    /// Seat-owned live launch custody, including Started and ordered withdrawal drain.
    /// Native wakes may use this snapshot to withdraw a dead local helper without graph reads.
    pub fn live_start_commands(&self) -> Result<Vec<Command>> {
        let connection = self.lock()?;
        let mut query = connection.prepare(
            "SELECT c.payload FROM commands c JOIN start_authorizations a ON a.side=c.side AND a.execution=c.execution
             WHERE c.side='seat' AND a.closed=0 ORDER BY c.epoch,c.sequence")?;
        query.query_map([], |row| row.get::<_,String>(0))?
            .map(|row| Ok(serde_json::from_str(&row?)?)).collect()
    }

    pub fn replay_commands(&self,from:u64)->Result<Vec<Command>> {
        let connection=self.lock()?;
        let epoch=current(&connection)?.epoch;
        let floor:u64=connection.query_row("SELECT home_floor FROM epochs WHERE epoch=?1",[epoch],|row|row.get(0))?;
        ensure!(from>floor,"result-expired: requested replay predates joint checkpoint");
        let maximum:u64=connection.query_row("SELECT MAX(COALESCE(MAX(sequence),0),?2) FROM commands WHERE side='home' AND epoch=?1",params![epoch,floor],|row|row.get(0))?;
        ensure!(from<=maximum+1,"replay cursor exceeds durable outbox");
        let mut query=connection.prepare("SELECT payload FROM commands WHERE side='home' AND epoch=?1 AND sequence>=?2 ORDER BY sequence")?;
        query.query_map(params![epoch,from],|row|row.get::<_,String>(0))?.map(|row|Ok(serde_json::from_str(&row?)?)).collect()
    }

    /// Persist the possibility of remote admission before the first network send.
    pub fn mark_sent(&self, command: &Command) -> Result<()> {
        let connection = self.lock()?;
        validate(&current(&connection)?, command)?;
        connection.execute(/* sqlite-large-value: allow-for-now lv-hebcba1c5d265 owner=fractal_peer::light_node::journal TODO=bound-or-reference-values-and-reclamation */ "UPDATE commands SET state='sent' WHERE side='home' AND execution=?1 AND state='queued'", [&command.execution_id])?;
        Ok(())
    }

    /// Snapshot withdrawal at the same durable send boundary as initial/replay delivery.
    pub fn prepare_delivery(&self, command: &Command) -> Result<Option<String>> {
        let connection = self.lock()?;
        validate(&current(&connection)?, command)?;
        connection.execute(/* sqlite-large-value: allow-for-now lv-h11b731d64a83 owner=fractal_peer::light_node::journal TODO=bound-or-reference-values-and-reclamation */ "UPDATE commands SET state='sent' WHERE side='home' AND execution=?1 AND state='queued'", [&command.execution_id])?;
        Ok(connection.query_row("SELECT cancel_reason FROM commands WHERE side='home' AND execution=?1", [&command.execution_id], |row| row.get(0))?)
    }

    pub fn acknowledged_high_water(&self) -> Result<u64> {
        Ok(self.lock()?.query_row("SELECT value FROM counters WHERE name='acknowledged'", [], |row| row.get(0))?)
    }

    /// Largest contiguous settled prefix, bounded additionally by receipt ACK custody.
    pub fn settled_prefix(&self, side: &str, epoch: u64) -> Result<u64> {
        ensure!(side == "home" || side == "seat", "invalid journal side");
        let connection = self.lock()?;
        let column = if side == "home" { "home_floor" } else { "seat_floor" };
        let mut prefix: u64 = connection.query_row(/* sqlite-large-value: allow-for-now lv-h191641c4d241 owner=fractal_peer::light_node::journal TODO=trace-statement-and-value-representation */ &format!("SELECT {column} FROM epochs WHERE epoch=?1"), [epoch], |row| row.get(0))?;
        let mut query = connection.prepare("SELECT sequence,state,local_finalization FROM commands WHERE side=?1 AND epoch=?2 ORDER BY sequence")?;
        let rows = query.query_map(params![side,epoch], |row| Ok((row.get::<_,u64>(0)?,row.get::<_,String>(1)?,row.get::<_,i64>(2)?)))?;
        for row in rows {
            let (sequence,state,finalization) = row?;
            if sequence != prefix + 1 || state != "settled" || finalization>0 { break; }
            let live_start: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM start_authorizations a JOIN commands c ON c.side=a.side AND c.execution=a.execution WHERE c.side=?1 AND c.epoch=?2 AND c.sequence=?3 AND a.closed=0)", params![side,epoch,sequence], |row| row.get(0))?;
            if live_start { break; }
            if side == "seat" {
                let pending: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM receipts WHERE epoch=?1 AND command_sequence=?2 AND acknowledged=0)", params![epoch,sequence], |row| row.get(0))?;
                if pending { break; }
            }
            prefix = sequence;
        }
        Ok(prefix)
    }

    pub fn admit(&self, command: &Command) -> Result<Admission> {
        self.admit_withdrawal(command, None)
    }

    /// Retained withdrawal is admitted with the original identity and contiguous receipt
    /// sequence, but never exposes fresh backend entry permission.
    pub fn admit_withdrawal(&self, command: &Command, cancellation: Option<&str>) -> Result<Admission> {
        ensure!(cancellation.is_none_or(|reason|reason.len()<=4096),"cancellation reason too large");
        command.operation.require_supported()?;
        let mut connection = self.lock()?;
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let selection = current(&tx)?;
        // Check immutable identities even when the attempted replay uses another epoch.
        check_identity(&tx, "seat", command)?;
        let floor: Option<u64> = tx.query_row("SELECT seat_floor FROM epochs WHERE epoch=?1", [command.binding.epoch], |row| row.get(0)).optional()?;
        if floor.is_some_and(|floor| command.sequence <= floor) { return Ok(Admission::ResultExpired); }
        validate(&selection, command)?;
        if let Some(existing) = load_command(&tx, "seat", command.binding.epoch, command.sequence)? {
            ensure!(same(&existing, command)?, "conflicting command sequence reuse");
            if let Some(reason)=cancellation {
                let state:String=tx.query_row("SELECT state FROM commands WHERE side='seat' AND execution=?1",[&command.execution_id],|row|row.get(0))?;
                if state=="admitted" {
                    tx.execute(/* sqlite-large-value: allow-for-now lv-h8657669b49f1 owner=fractal_peer::light_node::journal TODO=bound-or-reference-values-and-reclamation */ "UPDATE commands SET cancel_reason=COALESCE(cancel_reason,?2) WHERE side='seat' AND execution=?1",params![command.execution_id,reason])?;
                    let retained:String=tx.query_row("SELECT cancel_reason FROM commands WHERE side='seat' AND execution=?1",[&command.execution_id],|row|row.get(0))?;
                    local::revoke_execution_tx(&tx,&command.binding.host,&command.binding.authorization_revision)?;
                    settle(&tx,command,Outcome::Cancelled {reason:retained},&self.limits)?;
                    let receipt=last_receipt(&tx,command)?;
                    tx.commit()?;
                    self.changed.notify_waiters();
                    self.authority_changed.notify_waiters();
                    return Ok(Admission::Duplicate(receipt));
                }
            }
            return Ok(Admission::Duplicate(last_receipt(&tx, command)?));
        }
        let next: u64 = tx.query_row("SELECT MAX(COALESCE((SELECT MAX(sequence) FROM commands WHERE side='seat' AND epoch=?1),0),(SELECT seat_floor FROM epochs WHERE epoch=?1))+1", [selection.epoch], |row| row.get(0))?;
        if command.sequence != next { return Ok(Admission::Replay { next_sequence: next }); }
        install_start(&tx, "seat", command, &self.limits)?;
        self.insert(&tx, "seat", command, "admitted")?;
        let receipt = append_receipt(&tx, &selection, command, ReceiptState::Accepted, &self.limits)?;
        if let Some(reason)=cancellation {
            local::revoke_execution_tx(&tx,&command.binding.host,&command.binding.authorization_revision)?;
            tx.execute(/* sqlite-large-value: allow-for-now lv-h8bd3c4b8c8b9 owner=fractal_peer::light_node::journal TODO=bound-or-reference-values-and-reclamation */ "UPDATE commands SET cancel_reason=?2 WHERE side='seat' AND execution=?1",params![command.execution_id,reason])?;
            settle(&tx,command,Outcome::Cancelled {reason:reason.to_owned()},&self.limits)?;
        } else if let Some(outcome)=entry_refusal(&tx,command)? {
            settle(&tx,command,outcome,&self.limits)?;
        }
        tx.commit()?;
        self.changed.notify_waiters();
        if cancellation.is_some() {self.authority_changed.notify_waiters();}
        Ok(Admission::Accepted(receipt))
    }

    fn insert(&self, tx: &Transaction<'_>, side: &str, command: &Command, state: &str) -> Result<()> {
        let payload = serde_json::to_string(command)?;
        ensure!(payload.len() <= self.limits.max_payload_bytes, "backpressure: command payload limit");
        let count: usize = tx.query_row("SELECT COUNT(*) FROM commands", [], |row| row.get(0))?;
        let identities: usize = tx.query_row("SELECT COUNT(*) FROM identities", [], |row| row.get(0))?;
        ensure!(count < self.limits.max_commands && identities < self.limits.max_identities, "backpressure: journal capacity");
        check_identity(tx, side, command)?;
        tx.execute(/* sqlite-large-value: allow-for-now lv-h806adaaf48a4 owner=fractal_peer::light_node::journal TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO identities(side,decision,execution,digest,epoch,sequence) VALUES(?1,?2,?3,?4,?5,?6)", params![side, command.decision_id, command.execution_id, digest(command)?, command.binding.epoch, command.sequence])?;
        tx.execute(/* sqlite-large-value: allow-for-now lv-h93c3cc2b150d owner=fractal_peer::light_node::journal TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO commands(side,epoch,sequence,execution,decision,payload,state) VALUES(?1,?2,?3,?4,?5,?6,?7)", params![side, command.binding.epoch, command.sequence, command.execution_id, command.decision_id, payload, state])?;
        Ok(())
    }

    /// `enter` must perform the real last-moment authority check and obtain durable native
    /// custody synchronously. The conservative entry fence commits before calling it.
    /// Errors or process death leave recovery work, never a fresh-dispatch permission.
    pub fn enter(&self, execution: &str, enter: impl FnOnce(&Command, &Connection) -> Result<String>) -> Result<Recovery> {
        let mut connection = self.lock()?;
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let command = by_execution(&tx, execution)?;
        validate(&current(&tx)?, &command)?;
        let state: String = tx.query_row("SELECT state FROM commands WHERE side='seat' AND execution=?1", [execution], |row| row.get(0))?;
        ensure!(state == "admitted", "operation is not undispatched; reconcile existing custody");
        if let Some(outcome)=entry_refusal(&tx,&command)? {
            settle(&tx,&command,outcome,&self.limits)?;
            tx.commit()?;
            self.changed.notify_waiters();
            bail!("native entry refused before custody; terminal receipt retained");
        }
        tx.execute(/* sqlite-large-value: allow-for-now lv-hdd339b4e6193 owner=fractal_peer::light_node::journal TODO=bound-or-reference-values-and-reclamation */ "UPDATE commands SET state='entered' WHERE side='seat' AND execution=?1", [execution])?;
        tx.commit()?;
        self.changed.notify_waiters();
        self.enter_committed(&mut connection,command,enter)
    }

    // The caller has positively not invoked the native adapter in this process.
    // A crash before reaching here remains conservatively recoverable; a live refusal
    // here can instead settle with proof that native entry never happened.
    fn enter_committed(&self,connection:&mut Connection,command:Command,enter:impl FnOnce(&Command,&Connection)->Result<String>)->Result<Recovery> {
        let entry = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let state:String=entry.query_row("SELECT state FROM commands WHERE side='seat' AND execution=?1",[&command.execution_id],|row|row.get(0))?;
        ensure!(state=="entered","native custody already settled");
        validate(&current(&entry)?,&command)?;
        if let Some(outcome)=entry_refusal(&entry,&command)? {
            settle(&entry,&command,outcome,&self.limits)?;
            entry.commit()?;
            self.changed.notify_waiters();
            bail!("native entry refused before custody; terminal receipt retained");
        }
        let proof = enter(&command, &entry)?;
        ensure!(!proof.is_empty() && proof.len() <= self.limits.max_payload_bytes, "invalid native custody proof");
        entry.execute(/* sqlite-large-value: allow-for-now lv-he796ea71134f owner=fractal_peer::light_node::journal TODO=bound-or-reference-values-and-reclamation */ "UPDATE commands SET proof=?2 WHERE side='seat' AND execution=?1", params![command.execution_id, proof])?;
        entry.commit()?;
        self.changed.notify_waiters();
        Ok(Recovery { command, custody_proof: Some(proof), cancellation_reason: None })
    }

    pub fn undispatched(&self) -> Result<Vec<Command>> { commands(&*self.lock()?, "seat", Some("admitted")) }

    pub fn recoverable(&self) -> Result<Vec<Recovery>> {
        let connection = self.lock()?;
        let mut query = connection.prepare("SELECT payload,proof,cancel_reason FROM commands WHERE side='seat' AND (state='entered' OR local_finalization>0) ORDER BY epoch,sequence")?;
        let rows = query.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?, row.get::<_, Option<String>>(2)?)))?;
        rows.map(|row| { let (payload, custody_proof, cancellation_reason) = row?; Ok(Recovery { command: serde_json::from_str(&payload)?, custody_proof, cancellation_reason }) }).collect()
    }

    /// This connection's write count, paired with authenticated frame progress,
    /// prevents idle or self-notification loops while allowing live finalization retry.
    pub fn local_change_count(&self)->Result<u64> {
        Ok(self.lock()?.total_changes())
    }

    pub fn is_local_finalization_pending(&self,execution:&str)->Result<bool> {
        Ok(self.lock()?.query_row("SELECT local_finalization>0 FROM commands WHERE side='seat' AND execution=?1",[execution],|row|row.get(0)).optional()?.unwrap_or(false))
    }

    /// Retry only an exact, freshly proven still-live Stop after a pre-proof crash.
    /// The adapter must recheck that same generation inside the callback. Unlike fresh
    /// entry, withdrawal here cannot prove that an earlier process never signalled it.
    pub fn reconcile_entered_stop(&self,execution:&str,reconcile:impl FnOnce(&Command,&Connection)->Result<String>)->Result<Recovery> {
        let mut connection=self.lock()?;
        let tx=connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let command=by_execution(&tx,execution)?;
        let (state,proof):(String,Option<String>)=tx.query_row("SELECT state,proof FROM commands WHERE side='seat' AND execution=?1",[execution],|row|Ok((row.get(0)?,row.get(1)?)))?;
        ensure!(state=="entered" && proof.is_none(),"Stop reconciliation requires entered custody without proof");
        ensure!(matches!(command.operation,Operation::Stop),"only Stop permits generation-fenced reconciliation");
        validate(&current(&tx)?,&command)?;
        ensure!(entry_refusal(&tx,&command)?.is_none(),"Stop reconciliation authority was withdrawn; prior custody remains unknown");
        let proof=reconcile(&command,&tx)?;
        ensure!(!proof.is_empty() && proof.len()<=self.limits.max_payload_bytes,"invalid reconciled native custody proof");
        tx.execute(/* sqlite-large-value: allow-for-now lv-h286b9d1c1c8b owner=fractal_peer::light_node::journal TODO=bound-or-reference-values-and-reclamation */ "UPDATE commands SET proof=?2 WHERE side='seat' AND execution=?1",params![execution,proof])?;
        tx.commit()?;
        self.changed.notify_waiters();
        Ok(Recovery {command,custody_proof:Some(proof),cancellation_reason:None})
    }

    pub fn complete(&self, execution: &str, outcome: Outcome) -> Result<Receipt> {
        self.complete_inner(execution,outcome,None)
    }

    /// Commit terminal proof and the local Store-finalization obligation atomically.
    /// Positive marker values are terminal receipt sequences; -1 records completion.
    pub fn complete_requiring_local_finalization(&self,execution:&str,outcome:Outcome,custody_proof:&str)->Result<Receipt> {
        ensure!(!custody_proof.is_empty() && custody_proof.len()<=self.limits.max_payload_bytes,"invalid final native retirement proof");
        self.complete_inner(execution,outcome,Some(custody_proof))
    }

    fn complete_inner(&self,execution:&str,outcome:Outcome,final_proof:Option<&str>)->Result<Receipt> {
        let mut connection = self.lock()?;
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let command = by_execution(&tx, execution)?;
        let state: String = tx.query_row("SELECT state FROM commands WHERE side='seat' AND execution=?1", [execution], |row| row.get(0))?;
        if state == "settled" {
            let receipt = last_receipt(&tx, &command)?;
            ensure!(receipt.state == ReceiptState::Settled { outcome }, "conflicting completion reuse");
            if let Some(proof)=final_proof {
                let (marker,retained):(i64,Option<String>)=tx.query_row("SELECT local_finalization,proof FROM commands WHERE side='seat' AND execution=?1",[execution],|row|Ok((row.get(0)?,row.get(1)?)))?;
                ensure!(marker!=0,"cannot retrofit local finalization after ordinary completion");
                ensure!(retained.as_deref()==Some(proof),"conflicting final native retirement proof");
            }
            return Ok(receipt);
        }
        ensure!(state == "entered", "completion requires backend custody");
        let (cancellation,applied):(Option<String>,bool)=tx.query_row("SELECT cancel_reason,cancel_applied FROM commands WHERE side='seat' AND execution=?1",[execution],|row|Ok((row.get(0)?,row.get(1)?)))?;
        ensure!(cancellation.is_none() || applied,"native cancellation custody is not yet applied");
        if let Outcome::Started { incarnation } = &outcome {
            ensure!(matches!(&command.operation, Operation::Start { .. }) && !incarnation.is_empty() && cancellation.is_none(),
                "Started proof requires a live Start and an actual native incarnation");
        }
        let authority_before=tx.total_changes();
        if matches!(&outcome,Outcome::Stopped { .. } | Outcome::Detached { .. }) {
            let binding=&command.binding;
            local::revoke_execution_tx(&tx,&binding.host,&binding.authorization_revision)?;
            if matches!(&command.operation, Operation::Stop) && final_proof.is_none() {
                close_starts_stopped_by(&tx, &command, &outcome, &self.limits)?;
            }
        }
        let authority_changed=tx.total_changes()!=authority_before;
        let receipt = settle(&tx, &command, outcome, &self.limits)?;
        if let Some(proof)=final_proof {
            tx.execute(/* sqlite-large-value: allow-for-now lv-h7f51b642f523 owner=fractal_peer::light_node::journal TODO=bound-or-reference-values-and-reclamation */ "UPDATE commands SET local_finalization=?2,proof=?3 WHERE side='seat' AND execution=?1",params![execution,receipt.receipt_sequence,proof])?;
        }
        tx.commit()?;
        if authority_changed { self.authority_changed.notify_waiters(); }
        self.changed.notify_waiters();
        Ok(receipt)
    }

    /// The native adapter calls this only after its exact Store custody is finalized.
    pub fn finish_local_finalization(&self,execution:&str)->Result<()> {
        let mut connection=self.lock()?;
        let tx=connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let retained:Option<(String,i64)>=tx.query_row("SELECT state,local_finalization FROM commands WHERE side='seat' AND execution=?1",[execution],|row|Ok((row.get(0)?,row.get(1)?))).optional()?;
        let Some((state,marker))=retained else {
            let retired:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM identities i JOIN epochs e ON e.epoch=i.epoch WHERE i.side='seat' AND i.execution=?1 AND i.sequence<=e.seat_floor)",[execution],|row|row.get(0))?;
            ensure!(retired,"unknown local finalization execution");
            return Ok(());
        };
        ensure!(state=="settled" && marker!=0,"no terminal local finalization obligation");
        if marker>0 {
            let command=by_execution(&tx,execution)?;
            if matches!(&command.operation,Operation::Stop) {
                let receipt=last_receipt(&tx,&command)?;
                let ReceiptState::Settled {outcome}=receipt.state else {bail!("local finalization lacks terminal proof");};
                // Associated Start closure must not escape before exact native Store
                // finalization. Commit all its terminal receipts with barrier release.
                close_starts_stopped_by(&tx,&command,&outcome,&self.limits)?;
            }
        }
        let changed=tx.execute(/* sqlite-large-value: allow-for-now lv-h9bef278ec2f7 owner=fractal_peer::light_node::journal TODO=bound-or-reference-values-and-reclamation */ "UPDATE commands SET local_finalization=-1 WHERE side='seat' AND execution=?1 AND local_finalization>0",[execution])?;
        tx.commit()?;
        if changed!=0 {self.changed.notify_waiters();}
        Ok(())
    }

    /// Persist withdrawal before native revocation. Entered custody stays entered until
    /// recovery proves its terminal native result, including producer-closed cancellation.
    pub fn cancel_or_revoke(&self, execution: &str, reason: String, revoke: impl FnOnce(&Command, &Connection, &str) -> Result<()>) -> Result<()> {
        ensure!(reason.len() <= 4096, "cancellation reason too large");
        let mut connection = self.lock()?;
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let command = by_execution(&tx, execution)?;
        let (state, previous): (String,Option<String>) = tx.query_row("SELECT state,cancel_reason FROM commands WHERE side='seat' AND execution=?1", [execution], |row| Ok((row.get(0)?,row.get(1)?)))?;
        let live_start = matches!(&command.operation, Operation::Start { .. }) && tx.query_row(
            "SELECT NOT closed FROM start_authorizations WHERE side='seat' AND execution=?1", [execution], |row| row.get::<_,bool>(0))?;
        if state == "settled" && !live_start { return Ok(()); }
        let state = if state == "settled" {
            tx.execute(/* sqlite-large-value: allow-for-now lv-h5186cc67b6df owner=fractal_peer::light_node::journal TODO=bound-or-reference-values-and-reclamation */ "UPDATE commands SET state='entered' WHERE side='seat' AND execution=?1", [execution])?;
            "entered".to_string()
        } else { state };
        tx.execute(/* sqlite-large-value: allow-for-now lv-h2fcfecf8755f owner=fractal_peer::light_node::journal TODO=bound-or-reference-values-and-reclamation */ "UPDATE start_authorizations SET revoked=1 WHERE side='seat' AND execution=?1", [execution])?;
        // Independent valid revokers may race. Withdrawal is monotonic; retain the
        // first durable intent rather than terminating the authenticated session.
        let reason=previous.unwrap_or(reason);
        let authority_before=tx.total_changes();
        let binding=&command.binding;
        local::revoke_execution_tx(&tx, &binding.host, &binding.authorization_revision)?;
        let authority_changed=tx.total_changes()!=authority_before;
        tx.execute(/* sqlite-large-value: allow-for-now lv-hc3cd8ed405bc owner=fractal_peer::light_node::journal TODO=bound-or-reference-values-and-reclamation */ "UPDATE commands SET cancel_reason=?2 WHERE side='seat' AND execution=?1", params![execution,reason])?;
        let native_result:Result<()>=if state == "admitted" {
            settle(&tx,&command,Outcome::Cancelled { reason },&self.limits)?;
            tx.commit()?;
            Ok(())
        } else {
            ensure!(state == "entered", "invalid command custody state");
            tx.commit()?;
            (|| {
                let cancellation=connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
                let applied:bool=cancellation.query_row("SELECT cancel_applied FROM commands WHERE side='seat' AND execution=?1",[execution],|row|row.get(0))?;
                if !applied {
                    revoke(&command,&cancellation,&reason)?;
                    cancellation.execute(/* sqlite-large-value: allow-for-now lv-h555cdce14fe8 owner=fractal_peer::light_node::journal TODO=bound-or-reference-values-and-reclamation */ "UPDATE commands SET cancel_applied=1 WHERE side='seat' AND execution=?1", [execution])?;
                }
                cancellation.commit()?;
                Ok(())
            })()
        };
        if authority_changed { self.authority_changed.notify_waiters(); }
        self.changed.notify_waiters();
        native_result
    }

    pub fn request_cancel(&self, execution: &str, reason: &str) -> Result<()> {
        self.request_cancel_inner(execution, reason, false).map(|_| ())
    }

    /// Automatic home withdrawal must retain an earlier explicit operator intent.
    pub fn request_cancel_if_unset(&self, execution: &str, reason: &str) -> Result<bool> {
        self.request_cancel_inner(execution, reason, true)
    }

    fn request_cancel_inner(&self, execution: &str, reason: &str, retain_previous: bool) -> Result<bool> {
        ensure!(reason.len() <= 4096, "cancellation reason too large");
        let mut connection = self.lock()?;
        let connection = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let previous: Option<String> = connection.query_row("SELECT cancel_reason FROM commands WHERE side='home' AND execution=?1", [execution], |row| row.get(0))?;
        if let Some(previous) = previous {
            ensure!(retain_previous || previous == reason, "conflicting cancellation intent");
            return Ok(false);
        }
        connection.execute(/* sqlite-large-value: allow-for-now lv-h8764bd131643 owner=fractal_peer::light_node::journal TODO=bound-or-reference-values-and-reclamation */ "UPDATE commands SET cancel_reason=?2 WHERE side='home' AND execution=?1", params![execution,reason])?;
        connection.execute(/* sqlite-large-value: allow-for-now lv-h52f848373ef7 owner=fractal_peer::light_node::journal TODO=bound-or-reference-values-and-reclamation */ "UPDATE start_authorizations SET revoked=1 WHERE side='home' AND execution=?1", [execution])?;
        connection.commit()?;
        self.changed.notify_waiters();
        Ok(true)
    }

    pub fn command_receipt(&self,execution:&str)->Result<Option<Receipt>> {
        let connection=self.lock()?;
        let identity:Option<(u64,u64)>=connection.query_row("SELECT epoch,sequence FROM identities WHERE side='home' AND execution=?1",[execution],|row|Ok((row.get(0)?,row.get(1)?))).optional()?;
        let (epoch,sequence)=identity.context("unknown command execution identity")?;
        ensure!(load_command(&connection,"home",epoch,sequence)?.is_some(),"result-expired: command payload retired");
        let payload:Option<Option<String>>=connection.query_row("SELECT payload FROM received WHERE epoch=?1 AND command_sequence=?2 ORDER BY sequence DESC LIMIT 1",params![epoch,sequence],|row|row.get(0)).optional()?;
        match payload {
            None=>Ok(None),
            Some(Some(payload))=>Ok(Some(serde_json::from_str(&payload)?)),
            Some(None)=>bail!("result-expired: receipt payload retired"),
        }
    }

    pub fn pending_cancellations(&self) -> Result<Vec<(String,String)>> {
        let connection = self.lock()?;
        let mut query = connection.prepare("SELECT execution,cancel_reason FROM commands WHERE side='home' AND cancel_reason IS NOT NULL AND (state<>'settled' OR EXISTS(SELECT 1 FROM start_authorizations a WHERE a.side='home' AND a.execution=commands.execution AND a.closed=0)) ORDER BY epoch,sequence")?;
        Ok(query.query_map([], |row| Ok((row.get(0)?,row.get(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn closing(&self) -> Result<bool> { is_closing(&*self.lock()?) }

    pub fn pending_native_cancellations(&self) -> Result<Vec<Recovery>> {
        let connection = self.lock()?;
        let mut query = connection.prepare("SELECT payload,proof,cancel_reason FROM commands WHERE side='seat' AND state='entered' AND cancel_reason IS NOT NULL AND cancel_applied=0 ORDER BY epoch,sequence")?;
        query.query_map([],|row| Ok((row.get::<_,String>(0)?,row.get(1)?,row.get(2)?)))?.map(|row| {
            let (payload,custody_proof,cancellation_reason)=row?;
            Ok(Recovery { command:serde_json::from_str(&payload)?,custody_proof,cancellation_reason })
        }).collect()
    }

    pub fn enqueue_run_request(&self, request: &RunRequest) -> Result<()> {
        let mut connection = self.lock()?;
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let payload=serde_json::to_string(request)?;
        ensure!(payload.len() <= self.limits.max_payload_bytes, "backpressure: run request payload limit");
        let id=&request.assertion.request.request_id;
        let previous: Option<(String,bool)> = tx.query_row("SELECT digest,retired FROM effects WHERE request_id=?1", [id],|row| Ok((row.get(0)?,row.get(1)?))).optional()?;
        if let Some((digest,retired))=previous {
            ensure!(digest==run_request_digest(request)?, "conflicting run request identity");
            ensure!(!retired,"result-expired: run request was retired");
            return Ok(());
        }
        ensure!(!is_closing(&tx)?, "epoch is closing; new effects are fenced");
        let selected=current(&tx)?;
        let binding=&request.assertion.binding;
        ensure!(binding.home==selected.home && binding.host==selected.host && binding.epoch==selected.epoch && selected.grant.subjects.contains(&binding.subject), "run request selection is fenced");
        local::require_execution(&tx,binding)?;
        ensure!(request.assertion.request.payload_sha256==encoded_digest(&serde_json::to_string(&request.claim)?),"run request claim digest differs");
        let (active,total):(usize,usize)=tx.query_row("SELECT COUNT(payload),COUNT(*) FROM effects",[],|row|Ok((row.get(0)?,row.get(1)?)))?;
        ensure!(active<self.limits.max_commands && total<self.limits.max_identities,"backpressure: run request capacity");
        tx.execute(/* sqlite-large-value: allow-for-now lv-hbc8ed50798f2 owner=fractal_peer::light_node::journal TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO effects(request_id,digest,payload) VALUES(?1,?2,?3)",params![id,run_request_digest(request)?,payload])?;
        tx.commit()?;
        self.changed.notify_waiters();
        Ok(())
    }

    pub fn pending_run_requests(&self) -> Result<Vec<RunRequest>> {
        let connection=self.lock()?;
        let mut query=connection.prepare("SELECT payload FROM effects WHERE result IS NULL AND retired=0 ORDER BY rowid")?;
        query.query_map([],|row|row.get::<_,String>(0))?.map(|row|Ok(serde_json::from_str(&row?)?)).collect()
    }

    pub fn accept_run_result(&self,result:&RunResult)->Result<()> {
        let connection=self.lock()?;
        let (previous,retired):(Option<String>,bool)=connection.query_row("SELECT result,retired FROM effects WHERE request_id=?1",[&result.request_id],|row|Ok((row.get(0)?,row.get(1)?)))?;
        ensure!(!retired,"result-expired: run result was retired");
        let payload=serde_json::to_string(result)?;
        ensure!(payload.len()<=self.limits.max_payload_bytes,"run result payload limit");
        if let Some(previous)=previous { ensure!(serde_json::from_str::<RunResult>(&previous)?==*result,"conflicting run result"); return Ok(()); }
        connection.execute(/* sqlite-large-value: allow-for-now lv-h3d6e4209c6eb owner=fractal_peer::light_node::journal TODO=bound-or-reference-values-and-reclamation */ "UPDATE effects SET result=?2 WHERE request_id=?1",params![result.request_id,payload])?;
        self.changed.notify_waiters();
        Ok(())
    }

    pub fn run_result(&self,id:&str)->Result<Option<RunResult>> {
        let connection=self.lock()?;
        let (payload,retired):(Option<String>,bool)=connection.query_row("SELECT result,retired FROM effects WHERE request_id=?1",[id],|row|Ok((row.get(0)?,row.get(1)?)))?;
        ensure!(!retired,"result-expired: run result was retired");
        payload.map(|payload|Ok(serde_json::from_str(&payload)?)).transpose()
    }

    /// The caller explicitly accepts an exact retained result under its original run binding.
    /// A lost ACK response is retryable after retirement using the immutable request digest.
    pub fn acknowledge_run_result(&self, request: &RunRequest, accepted: &RunResult) -> Result<()> {
        let id = &request.assertion.request.request_id;
        ensure!(accepted.request_id == *id, "accepted run result names another request");
        let mut connection = self.lock()?;
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let (digest, payload, retired): (String, Option<String>, bool) = tx.query_row(
            "SELECT digest,result,retired FROM effects WHERE request_id=?1", [id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
        ensure!(digest == run_request_digest(request)?, "run result acknowledgement belongs to another request or execution");
        if retired { return Ok(()); }
        let payload = payload.context("run result has not reached durable custody")?;
        ensure!(serde_json::from_str::<RunResult>(&payload)? == *accepted, "accepted run result differs from retained custody");
        tx.execute(/* sqlite-large-value: allow-for-now lv-h72a762bf7354 owner=fractal_peer::light_node::journal TODO=bound-or-reference-values-and-reclamation */ "UPDATE effects SET payload=NULL,result=NULL,retired=1 WHERE request_id=?1", [id])?;
        tx.commit()?;
        self.changed.notify_waiters();
        Ok(())
    }

    pub fn offer_execution(&self, binding: &ExecutionBinding) -> Result<Offer> {
        let mut connection=self.lock()?;
        let tx=connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let selected=current(&tx)?;
        if binding.epoch!=selected.epoch {
            return Err(crate::model::FractalError::new("local-authority-backpressure","native authority offer belongs to a superseded epoch").into());
        }
        validate_execution(&selected,binding)?;
        if is_closing(&tx)? {
            return Err(crate::model::FractalError::new("local-authority-backpressure","epoch is closing; native authority offers await successor installation").into());
        }
        let before=tx.total_changes();
        let active=local::offer_execution_tx(&tx,binding)?;
        let changed=tx.total_changes()!=before;
        tx.commit()?;
        if changed { self.authority_changed.notify_waiters(); self.changed.notify_waiters(); }
        Ok(if active { Offer::Admitted } else { Offer::WaitingForRevocation })
    }

    pub fn activate_and_issue(&self,binding:&ExecutionBinding,lifetime_ms:u64,now_ms:u64)->Result<String> {
        let mut connection=self.lock()?;
        let tx=connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let selected=current(&tx)?;
        if binding.epoch!=selected.epoch {
            return Err(crate::model::FractalError::new("local-authority-backpressure","credential binding belongs to a superseded epoch").into());
        }
        validate_execution(&selected,binding)?;
        if is_closing(&tx)? {
            return Err(crate::model::FractalError::new("local-authority-backpressure","epoch is closing; credential issuance awaits successor installation").into());
        }
        let before=tx.total_changes();
        let active=local::offer_execution_tx(&tx,binding)?;
        let authority_changed=tx.total_changes()!=before;
        if !active {
            tx.commit()?;
            if authority_changed { self.authority_changed.notify_waiters(); self.changed.notify_waiters(); }
            return Err(crate::model::FractalError::new("local-authority-backpressure","prior execution revocation awaits home acknowledgment").into());
        }
        let secret=local::issue_local_credential_tx(&tx,binding,lifetime_ms,now_ms);
        // An admission newly offered above must survive the expected wait-for-home-ACK
        // error; otherwise the session would never have anything durable to transmit.
        tx.commit()?;
        if authority_changed { self.authority_changed.notify_waiters(); self.changed.notify_waiters(); }
        Ok(secret?)
    }

    pub fn revoke_execution(&self,binding:&ExecutionBinding)->Result<()> {
        let mut connection=self.lock()?;
        let tx=connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let before=tx.total_changes();
        local::revoke_execution_tx(&tx,&binding.host,&binding.authorization_revision)?;
        let changed=tx.total_changes()!=before;
        tx.commit()?;
        if changed { self.authority_changed.notify_waiters(); self.changed.notify_waiters(); }
        Ok(())
    }

    pub fn pending_authority(&self)->Result<Vec<local::AuthorityChange>> {
        let connection=self.lock()?;
        Ok(local::pending_changes(&connection,current(&connection)?.epoch)?)
    }

    pub fn acknowledge_authority(&self,through:u64)->Result<()> {
        let mut connection=self.lock()?;
        let tx=connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let before=tx.total_changes();
        local::acknowledge_changes_tx(&tx,current(&tx)?.epoch,through)?;
        let changed=tx.total_changes()!=before;
        tx.commit()?;
        if changed { self.authority_changed.notify_waiters(); self.changed.notify_waiters(); }
        Ok(())
    }

    pub fn received_authority_high_water(&self)->Result<u64> {
        let connection=self.lock()?;
        Ok(connection.query_row("SELECT COALESCE(MAX(sequence),0) FROM authority_received WHERE epoch=?1",[current(&connection)?.epoch],|row|row.get(0))?)
    }

    /// The graph adapter commits before this journal ACK. Retrying that adapter is safe:
    /// it admits/revokes the exact immutable authority revision, never executes a claim.
    pub fn accept_authority(&self,change:&local::AuthorityChange,apply:impl FnOnce(&local::AuthorityChange,&str)->Result<()>)->Result<u64> {
        let mut connection=self.lock()?;
        let tx=connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let selected=current(&tx)?;
        validate_execution(&selected,&change.binding)?;
        let digest=encoded_digest(&serde_json::to_string(change)?);
        let previous:Option<String>=tx.query_row("SELECT digest FROM authority_received WHERE epoch=?1 AND sequence=?2",params![selected.epoch,change.sequence],|row|row.get(0)).optional()?;
        if let Some(previous)=previous { ensure!(previous==digest,"conflicting authority change replay"); return Ok(change.sequence); }
        let high:u64=tx.query_row("SELECT COALESCE(MAX(sequence),0) FROM authority_received WHERE epoch=?1",[selected.epoch],|row|row.get(0))?;
        ensure!(change.sequence==high+1,"authority change gap");
        let count:usize=tx.query_row("SELECT COUNT(*) FROM authority_received",[],|row|row.get(0))?;
        ensure!(count<self.limits.max_identities*2,"backpressure: retained authority receipts");
        let binding=serde_json::to_string(&change.binding)?;
        ensure!(binding.len()<=self.limits.max_payload_bytes,"authority binding payload limit");
        let previous:Option<String>=tx.query_row("SELECT binding FROM reported WHERE epoch=?1 AND revision=?2",params![selected.epoch,change.binding.authorization_revision],|row|row.get(0)).optional()?;
        if let Some(previous)=previous { ensure!(previous==binding,"conflicting reported authority revision"); }
        apply(change,&selected.host_key)?;
        tx.execute(/* sqlite-large-value: allow-for-now lv-h77b56b17f8fc owner=fractal_peer::light_node::journal TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO authority_received(epoch,sequence,digest) VALUES(?1,?2,?3)",params![selected.epoch,change.sequence,digest])?;
        tx.execute(/* sqlite-large-value: allow-for-now lv-hbc80366c1c2b owner=fractal_peer::light_node::journal TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO reported(epoch,revision,binding,revoked) VALUES(?1,?2,?3,?4) ON CONFLICT(epoch,revision) DO UPDATE SET revoked=MAX(revoked,excluded.revoked)",params![selected.epoch,change.binding.authorization_revision,binding,change.revoked])?;
        tx.commit()?;
        self.authority_changed.notify_waiters(); self.changed.notify_waiters();
        Ok(change.sequence)
    }

    /// Custody identity, not a runtime-health or graph-freshness assertion.
    pub fn reported_executions(&self)->Result<Vec<ExecutionBinding>> {
        let connection=self.lock()?;
        let selected=current(&connection)?;
        let mut bindings=std::collections::BTreeMap::new();
        for binding in local::active_bindings(&connection)? {
            if binding.epoch==selected.epoch && binding.host==selected.host {
                bindings.insert(binding.authorization_revision.clone(),binding);
            }
        }
        let mut query=connection.prepare("SELECT binding FROM reported WHERE epoch=?1 AND revoked=0 ORDER BY revision")?;
        for payload in query.query_map([selected.epoch],|row|row.get::<_,String>(0))? {
            let binding:ExecutionBinding=serde_json::from_str(&payload?)?;
            bindings.insert(binding.authorization_revision.clone(),binding);
        }
        ensure!(bindings.len()<=self.limits.max_commands,"backpressure: reported execution capacity");
        Ok(bindings.into_values().collect())
    }

    pub fn pending_receipts(&self) -> Result<Vec<Receipt>> {
        let connection = self.lock()?;
        let mut query = connection.prepare("SELECT r.payload FROM receipts r WHERE r.acknowledged=0 AND NOT EXISTS(SELECT 1 FROM commands c WHERE c.side='seat' AND c.epoch=r.epoch AND c.local_finalization>0 AND c.local_finalization<=r.sequence) ORDER BY r.epoch,r.sequence")?;
        query.query_map([], |row| row.get::<_, String>(0))?.map(|row| Ok(serde_json::from_str(&row?)?)).collect()
    }

    /// Replay durable local finalization obligations independently of receipt ACKs.
    pub fn pending_terminal_results(&self) -> Result<Vec<(Recovery, Outcome)>> {
        let connection = self.lock()?;
        let mut query = connection.prepare(
            "SELECT commands.payload,receipts.payload,commands.proof,commands.cancel_reason FROM commands
             JOIN receipts ON receipts.epoch=commands.epoch
               AND receipts.sequence=commands.local_finalization
             WHERE commands.side='seat' AND commands.local_finalization>0
             ORDER BY commands.epoch,commands.sequence",
        )?;
        let mut results = Vec::new();
        for row in query.query_map([], |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, String>(1)?,row.get::<_,Option<String>>(2)?,row.get::<_,Option<String>>(3)?)))? {
            let (command, receipt,custody_proof,cancellation_reason) = row?;
            let receipt: Receipt = serde_json::from_str(&receipt)?;
            if let ReceiptState::Settled { outcome } = receipt.state {
                let command: Command = serde_json::from_str(&command.context("pending local finalization lost its command custody")?)?;
                ensure!(command.execution_id == receipt.execution_id && command.binding.epoch == receipt.epoch
                    && command.sequence == receipt.command_sequence, "terminal receipt custody differs");
                results.push((Recovery {command,custody_proof,cancellation_reason}, outcome));
            } else {bail!("pending local finalization lacks terminal proof");}
        }
        Ok(results)
    }

    /// Home custody commits before the caller may transmit the receipt ACK.
    pub fn accept_receipt(&self, receipt: &Receipt) -> Result<u64> {
        ensure!(receipt.version == VERSION, "unsupported receipt version");
        let mut connection = self.lock()?;
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        ensure!(receipt.epoch == current(&tx)?.epoch, "receipt epoch is fenced");
        let payload = serde_json::to_string(receipt)?;
        ensure!(payload.len() <= self.limits.max_payload_bytes, "receipt payload limit");
        let previous: Option<String> = tx.query_row("SELECT digest FROM received WHERE epoch=?1 AND sequence=?2", params![receipt.epoch,receipt.receipt_sequence], |row| row.get(0)).optional()?;
        if let Some(previous) = previous {
            ensure!(previous == encoded_digest(&payload), "conflicting receipt reuse");
            return Ok(receipt.receipt_sequence);
        }
        let high: u64 = tx.query_row("SELECT value FROM counters WHERE name='received'", [], |row| row.get(0))?;
        ensure!(receipt.receipt_sequence == high + 1, "receipt gap: replay from {}", high + 1);
        let command = load_command(&tx, "home", receipt.epoch, receipt.command_sequence)?.context("result-expired or unknown execution")?;
        let selection = epoch_selection(&tx, receipt.epoch)?;
        ensure!(receipt.execution_id == command.execution_id && receipt.home == selection.home && receipt.host == selection.host && receipt.home_key == selection.home_key && receipt.host_key == selection.host_key, "receipt provenance differs");
        let (active, retained): (usize, usize) = tx.query_row("SELECT COUNT(payload),COUNT(*) FROM received", [], |row| Ok((row.get(0)?,row.get(1)?)))?;
        ensure!(active < self.limits.max_commands * 3 && retained < self.limits.max_identities * 3, "backpressure: receipt custody capacity");
        tx.execute(/* sqlite-large-value: allow-for-now lv-hea697bc2d549 owner=fractal_peer::light_node::journal TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO received(sequence,digest,epoch,command_sequence,payload) VALUES(?1,?2,?3,?4,?5)", params![receipt.receipt_sequence,encoded_digest(&payload),receipt.epoch,receipt.command_sequence,payload])?;
        tx.execute(/* sqlite-large-value: allow-for-now lv-h9799d5cfd34a owner=fractal_peer::light_node::journal TODO=bound-or-reference-values-and-reclamation */ "UPDATE counters SET value=?1 WHERE name='received'", [receipt.receipt_sequence])?;
        if let ReceiptState::Settled { outcome } = &receipt.state {
            close_start(&tx, "home", &command, outcome)?;
            tx.execute(/* sqlite-large-value: allow-for-now lv-hea174ad4467c owner=fractal_peer::light_node::journal TODO=bound-or-reference-values-and-reclamation */ "UPDATE commands SET state='settled' WHERE side='home' AND epoch=?1 AND sequence=?2", params![receipt.epoch, receipt.command_sequence])?;
        }
        tx.commit()?;
        self.changed.notify_waiters();
        Ok(receipt.receipt_sequence)
    }

    pub fn received_high_water(&self) -> Result<u64> { Ok(self.lock()?.query_row("SELECT value FROM counters WHERE name='received'", [], |row| row.get(0))?) }

    pub fn acknowledge_receipts(&self, through: u64) -> Result<()> {
        let connection = self.lock()?;
        let maximum: u64 = connection.query_row("SELECT value FROM counters WHERE name='issued'", [], |row| row.get(0))?;
        ensure!(through <= maximum, "receipt acknowledgment exceeds durable custody");
        let tx = connection.unchecked_transaction()?;
        let blocked:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM commands WHERE side='seat' AND epoch=?1 AND local_finalization>0 AND local_finalization<=?2)",params![current(&tx)?.epoch,through],|row|row.get(0))?;
        ensure!(!blocked,"receipt acknowledgment blocked by local finalization");
        tx.execute(/* sqlite-large-value: allow-for-now lv-hcdd54fc3ac96 owner=fractal_peer::light_node::journal TODO=bound-or-reference-values-and-reclamation */ "UPDATE receipts SET acknowledged=1 WHERE epoch=?1 AND sequence<=?2", params![current(&tx)?.epoch,through])?;
        tx.execute(/* sqlite-large-value: allow-for-now lv-h113edb40022a owner=fractal_peer::light_node::journal TODO=bound-or-reference-values-and-reclamation */ "UPDATE counters SET value=MAX(value,?1) WHERE name='acknowledged'", [through])?;
        tx.commit()?;
        self.changed.notify_waiters();
        Ok(())
    }

    /// Called only after both peers have agreed on this settled command prefix. Compact
    /// identity hashes and epoch fences remain authoritative after payload reclamation.
    pub fn checkpoint(&self, side: &str, epoch: u64, through: u64) -> Result<()> {
        ensure!(side == "home" || side == "seat", "invalid journal side");
        let mut connection = self.lock()?;
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let column = if side == "home" { "home_floor" } else { "seat_floor" };
        let floor: u64 = tx.query_row(/* sqlite-large-value: allow-for-now lv-h8eee3a85f96b owner=fractal_peer::light_node::journal TODO=trace-statement-and-value-representation */ &format!("SELECT {column} FROM epochs WHERE epoch=?1"), [epoch], |row| row.get(0))?;
        ensure!(through >= floor, "checkpoint floor cannot regress");
        let unsettled: u64 = tx.query_row("SELECT COUNT(*) FROM commands WHERE side=?1 AND epoch=?2 AND sequence<=?3 AND (state<>'settled' OR local_finalization>0)", params![side, epoch, through], |row| row.get(0))?;
        ensure!(unsettled == 0, "checkpoint blocked by unsettled custody");
        let active_start: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM commands c JOIN start_authorizations a ON a.side=c.side AND a.execution=c.execution WHERE c.side=?1 AND c.epoch=?2 AND c.sequence<=?3 AND a.closed=0)", params![side,epoch,through], |row| row.get(0))?;
        ensure!(!active_start, "checkpoint blocked by live Start authorization");
        let maximum: u64 = tx.query_row("SELECT MAX(COALESCE(MAX(sequence),0),?3) FROM commands WHERE side=?1 AND epoch=?2", params![side, epoch, floor], |row| row.get(0))?;
        ensure!(through <= maximum, "checkpoint exceeds admitted prefix");
        if side == "seat" {
            let pending: u64 = tx.query_row("SELECT COUNT(*) FROM receipts WHERE epoch=?1 AND command_sequence<=?2 AND acknowledged=0", params![epoch, through], |row| row.get(0))?;
            ensure!(pending == 0, "checkpoint blocked by unacknowledged receipts");
            tx.execute(/* sqlite-large-value: allow-for-now lv-h23bfa73b5860 owner=fractal_peer::light_node::journal TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM receipts WHERE epoch=?1 AND command_sequence<=?2 AND acknowledged=1", params![epoch, through])?;
        } else {
            tx.execute(/* sqlite-large-value: allow-for-now lv-hc882d2a1ceca owner=fractal_peer::light_node::journal TODO=bound-or-reference-values-and-reclamation */ "UPDATE received SET payload=NULL WHERE epoch=?1 AND command_sequence<=?2", params![epoch,through])?;
        }
        tx.execute(/* sqlite-large-value: allow-for-now lv-hd2a6d3bd9e80 owner=fractal_peer::light_node::journal TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM commands WHERE side=?1 AND epoch=?2 AND sequence<=?3", params![side, epoch, through])?;
        tx.execute(&format!(/* sqlite-large-value: allow-for-now lv-hc5db8e607bbb owner=fractal_peer::light_node::journal TODO=bound-or-reference-values-and-reclamation */ "UPDATE epochs SET {column}=?2 WHERE epoch=?1"), params![epoch, through])?;
        tx.commit()?;
        Ok(())
    }
}

fn current(connection: &Connection) -> Result<Selection> {
    let payload: String = connection.query_row("SELECT selection FROM epochs WHERE closed<>1", [], |row| row.get(0)).context("no selected light-node home")?;
    Ok(serde_json::from_str(&payload)?)
}
fn epoch_selection(connection: &Connection, epoch: u64) -> Result<Selection> {
    let payload: String = connection.query_row("SELECT selection FROM epochs WHERE epoch=?1", [epoch], |row| row.get(0))?;
    Ok(serde_json::from_str(&payload)?)
}
fn validate(selection: &Selection, command: &Command) -> Result<()> {
    command.operation.require_supported()?;
    let b = &command.binding;
    ensure!(command.version == VERSION, "unsupported command version");
    ensure!(command.sequence > 0 && command.sequence <= i64::MAX as u64 && !command.execution_id.is_empty() && !command.decision_id.is_empty(), "invalid command identity");
    ensure!(b.home == selection.home && b.host == selection.host && b.epoch == selection.epoch, "command home/host/epoch fenced");
    ensure!(selection.grant.subjects.iter().any(|subject| subject == &b.subject) && selection.grant.operations.contains(&command.operation.kind()), "command outside provisioned grant");
    b.validate()?;
    Ok(())
}
fn same(left: &Command, right: &Command) -> Result<bool> { Ok(serde_json::to_value(left)? == serde_json::to_value(right)?) }
fn digest(command: &Command) -> Result<String> {
    use sha2::{Digest as _, Sha256};
    Ok(hex::encode(Sha256::digest(serde_json::to_vec(command)?)))
}
type Identity = (String, String, u64, u64);
fn identity(connection: &Connection, side: &str, decision: &str) -> Result<Option<Identity>> {
    Ok(connection.query_row("SELECT execution,digest,epoch,sequence FROM identities WHERE side=?1 AND decision=?2", params![side, decision], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))).optional()?)
}
fn check_identity(connection: &Connection, side: &str, command: &Command) -> Result<()> {
    let existing: Option<(String,String)> = connection.query_row("SELECT decision,digest FROM identities WHERE side=?1 AND (decision=?2 OR execution=?3)", params![side, command.decision_id, command.execution_id], |row| Ok((row.get(0)?,row.get(1)?))).optional()?;
    if let Some((decision, hash)) = existing { ensure!(decision == command.decision_id && hash == digest(command)?, "conflicting immutable execution or decision reuse"); }
    Ok(())
}
fn load_command(connection: &Connection, side: &str, epoch: u64, sequence: u64) -> Result<Option<Command>> {
    let payload: Option<String> = connection.query_row("SELECT payload FROM commands WHERE side=?1 AND epoch=?2 AND sequence=?3", params![side, epoch, sequence], |row| row.get(0)).optional()?;
    payload.map(|payload| Ok(serde_json::from_str(&payload)?)).transpose()
}
fn by_execution(connection: &Connection, execution: &str) -> Result<Command> {
    let payload: String = connection.query_row("SELECT payload FROM commands WHERE side='seat' AND execution=?1", [execution], |row| row.get(0))?;
    Ok(serde_json::from_str(&payload)?)
}
fn commands(connection: &Connection, side: &str, state: Option<&str>) -> Result<Vec<Command>> {
    let mut query = connection.prepare("SELECT payload FROM commands WHERE side=?1 AND ((?2 IS NULL AND state<>'settled') OR state=?2) ORDER BY epoch,sequence")?;
    query.query_map(params![side,state], |row| row.get::<_,String>(0))?.map(|row| Ok(serde_json::from_str(&row?)?)).collect()
}
fn last_receipt(connection: &Connection, command: &Command) -> Result<Receipt> {
    let payload: String = connection.query_row("SELECT payload FROM receipts WHERE epoch=?1 AND command_sequence=?2 ORDER BY sequence DESC LIMIT 1", params![command.binding.epoch,command.sequence], |row| row.get(0))?;
    Ok(serde_json::from_str(&payload)?)
}
fn append_receipt(tx: &Transaction<'_>, selection: &Selection, command: &Command, state: ReceiptState, limits: &Limits) -> Result<Receipt> {
    let count: usize = tx.query_row("SELECT COUNT(*) FROM receipts", [], |row| row.get(0))?;
    ensure!(count < limits.max_commands * 3, "backpressure: durable receipt capacity");
    let sequence: u64 = tx.query_row(/* sqlite-large-value: allow-for-now lv-h577b713b5a24 owner=fractal_peer::light_node::journal TODO=bound-or-reference-values-and-reclamation */ "UPDATE counters SET value=value+1 WHERE name='issued' RETURNING value", [], |row| row.get(0))?;
    tx.execute(/* sqlite-large-value: allow-for-now lv-h7bc5ac937863 owner=fractal_peer::light_node::journal TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO receipts(epoch,sequence,command_sequence,payload) VALUES(?1,?2,?3,'')", params![command.binding.epoch,sequence,command.sequence])?;
    let receipt = Receipt { version: VERSION, receipt_sequence: sequence, epoch: command.binding.epoch, command_sequence: command.sequence, execution_id: command.execution_id.clone(), home: selection.home.clone(), home_key: selection.home_key.clone(), host: selection.host.clone(), host_key: selection.host_key.clone(), state };
    let payload = serde_json::to_string(&receipt)?;
    ensure!(payload.len() <= limits.max_payload_bytes, "receipt payload limit");
    tx.execute(/* sqlite-large-value: allow-for-now lv-h92381651afd4 owner=fractal_peer::light_node::journal TODO=bound-or-reference-values-and-reclamation */ "UPDATE receipts SET payload=?3 WHERE epoch=?1 AND sequence=?2", params![receipt.epoch,receipt.receipt_sequence,payload])?;
    Ok(receipt)
}
fn settle(tx: &Transaction<'_>, command: &Command, outcome: Outcome, limits: &Limits) -> Result<Receipt> {
    let selection = epoch_selection(tx, command.binding.epoch)?;
    close_start(tx, "seat", command, &outcome)?;
    tx.execute(/* sqlite-large-value: allow-for-now lv-h58e17ebf7853 owner=fractal_peer::light_node::journal TODO=bound-or-reference-values-and-reclamation */ "UPDATE commands SET state='settled' WHERE side='seat' AND execution=?1", [&command.execution_id])?;
    append_receipt(tx, &selection, command, ReceiptState::Settled { outcome }, limits)
}

#[cfg(test)]
#[path = "journal_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "journal_ack_tests.rs"]
mod acknowledgement_tests;

fn encoded_digest(payload: &str) -> String {
    use sha2::{Digest as _, Sha256};
    hex::encode(Sha256::digest(payload.as_bytes()))
}

fn is_closing(connection: &Connection) -> Result<bool> {
    Ok(connection.query_row("SELECT closed=2 FROM epochs WHERE closed<>1",[],|row|row.get(0))?)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Offer { Admitted, WaitingForRevocation }

fn validate_execution(selection:&Selection,binding:&ExecutionBinding)->Result<()> {
    binding.validate()?;
    ensure!(binding.home==selection.home && binding.host==selection.host && binding.epoch==selection.epoch && selection.grant.subjects.contains(&binding.subject),"execution binding selection/scope is fenced");
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EpochInstall { Installed, Draining }

fn prepare_closing(tx:&Transaction<'_>,limits:&Limits)->Result<()> {
    let old=current(tx)?;
    tx.execute(/* sqlite-large-value: allow-for-now lv-h9568d516e2dc owner=fractal_peer::light_node::journal TODO=bound-or-reference-values-and-reclamation */ "UPDATE start_authorizations SET revoked=1 WHERE closed=0", [])?;
    tx.execute(/* sqlite-large-value: allow-for-now lv-h547eede63354 owner=fractal_peer::light_node::journal TODO=bound-or-reference-values-and-reclamation */ "UPDATE commands SET state=CASE side WHEN 'seat' THEN 'entered' ELSE 'sent' END WHERE state='settled' AND EXISTS(SELECT 1 FROM start_authorizations a WHERE a.side=commands.side AND a.execution=commands.execution AND a.closed=0)", [])?;
    for command in commands(tx,"seat",Some("admitted"))? {
        settle(tx,&command,Outcome::Cancelled {reason:"epoch replacement before backend entry".into()},limits)?;
    }
    local::revoke_epoch_tx(tx,&old.host,old.epoch)?;
    tx.execute(/* sqlite-large-value: allow-for-now lv-h682a40a9a666 owner=fractal_peer::light_node::journal TODO=bound-or-reference-values-and-reclamation */ "UPDATE commands SET cancel_reason='epoch replacement' WHERE cancel_reason IS NULL AND state IN ('queued','entered','sent','admitted')",[])?;
    tx.execute(/* sqlite-large-value: allow-for-now lv-hc5522da5d7ea owner=fractal_peer::light_node::journal TODO=bound-or-reference-values-and-reclamation */ "UPDATE epochs SET closed=2 WHERE closed=0",[])?;
    Ok(())
}

fn drain_ready(connection:&Connection)->Result<bool> {
    if !is_closing(connection)? {return Ok(false);}
    let selected=current(connection)?;
    let unresolved:u64=connection.query_row("SELECT (SELECT COUNT(*) FROM commands WHERE state<>'settled' OR local_finalization>0) + (SELECT COUNT(*) FROM receipts WHERE acknowledged=0) + (SELECT COUNT(*) FROM effects WHERE result IS NULL AND retired=0) + (SELECT COUNT(*) FROM reported WHERE epoch=?1 AND revoked=0)",[selected.epoch],|row|row.get(0))?;
    Ok(unresolved==0 && local::pending_changes(connection,selected.epoch)?.is_empty())
}

fn run_request_digest(request:&RunRequest)->Result<String> {
    Ok(encoded_digest(&serde_json::to_string(&(&request.assertion.binding,&request.assertion.request,&request.claim))?))
}

fn entry_refusal(connection:&Connection,command:&Command)->Result<Option<Outcome>> {
    if is_closing(connection)? {
        return Ok(Some(Outcome::Cancelled {reason:"epoch is closing before backend entry".into()}));
    }
    let cancelled:Option<String>=connection.query_row("SELECT cancel_reason FROM commands WHERE side='seat' AND execution=?1",[&command.execution_id],|row|row.get(0))?;
    if let Some(reason)=cancelled {return Ok(Some(Outcome::Cancelled {reason}));}
    if matches!(&command.operation, Operation::Start { .. }) {
        return Ok(require_start(connection, command).err().map(|error| Outcome::Refused { reason: error.to_string() }));
    }
    if let Err(error)=local::require_execution(connection,&command.binding) {
        if matches!(error.code,"local-execution-authority-refused" | "local-authority-backpressure") {
            return Ok(Some(Outcome::Refused {reason:format!("{}: {}",error.code,error.message)}));
        }
        return Err(error.into());
    }
    Ok(None)
}

// Additive schema preserves all authoritative Observe/Stop and credential rows.
const START_SCHEMA: &str = /* sqlite-large-value: allow-for-now lv-h74418ba2dde0 owner=fractal_peer::light_node::journal TODO=chunk-or-reference-variable-columns */ "
CREATE TABLE IF NOT EXISTS start_authorizations(
 side TEXT NOT NULL, epoch INTEGER NOT NULL, host TEXT NOT NULL, subject TEXT NOT NULL,
 revision INTEGER NOT NULL, digest TEXT NOT NULL, execution TEXT NOT NULL,
 revoked INTEGER NOT NULL DEFAULT 0, closed INTEGER NOT NULL DEFAULT 0,
 PRIMARY KEY(side,epoch,host,subject,revision), UNIQUE(side,execution));";

fn install_start(tx: &Transaction<'_>, side: &str, command: &Command, limits: &Limits) -> Result<()> {
    let Operation::Start { authorization } = &command.operation else { return Ok(()); };
    authorization.verify()?;
    let binding = &command.binding;
    ensure!(binding.run.is_none() && binding.authorization_revision == authorization.revision.to_string(),
        "Start seat launch binding must carry its exact authorization revision and no run authority");
    ensure!(authorization.member.host == binding.host && authorization.launch_member.host == binding.host
        && authorization.member.runtime_id == binding.runtime && authorization.launch_member.runtime_id == binding.runtime,
        "Start launch member differs from execution scope");
    let previous: Option<(u64, bool)> = tx.query_row(
        "SELECT revision,closed FROM start_authorizations WHERE side=?1 AND host=?2 AND subject=?3 ORDER BY epoch DESC,revision DESC LIMIT 1",
        params![side,binding.host,binding.subject], |row| Ok((row.get(0)?,row.get(1)?))).optional()?;
    if let Some((revision, closed)) = previous {
        ensure!(closed, "Start replacement waits for predecessor withdrawal receipt and custody closure");
        ensure!(authorization.revision > revision, "Start authorization revision is stale");
    }
    let count: usize = tx.query_row("SELECT COUNT(*) FROM start_authorizations", [], |row| row.get(0))?;
    ensure!(count < limits.max_identities, "backpressure: Start authorization fence capacity");
    tx.execute(/* sqlite-large-value: allow-for-now lv-h8d7c117df52a owner=fractal_peer::light_node::journal TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO start_authorizations(side,epoch,host,subject,revision,digest,execution) VALUES(?1,?2,?3,?4,?5,?6,?7)",
        params![side,binding.epoch,binding.host,binding.subject,authorization.revision,authorization.content_sha256,command.execution_id])?;
    Ok(())
}

/// Physical launch gates call this with the connection already held by `Journal::enter`.
/// It never calls a backend or offers run authority and cannot start from an ACK.
pub fn require_start(connection: &Connection, command: &Command) -> Result<()> {
    ensure!(same(&by_execution(connection, &command.execution_id)?, command)?, "Start command differs from durable admitted identity");
    validate(&current(connection)?, command)?;
    ensure!(!is_closing(connection)?, "Start epoch is closing");
    let Operation::Start { authorization } = &command.operation else { bail!("operation is not Start"); };
    authorization.verify()?;
    let (revision, digest, revoked, closed): (u64, String, bool, bool) = connection.query_row(
        "SELECT revision,digest,revoked,closed FROM start_authorizations WHERE side='seat' AND execution=?1",
        [&command.execution_id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)))?;
    ensure!(revision == authorization.revision && digest == authorization.content_sha256 && !revoked && !closed,
        "Start authorization is stale, revoked or content-mismatched");
    let cancellation: Option<String> = connection.query_row("SELECT cancel_reason FROM commands WHERE side='seat' AND execution=?1", [&command.execution_id], |row| row.get(0))?;
    ensure!(cancellation.is_none(), "Start was cancelled");
    Ok(())
}

fn close_start(connection: &Connection, side: &str, command: &Command, outcome: &Outcome) -> Result<()> {
    if matches!(&command.operation, Operation::Start { .. }) && !matches!(outcome, Outcome::Started { .. }) {
        connection.execute(/* sqlite-large-value: allow-for-now lv-hf18e8963c367 owner=fractal_peer::light_node::journal TODO=bound-or-reference-values-and-reclamation */ "UPDATE start_authorizations SET revoked=1,closed=1 WHERE side=?1 AND execution=?2", params![side,command.execution_id])?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "start_tests.rs"]
pub(super) mod start_tests;

fn close_starts_stopped_by(tx: &Transaction<'_>, stop: &Command, outcome: &Outcome, limits: &Limits) -> Result<()> {
    let Outcome::Stopped { incarnation } = outcome else { return Ok(()); };
    ensure!(incarnation == &stop.binding.incarnation, "Stop proof differs from exact target generation");
    let mut query = tx.prepare("SELECT c.payload FROM commands c JOIN start_authorizations a ON a.side=c.side AND a.execution=c.execution WHERE c.side='seat' AND a.closed=0 AND a.host=?1 AND a.subject=?2")?;
    let starts: Vec<Command> = query.query_map(params![stop.binding.host,stop.binding.subject], |row| row.get::<_,String>(0))?
        .map(|row| Ok(serde_json::from_str(&row?)?)).collect::<Result<_>>()?;
    drop(query);
    for start in starts {
        if start.binding.runtime != stop.binding.runtime { continue; }
        let receipt = last_receipt(tx, &start)?;
        if matches!(receipt.state, ReceiptState::Settled { outcome: Outcome::Started { incarnation: ref actual } } if actual == incarnation) {
            settle(tx, &start, outcome.clone(), limits)?;
        }
    }
    Ok(())
}
