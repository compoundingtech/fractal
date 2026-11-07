//! Unwired host executor. Activation must serialize events per scope and supply a client
//! authenticated as the staged actor. Endpoint identities must be credential-free.
use fractal_sidebar::{client, edits::{Edits, Effect, Event}, paging::Scope};
use fractal_client::{ActionRequest, ActionResult, ClientError, Envelope};
use std::{collections::VecDeque, fs, io::{self, Write}, os::unix::fs::OpenOptionsExt, path::PathBuf};

/// Dedicated live-loop inputs, not resource-window updates. Tokens reject obsolete reads.
pub enum LiveEvent {
    Membership(fractal_sidebar::paging::Event),
    Edit { scope: Scope, event: Event },
}
#[derive(Debug)]
pub enum Error {
    Storage(io::Error), Codec(serde_json::Error), Restore(fractal_sidebar::edits::RestoreError),
    NoStateDirectory, ActorMismatch, ScopeMismatch,
}
pub struct Store { path: PathBuf, scope: Scope, pub edits: Edits, blocked: bool }
impl Store {
    pub fn path(scope: &Scope) -> Result<PathBuf, Error> {
        let (dir, _) = super::glass_store::person_state(&scope.owner).ok_or(Error::NoStateDirectory)?;
        Ok(dir.join(Self::filename(scope)?))
    }
    fn filename(scope: &Scope) -> Result<String, Error> {
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(serde_json::to_vec(scope).map_err(Error::Codec)?);
        Ok(format!("sidebar-journal-v1-{digest:x}.json"))
    }
    pub fn new(path: PathBuf, edits: Edits) -> Self { Self { path, scope: edits.scope().clone(), edits, blocked: false } }
    /// Missing means no pending edit; every other read/restore failure is explicit.
    pub fn restore(path: PathBuf, scope: &Scope) -> Result<Option<(Self, Vec<Effect>)>, Error> {
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(Error::Storage(error)),
        };
        let (edits, effects) = Edits::restore(&bytes, scope).map_err(Error::Restore)?;
        Ok(Some((Self::new(path, edits), effects)))
    }
    fn persist(&self) -> Result<(), Error> {
        let bytes = self.edits.journal().map_err(Error::Codec)?;
        let parent = self.path.parent().ok_or_else(|| Error::Storage(io::Error::other("journal has no parent")))?;
        // Sync each new directory and its parent before acknowledging the record below.
        durable_directory(parent).map_err(Error::Storage)?;
        let temp = self.path.with_extension(format!("{}.tmp", uuid::Uuid::now_v7()));
        let result = (|| -> io::Result<()> {
            let mut file = fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&temp)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            fs::rename(&temp, &self.path)?;
            fs::File::open(parent)?.sync_all()
        })();
        if result.is_err() { let _ = fs::remove_file(&temp); }
        result.map_err(Error::Storage)
    }
    fn remove(&self) -> Result<(), Error> {
        match fs::remove_file(&self.path) {
            Ok(()) => fs::File::open(self.path.parent().expect("journal parent")).and_then(|dir| dir.sync_all()).map_err(Error::Storage),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(Error::Storage(error)),
        }
    }
    /// Recovery effects and ordinary reducer effects take the identical durable path.
    /// A failed barrier poisons this executor: reopen from disk before any further sends.
    pub async fn execute<F, Fut>(&mut self, effects: Vec<Effect>, selected: impl Fn() -> Scope, actor: &str, mut send: F) -> Result<Vec<Effect>, Error>
    where F: FnMut(ActionRequest) -> Fut,
          Fut: Future<Output = Result<Envelope<ActionResult>, ClientError>> {
        if self.blocked { return Err(Error::Storage(io::Error::other("journal requires recovery"))); }
        let mut queue: VecDeque<_> = effects.into();
        let mut external = Vec::new();
        while let Some(effect) = queue.pop_front() {
            if selected() != self.scope { return Err(Error::ScopeMismatch); }
            let result = match effect {
                Effect::PersistPending | Effect::PersistCompletion => self.persist(),
                Effect::PersistRemoval => self.remove(),
                _ => Ok(()),
            };
            if let Err(error) = result {
                self.blocked = true;
                self.edits.reduce(Event::StorageFailed);
                return Err(error);
            }
            let next = match effect {
                Effect::PersistPending | Effect::PersistCompletion => {
                    self.edits.request().map(|request| request.identity.clone()).map(|id| {
                        self.edits.reduce(if effect == Effect::PersistPending { Event::Persisted(id) } else { Event::CompletionPersisted(id) })
                    }).unwrap_or_default()
                }
                Effect::Send => {
                    let request = self.edits.request().expect("Send has a staged request");
                    if request.identity.actor != actor { return Err(Error::ActorMismatch); }
                    let wire = client::write_request(request).map_err(Error::Codec)?;
                    let event = client::write_result(request, send(wire).await);
                    self.edits.reduce(event)
                }
                Effect::ReadBase | Effect::Replan => { external.push(effect); Vec::new() }
                Effect::PersistRemoval => Vec::new(),
            };
            for effect in next.into_iter().rev() { queue.push_front(effect); }
        }
        Ok(external)
    }
}

fn durable_directory(path: &std::path::Path) -> io::Result<()> {
    fs::create_dir_all(path)?;
    // Include existing ancestors: a prior failed sync may have left a visible but
    // non-durable directory. Reopening after failure must repair that barrier too.
    for ancestor in path.ancestors().filter(|path| !path.as_os_str().is_empty()) {
        fs::File::open(ancestor)?.sync_all()?;
    }
    Ok(())
}

/// Execute only the two sidebar action kinds, retaining the adapter's exact payload/fence.
pub async fn send(client: &fractal_client::Client, request: ActionRequest) -> Result<Envelope<ActionResult>, ClientError> {
    #[derive(serde::Deserialize)]
    struct Wire { id: String, idempotency_key: String, fence: fractal_client::Fence, parameters: serde_json::Value }
    let kind = request.action_type().clone();
    let wire: Wire = serde_json::from_value(serde_json::to_value(request).map_err(|e| ClientError::Protocol(e.to_string()))?)
        .map_err(|e| ClientError::Protocol(e.to_string()))?;
    match kind {
        fractal_client::ActionType::ArrangementEdit => client.arrangement_edit(wire.id, wire.idempotency_key, wire.fence,
            serde_json::from_value(wire.parameters).map_err(|e| ClientError::Protocol(e.to_string()))?).await,
        fractal_client::ActionType::ArrangementMembershipEdit => client.arrangement_membership_edit(wire.id, wire.idempotency_key, wire.fence,
            serde_json::from_value(wire.parameters).map_err(|e| ClientError::Protocol(e.to_string()))?).await,
        _ => Err(ClientError::Protocol("not a sidebar action".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fractal_sidebar::{edits::{Base, Identity, Status}, model::{NodeRef, Parent}, paging::Snapshot, plan::{MoveIntent, V2Plan}};
    fn scope() -> Scope { Scope { owner: "owner/test".into(), container: "container/test".into(), endpoint: "endpoint/test".into(), host: "host/test".into(), projection: "1".into() } }
    fn id(n: u8) -> Identity { Identity { actor: "actor/test".into(), action_id: format!("action/{n}"), idempotency_key: format!("key/{n}") } }
    fn base(n: u8) -> Base { Base { scope: scope(), snapshot: Snapshot { id: format!("snapshot/{n}"), host_id: scope().host, store_index: n.into(), projection_version: "1".into(), created_at: "created".into(), published_at: None }, revision: format!("revision/{n}"), ready: true } }
    fn members() -> Vec<fractal_schema::ordered_membership::Operation> {
        vec![fractal_schema::ordered_membership::Operation::Place { member: "member/test".into(), bucket: None, key: "a0".into() }]
    }
    fn staged(path: PathBuf, compound: bool) -> (Store, Vec<Effect>) {
        let mut edits = Edits::new(scope(), MoveIntent { node: NodeRef::Subject { subject: "member/test".into() }, parent: Parent::Root {}, before: None });
        let effects = edits.reduce(Event::Plan { base: base(1), plan: V2Plan {
            folders: if compound { vec![fractal_schema::arrangements::Operation::FolderRename { id: "folder/test".into(), name: "Folder".into() }] } else { vec![] },
            memberships: members(),
        }, folder: id(1), membership: id(2) });
        (Store::new(path, edits), effects)
    }
    async fn lost(_: ActionRequest) -> Result<Envelope<ActionResult>, ClientError> { Err(ClientError::Transport("lost".into())) }
    #[tokio::test]
    async fn sidebar_store_barrier_restart_and_removal() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("pending.json");
        let (mut store, effects) = staged(path.clone(), false);
        let exact = store.edits.request().cloned().unwrap();
        let mut sent = 0;
        store.execute(effects, scope, "actor/test", |wire| {
            sent += 1;
            let (restored, _) = Edits::restore(&fs::read(&path).unwrap(), &scope()).unwrap();
            assert_eq!(restored.request(), Some(&exact));
            assert_eq!(wire, client::write_request(&exact).unwrap());
            lost(wire)
        }).await.unwrap();
        assert_eq!(sent, 1);
        assert_eq!(store.edits.status, Status::UnknownDelivery);
        let (mut store, effects) = Store::restore(path.clone(), &scope()).unwrap().unwrap();
        assert_eq!(effects, vec![Effect::PersistPending]);
        store.execute(effects, scope, "actor/test", |wire| {
            assert_eq!(wire, client::write_request(&exact).unwrap()); lost(wire)
        }).await.unwrap();
        let effects = store.edits.reduce(Event::Observed(id(2)));
        // Crash after observation was durably written, before its barrier acknowledgement.
        store.persist().unwrap();
        let (mut store, recovery) = Store::restore(path.clone(), &scope()).unwrap().unwrap();
        assert_eq!(effects, recovery);
        store.execute(recovery, scope, "actor/test", |_| async { panic!("completed request resent") }).await.unwrap();
        assert!(!path.exists());
        assert!(Store::restore(path, &scope()).unwrap().is_none());
    }
    #[tokio::test]
    async fn sidebar_store_each_crash_stage_and_partial_completion() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("pending.json");
        let (mut store, _) = staged(path.clone(), true);
        for stage in 0..7 {
            match stage {
                1 => { store.edits.reduce(Event::Persisted(id(1))); }
                2 => { store.edits.reduce(Event::Unknown(id(1))); }
                3 => { store.edits.reduce(Event::Ack(id(1))); }
                4 => { store.edits.reduce(Event::Observed(id(1))); }
                5 => { store.edits.reduce(Event::CompletionPersisted(id(1))); }
                6 => { store.edits.reduce(Event::Fresh(base(2), Ok(members()))); }
                _ => {}
            }
            store.persist().unwrap();
            let (mut restored, effects) = Store::restore(path.clone(), &scope()).unwrap().unwrap();
            let mut sends = 0;
            let external = restored.execute(effects, scope, "actor/test", |wire| { sends += 1; lost(wire) }).await.unwrap();
            assert_eq!(sends, usize::from(matches!(stage, 0 | 1 | 2 | 6)));
            if matches!(stage, 3 | 4 | 5) { assert!(external.contains(&Effect::ReadBase)); }
        }
        let effects = store.edits.reduce(Event::Refused(id(2), fractal_sidebar::edits::Refusal::Unauthorized));
        // First acknowledge sending, then refuse the dependent step.
        assert!(effects.is_empty());
        store.edits.reduce(Event::Persisted(id(2)));
        let effects = store.edits.reduce(Event::Refused(id(2), fractal_sidebar::edits::Refusal::Unauthorized));
        store.execute(effects, scope, "actor/test", |_| async { panic!("refused request resent") }).await.unwrap();
        let (mut restored, effects) = Store::restore(path, &scope()).unwrap().unwrap();
        assert_eq!(restored.edits.status, Status::PartiallyApplied);
        restored.execute(effects, scope, "actor/test", |_| async { panic!("partial request resent") }).await.unwrap();
    }
    #[tokio::test]
    async fn sidebar_store_storage_failure_and_private_record() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let blocker = root.path().join("file");
        fs::write(&blocker, b"not a directory").unwrap();
        let (mut store, effects) = staged(blocker.join("pending.json"), false);
        assert!(matches!(store.execute(effects, scope, "actor/test", |_| async { panic!("unjournaled send") }).await, Err(Error::Storage(_))));
        assert_eq!(store.edits.refusal, Some(fractal_sidebar::edits::Refusal::Storage));
        assert!(store.execute(vec![Effect::Send], scope, "actor/test", |_| async { panic!("poisoned send") }).await.is_err());
        let path = root.path().join("private.json");
        let (mut store, effects) = staged(path.clone(), false);
        // Authentication remains captured by the transport, never by Edits or its record.
        let credential = ["private", "session", "credential"].join("-");
        store.execute(effects, scope, "actor/test", |wire| { assert!(!credential.is_empty()); lost(wire) }).await.unwrap();
        assert!(!String::from_utf8(fs::read(&path).unwrap()).unwrap().contains(&credential));
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        let mut wrong = scope(); wrong.endpoint.push_str("/other");
        assert!(matches!(Store::restore(path, &wrong), Err(Error::Restore(_))));
    }
    #[tokio::test]
    async fn sidebar_store_scope_switch_fences_every_effect() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("pending.json");
        let (mut store, _) = staged(path.clone(), false);
        store.persist().unwrap();
        let original = fs::read(&path).unwrap();
        for effect in [Effect::Send, Effect::ReadBase, Effect::Replan, Effect::PersistPending, Effect::PersistCompletion, Effect::PersistRemoval] {
            for field in 0..5 {
                let mut selected = scope();
                match field { 0 => selected.owner.push('x'), 1 => selected.container.push('x'), 2 => selected.endpoint.push('x'), 3 => selected.host.push('x'), _ => selected.projection.push('x') }
                assert!(matches!(store.execute(vec![effect], || selected.clone(), "actor/test", |_| async { panic!("obsolete send") }).await, Err(Error::ScopeMismatch)));
                assert_eq!(fs::read(&path).unwrap(), original);
                assert_eq!(store.edits.status, Status::AwaitingPersistence);
            }
        }
        // Re-read the selection after await, before executing the acknowledgement's effects.
        let selected = std::cell::RefCell::new(scope());
        let error = store.execute(vec![Effect::PersistPending], || selected.borrow().clone(), "actor/test", |wire| {
            selected.borrow_mut().host.push('x'); lost(wire)
        }).await.unwrap_err();
        assert!(matches!(error, Error::ScopeMismatch));
        assert_eq!(fs::read(&path).unwrap(), original);
    }
    #[tokio::test]
    async fn sidebar_store_first_use_creates_durable_directory_chain() {
        let root = tempfile::tempdir().unwrap();
        let parent = root.path().join("new-state/tui/journals");
        let path = parent.join("pending.json");
        let (mut store, effects) = staged(path.clone(), false);
        let exact = store.edits.request().cloned().unwrap();
        store.execute(effects, scope, "actor/test", |wire| {
            assert!(parent.is_dir());
            let (restored, _) = Store::restore(path.clone(), &scope()).unwrap().unwrap();
            assert_eq!(restored.edits.request(), Some(&exact)); lost(wire)
        }).await.unwrap();
    }
    #[test]
    fn sidebar_store_filename_is_stable_across_toolchains() {
        assert_eq!(Store::filename(&scope()).unwrap(),
            "sidebar-journal-v1-b0811e24e3c295beee364ec8f1d7475b415a8eb5e61ce603b3467ad639b24cbf.json");
    }
}
