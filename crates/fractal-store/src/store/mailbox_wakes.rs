//! Local, disposable routing of mailbox wakes. Durable claims remain the source of truth.
//! One dispatcher follows the existing post-commit feed, instead of every stream querying it.
use super::*;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use tokio::sync::watch;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum Dependency {
    Subject(String),
    Recipient(String),
    Owner(String, String),
    /// An identity delta, routed by recipient and by any existing subject dependency.
    Message(String, String),
}

#[derive(Default)]
pub struct Pending {
    durable: bool,
    messages: BTreeSet<String>,
}

#[derive(Default)]
pub struct StreamWork {
    pub notifications: AtomicU64,
    pub updates: AtomicU64,
    pub unchanged: AtomicU64,
    pub snapshots: AtomicU64,
    pub routed_updates: AtomicU64,
}

#[derive(Clone)]
pub struct Observer {
    sender: watch::Sender<u64>,
    pending: Arc<Mutex<Pending>>,
    work: Arc<StreamWork>,
}

#[derive(Default)]
pub struct Registry {
    next: u64,
    pub subscribers: HashMap<Dependency, HashMap<u64, Observer>>,
    streams: HashMap<u64, (String, String, Arc<StreamWork>)>,
}

#[derive(Default)]
pub struct Wakes {
    pub registry: Mutex<Registry>,
    pub failures: AtomicU64,
    pub degraded: AtomicBool,
}

pub struct Subscription {
    pub wakes: Arc<Wakes>,
    id: u64,
    sender: watch::Sender<u64>,
    pub changed: watch::Receiver<u64>,
    base: HashSet<Dependency>,
    dependencies: HashSet<Dependency>,
    pending: Arc<Mutex<Pending>>,
    pub work: Arc<StreamWork>,
}

impl Wakes {
    pub fn subscribe(self: &Arc<Self>, fence: &crate::mailbox::Fence) -> Subscription {
        let base = HashSet::from([
            Dependency::Subject(fence.subject.clone()),
            Dependency::Recipient(normalize_message_party(&fence.subject)),
            Dependency::Owner(fence.subject.clone(), fence.component.clone()),
        ]);
        let (sender, changed) = watch::channel(0);
        let pending = Arc::new(Mutex::new(Pending::default()));
        let work = Arc::new(StreamWork::default());
        let observer = Observer { sender: sender.clone(), pending: pending.clone(), work: work.clone() };
        let mut registry = self.registry.lock().unwrap_or_else(PoisonError::into_inner);
        registry.next += 1;
        let id = registry.next;
        registry.streams.insert(id, (fence.subject.clone(), fence.component.clone(), work.clone()));
        for key in &base {
            registry
                .subscribers
                .entry(key.clone())
                .or_default()
                .insert(id, observer.clone());
        }
        Subscription {
            wakes: self.clone(),
            id,
            sender,
            changed,
            dependencies: base.clone(),
            base,
            pending,
            work,
        }
    }

    pub fn notify(&self, dependencies: impl IntoIterator<Item = Dependency>) {
        let registry = self.registry.lock().unwrap_or_else(PoisonError::into_inner);
        let mut notified = HashSet::new();
        for dependency in dependencies {
            let (first, second, message) = match dependency {
                Dependency::Message(subject, recipient) => (
                    Dependency::Subject(subject.clone()), Some(Dependency::Recipient(recipient)),
                    Some(subject),
                ),
                Dependency::Subject(subject) if subject.starts_with("message/") => (
                    Dependency::Subject(subject.clone()), None, Some(subject),
                ),
                key => (key, None, None),
            };
            for key in std::iter::once(first).chain(second) {
                if let Some(subscribers) = registry.subscribers.get(&key) {
                    for (id, observer) in subscribers {
                        let mut pending = observer.pending.lock().unwrap_or_else(PoisonError::into_inner);
                        if let Some(subject) = &message {
                            if !pending.durable && pending.messages.len() < 4_096 {
                                pending.messages.insert(subject.clone());
                            } else {
                                pending.durable = true;
                                pending.messages.clear();
                            }
                        } else {
                            pending.durable = true;
                            pending.messages.clear();
                        }
                        if notified.insert(*id) {
                            observer.work.notifications.fetch_add(1, Ordering::Relaxed);
                            observer.sender.send_modify(|generation| *generation = generation.wrapping_add(1));
                        }
                    }
                }
            }
        }
    }

    pub fn failure(&self, stage: &str, error: impl std::fmt::Display) {
        self.failures.fetch_add(1, Ordering::Relaxed);
        self.degraded.store(true, Ordering::Relaxed);
        tracing::warn!(stage, error = %error, "mailbox wake dispatcher failed; safety reads remain active; retrying in five seconds");
    }

    pub fn resync(&self) {
        let keys = self
            .registry
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .subscribers
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        self.notify(keys);
    }

    pub fn owner_changed(&self, subject: &str, component: &str) {
        self.notify([Dependency::Owner(subject.into(), component.into())]);
    }

    pub fn work_snapshot(&self) -> serde_json::Value {
        let registry = self.registry.lock().unwrap_or_else(PoisonError::into_inner);
        let streams = registry.streams.iter().map(|(id, (subject, component, work))| {
            serde_json::json!({
                "id": id, "subject": subject, "component": component,
                "notifications": work.notifications.load(Ordering::Relaxed),
                "updates": work.updates.load(Ordering::Relaxed),
                "unchanged": work.unchanged.load(Ordering::Relaxed),
                "snapshots": work.snapshots.load(Ordering::Relaxed),
                "routed_updates": work.routed_updates.load(Ordering::Relaxed),
            })
        }).collect::<Vec<_>>();
        serde_json::json!({"open_streams": streams.len(), "streams": streams})
    }
}

impl Subscription {
    /// Consume coalesced semantic identities before starting a pinned read. Any owner,
    /// seat, repair or unknown notification retains the durable reconciliation path.
    pub fn take_messages(&mut self) -> Option<BTreeSet<String>> {
        let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        self.changed.borrow_and_update();
        let pending = std::mem::take(&mut *pending);
        (!pending.durable && !pending.messages.is_empty()).then_some(pending.messages)
    }

    fn observer(&self) -> Observer {
        Observer { sender: self.sender.clone(), pending: self.pending.clone(), work: self.work.clone() }
    }

    /// Also wake on ownership changes of another component of the same seat. A title stream that
    /// pushes attachment needs its seat's delivery binding changes, which are not graph claims.
    pub fn owner_of(&mut self, subject: &str, component: &str) {
        let key = Dependency::Owner(subject.into(), component.into());
        if !self.base.insert(key.clone()) {
            return;
        }
        let mut registry = self
            .wakes
            .registry
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if self.dependencies.insert(key.clone()) {
            registry
                .subscribers
                .entry(key)
                .or_default()
                .insert(self.id, self.observer());
        }
    }

    /// Replace message dependencies under the same lock used by the dispatcher. The recipient
    /// dependency also routes every message mutation, covering a newly read message whose
    /// subject has not been registered yet.
    pub fn messages(&mut self, subjects: &[String]) {
        let next: HashSet<_> = self
            .base
            .iter()
            .cloned()
            .chain(subjects.iter().cloned().map(Dependency::Subject))
            .collect();
        let mut registry = self
            .wakes
            .registry
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        for key in self.dependencies.difference(&next) {
            remove(&mut registry, key, self.id);
        }
        for key in next.difference(&self.dependencies) {
            registry
                .subscribers
                .entry(key.clone())
                .or_default()
                .insert(self.id, self.observer());
        }
        self.dependencies = next;
    }
}

fn remove(registry: &mut Registry, key: &Dependency, id: u64) {
    if let Some(subscribers) = registry.subscribers.get_mut(key) {
        subscribers.remove(&id);
        if subscribers.is_empty() {
            registry.subscribers.remove(key);
        }
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        let mut registry = self
            .wakes
            .registry
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        registry.streams.remove(&self.id);
        for key in &self.dependencies {
            remove(&mut registry, key, self.id);
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Store {
    pub fn miss_mailbox_owner_for_test(&self, subject: &str, component: &str) {
        self.fractal
            .mailbox_wakes
            .get()
            .unwrap()
            .registry
            .lock()
            .unwrap()
            .subscribers
            .remove(&Dependency::Owner(subject.into(), component.into()));
    }
    pub fn miss_mailbox_recipient_for_test(&self, subject: &str) {
        self.fractal
            .mailbox_wakes
            .get()
            .unwrap()
            .registry
            .lock()
            .unwrap()
            .subscribers
            .remove(&Dependency::Recipient(normalize_message_party(subject)));
    }
}
