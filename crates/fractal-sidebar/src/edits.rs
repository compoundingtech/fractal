//! Pure, unwired v2 edit lifecycle. Hosts run the planner against complete confirmed inputs,
//! including the visible intent overlay; acknowledgements never stand in for readback proof.
//! `Base::ready` attests both arrangement resolution and membership paging are complete/current.
//! A compound plan retains only the dependent identity: memberships must be replanned after folders.
use crate::{paging::{Scope, Snapshot}, plan::{MoveIntent, V2Plan}};
use fractal_schema::{arrangements, ordered_membership};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Identity { pub actor: String, pub action_id: String, pub idempotency_key: String }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Base { pub scope: Scope, pub snapshot: Snapshot, pub revision: String, pub ready: bool }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Operations { Folders(Vec<arrangements::Operation>), Memberships(Vec<ordered_membership::Operation>) }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request { pub identity: Identity, pub fence: Base, pub operations: Operations }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Status { Intent, AwaitingPersistence, Sending, UnknownDelivery, AwaitingObservation, AwaitingCompletionPersistence, AwaitingFreshBase, Observed, Refused, PartiallyApplied }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Refusal { InvalidPlan, StaleFence, Unauthorized, DestinationMissing, Conflict, Storage }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect { PersistPending, Send(Delivery), ReadBase, PersistCompletion, PersistRemoval, Replan }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Persistence { Pending, Completion, Removal }
/// Per-send provenance, never journaled. A restored request may already have applied.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Delivery { First, #[default] Replay }
pub enum Event {
    Plan { base: Base, plan: V2Plan, folder: Identity, membership: Identity },
    Persisted(Identity), CompletionPersisted(Identity), Ack(Identity), Unknown(Identity), Replay,
    /// Host proves the staged operations from authoritative state, not an action receipt.
    Observed(Identity), Refused(Identity, Refusal), StorageFailed { identity: Identity, phase: Persistence }, Retry,
    /// Replan memberships against this complete post-folder base; deleted destinations refuse.
    Fresh(Base, Result<Vec<ordered_membership::Operation>, Refusal>),
}
/// Version failures are distinct from corrupt records and records belonging to another scope.
#[derive(Debug)]
pub enum RestoreError { Version(u32), InvalidRecord(serde_json::Error), ScopeMismatch }
pub const JOURNAL_VERSION: u32 = 1;
/// Includes bounded request payload plus identity history and journal envelope.
pub const MAX_JOURNAL_BYTES: usize = 2 * ordered_membership::MAX_EDIT_BYTES;
#[derive(Serialize)]
struct Journal<'a> { version: u32, state: &'a Edits }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredJournal<'a> {
    version: u32,
    #[serde(borrow)]
    state: &'a serde_json::value::RawValue,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Edits {
    scope: Scope,
    intent: MoveIntent,
    pub status: Status,
    pub refusal: Option<Refusal>,
    request: Option<Request>,
    dependent: Option<Identity>,
    folder_completed: bool,
    used: Vec<Identity>,
    #[serde(skip)]
    next_delivery: Delivery,
}
fn invalid_record(message: &str) -> RestoreError {
    RestoreError::InvalidRecord(<serde_json::Error as serde::de::Error>::custom(message))
}
fn valid_identity(id: &Identity) -> bool {
    [&id.actor, &id.action_id, &id.idempotency_key].iter().all(|value| !value.trim().is_empty() && value.len() <= 512)
}
fn check_journal_size(size: usize) -> Result<(), serde_json::Error> {
    if size > MAX_JOURNAL_BYTES { return Err(<serde_json::Error as serde::de::Error>::custom("journal exceeds size limit")); }
    Ok(())
}
impl Edits {
    pub fn new(scope: Scope, intent: MoveIntent) -> Self {
        Self { scope, intent, status: Status::Intent, refusal: None, request: None, dependent: None, folder_completed: false, used: Vec::new(), next_delivery: Delivery::First }
    }
    pub fn scope(&self) -> &Scope { &self.scope }
    pub fn intent(&self) -> Option<&MoveIntent> { (self.status != Status::Observed).then_some(&self.intent) }
    /// Send/replay effects always borrow this immutable request; never regenerate its identity/fence.
    pub fn request(&self) -> Option<&Request> { self.request.as_ref() }
    /// Capture immediately for each persistence effect, before processing another event.
    /// The host atomically replaces the scoped record and acknowledges only that write.
    /// This contains identities and fences, never connection credentials or transport objects.
    /// Encoding errors are storage failures: report `StorageFailed` for that identity/phase,
    /// never a persistence acknowledgement. The same byte ceiling applies on recovery.
    pub fn journal(&self) -> Result<Vec<u8>, serde_json::Error> {
        let bytes = serde_json::to_vec(&Journal { version: JOURNAL_VERSION, state: self })?;
        check_journal_size(bytes.len())?;
        Ok(bytes)
    }
    /// Restore only host-owned durable bytes. New plans still require a fresh complete base.
    /// Sending is ambiguous after a crash: re-persist, then replay the immutable request.
    /// Observation/completion markers never replay an already completed step.
    pub fn restore(bytes: &[u8], scope: &Scope) -> Result<(Self, Vec<Effect>), RestoreError> {
        check_journal_size(bytes.len()).map_err(RestoreError::InvalidRecord)?;
        let journal: StoredJournal<'_> = serde_json::from_slice(bytes).map_err(RestoreError::InvalidRecord)?;
        if journal.version != JOURNAL_VERSION { return Err(RestoreError::Version(journal.version)); }
        let mut state: Self = serde_json::from_str(journal.state.get()).map_err(RestoreError::InvalidRecord)?;
        if state.scope != *scope {
            return Err(RestoreError::ScopeMismatch);
        }
        state.validate_record()?;
        let effects = match state.status {
            Status::AwaitingPersistence | Status::Sending | Status::UnknownDelivery => {
                state.status = Status::AwaitingPersistence; vec![Effect::PersistPending]
            }
            Status::AwaitingCompletionPersistence => vec![Effect::PersistCompletion],
            Status::Observed => vec![Effect::PersistRemoval],
            Status::Intent => vec![Effect::ReadBase, Effect::Replan],
            Status::AwaitingObservation | Status::AwaitingFreshBase | Status::Refused | Status::PartiallyApplied => vec![Effect::ReadBase],
        };
        Ok((state, effects))
    }
    fn validate_record(&self) -> Result<(), RestoreError> {
        let invalid = || invalid_record("inconsistent edit journal");
        let mut actions = std::collections::BTreeSet::new();
        let mut keys = std::collections::BTreeSet::new();
        for id in &self.used {
            if !valid_identity(id) || !actions.insert(&id.action_id) || !keys.insert(&id.idempotency_key) { return Err(invalid()); }
        }
        if self.dependent.as_ref().is_some_and(|id| !self.unused(id)) { return Err(invalid()); }
        let folders = self.request.as_ref().is_some_and(|r| matches!(r.operations, Operations::Folders(_)));
        if let Some(request) = &self.request {
            if !self.valid_base(&request.fence) || !self.used.contains(&request.identity)
                || arrangements::validate_actor(&self.scope.container, Some(&request.identity.actor)).is_err() { return Err(invalid()); }
            let count = match &request.operations { Operations::Folders(ops) => ops.len(), Operations::Memberships(ops) => ops.len() };
            if count == 0 || count > ordered_membership::MAX_OPERATIONS { return Err(invalid()); }
            if let Operations::Folders(ops) = &request.operations
                && ops.iter().any(|op| !matches!(op, arrangements::Operation::FolderCreate { .. } | arrangements::Operation::FolderRename { .. } | arrangements::Operation::FolderMove { .. } | arrangements::Operation::FolderDelete { .. })) { return Err(invalid()); }
            let operations = match &request.operations {
                Operations::Folders(ops) => serde_json::to_value(ops),
                Operations::Memberships(ops) => serde_json::to_value(ops),
            }.map_err(RestoreError::InvalidRecord)?;
            let fields = std::collections::BTreeMap::from([
                ("owner".into(), serde_json::json!(self.scope.owner)),
                ("version".into(), serde_json::json!(2)), ("operations".into(), operations),
            ]);
            let admitted = match &request.operations {
                Operations::Folders(_) => arrangements::operations(&self.scope.container, &fields).map(|_| ()),
                Operations::Memberships(_) => ordered_membership::operations(&self.scope.container, &fields).map(|_| ()),
            };
            if admitted.is_err() { return Err(invalid()); }
        } else if !self.used.is_empty() || self.folder_completed { return Err(invalid()); }
        let valid = match self.status {
            Status::Intent => self.refusal.is_none() && self.dependent.is_none(),
            Status::Refused => self.refusal.is_some() && !self.folder_completed,
            Status::PartiallyApplied => self.refusal.is_some() && self.folder_completed,
            Status::AwaitingFreshBase => self.refusal.is_none() && folders && self.folder_completed && self.dependent.is_some(),
            Status::AwaitingCompletionPersistence | Status::Observed => self.refusal.is_none() && self.request.is_some()
                && (!folders || self.folder_completed) && (self.status != Status::Observed || self.dependent.is_none()),
            Status::AwaitingPersistence | Status::Sending | Status::UnknownDelivery | Status::AwaitingObservation =>
                self.refusal.is_none() && self.request.is_some(),
        };
        if !valid || (self.dependent.is_some() && !folders) { return Err(invalid()); }
        Ok(())
    }
    fn valid_base(&self, base: &Base) -> bool {
        base.ready && base.scope == self.scope && base.snapshot.host_id == self.scope.host && base.snapshot.projection_version == self.scope.projection
    }
    fn unused(&self, id: &Identity) -> bool {
        valid_identity(id) && !self.used.iter().any(|used| used.action_id == id.action_id || used.idempotency_key == id.idempotency_key)
    }
    fn refuse(&mut self, reason: Refusal) -> Vec<Effect> {
        self.refusal = Some(reason); self.status = if self.folder_completed { Status::PartiallyApplied } else { Status::Refused };
        vec![Effect::PersistPending, Effect::ReadBase]
    }
    fn stage(&mut self, identity: Identity, fence: Base, operations: Operations) -> Vec<Effect> {
        if !self.unused(&identity) { return self.refuse(Refusal::InvalidPlan); }
        self.used.push(identity.clone()); self.request = Some(Request { identity, fence, operations });
        self.next_delivery = Delivery::First;
        self.status = Status::AwaitingPersistence; vec![Effect::PersistPending]
    }
    pub fn reduce(&mut self, event: Event) -> Vec<Effect> {
        match event {
            Event::Plan { base, plan, folder, membership } if self.status == Status::Intent => {
                if !self.valid_base(&base) { return vec![Effect::ReadBase]; }
                if self.request.as_ref().is_some_and(|previous| base.snapshot.id == previous.fence.snapshot.id || base.snapshot.store_index < previous.fence.snapshot.store_index) { return vec![Effect::ReadBase]; }
                if plan.folders.iter().any(|operation| !matches!(operation, arrangements::Operation::FolderCreate { .. } | arrangements::Operation::FolderRename { .. } | arrangements::Operation::FolderMove { .. } | arrangements::Operation::FolderDelete { .. })) { return self.refuse(Refusal::InvalidPlan); }
                if plan.folders.is_empty() && plan.memberships.is_empty() { return self.refuse(Refusal::InvalidPlan); }
                if !plan.folders.is_empty() {
                    if !plan.memberships.is_empty() {
                        if !self.unused(&membership) || folder.action_id == membership.action_id || folder.idempotency_key == membership.idempotency_key { return self.refuse(Refusal::InvalidPlan); }
                        self.dependent = Some(membership);
                    }
                    return self.stage(folder, base, Operations::Folders(plan.folders));
                }
                self.stage(membership, base, Operations::Memberships(plan.memberships))
            }
            Event::Fresh(base, plan) if self.status == Status::AwaitingFreshBase => {
                if !self.valid_base(&base) { return vec![Effect::ReadBase]; }
                let previous = &self.request.as_ref().expect("completed folder request").fence;
                if base.snapshot.id == previous.snapshot.id || base.revision == previous.revision || base.snapshot.store_index < previous.snapshot.store_index { return vec![Effect::ReadBase]; }
                let operations = match plan { Ok(operations) if !operations.is_empty() => operations, Ok(_) => return self.refuse(Refusal::InvalidPlan), Err(reason) => return self.refuse(reason) };
                let identity = self.dependent.take().expect("dependent membership identity");
                self.stage(identity, base, Operations::Memberships(operations))
            }
            Event::Retry if matches!(self.status, Status::Refused | Status::PartiallyApplied) => {
                self.status = Status::Intent; self.refusal = None; self.dependent = None; vec![Effect::ReadBase, Effect::Replan]
            }
            Event::StorageFailed { identity, phase } if self.request.as_ref().is_some_and(|request| request.identity == identity)
                && matches!((self.status, phase), (Status::AwaitingPersistence, Persistence::Pending) | (Status::AwaitingCompletionPersistence, Persistence::Completion)) => self.refuse(Refusal::Storage),
            Event::Replay if self.status == Status::UnknownDelivery => { self.status = Status::Sending; vec![Effect::Send(Delivery::Replay)] }
            Event::Persisted(id) if self.status == Status::AwaitingPersistence && self.request.as_ref().is_some_and(|request| request.identity == id) => {
                self.status = Status::Sending; vec![Effect::Send(std::mem::replace(&mut self.next_delivery, Delivery::Replay))]
            }
            Event::CompletionPersisted(id) if self.status == Status::AwaitingCompletionPersistence && self.request.as_ref().is_some_and(|request| request.identity == id) => {
                if self.dependent.is_some() { self.status = Status::AwaitingFreshBase; vec![Effect::ReadBase] }
                else { self.status = Status::Observed; vec![Effect::PersistRemoval] }
            }
            Event::Ack(id) | Event::Unknown(id) | Event::Observed(id) | Event::Refused(id, _) if self.request.as_ref().is_none_or(|request| request.identity != id) => Vec::new(),
            Event::Ack(_) if matches!(self.status, Status::Sending | Status::UnknownDelivery) => { self.status = Status::AwaitingObservation; vec![Effect::PersistPending, Effect::ReadBase] }
            Event::Unknown(_) if self.status == Status::Sending => { self.status = Status::UnknownDelivery; vec![Effect::PersistPending] }
            Event::Observed(_) if matches!(self.status, Status::Sending | Status::UnknownDelivery | Status::AwaitingObservation) => {
                if matches!(self.request.as_ref().unwrap().operations, Operations::Folders(_)) { self.folder_completed = true; }
                self.status = Status::AwaitingCompletionPersistence; vec![Effect::PersistCompletion]
            }
            Event::Refused(_, reason) if matches!(self.status, Status::Sending | Status::UnknownDelivery | Status::AwaitingObservation) => self.refuse(reason),
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{NodeRef, Parent};
    const FOLDER: &str = "01900000-0000-7000-8000-000000000001";
    fn scope() -> Scope { Scope { owner: "person/test".into(), container: format!("arrangement/person/test/{FOLDER}"), endpoint: "endpoint-a".into(), host: "host-a".into(), projection: "1".into() } }
    fn base(n: u64) -> Base { Base { scope: scope(), snapshot: Snapshot { id: format!("snapshot-{n}"), host_id: "host-a".into(), store_index: n, projection_version: "1".into(), created_at: "created".into(), published_at: None }, revision: format!("revision-{n}"), ready: true } }
    fn id(n: u64) -> Identity { Identity { actor: "person/test".into(), action_id: format!("action-{n}"), idempotency_key: format!("key-{n}") } }
    fn members() -> Vec<ordered_membership::Operation> { vec![ordered_membership::Operation::Place { member: "agent/fleet/fixture-sidebar/member-a".into(), bucket: Some(FOLDER.into()), key: "a0".into() }] }
    fn plan(folder: bool, member: bool) -> V2Plan { V2Plan { folders: if folder { vec![arrangements::Operation::FolderRename { id: FOLDER.into(), name: "Folder".into() }] } else { vec![] }, memberships: if member { members() } else { vec![] } } }
    fn state() -> Edits { Edits::new(scope(), MoveIntent { node: NodeRef::Subject { subject: "member-a".into() }, parent: Parent::Folder { id: "folder-a".into() }, before: None }) }
    fn stage(state: &mut Edits, folder: bool, member: bool) {
        assert_eq!(state.reduce(Event::Plan { base: base(1), plan: plan(folder, member), folder: id(1), membership: id(2) }), vec![Effect::PersistPending]);
        assert_eq!(state.status, Status::AwaitingPersistence);
    }
    #[test]
    fn folder_first_requires_observation_persistence_and_fresh_fences() {
        let mut state = state(); stage(&mut state, true, true);
        assert!(matches!(state.request().unwrap().operations, Operations::Folders(_)));
        assert_eq!(state.reduce(Event::Persisted(id(1))), vec![Effect::Send(Delivery::First)]);
        assert_eq!(state.reduce(Event::Ack(id(1))), vec![Effect::PersistPending, Effect::ReadBase]);
        assert!(state.intent().is_some()); assert!(state.reduce(Event::Fresh(base(2), Ok(members()))).is_empty());
        assert_eq!(state.reduce(Event::Observed(id(1))), vec![Effect::PersistCompletion]);
        assert!(state.reduce(Event::Fresh(base(2), Ok(members()))).is_empty());
        assert!(state.reduce(Event::Persisted(id(1))).is_empty());
        assert_eq!(state.reduce(Event::CompletionPersisted(id(1))), vec![Effect::ReadBase]);
        for field in 0..6 {
            let mut stale = base(2);
            match field { 0 => stale.snapshot.id = base(1).snapshot.id, 1 => stale.revision = base(1).revision, 2 => stale.ready = false, 3 => stale.scope.owner.push('b'), 4 => stale.snapshot.store_index = 0, _ => stale.snapshot.host_id.push('b') }
            assert_eq!(state.reduce(Event::Fresh(stale, Ok(members()))), vec![Effect::ReadBase]);
            assert_eq!(state.status, Status::AwaitingFreshBase);
        }
        assert_eq!(state.reduce(Event::Fresh(base(2), Ok(members()))), vec![Effect::PersistPending]);
        assert!(matches!(state.request().unwrap().operations, Operations::Memberships(_)));
        assert_eq!(state.request().unwrap().fence, base(2));
        assert!(state.reduce(Event::Persisted(id(1))).is_empty());
        assert_eq!(state.reduce(Event::Persisted(id(2))), vec![Effect::Send(Delivery::First)]);
        state.reduce(Event::Ack(id(2))); assert!(state.intent().is_some());
        state.reduce(Event::Observed(id(2))); assert!(state.intent().is_some());
        assert_eq!(state.reduce(Event::CompletionPersisted(id(2))), vec![Effect::PersistRemoval]);
        assert_eq!(state.status, Status::Observed); assert!(state.intent().is_none());
    }
    #[test]
    fn unknown_replays_exact_request_while_refusal_never_replays() {
        for folder in [false, true] {
            let mut state = state(); stage(&mut state, folder, !folder); let identity = if folder { id(1) } else { id(2) };
            assert!(state.reduce(Event::Replay).is_empty());
            state.reduce(Event::Persisted(identity.clone())); let exact = state.request().cloned().unwrap();
            for _ in 0..4 {
                assert_eq!(state.reduce(Event::Unknown(identity.clone())), vec![Effect::PersistPending]);
                assert_eq!(state.status, Status::UnknownDelivery);
                assert_eq!(state.reduce(Event::Replay), vec![Effect::Send(Delivery::Replay)]); assert_eq!(state.request(), Some(&exact));
            }
            state.reduce(Event::Refused(identity.clone(), Refusal::StaleFence));
            assert_eq!(state.status, Status::Refused); assert!(state.reduce(Event::Replay).is_empty());
            assert!(state.reduce(Event::Ack(identity)).is_empty()); assert_eq!(state.request(), Some(&exact));
            assert_eq!(state.reduce(Event::Retry), vec![Effect::ReadBase, Effect::Replan]);
            state.reduce(Event::Plan { base: base(2), plan: plan(folder, !folder), folder: id(1), membership: id(2) });
            assert_eq!(state.refusal, Some(Refusal::InvalidPlan));
        }
    }
    #[test]
    fn partial_application_and_deleted_destination_are_visible() {
        for failed_destination in [false, true] {
            let mut state = state(); stage(&mut state, true, true);
            state.reduce(Event::Persisted(id(1))); state.reduce(Event::Observed(id(1))); state.reduce(Event::CompletionPersisted(id(1)));
            if failed_destination { state.reduce(Event::Fresh(base(2), Err(Refusal::DestinationMissing))); }
            else {
                state.reduce(Event::Fresh(base(2), Ok(members()))); state.reduce(Event::Persisted(id(2)));
                state.reduce(Event::Refused(id(2), Refusal::Unauthorized));
            }
            assert_eq!(state.status, Status::PartiallyApplied); assert!(state.intent().is_some());
            assert!(state.reduce(Event::Replay).is_empty()); assert!(state.folder_completed);
        }
    }
    #[test]
    fn stale_bases_keep_intent_and_late_callbacks_cannot_advance_steps() {
        for field in 0..5 {
            let mut state = state(); let mut stale = base(1);
            match field { 0 => stale.ready = false, 1 => stale.scope.container.push('b'), 2 => stale.scope.endpoint.push('b'), 3 => stale.snapshot.host_id.push('b'), _ => stale.snapshot.projection_version.push('b') }
            assert_eq!(state.reduce(Event::Plan { base: stale, plan: plan(false, true), folder: id(1), membership: id(2) }), vec![Effect::ReadBase]);
            assert_eq!(state.status, Status::Intent); assert!(state.intent().is_some()); assert!(state.request().is_none());
            stage(&mut state, false, true);
            for event in [Event::Ack(id(99)), Event::Unknown(id(99)), Event::Observed(id(99)), Event::Refused(id(99), Refusal::Conflict), Event::Persisted(id(99))] { assert!(state.reduce(event).is_empty()); }
            assert_eq!(state.status, Status::AwaitingPersistence);
            state.reduce(Event::StorageFailed { identity: id(2), phase: Persistence::Pending }); assert_eq!(state.refusal, Some(Refusal::Storage));
            assert!(state.reduce(Event::Persisted(id(2))).is_empty()); assert!(state.reduce(Event::Replay).is_empty());
        }
    }
    #[test]
    fn observation_is_authority_and_duplicate_callbacks_do_not_cross_barriers() {
        for phase in 0..3 {
            let mut state = state(); stage(&mut state, false, true);
            assert!(state.reduce(Event::Observed(id(2))).is_empty());
            assert!(state.reduce(Event::CompletionPersisted(id(2))).is_empty());
            state.reduce(Event::Persisted(id(2)));
            if phase == 1 { state.reduce(Event::Unknown(id(2))); }
            if phase == 2 { state.reduce(Event::Ack(id(2))); }
            assert_eq!(state.reduce(Event::Observed(id(2))), vec![Effect::PersistCompletion]);
            assert!(state.reduce(Event::Persisted(id(2))).is_empty());
            assert!(state.reduce(Event::Ack(id(2))).is_empty());
            assert!(state.reduce(Event::Unknown(id(2))).is_empty());
            assert!(state.reduce(Event::Refused(id(2), Refusal::Conflict)).is_empty());
            assert!(state.reduce(Event::CompletionPersisted(id(99))).is_empty());
            assert!(state.intent().is_some());
            state.reduce(Event::CompletionPersisted(id(2))); assert!(state.intent().is_none());
            assert!(state.reduce(Event::CompletionPersisted(id(2))).is_empty());
        }
    }
    #[test]
    fn invalid_plans_and_reused_identities_refuse() {
        for case in 0..5 {
            let mut state = state(); let mut input = plan(true, true); let mut folder = id(1); let mut membership = id(2);
            match case {
                0 => { input.folders.clear(); input.memberships.clear(); }
                1 => membership.action_id.clone_from(&folder.action_id),
                2 => membership.idempotency_key.clone_from(&folder.idempotency_key),
                3 => folder.action_id.clear(),
                _ => input.folders = vec![arrangements::Operation::SubjectPlace { subject: "member-a".into(), folder: None, key: "a0".into() }],
            }
            state.reduce(Event::Plan { base: base(1), plan: input, folder, membership });
            assert_eq!(state.refusal, Some(Refusal::InvalidPlan)); assert!(state.request().is_none());
        }
    }
    #[test]
    fn explicit_retry_requires_a_new_confirmed_base_and_new_identities() {
        let mut state = state(); stage(&mut state, false, true);
        state.reduce(Event::Persisted(id(2))); state.reduce(Event::Refused(id(2), Refusal::StaleFence));
        assert!(state.reduce(Event::Fresh(base(2), Ok(members()))).is_empty());
        state.reduce(Event::Retry);
        assert_eq!(state.reduce(Event::Plan { base: base(1), plan: plan(false, true), folder: id(3), membership: id(4) }), vec![Effect::ReadBase]);
        assert_eq!(state.status, Status::Intent);
        assert_eq!(state.reduce(Event::Plan { base: base(2), plan: plan(false, true), folder: id(3), membership: id(4) }), vec![Effect::PersistPending]);
        assert_eq!(state.request().unwrap().identity, id(4)); assert_eq!(state.request().unwrap().fence, base(2));
        assert!(state.reduce(Event::Ack(id(2))).is_empty());
    }
    #[test]
    fn retry_revalidation_need_not_change_the_opaque_subject_revision() {
        let mut state = state(); stage(&mut state, false, true);
        state.reduce(Event::Persisted(id(2))); state.reduce(Event::Refused(id(2), Refusal::Unauthorized));
        state.reduce(Event::Retry); let mut fresh = base(2); fresh.revision = base(1).revision;
        assert_eq!(state.reduce(Event::Plan { base: fresh, plan: plan(false, true), folder: id(3), membership: id(4) }), vec![Effect::PersistPending]);
        assert_eq!(state.request().unwrap().identity, id(4));
    }
    #[test]
    fn folder_only_finishes_and_completion_storage_failure_preserves_partial_status() {
        for storage_failed in [false, true] {
            let mut state = state(); stage(&mut state, true, false);
            state.reduce(Event::Persisted(id(1))); state.reduce(Event::Ack(id(1)));
            state.reduce(Event::Observed(id(1)));
            if storage_failed {
                state.reduce(Event::StorageFailed { identity: id(1), phase: Persistence::Completion });
                assert_eq!(state.status, Status::PartiallyApplied); assert!(state.intent().is_some());
                assert!(state.reduce(Event::CompletionPersisted(id(1))).is_empty());
            } else {
                assert_eq!(state.reduce(Event::CompletionPersisted(id(1))), vec![Effect::PersistRemoval]);
                assert_eq!(state.status, Status::Observed); assert!(state.intent().is_none());
            }
            assert!(state.reduce(Event::Replay).is_empty());
        }
    }
    #[test]
    fn journal_crash_replay_requires_matching_persistence_and_keeps_exact_request() {
        for phase in 0..4 {
            let mut original = state(); stage(&mut original, false, true);
            if phase > 0 { original.reduce(Event::Persisted(id(2))); }
            if phase == 2 { original.reduce(Event::Unknown(id(2))); }
            if phase == 3 { original.reduce(Event::Ack(id(2))); }
            let bytes = original.journal().unwrap();
            let (mut restored, effects) = Edits::restore(&bytes, &scope()).unwrap();
            assert_eq!(restored.request(), original.request()); assert_eq!(restored.intent(), original.intent());
            assert!(restored.reduce(Event::Replay).is_empty());
            if phase == 3 {
                assert_eq!(effects, vec![Effect::ReadBase]);
                assert_eq!(restored.status, Status::AwaitingObservation);
                assert!(restored.reduce(Event::Persisted(id(2))).is_empty());
            } else {
                assert_eq!(effects, vec![Effect::PersistPending]);
                for event in [Event::Persisted(id(99)), Event::Ack(id(2)), Event::Observed(id(2)), Event::CompletionPersisted(id(2))] {
                    assert!(restored.reduce(event).is_empty());
                }
                assert_eq!(restored.reduce(Event::Persisted(id(2))), vec![Effect::Send(Delivery::Replay)]);
                assert_eq!(restored.request(), original.request());
                assert!(restored.reduce(Event::Persisted(id(2))).is_empty());
            }
        }
    }
    #[test]
    fn journal_partial_completion_survives_every_dependent_step_boundary() {
        let mut original = state(); stage(&mut original, true, true);
        original.reduce(Event::Persisted(id(1))); original.reduce(Event::Observed(id(1)));
        let (mut restored, effects) = Edits::restore(&original.journal().unwrap(), &scope()).unwrap();
        assert_eq!(effects, vec![Effect::PersistCompletion]); assert!(restored.folder_completed);
        assert!(restored.reduce(Event::Replay).is_empty());
        assert!(restored.reduce(Event::Fresh(base(2), Ok(members()))).is_empty());
        assert_eq!(restored.reduce(Event::CompletionPersisted(id(1))), vec![Effect::ReadBase]);
        let (mut restored, effects) = Edits::restore(&restored.journal().unwrap(), &scope()).unwrap();
        assert_eq!(effects, vec![Effect::ReadBase]); assert_eq!(restored.status, Status::AwaitingFreshBase);
        assert!(restored.reduce(Event::CompletionPersisted(id(1))).is_empty());
        assert_eq!(restored.reduce(Event::Fresh(base(1), Ok(members()))), vec![Effect::ReadBase]);
        assert_eq!(restored.reduce(Event::Fresh(base(2), Ok(members()))), vec![Effect::PersistPending]);
        let exact = restored.request().cloned().unwrap();
        let (mut restored, effects) = Edits::restore(&restored.journal().unwrap(), &scope()).unwrap();
        assert_eq!(effects, vec![Effect::PersistPending]); assert!(restored.folder_completed);
        assert_eq!(restored.request(), Some(&exact)); assert_eq!(exact.identity, id(2));
        assert_eq!(exact.fence, base(2)); assert!(matches!(exact.operations, Operations::Memberships(_)));
        assert!(restored.reduce(Event::Persisted(id(1))).is_empty());
        restored.reduce(Event::Persisted(id(2))); restored.reduce(Event::Refused(id(2), Refusal::Unauthorized));
        let (mut restored, _) = Edits::restore(&restored.journal().unwrap(), &scope()).unwrap();
        assert_eq!(restored.status, Status::PartiallyApplied); assert_eq!(restored.refusal, Some(Refusal::Unauthorized));
        assert!(restored.intent().is_some()); assert!(restored.reduce(Event::Replay).is_empty());
    }
    #[test]
    fn journal_observed_steps_never_reapply_even_if_removal_was_not_persisted() {
        let mut original = state(); stage(&mut original, false, true);
        original.reduce(Event::Persisted(id(2))); original.reduce(Event::Observed(id(2)));
        let (mut restored, effects) = Edits::restore(&original.journal().unwrap(), &scope()).unwrap();
        assert_eq!(effects, vec![Effect::PersistCompletion]); assert!(restored.reduce(Event::Replay).is_empty());
        assert_eq!(restored.reduce(Event::CompletionPersisted(id(2))), vec![Effect::PersistRemoval]);
        let (mut restored, effects) = Edits::restore(&restored.journal().unwrap(), &scope()).unwrap();
        assert_eq!(effects, vec![Effect::PersistRemoval]); assert!(restored.intent().is_none());
        for event in [Event::Replay, Event::Persisted(id(2)), Event::Observed(id(2)), Event::CompletionPersisted(id(2))] {
            assert!(restored.reduce(event).is_empty());
        }
    }
    #[test]
    fn journal_refuses_versions_before_decoding_state_and_rejects_wrong_scope() {
        for version in [0, JOURNAL_VERSION + 1, u32::MAX] {
            let bytes = format!(r#"{{"version":{version},"state":{{"future":"unknown shape"}}}}"#);
            assert!(matches!(Edits::restore(bytes.as_bytes(), &scope()), Err(RestoreError::Version(found)) if found == version));
        }
        let mut original = state(); stage(&mut original, false, true);
        for field in 0..5 {
            let mut other = scope();
            match field { 0 => other.owner.push('b'), 1 => other.container.push('b'), 2 => other.endpoint.push('b'), 3 => other.host.push('b'), _ => other.projection.push('b') }
            assert!(matches!(Edits::restore(&original.journal().unwrap(), &other), Err(RestoreError::ScopeMismatch)));
        }
        assert!(matches!(Edits::restore(br#"{"version":1,"state":{}}"#, &scope()), Err(RestoreError::InvalidRecord(_))));
        assert!(matches!(Edits::restore(b"not json", &scope()), Err(RestoreError::InvalidRecord(_))));
    }
    #[test]
    fn late_pending_failure_cannot_refuse_observed_completion() {
        let mut state = state(); stage(&mut state, false, true);
        state.reduce(Event::Persisted(id(2))); state.reduce(Event::Ack(id(2))); state.reduce(Event::Observed(id(2)));
        for (identity, phase) in [(id(2), Persistence::Pending), (id(99), Persistence::Completion), (id(2), Persistence::Removal)] {
            assert!(state.reduce(Event::StorageFailed { identity, phase }).is_empty());
            assert_eq!(state.status, Status::AwaitingCompletionPersistence); assert_eq!(state.refusal, None);
        }
        assert_eq!(state.reduce(Event::CompletionPersisted(id(2))), vec![Effect::PersistRemoval]);
        assert!(state.reduce(Event::StorageFailed { identity: id(2), phase: Persistence::Removal }).is_empty());
        assert_eq!(state.status, Status::Observed);
    }
    #[test]
    fn late_folder_storage_failure_cannot_refuse_dependent_membership_barrier() {
        let mut state = state(); stage(&mut state, true, true);
        state.reduce(Event::Persisted(id(1))); state.reduce(Event::Observed(id(1))); state.reduce(Event::CompletionPersisted(id(1)));
        state.reduce(Event::Fresh(base(2), Ok(members())));
        for phase in [Persistence::Pending, Persistence::Completion, Persistence::Removal] {
            assert!(state.reduce(Event::StorageFailed { identity: id(1), phase }).is_empty());
            assert_eq!(state.status, Status::AwaitingPersistence); assert_eq!(state.refusal, None);
        }
        assert_eq!(state.reduce(Event::Persisted(id(2))), vec![Effect::Send(Delivery::First)]);
    }
    fn refused_record(change: impl FnOnce(&mut serde_json::Value)) {
        let mut original = state(); stage(&mut original, true, true);
        let mut record: serde_json::Value = serde_json::from_slice(&original.journal().unwrap()).unwrap();
        change(&mut record);
        assert!(matches!(Edits::restore(&serde_json::to_vec(&record).unwrap(), &scope()), Err(RestoreError::InvalidRecord(_))));
    }
    #[test]
    fn journal_rejects_missing_requests_in_active_and_complete_states() {
        for status in ["AwaitingPersistence", "Sending", "UnknownDelivery", "AwaitingObservation", "AwaitingCompletionPersistence", "AwaitingFreshBase", "Observed"] {
            refused_record(|r| { r["state"]["status"] = status.into(); r["state"]["request"] = serde_json::Value::Null; });
        }
    }
    #[test]
    fn journal_rejects_fresh_base_without_dependency_or_completion() {
        for field in ["dependent", "folder_completed"] {
            refused_record(|r| { r["state"]["status"] = "AwaitingFreshBase".into(); r["state"][field] = if field == "dependent" { serde_json::Value::Null } else { false.into() }; });
        }
    }
    #[test]
    fn journal_rejects_invalid_refusal_and_completion_statuses() {
        for status in ["Intent", "Refused", "PartiallyApplied", "Observed", "AwaitingCompletionPersistence"] {
            refused_record(|r| { r["state"]["status"] = status.into(); });
        }
    }
    #[test]
    fn journal_rejects_duplicate_empty_or_missing_used_identities() {
        refused_record(|r| { let id = r["state"]["used"][0].clone(); r["state"]["used"].as_array_mut().unwrap().push(id); });
        for field in ["actor", "action_id", "idempotency_key"] { refused_record(|r| { r["state"]["used"][0][field] = "".into(); }); }
        refused_record(|r| { r["state"]["used"] = serde_json::json!([]); });
    }
    #[test]
    fn journal_rejects_mismatched_snapshot_fences() {
        for field in ["host_id", "projection_version"] { refused_record(|r| { r["state"]["request"]["fence"]["snapshot"][field] = "wrong".into(); }); }
        refused_record(|r| { r["state"]["request"]["fence"]["scope"]["host"] = "wrong".into(); });
    }
    #[test]
    fn journal_rejects_oversized_input_before_parsing() {
        assert!(matches!(Edits::restore(&vec![b' '; MAX_JOURNAL_BYTES + 1], &scope()), Err(RestoreError::InvalidRecord(_))));
    }
    #[test]
    fn journal_rejects_duplicate_and_excessive_operations() {
        for count in [2, ordered_membership::MAX_OPERATIONS + 1] {
            refused_record(|r| { let op = r["state"]["request"]["operations"]["Folders"][0].clone(); r["state"]["request"]["operations"]["Folders"] = serde_json::json!(vec![op; count]); });
        }
    }
    #[test]
    fn journal_rejects_oversized_operation_bytes_and_invalid_admission() {
        refused_record(|r| { r["state"]["request"]["operations"]["Folders"][0]["name"] = "x".repeat(ordered_membership::MAX_EDIT_BYTES).into(); });
        refused_record(|r| { r["state"]["request"]["operations"]["Folders"][0]["id"] = "invalid".into(); });
    }
    #[test]
    fn journal_rejects_unknown_nested_fields_and_invalid_versions() {
        for path in ["/state/request", "/state/request/identity", "/state/request/fence"] { refused_record(|r| { r.pointer_mut(path).unwrap()["extra"] = true.into(); }); }
        refused_record(|r| { r.as_object_mut().unwrap().remove("version"); });
        refused_record(|r| { r["version"] = "1".into(); });
    }
    #[test]
    fn oversized_encoding_fails_the_persistence_barrier_without_sending() {
        for phase in [Persistence::Pending, Persistence::Completion] {
            let mut state = state(); stage(&mut state, false, true);
            if phase == Persistence::Completion { state.reduce(Event::Persisted(id(2))); state.reduce(Event::Observed(id(2))); }
            state.used.extend((3..40_000).map(id));
            assert!(state.journal().is_err());
            state.reduce(Event::StorageFailed { identity: id(2), phase });
            assert_eq!(state.refusal, Some(Refusal::Storage));
            assert!(state.reduce(Event::Persisted(id(2))).is_empty());
            assert!(state.reduce(Event::CompletionPersisted(id(2))).is_empty());
            assert!(state.reduce(Event::Replay).is_empty());
        }
    }
    #[test]
    fn journal_at_exact_size_ceiling_round_trips() {
        let mut state = state(); stage(&mut state, false, true);
        state.intent.before = Some(NodeRef::Subject { subject: String::new() });
        let padding = MAX_JOURNAL_BYTES - state.journal().unwrap().len();
        state.intent.before = Some(NodeRef::Subject { subject: "a".repeat(padding) });
        let bytes = state.journal().unwrap(); assert_eq!(bytes.len(), MAX_JOURNAL_BYTES);
        let (restored, effects) = Edits::restore(&bytes, &scope()).unwrap();
        assert_eq!(restored.request(), state.request()); assert_eq!(restored.intent(), state.intent());
        assert_eq!(effects, vec![Effect::PersistPending]); assert_eq!(restored.journal().unwrap(), bytes);
    }
}
