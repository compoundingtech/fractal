//! Which claims this node wrote, found off the writer, once for every refresher that pauses on
//! them. A refresher that folds a published list pauses about a second between folds; a claim
//! this node commits that its fold reads cuts that pause short, so the person who wrote it sees
//! it after one fold, while claims other nodes wrote, and local claims the fold does not read,
//! keep the full pause. Which claims a fold reads is the fold's own change check, asked about
//! this node's claims only.
use super::*;
use std::future::Future;
use std::sync::Weak;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

/// The most claims one look reads. A longer run, as through a replication catch-up, is read a
/// run of this many at a time, each on its own blocking task, so no read grows with the catch-up
/// and none of it is skipped. There is no index by origin.
pub const LOCAL_SCAN: u64 = 10_000;

/// A refresher's fold's own change check, asked about this node's claims only: whether a claim
/// this node wrote after the first store index, through the second, changes what the fold
/// publishes. The scanner asks it once per run it reads, on the blocking pool, and only for a run
/// that holds a claim this node wrote. A check that fails counts as a change: it costs at most
/// one fold, never a missed claim.
pub type LocalCheck = Arc<dyn Fn(u64, u64) -> Result<bool> + Send + Sync>;

/// A check that every claim this node writes changes the fold: for a fold with no cheaper check.
pub fn every_local_claim(store: &Arc<Store>) -> LocalCheck {
    let store = Arc::clone(store);
    Arc::new(move |after, through| Ok(store.first_local_commit_after(after, through)?.is_some()))
}

/// What the scanner has read: every claim through `scanned`, and for each check, the end of the
/// newest run it read that holds a claim this node wrote that the check says changes its fold.
/// That end bounds the claim from above: a refresher whose fold read the claim but not the whole
/// run folds once more at its floor, never misses one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LocalCut {
    pub scanned: u64,
    pub newest_local: BTreeMap<&'static str, u64>,
}

impl LocalCut {
    /// The end of the newest run that holds a claim this node wrote that `check` says changes
    /// its fold (0 when none has).
    pub fn newest_local_of(&self, check: &str) -> u64 {
        self.newest_local.get(check).copied().unwrap_or(0)
    }
}

/// Each holder's check, and how far it has been asked: every claim after its registration point
/// through that index. A check asked through less than the scanner has read (as one registered
/// while a look ran) is asked about the rest on the next look, so none of its claims is passed
/// over.
type Checks = Arc<Mutex<BTreeMap<&'static str, (LocalCheck, u64)>>>;

/// One store's scanner, shared by every refresher that holds a [`LocalCommits`]: it ends when
/// the last of them drops it.
struct Scanner {
    cut: tokio::sync::watch::Receiver<LocalCut>,
    checks: Checks,
    looks: Arc<AtomicU64>,
    checked: Arc<AtomicU64>,
    looking: Arc<AtomicBool>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Scanner {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// The store's one scanner while any refresher holds it.
#[derive(Default)]
pub struct LocalCommitsSlot(Mutex<Weak<Scanner>>);

/// A refresher's hold on its store's scanner, waiting only for claims its fold's check names.
#[derive(Clone)]
pub struct LocalCommits {
    scanner: Arc<Scanner>,
    cut: tokio::sync::watch::Receiver<LocalCut>,
    check: &'static str,
}

impl Store {
    /// This store's scanner, started on first use from the current index: claims before it were
    /// folded by every refresher's first fold. Every holder shares it, so each commit is read
    /// once however many refreshers pause on it. Each holder names its fold's `check`; holders
    /// that name the same check share it, and the first one's closure is the one asked. Called
    /// from within a Tokio runtime.
    pub fn local_commits(self: &Arc<Self>, name: &'static str, check: LocalCheck) -> LocalCommits {
        let slot = self.extension::<LocalCommitsSlot>();
        let mut held = slot.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let scanner = match held.upgrade() {
            Some(scanner) => scanner,
            None => {
                let scanner = LocalCommits::scanning(self, self.index().unwrap_or(0), LOCAL_SCAN);
                *held = Arc::downgrade(&scanner);
                scanner
            }
        };
        LocalCommits::holding(scanner, name, check)
    }
}

impl LocalCommits {
    /// A scanner of its own, reading from `from` at most `scan` claims a look, for tests; it
    /// reads what is already committed after `from` at once.
    #[cfg(any(test, feature = "test-support"))]
    pub fn from_index(store: &Arc<Store>, from: u64, scan: u64, name: &'static str, check: LocalCheck) -> Self {
        Self::holding(Self::scanning(store, from, scan), name, check)
    }

    /// Another hold on this holder's scanner, under its own check, for tests.
    #[cfg(any(test, feature = "test-support"))]
    pub fn also(&self, name: &'static str, check: LocalCheck) -> Self {
        Self::holding(Arc::clone(&self.scanner), name, check)
    }

    /// Register `check` from what the scanner has published as read, never from a look still
    /// running: a claim that look passes over is the check's on the next.
    fn holding(scanner: Arc<Scanner>, name: &'static str, check: LocalCheck) -> Self {
        let from = scanner.cut.borrow().scanned;
        scanner.checks.lock().unwrap_or_else(std::sync::PoisonError::into_inner).entry(name).or_insert((check, from));
        let cut = scanner.cut.clone();
        LocalCommits { scanner, cut, check: name }
    }

    fn scanning(store: &Arc<Store>, from: u64, scan: u64) -> Arc<Scanner> {
        Self::scanning_gated(store, from, scan, None)
    }

    /// A scanner of its own whose first look, once it has taken its list of checks and before it
    /// reads the store's index, says so on the first receiver and then waits until the sender
    /// sends: for tests that register a check while a look runs.
    #[cfg(any(test, feature = "test-support"))]
    pub fn from_index_gated(
        store: &Arc<Store>,
        from: u64,
        scan: u64,
        name: &'static str,
        check: LocalCheck,
    ) -> (Self, std::sync::mpsc::Receiver<()>, std::sync::mpsc::SyncSender<()>) {
        let (snapshotted, taken) = std::sync::mpsc::sync_channel(1);
        let (gate, held) = std::sync::mpsc::sync_channel(0);
        let scanner = Self::scanning_gated(store, from, scan, Some((snapshotted, held)));
        (Self::holding(scanner, name, check), taken, gate)
    }

    fn scanning_gated(
        store: &Arc<Store>,
        from: u64,
        scan: u64,
        gate: Option<Gate>,
    ) -> Arc<Scanner> {
        let (sender, cut) = tokio::sync::watch::channel(LocalCut { scanned: from, newest_local: BTreeMap::new() });
        let checks = Checks::default();
        let looks = Arc::new(AtomicU64::new(0));
        let checked = Arc::new(AtomicU64::new(0));
        let looking = Arc::new(AtomicBool::new(false));
        // The writer's callback only wakes the scanner; it never reads.
        let commits = Arc::new(tokio::sync::Notify::new());
        let observer = {
            let commits = Arc::clone(&commits);
            store.observe_commits(move |_| commits.notify_one())
        };
        let task = tokio::spawn(scan_commits(
            Arc::clone(store),
            observer,
            commits,
            sender,
            scan,
            Scanned {
                checks: Arc::clone(&checks),
                looks: Arc::clone(&looks),
                checked: Arc::clone(&checked),
                looking: Arc::clone(&looking),
                gate: Arc::new(Mutex::new(gate)),
            },
        ));
        Arc::new(Scanner { cut, checks, looks, checked, looking, task })
    }

    /// Wait until the scanner has found a claim this node wrote after `through` that this
    /// holder's check says changes its fold. While none is found this waits for ever: a pause that
    /// waits on it ends at its own timer.
    pub async fn after(&mut self, through: u64) {
        let check = self.check;
        if self.cut.wait_for(|cut| cut.newest_local_of(check) > through).await.is_err() {
            std::future::pending::<()>().await;
        }
    }

    /// Wait until the scanner has found a claim this node wrote after `through` that its fold
    /// reads, then until `floor`: what a refresher whose pause has its own shape waits
    /// on to cut it.
    pub async fn after_until(&mut self, through: u64, floor: tokio::time::Instant) {
        self.after(through).await;
        tokio::time::sleep_until(floor).await;
    }

    /// What the scanner has read so far.
    pub fn cut(&self) -> LocalCut {
        self.cut.borrow().clone()
    }

    /// How many looks the scanner has started, for tests.
    pub fn looks(&self) -> u64 {
        self.scanner.looks.load(Ordering::Acquire)
    }

    /// How many times the scanner has asked a check, for tests: once per check for each run that
    /// holds a claim this node wrote.
    pub fn checked(&self) -> u64 {
        self.scanner.checked.load(Ordering::Acquire)
    }

    /// Whether a look is queued or running, for tests.
    pub fn looking(&self) -> bool {
        self.scanner.looking.load(Ordering::Acquire)
    }
}

/// What the scanner shares with its holders.
struct Scanned {
    checks: Checks,
    looks: Arc<AtomicU64>,
    checked: Arc<AtomicU64>,
    looking: Arc<AtomicBool>,
    /// For tests: the first look waits here before it reads the index.
    gate: Arc<Mutex<Option<Gate>>>,
}

/// For tests: where the first look says it has taken its checks, and what it then waits for.
type Gate = (std::sync::mpsc::SyncSender<()>, std::sync::mpsc::Receiver<()>);

/// Read each run of commits once, one look at a time on the blocking pool, and publish what it
/// found. Each check is asked once a look about the claims after where it was last asked, through
/// the look's end, and only when that run holds a claim this node wrote. A wake during a look
/// leaves its permit for the next; a failed look costs only the shortcut, and the next commit
/// reads from where the last good look ended.
async fn scan_commits(
    store: Arc<Store>,
    _observer: smallclaims::sqlite::CommitObserver,
    commits: Arc<tokio::sync::Notify>,
    sender: tokio::sync::watch::Sender<LocalCut>,
    scan: u64,
    shared: Scanned,
) {
    let mut scanned = sender.borrow().scanned;
    // What was committed after `from` before the observer registered is read at once.
    let mut more = store.index().is_ok_and(|index| index > scanned);
    loop {
        if !more {
            tokio::select! {
                () = commits.notified() => {}
                () = sender.closed() => return,
            }
        }
        shared.looks.fetch_add(1, Ordering::AcqRel);
        shared.looking.store(true, Ordering::Release);
        let reader = Arc::clone(&store);
        let checks: Vec<(&'static str, LocalCheck, u64)> = shared
            .checks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .map(|(name, (check, through))| (*name, Arc::clone(check), *through))
            .collect();
        let checked = Arc::clone(&shared.checked);
        let gate = shared.gate.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take();
        let read = tokio::task::spawn_blocking(move || {
            // The checks above are this look's: one registered from here on is the next look's.
            if let Some((snapshotted, gate)) = gate {
                let _ = snapshotted.send(());
                let _ = gate.recv();
            }
            let index = reader.index()?;
            let upto = index.min(scanned.saturating_add(scan));
            let mut asked = Vec::new();
            // Checks asked through the same index share one read of whether the run holds a
            // claim this node wrote.
            let mut local = BTreeMap::new();
            for (name, check, through) in checks {
                let to = index.min(through.saturating_add(scan));
                if to <= through {
                    continue;
                }
                let holds = match local.get(&(through, to)) {
                    Some(holds) => *holds,
                    None => {
                        let holds = reader.first_local_commit_after(through, to)?.is_some();
                        local.insert((through, to), holds);
                        holds
                    }
                };
                let found = holds && {
                    checked.fetch_add(1, Ordering::AcqRel);
                    check(through, to).unwrap_or(true)
                };
                asked.push((name, to, found));
            }
            anyhow::Ok((upto.max(scanned), asked, index))
        })
        .await;
        shared.looking.store(false, Ordering::Release);
        more = false;
        if let Ok(Ok((upto, asked, index))) = read {
            scanned = upto;
            let mut checks = shared.checks.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            for (name, to, _) in &asked {
                if let Some((_, through)) = checks.get_mut(name) {
                    *through = (*through).max(*to);
                }
            }
            // The rest of a long run, and a check behind it, are read at once, with no commit to
            // wait for.
            more = upto < index || checks.values().any(|(_, through)| *through < index);
            drop(checks);
            sender.send_modify(|cut| {
                cut.scanned = upto;
                for (name, to, found) in asked {
                    if found {
                        let newest = cut.newest_local.entry(name).or_default();
                        *newest = (*newest).max(to);
                    }
                }
            });
        }
    }
}

/// Why a refresher's pause between two folds ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PauseEnd {
    Full,
    FreshRead,
    LocalClaim,
}

/// How a refresher paces its folds: the pause after a fold that a fresh read cannot cut below,
/// the one a claim this node wrote cannot cut below, and the full pause otherwise.
#[derive(Clone, Copy, Debug)]
pub struct Pace {
    pub floor: Duration,
    pub local: Duration,
    pub full: Duration,
}

/// Pause after a fold that took `folded`. A fresh read ends it once it has lasted as long as the
/// fold, and at least the floor, so folding stays under about half a core however often readers
/// ask. A claim this node commits after `through` about a subject the fold reads, an index the
/// fold has surely read (a claim it folded after that costs one more fold, never a missed one),
/// ends it once it has lasted twice the fold, and at least the local floor, so a stream of local
/// writes folds no more often than the local floor allows and a fold of up to half the full
/// pause takes at most a third of a core. The cut never lengthens the pause: a longer fold's
/// local floor is the full pause, and the half-core bound it had. Otherwise it lasts the full
/// pause, or the fold's length if longer: claims other nodes wrote, and local claims the fold
/// does not read, never cut it, so a busy replication stream or a node's other writes fold no
/// more often.
///
/// The scanner runs on the blocking pool, but the pause never waits on it: a busy pool or a slow
/// look only costs the shortcut, never the timer or a fresh read.
pub async fn pause_after_fold(
    mut local: Option<&mut LocalCommits>,
    fresh: impl Future<Output = ()>,
    pace: Pace,
    folded: Duration,
    through: u64,
) -> PauseEnd {
    let started = tokio::time::Instant::now();
    let full = started + folded.max(pace.full);
    let fresh_floor = started + folded.max(pace.floor);
    // A local claim only ever shortens the pause: once its floor reaches the full pause, the
    // full pause ends it.
    let local_floor = started + (folded * 2).max(pace.local);
    let local_cuts = local_floor < full;
    tokio::pin!(fresh);
    let mut fresh_wanted = false;
    let mut local_found = false;
    loop {
        let floor = match (fresh_wanted, local_found) {
            (true, _) => Some(fresh_floor),
            (false, true) if local_cuts => Some(local_floor),
            (false, _) => None,
        };
        tokio::select! {
            () = tokio::time::sleep_until(full) => return PauseEnd::Full,
            () = async { tokio::time::sleep_until(floor.unwrap_or(full)).await }, if floor.is_some() => {
                return if fresh_wanted { PauseEnd::FreshRead } else { PauseEnd::LocalClaim };
            }
            () = &mut fresh, if !fresh_wanted => fresh_wanted = true,
            () = async {
                match local.as_deref_mut() {
                    Some(local) => local.after(through).await,
                    None => std::future::pending().await,
                }
            }, if !local_found => local_found = true,
        }
    }
}
