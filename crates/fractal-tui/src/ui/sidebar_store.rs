//! Unwired host executor. Activation must serialize events per scope and supply a client
//! authenticated as the staged actor. Endpoint identities must be credential-free.
use fractal_sidebar::{client, edits::{Edits, Effect, Event, Persistence}, paging::Scope};
use fractal_client::{ActionRequest, ActionResult, ClientError, Envelope};
use std::{collections::VecDeque, fs, io::{self, Read, Write}, os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt}, path::PathBuf};

/// Dedicated live-loop inputs, not resource-window updates. Tokens reject obsolete reads.
pub enum LiveEvent {
    Membership(Box<fractal_sidebar::paging::Event>),
    Edit { scope: Scope, event: Box<Event> },
}
#[derive(Debug)]
pub enum Error {
    Storage(io::Error), Codec(serde_json::Error), Restore(fractal_sidebar::edits::RestoreError),
    NoStateDirectory, ActorMismatch, ScopeMismatch, MissingRequest, InsecureJournal, EndpointCredentials, InvalidEndpoint,
}
pub struct Store { path: PathBuf, scope: Scope, pub edits: Edits, blocked: bool }
impl Store {
    pub fn path(scope: &Scope) -> Result<PathBuf, Error> {
        let (dir, _) = super::glass_store::person_state(&scope.owner).ok_or(Error::NoStateDirectory)?;
        Ok(dir.join(Self::filename(scope)?))
    }
    fn filename(scope: &Scope) -> Result<String, Error> {
        validate_endpoint(scope)?;
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(serde_json::to_vec(scope).map_err(Error::Codec)?);
        Ok(format!("sidebar-journal-v1-{digest:x}.json"))
    }
    pub fn new(path: PathBuf, edits: Edits) -> Self { Self { path, scope: edits.scope().clone(), edits, blocked: false } }
    /// Missing means no pending edit; every other read/restore failure is explicit.
    pub fn restore(path: PathBuf, scope: &Scope) -> Result<Option<(Self, Vec<Effect>)>, Error> {
        validate_endpoint(scope)?;
        let file = match fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(Error::Storage(error)),
        };
        let metadata = file.metadata().map_err(Error::Storage)?;
        // SAFETY: geteuid has no preconditions; mirror requests::mine's ownership check.
        if !metadata.is_file() || metadata.uid() != unsafe { libc::geteuid() } || metadata.permissions().mode() & 0o077 != 0 {
            return Err(Error::InsecureJournal);
        }
        let mut bytes = Vec::new();
        file.take(fractal_sidebar::edits::MAX_JOURNAL_BYTES as u64 + 1).read_to_end(&mut bytes).map_err(Error::Storage)?;
        if bytes.len() > fractal_sidebar::edits::MAX_JOURNAL_BYTES {
            return Err(Error::Restore(fractal_sidebar::edits::RestoreError::InvalidRecord(<serde_json::Error as serde::de::Error>::custom("journal exceeds size limit"))));
        }
        let (edits, effects) = Edits::restore(&bytes, scope).map_err(Error::Restore)?;
        Ok(Some((Self::new(path, edits), effects)))
    }
    fn persist(&self) -> Result<(), Error> {
        validate_endpoint(&self.scope)?;
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
            validate_endpoint(&self.scope)?;
            let result = match effect {
                Effect::PersistPending | Effect::PersistCompletion => self.persist(),
                Effect::PersistRemoval => self.remove(),
                _ => Ok(()),
            };
            if let Err(error) = result {
                self.blocked = true;
                if let Some(request) = self.edits.request() {
                    let identity = request.identity.clone();
                    let phase = match effect { Effect::PersistPending => Persistence::Pending, Effect::PersistCompletion => Persistence::Completion, Effect::PersistRemoval => Persistence::Removal, _ => unreachable!("only persistence effects fail storage") };
                    self.edits.reduce(Event::StorageFailed { identity, phase });
                }
                return Err(error);
            }
            let next = match effect {
                Effect::PersistPending | Effect::PersistCompletion => {
                    self.edits.request().map(|request| request.identity.clone()).map(|id| {
                        self.edits.reduce(if effect == Effect::PersistPending { Event::Persisted(id) } else { Event::CompletionPersisted(id) })
                    }).unwrap_or_default()
                }
                Effect::Send(delivery) => {
                    let request = self.edits.request().ok_or(Error::MissingRequest)?;
                    if request.identity.actor != actor { return Err(Error::ActorMismatch); }
                    let wire = client::write_request(request).map_err(Error::Codec)?;
                    let event = client::write_result(request, delivery, send(wire).await);
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

fn validate_endpoint(scope: &Scope) -> Result<(), Error> {
    // reqwest reexports url::Url: use the transport's normalization, not an authority split.
    if let Ok(url) = reqwest::Url::parse(&scope.endpoint)
        && matches!(url.scheme(), "http" | "https") {
        return if url.username().is_empty() && url.password().is_none() { Ok(()) } else { Err(Error::EndpointCredentials) };
    }
    if scope.endpoint.split_once(':').is_some_and(|(scheme, _)| scheme.trim().eq_ignore_ascii_case("http") || scheme.trim().eq_ignore_ascii_case("https")) {
        return Err(Error::InvalidEndpoint);
    }
    let authority = scope.endpoint.split_once("://").map_or(scope.endpoint.as_str(), |(_, rest)| rest)
        .trim_start_matches("//").split(['/', '?', '#']).next().unwrap_or_default();
    if authority.contains('@') { return Err(Error::EndpointCredentials); }
    Ok(())
}

fn durable_directory(path: &std::path::Path) -> io::Result<()> {
    fs::DirBuilder::new().recursive(true).mode(0o700).create(path)?;
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
    use fractal_sidebar::{edits::{Base, Delivery, Identity, Status}, model::{NodeRef, Parent}, paging::Snapshot, plan::{MoveIntent, V2Plan}};
    fn scope() -> Scope { Scope { owner: "person/test".into(), container: "arrangement/person/test/01900000-0000-7000-8000-000000000001".into(), endpoint: "endpoint/test".into(), host: "host/test".into(), projection: "1".into() } }
    fn id(n: u8) -> Identity { Identity { actor: "person/test".into(), action_id: format!("action/{n}"), idempotency_key: format!("key/{n}") } }
    fn base(n: u8) -> Base { Base { scope: scope(), snapshot: Snapshot { id: format!("snapshot/{n}"), host_id: scope().host, store_index: n.into(), projection_version: "1".into(), created_at: "created".into(), published_at: None }, revision: format!("revision/{n}"), ready: true } }
    fn members() -> Vec<fractal_schema::ordered_membership::Operation> {
        vec![fractal_schema::ordered_membership::Operation::Place { member: "agent/fleet/fixture-sidebar/test".into(), bucket: None, key: "a0".into() }]
    }
    fn staged(path: PathBuf, compound: bool) -> (Store, Vec<Effect>) {
        let mut edits = Edits::new(scope(), MoveIntent { node: NodeRef::Subject { subject: "agent/fleet/fixture-sidebar/test".into() }, parent: Parent::Root {}, before: None });
        let effects = edits.reduce(Event::Plan { base: base(1), plan: V2Plan {
            folders: if compound { vec![fractal_schema::arrangements::Operation::FolderRename { id: "01900000-0000-7000-8000-000000000001".into(), name: "Folder".into() }] } else { vec![] },
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
        store.execute(effects, scope, "person/test", |wire| {
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
        store.execute(effects, scope, "person/test", |wire| {
            assert_eq!(wire, client::write_request(&exact).unwrap()); lost(wire)
        }).await.unwrap();
        let effects = store.edits.reduce(Event::Observed(id(2)));
        // Crash after observation was durably written, before its barrier acknowledgement.
        store.persist().unwrap();
        let (mut store, recovery) = Store::restore(path.clone(), &scope()).unwrap().unwrap();
        assert_eq!(effects, recovery);
        store.execute(recovery, scope, "person/test", |_| async { panic!("completed request resent") }).await.unwrap();
        assert!(!path.exists());
        assert!(Store::restore(path, &scope()).unwrap().is_none());
    }
    #[tokio::test]
    async fn sidebar_store_restored_send_preserves_replay_provenance() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("pending.json");
        let (mut store, effects) = staged(path.clone(), false);
        store.execute(effects, scope, "person/test", lost).await.unwrap();
        assert_eq!(store.edits.status, Status::UnknownDelivery);
        let (mut restored, effects) = Store::restore(path, &scope()).unwrap().unwrap();
        let exact = restored.edits.request().cloned().unwrap();
        let mut sends = 0;
        restored.execute(effects, scope, "person/test", |wire| {
            sends += 1;
            assert_eq!(wire, client::write_request(&exact).unwrap());
            async {
                Err(ClientError::Api(fractal_client::ErrorCode::Forbidden, "refused replay".into(),
                    Box::new(fractal_client::ErrorEnvelope {
                        api_version: "0".into(), error_version: "1".into(), request_id: "request/test".into(),
                        code: fractal_client::ErrorCode::Forbidden, message: "refused replay".into(),
                        retryable: false, retry_after_ms: None,
                        details: [("applied".into(), serde_json::json!("none"))].into(),
                    })))
            }
        }).await.unwrap();
        assert_eq!(sends, 1);
        assert_eq!(restored.edits.status, Status::UnknownDelivery);
        assert_eq!(restored.edits.request(), Some(&exact));
        assert!(restored.edits.refusal.is_none());
        assert!(restored.edits.reduce(Event::Retry).is_empty());
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
            let external = restored.execute(effects, scope, "person/test", |wire| { sends += 1; lost(wire) }).await.unwrap();
            assert_eq!(sends, usize::from(matches!(stage, 0 | 1 | 2 | 6)));
            if matches!(stage, 3..=5) { assert!(external.contains(&Effect::ReadBase)); }
        }
        let effects = store.edits.reduce(Event::Refused(id(2), fractal_sidebar::edits::Refusal::Unauthorized));
        // First acknowledge sending, then refuse the dependent step.
        assert!(effects.is_empty());
        store.edits.reduce(Event::Persisted(id(2)));
        let effects = store.edits.reduce(Event::Refused(id(2), fractal_sidebar::edits::Refusal::Unauthorized));
        store.execute(effects, scope, "person/test", |_| async { panic!("refused request resent") }).await.unwrap();
        let (mut restored, effects) = Store::restore(path, &scope()).unwrap().unwrap();
        assert_eq!(restored.edits.status, Status::PartiallyApplied);
        restored.execute(effects, scope, "person/test", |_| async { panic!("partial request resent") }).await.unwrap();
    }
    #[tokio::test]
    async fn sidebar_store_storage_failure_and_private_record() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let blocker = root.path().join("file");
        fs::write(&blocker, b"not a directory").unwrap();
        let (mut store, effects) = staged(blocker.join("pending.json"), false);
        assert!(matches!(store.execute(effects, scope, "person/test", |_| async { panic!("unjournaled send") }).await, Err(Error::Storage(_))));
        assert_eq!(store.edits.refusal, Some(fractal_sidebar::edits::Refusal::Storage));
        assert!(store.execute(vec![Effect::Send(Delivery::First)], scope, "person/test", |_| async { panic!("poisoned send") }).await.is_err());
        let path = root.path().join("private.json");
        let (mut store, effects) = staged(path.clone(), false);
        // Authentication remains captured by the transport, never by Edits or its record.
        let credential = ["private", "session", "credential"].join("-");
        store.execute(effects, scope, "person/test", |wire| { assert!(!credential.is_empty()); lost(wire) }).await.unwrap();
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
        for effect in [Effect::Send(Delivery::First), Effect::ReadBase, Effect::Replan, Effect::PersistPending, Effect::PersistCompletion, Effect::PersistRemoval] {
            for field in 0..5 {
                let mut selected = scope();
                match field { 0 => selected.owner.push('x'), 1 => selected.container.push('x'), 2 => selected.endpoint.push('x'), 3 => selected.host.push('x'), _ => selected.projection.push('x') }
                assert!(matches!(store.execute(vec![effect], || selected.clone(), "person/test", |_| async { panic!("obsolete send") }).await, Err(Error::ScopeMismatch)));
                assert_eq!(fs::read(&path).unwrap(), original);
                assert_eq!(store.edits.status, Status::AwaitingPersistence);
            }
        }
        // Re-read the selection after await, before executing the acknowledgement's effects.
        let selected = std::cell::RefCell::new(scope());
        let error = store.execute(vec![Effect::PersistPending], || selected.borrow().clone(), "person/test", |wire| {
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
        store.execute(effects, scope, "person/test", |wire| {
            assert!(parent.is_dir());
            for directory in [parent.as_path(), parent.parent().unwrap(), parent.parent().unwrap().parent().unwrap()] {
                assert_eq!(fs::metadata(directory).unwrap().permissions().mode() & 0o777, 0o700);
            }
            let (restored, _) = Store::restore(path.clone(), &scope()).unwrap().unwrap();
            assert_eq!(restored.edits.request(), Some(&exact)); lost(wire)
        }).await.unwrap();
    }
    #[test]
    fn sidebar_store_filename_is_stable_across_toolchains() {
        let scope = Scope { owner: "owner/test".into(), container: "container/test".into(), endpoint: "endpoint/test".into(), host: "host/test".into(), projection: "1".into() };
        assert_eq!(Store::filename(&scope).unwrap(),
            "sidebar-journal-v1-b0811e24e3c295beee364ec8f1d7475b415a8eb5e61ce603b3467ad639b24cbf.json");
    }
    #[test]
    fn sidebar_store_restore_bounds_and_checks_private_files() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("pending.json");
        let file = fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path).unwrap();
        file.set_len(fractal_sidebar::edits::MAX_JOURNAL_BYTES as u64 + 1).unwrap();
        assert!(matches!(Store::restore(path.clone(), &scope()), Err(Error::Restore(_))));
        fs::write(&path, b"corrupt").unwrap();
        assert!(matches!(Store::restore(path.clone(), &scope()), Err(Error::Restore(_))));
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(matches!(Store::restore(path, &scope()), Err(Error::InsecureJournal)));
    }
    #[tokio::test]
    async fn sidebar_store_rejects_credential_endpoints_and_missing_requests() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("pending.json");
        let (mut store, _) = staged(path.clone(), false);
        let intent = store.edits.intent().unwrap().clone();
        store.edits = Edits::new(scope(), intent);
        assert!(matches!(store.execute(vec![Effect::Send(Delivery::First)], scope, "person/test", |_| async { panic!("missing request sent") }).await, Err(Error::MissingRequest)));
        for endpoint in ["https://person:example@example.invalid", "//person@example.invalid", "person@example.invalid"] {
            let mut selected = scope(); selected.endpoint = endpoint.into();
            assert!(matches!(Store::filename(&selected), Err(Error::EndpointCredentials)));
            assert!(matches!(Store::restore(path.clone(), &selected), Err(Error::EndpointCredentials)));
            store.scope = selected.clone();
            assert!(matches!(store.execute(vec![Effect::PersistPending], || selected.clone(), "person/test", |_| async { panic!("credential scope sent") }).await, Err(Error::EndpointCredentials)));
            assert!(!path.exists());
        }
    }
    #[test]
    fn sidebar_store_http_userinfo_is_checked_after_url_normalization() {
        for endpoint in [
            ["https:///user", "example@example.invalid"].join(":"),
            ["https:", r"\\user@example.invalid"].concat(),
            "HtTpS://user@example.invalid".into(),
        ] {
            let mut selected = scope(); selected.endpoint = endpoint;
            assert!(matches!(Store::filename(&selected), Err(Error::EndpointCredentials)));
        }
        for endpoint in ["https://example.invalid/path", "HtTpS://example.invalid", "endpoint/test", "unix:/run/example.sock"] {
            let mut selected = scope(); selected.endpoint = endpoint.into();
            assert!(Store::filename(&selected).is_ok());
        }
    }
}
