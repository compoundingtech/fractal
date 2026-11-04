//! Shared bounded windows, never authority. The existing socket feed still publishes frames.
//! A future graph-watch consumer can replace `revision` with view/frontier invalidations;
//! authorized row reads and socket snapshot/diff delivery remain the same.
use crate::ArrangementsStore;
use crate::SeatsStore;
use crate::PublishedViewsStore;
use super::*;
use std::sync::Weak;
use std::sync::atomic::{AtomicU64, Ordering};

const STORES: usize = 64;
pub const WINDOWS: usize = 64;
pub const SESSION_WINDOWS: usize = 16;
const KIND_LIMIT: usize = 10_000;
pub const COLLECTIONS: [&str; 15] = [
    "missions",
    "attention",
    "agents",
    "work",
    "glasses",
    "arrangements",
    "ordered-memberships",
    "summary",
    "summary-missions",
    "clients", "usage", "machines", "devices", "sessions", "terminals",
];

#[derive(Default)]
pub struct Revisions {
    index: u64,
    commits: u64,
    local: u64,
    values: [u64; COLLECTIONS.len()],
}

pub struct Cached {
    pub revision: u64,
    pub period: u128,
    pub valid_from: u128,
    pub valid_until: Option<u128>,
    pub items: Vec<Value>,
    pub has_more: bool,
    pub membership: Option<fractal_schema::ordered_membership::State>,
}

#[derive(Default)]
pub struct Observed {
    index: u64,
    local: u64,
}

#[derive(Default)]
pub struct Entry {
    pub admission: Arc<tokio::sync::Mutex<()>>,
    pub cached: Mutex<Option<Arc<Cached>>>,
}

pub type WindowEntry = Arc<Entry>;
pub type WindowEntries = VecDeque<(String, String, WindowEntry)>;

#[derive(Clone)]
pub struct Prepared {
    key: String,
    pub entry: WindowEntry,
}

impl Prepared {
    pub fn try_admit(&self) -> Option<tokio::sync::OwnedMutexGuard<()>> {
        self.entry.admission.clone().try_lock_owned().ok()
    }

    pub async fn admit(&self) -> tokio::sync::OwnedMutexGuard<()> {
        let started = Instant::now();
        let guard = self.entry.admission.clone().lock_owned().await;
        crate::performance::record_request("collection/admission-wait", None, started.elapsed());
        guard
    }
}

pub struct ReadFence {
    pub index: u64,
    pub now: u128,
    pub commits: u64,
    pub prepared: Option<Prepared>,
}

/// Held by all sockets using this Store, and by physical workers until they finish.
/// The registry and commit callback keep only weak references. No follower task is started.
pub struct Windows {
    commits: AtomicU64,
    revisions: Mutex<Revisions>,
    /// Held while one socket weighs new commits, so sockets woken by the same commit wait for
    /// its answer instead of each opening a snapshot to weigh it again.
    weighing: Mutex<()>,
    observed: Mutex<Observed>,
    pub entries: Mutex<WindowEntries>,
    observer: Mutex<Option<smallclaims::sqlite::CommitObserver>>,
    #[cfg(any(test, feature = "test-support"))]
    builds: std::sync::atomic::AtomicUsize,
}

type Registry = Vec<(Weak<Store>, Weak<Windows>)>;
static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();

impl Windows {
    pub fn commits(&self) -> u64 {
        self.commits.load(Ordering::Acquire)
    }
    pub fn attach(store: &Arc<Store>) -> Option<Arc<Self>> {
        let mut registry = REGISTRY
            .get_or_init(Default::default)
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        registry.retain(|(store, windows)| store.strong_count() > 0 && windows.strong_count() > 0);
        let store_key = Arc::downgrade(store);
        if let Some(windows) = registry
            .iter()
            .find_map(|(key, windows)| key.ptr_eq(&store_key).then(|| windows.upgrade()).flatten())
        {
            return Some(windows);
        }
        if registry.len() >= STORES {
            return None;
        }
        let windows = Arc::new(Self {
            commits: AtomicU64::new(0),
            revisions: Mutex::new(Revisions::default()),
            weighing: Mutex::new(()),
            observed: Mutex::new(Observed {
                index: store.index().unwrap_or(0),
                local: 0,
            }),
            entries: Mutex::new(VecDeque::new()),
            observer: Mutex::new(None),
            #[cfg(any(test, feature = "test-support"))]
            builds: std::sync::atomic::AtomicUsize::new(0),
        });
        let weak = Arc::downgrade(&windows);
        let observed_store = store_key.clone();
        // Register before any snapshot. The callback does no SQL or publication; a shared
        // snapshot reader weighs bounded kind metadata once for all equivalent sockets.
        let observer = store.observe_commits(move |_| {
            if let Some(windows) = weak.upgrade() {
                let mut observed = windows
                    .observed
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                // The writer updates its atomic committed index before this callback. Retain
                // a separate epoch for commits with no claim advance: a later ignored claim
                // cannot erase their invalidation. No SQL runs on the writer callback.
                match observed_store
                    .upgrade()
                    .and_then(|store| store.index().ok())
                {
                    Some(index) if index > observed.index => observed.index = index,
                    _ => observed.local = observed.local.wrapping_add(1),
                }
                windows.commits.fetch_add(1, Ordering::Release);
            }
        });
        *windows
            .observer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(observer);
        registry.push((store_key, Arc::downgrade(&windows)));
        Some(windows)
    }

    fn entry(&self, key: String, session: String) -> Option<WindowEntry> {
        // Inputs are untrusted; do not retain oversized query/authority keys.
        if key.len() > 4096 {
            return None;
        }
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(position) = entries.iter().position(|(held, _, _)| *held == key) {
            let entry = entries.remove(position).expect("found entry");
            let result = entry.2.clone();
            entries.push_back(entry);
            return Some(result);
        }
        let session_full = entries
            .iter()
            .filter(|(_, owner, _)| *owner == session)
            .count()
            >= SESSION_WINDOWS;
        if session_full || entries.len() >= WINDOWS {
            // Query churn beyond a session's quota evicts only its own inactive entries.
            // Reservations protect both queued async readers and physical computations.
            let position = entries.iter().position(|(_, owner, entry)| {
                (!session_full || *owner == session) && Arc::strong_count(entry) == 1
            })?;
            entries.remove(position);
        }
        let entry = Arc::new(Entry::default());
        entries.push_back((key, session, entry.clone()));
        Some(entry)
    }

    fn revision(
        &self,
        store: &Store,
        index: u64,
        collection: &str,
        snapshot_commits: u64,
    ) -> anyhow::Result<Option<u64>> {
        let mut revisions = self
            .revisions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut observed = self
            .observed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let commits = self.commits.load(Ordering::Acquire);
        // A slower physical reader can hold an older SQLite snapshot. It cannot update the
        // shared frontier or reuse rows from a newer one.
        if index < revisions.index
            || commits != snapshot_commits
            || revisions.commits > snapshot_commits
        {
            return Ok(None);
        }
        // Seed from the first authorized snapshot in case registration raced a commit.
        observed.index = observed.index.max(index);
        let local = observed.local;
        drop(observed);
        if index != revisions.index || commits != revisions.commits {
            let connection = store.readers.get();
            let mut kinds = connection.prepare_cached("SELECT kind FROM claims WHERE store_index>?1 AND store_index<=?2 ORDER BY store_index LIMIT ?3")?;
            let kinds = kinds
                .query_map(
                    rusqlite::params![revisions.index, index, KIND_LIMIT + 1],
                    |row| row.get::<_, String>(0),
                )?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            // A non-claim commit can change local availability, ordering or lease state.
            // Overflow, replay/trim and metadata errors must never certify an unchanged view.
            let all = local != revisions.local
                || index == revisions.index
                || kinds.is_empty()
                || kinds.len() > KIND_LIMIT;
            let arrangements = store.arrangements_changed(revisions.index, index)?;
            let memberships = store.ordered_memberships_changed(revisions.index, index)?;
            for (position, name) in COLLECTIONS.iter().enumerate() {
                if all
                    || (*name == "arrangements" && arrangements)
                    || (*name == "ordered-memberships" && memberships)
                    || kinds.iter().any(|kind| !collection_ignores(name, kind))
                {
                    revisions.values[position] = revisions.values[position].wrapping_add(1);
                }
            }
            revisions.index = index;
            revisions.commits = commits;
            revisions.local = local;
        }
        Ok(COLLECTIONS
            .iter()
            .position(|name| *name == collection)
            .map(|position| revisions.values[position]))
    }

    /// Every collection's revision, when another socket already weighed the newest commit: then
    /// no snapshot or worker is needed. `None` means [`Self::changes`] must weigh it.
    pub fn current_changes(&self, store: &Store) -> Option<[u64; COLLECTIONS.len()]> {
        let commits = self.commits();
        let index = store.index().ok()?;
        let revisions = self
            .revisions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        (revisions.commits == commits && revisions.index == index).then_some(revisions.values)
    }

    /// Weigh the commits since the last look, once for every socket on this store. Call it only
    /// from a socket's own blocking worker, never from a request path that holds a read budget:
    /// a waiter on `weighing` must not hold a reader the weigher needs (#2019's bounded pool).
    pub fn changes(&self, store: &Store) -> anyhow::Result<[u64; COLLECTIONS.len()]> {
        let _weighing = self.weighing.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(revisions) = self.current_changes(store) {
            return Ok(revisions);
        }
        let commits = self.commits();
        store.read_snapshot(|index| {
            let mut values = [0; COLLECTIONS.len()];
            for (position, collection) in COLLECTIONS.iter().enumerate() {
                values[position] = self
                    .revision(store, index, collection, commits)?
                    .ok_or_else(|| {
                        anyhow::anyhow!("collection frontier changed during metadata read")
                    })?;
            }
            Ok(values)
        })
    }

    pub fn changed(collection: &str, before: &[u64; COLLECTIONS.len()], after: &[u64; COLLECTIONS.len()]) -> bool {
        COLLECTIONS
            .iter()
            .position(|name| *name == collection)
            .is_none_or(|position| before[position] != after[position])
    }

    fn key(state: &AppState, session: &ClientSession, request: &CollectionSubscribe) -> String {
        // These projections have no per-grant data once authority has been revalidated.
        // Limits are window slicing, not projection inputs. Attention's effective person
        // and compatibility view are its visibility boundary; missions are fleet-public.
        if request.collection == "missions" {
            return json!({"node":state.node, "fleet":state.fleet_id,
                "collection":"missions"}).to_string();
        }
        if request.collection == "attention"
            && let Ok(person) = person_filter(session, request.person.as_deref())
        {
            return json!({"node":state.node, "fleet":state.fleet_id,
                "collection":"attention", "person":person,
                "custom_forms":session.custom_forms}).to_string();
        }
        json!({
            "node":state.node, "fleet":state.fleet_id,
            "actor":session.actor, "authority":session.authority_actor,
            "grant":session.pairing_grant, "transport":session.transport,
            "scopes":session.scopes, "custom_forms":session.custom_forms,
            "conversation_blocks":session.conversation_blocks,
            "collection":request.collection, "limit":request.limit.unwrap_or(CLIENT_DEFAULT_PAGE_ITEMS),
            "since_ms":request.since_ms, "until_ms":request.until_ms,
            "person":request.person, "subject":request.subject, "filter_actor":request.actor,
            "agent":request.agent, "status":request.status,
        }).to_string()
    }

    /// Reserve a window before gate admission and its physical SQLite query.
    pub fn prepare(
        &self,
        state: &AppState,
        session: &ClientSession,
        request: &CollectionSubscribe,
    ) -> Option<Prepared> {
        let key = Self::key(state, session, request);
        let owner = json!({"node":state.node, "fleet":state.fleet_id,
            "actor":session.actor, "authority":session.authority_actor,
            "grant":session.pairing_grant, "transport":session.transport})
        .to_string();
        let entry = self.entry(key.clone(), owner)?;
        Some(Prepared { key, entry })
    }

    /// Called inside the authorized SQLite snapshot after gate admission. Cache mutexes
    /// hold only Arc loads/stores; computation, serialization and deep clones run outside them.
    pub fn read(
        &self,
        state: &AppState,
        session: &ClientSession,
        request: &CollectionSubscribe,
        fence: ReadFence,
        compute: impl FnOnce() -> anyhow::Result<(Vec<Value>, bool, Option<u128>, Option<fractal_schema::ordered_membership::State>)>,
    ) -> anyhow::Result<(Vec<Value>, bool, Option<fractal_schema::ordered_membership::State>)> {
        let ReadFence {
            index,
            now,
            commits: snapshot_commits,
            prepared,
        } = fence;
        #[cfg(any(test, feature = "test-support"))]
        let compute = || {
            self.builds.fetch_add(1, Ordering::SeqCst);
            compute()
        };
        if matches!(request.collection.as_str(), "summary" | "clients" | "usage" | "machines" | "devices" | "sessions") { return compute().map(|(items, more, _, membership)| (items, more, membership)); }
        // A published view is already the shared copy, and it can be published anew without a
        // commit (a deadline passing): caching it by commit revision would serve the older one.
        if state.store.collection_view_published(&request.collection) {
            return compute().map(|(items, more, _, membership)| (items, more, membership));
        }
        let Some(prepared) = prepared else {
            return compute().map(|(items, more, _, membership)| (items, more, membership));
        };
        // Authority can change while async admission waits. The current snapshot's key
        // must still match; otherwise this read computes without publishing a cache entry.
        if prepared.key != Self::key(state, session, request) {
            return compute().map(|(items, more, _, membership)| (items, more, membership));
        }
        // A commit after this reader began can update local tables without advancing the
        // claim index. Such a snapshot cannot reuse/cache a newer local-state generation.
        if self.commits() != snapshot_commits {
            return compute().map(|(items, more, _, membership)| (items, more, membership));
        }
        let Some(revision) =
            self.revision(&state.store, index, &request.collection, snapshot_commits)?
        else {
            return compute().map(|(items, more, _, membership)| (items, more, membership));
        };
        // Work uses its selected projection time. Missions and attention use exact
        // transition fences, not a coarse shared wall-clock bucket.
        let period = match request.collection.as_str() {
            // Summary windows keep the shared wall-clock bucket; missions and attention
            // above use exact transition fences instead.
            "summary" | "summary-missions" => now / ATTENTION_CLOCK_INTERVAL.as_millis(),
            // Work's deterministic selected projection time is itself an input, even if a
            // claim kind does not otherwise change its rows (elapsed budgets/lease expiry).
            "work" => state.store.projection_time_at(index)?,
            _ => 0,
        };
        let cached = prepared
            .entry
            .cached
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if let Some(cached) = cached
            && cached.revision == revision
            && cached.period == period
            && now >= cached.valid_from
            && cached.valid_until.is_none_or(|until| now < until)
            && (request.collection != "summary-missions" || cached.items.first()
                .and_then(|v| v["evaluated_at"].as_str()).and_then(|s| s.parse::<u128>().ok())
                .is_some_and(|evaluated| evaluated <= now))
        {
            return Ok((cached.items.clone(), cached.has_more, cached.membership));
        }
        let (items, has_more, valid_until, membership) = compute()?;
        let valid_until = if request.collection == "agents" {
            state.store.agent_roster_valid_until(index, request.agent.as_deref())
        } else if request.collection == "summary-missions" {
            items.first().and_then(|v| v["valid_until"].as_str()).map(str::parse).transpose()?
        } else {
            valid_until
        };
        // At most 64 bounded responses per Store. Oversized results remain uncached.
        let cached = (serde_json::to_vec(&items)?.len() <= CLIENT_MAX_RESPONSE_BYTES).then(|| {
            Arc::new(Cached {
                revision,
                period,
                valid_from: now,
                valid_until,
                items: items.clone(),
                has_more,
                membership,
            })
        });
        *prepared
            .entry
            .cached
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = cached;
        Ok((items, has_more, membership))
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn builds(&self) -> usize {
        self.builds.load(Ordering::SeqCst)
    }
}

