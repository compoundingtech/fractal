//! Local, disposable routing of mailbox wakes. Durable claims remain the source of truth.
//! One dispatcher follows the existing post-commit feed, instead of every stream querying it.
use super::*;
use crate::model::DoctorCheck;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use tokio::sync::watch;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum Dependency {
    Subject(String),
    Recipient(String),
    Owner(String, String),
}

#[derive(Default)]
pub struct Registry {
    next: u64,
    subscribers: HashMap<Dependency, HashMap<u64, watch::Sender<u64>>>,
}

#[derive(Default)]
pub struct Wakes {
    registry: Mutex<Registry>,
    failures: AtomicU64,
    degraded: AtomicBool,
}

pub struct Subscription {
    wakes: Arc<Wakes>,
    id: u64,
    sender: watch::Sender<u64>,
    pub changed: watch::Receiver<u64>,
    base: HashSet<Dependency>,
    dependencies: HashSet<Dependency>,
}

impl Wakes {
    fn subscribe(self: &Arc<Self>, fence: &crate::mailbox::Fence) -> Subscription {
        let base = HashSet::from([
            Dependency::Subject(fence.subject.clone()),
            Dependency::Recipient(normalize_message_party(&fence.subject)),
            Dependency::Owner(fence.subject.clone(), fence.component.clone()),
        ]);
        let (sender, changed) = watch::channel(0);
        let mut registry = self.registry.lock().unwrap_or_else(PoisonError::into_inner);
        registry.next += 1;
        let id = registry.next;
        for key in &base {
            registry
                .subscribers
                .entry(key.clone())
                .or_default()
                .insert(id, sender.clone());
        }
        Subscription {
            wakes: self.clone(),
            id,
            sender,
            changed,
            dependencies: base.clone(),
            base,
        }
    }

    fn notify(&self, dependencies: impl IntoIterator<Item = Dependency>) {
        let registry = self.registry.lock().unwrap_or_else(PoisonError::into_inner);
        let mut notified = HashSet::new();
        for key in dependencies {
            if let Some(subscribers) = registry.subscribers.get(&key) {
                for (id, sender) in subscribers {
                    if notified.insert(*id) {
                        sender.send_modify(|generation| *generation = generation.wrapping_add(1));
                    }
                }
            }
        }
    }

    fn failure(&self, stage: &str, error: impl std::fmt::Display) {
        self.failures.fetch_add(1, Ordering::Relaxed);
        self.degraded.store(true, Ordering::Relaxed);
        tracing::warn!(stage, error = %error, "mailbox wake dispatcher failed; safety reads remain active; retrying in five seconds");
    }

    fn resync(&self) {
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
}

impl Subscription {
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
                .insert(self.id, self.sender.clone());
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
        for key in &self.dependencies {
            remove(&mut registry, key, self.id);
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Store {
    pub fn miss_mailbox_owner_for_test(&self, subject: &str, component: &str) {
        self.smalltalk
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
        self.smalltalk
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
