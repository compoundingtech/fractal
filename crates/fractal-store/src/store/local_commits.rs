//! Which claims this node wrote, found off the writer, once for every refresher that pauses on
//! them. A refresher that folds a published list pauses about a second between folds; a claim
//! this node commits cuts that pause short, so the person who wrote it sees it after one fold,
//! while claims other nodes wrote keep the full pause.
use super::*;
use std::future::Future;
use std::sync::Weak;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

/// The most claims one look reads. A longer run, as through a replication catch-up, is read a
/// run of this many at a time, each on its own blocking task, so no read grows with the catch-up
/// and none of it is skipped. There is no index by origin.
pub const LOCAL_SCAN: u64 = 10_000;

/// What the scanner has read: every claim through `scanned`, and the end of the newest run it read
/// that holds a claim this node wrote (0 when none has). That end bounds the claim from above: a
/// refresher whose fold read the claim but not the whole run folds once more at its floor, never
/// misses one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LocalCut {
    pub scanned: u64,
    pub newest_local: u64,
}

/// One store's scanner, shared by every refresher that holds a [`LocalCommits`]: it ends when
/// the last of them drops it.
struct Scanner {
    cut: tokio::sync::watch::Receiver<LocalCut>,
    looks: Arc<AtomicU64>,
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

/// A refresher's hold on its store's scanner.
#[derive(Clone)]
pub struct LocalCommits {
    scanner: Arc<Scanner>,
    cut: tokio::sync::watch::Receiver<LocalCut>,
}

impl Store {
    /// This store's scanner, started on first use from the current index: claims before it were
    /// folded by every refresher's first fold. Every holder shares it, so each commit is read
    /// once however many refreshers pause on it. Called from within a Tokio runtime.
    pub fn local_commits(self: &Arc<Self>) -> LocalCommits {
        let slot = self.extension::<LocalCommitsSlot>();
        let mut held = slot.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(scanner) = held.upgrade() {
            let cut = scanner.cut.clone();
            return LocalCommits { scanner, cut };
        }
        let from = self.index().unwrap_or(0);
        let commits = LocalCommits::scanning(self, from, LOCAL_SCAN);
        *held = Arc::downgrade(&commits.scanner);
        commits
    }
}

impl LocalCommits {
    /// A scanner of its own, reading from `from` at most `scan` claims a look, for tests; it
    /// reads what is already committed after `from` at once.
    #[cfg(any(test, feature = "test-support"))]
    pub fn from_index(store: &Arc<Store>, from: u64, scan: u64) -> Self {
        Self::scanning(store, from, scan)
    }

    fn scanning(store: &Arc<Store>, from: u64, scan: u64) -> Self {
        let (sender, cut) = tokio::sync::watch::channel(LocalCut { scanned: from, newest_local: 0 });
        let looks = Arc::new(AtomicU64::new(0));
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
            Arc::clone(&looks),
            Arc::clone(&looking),
        ));
        let scanner = Arc::new(Scanner { cut: cut.clone(), looks, looking, task });
        LocalCommits { scanner, cut }
    }

    /// Wait until the scanner has found a claim this node wrote after `through`. While none is
    /// found this waits for ever: a pause that waits on it ends at its own timer.
    pub async fn after(&mut self, through: u64) {
        if self.cut.wait_for(|cut| cut.newest_local > through).await.is_err() {
            std::future::pending::<()>().await;
        }
    }

    /// Wait until the scanner has found a claim this node wrote after `through`, then until
    /// `floor`: what a refresher whose pause has its own shape waits on to cut it.
    pub async fn after_until(&mut self, through: u64, floor: tokio::time::Instant) {
        self.after(through).await;
        tokio::time::sleep_until(floor).await;
    }

    /// What the scanner has read so far.
    pub fn cut(&self) -> LocalCut {
        *self.cut.borrow()
    }

    /// How many looks the scanner has started, for tests.
    pub fn looks(&self) -> u64 {
        self.scanner.looks.load(Ordering::Acquire)
    }

    /// Whether a look is queued or running, for tests.
    pub fn looking(&self) -> bool {
        self.scanner.looking.load(Ordering::Acquire)
    }
}

/// Read each run of commits once, one look at a time on the blocking pool, and publish what it
/// found. A wake during a look leaves its permit for the next; a failed look costs only the
/// shortcut, and the next commit reads from where the last good look ended.
async fn scan_commits(
    store: Arc<Store>,
    _observer: smallclaims::sqlite::CommitObserver,
    commits: Arc<tokio::sync::Notify>,
    sender: tokio::sync::watch::Sender<LocalCut>,
    scan: u64,
    looks: Arc<AtomicU64>,
    looking: Arc<AtomicBool>,
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
        looks.fetch_add(1, Ordering::AcqRel);
        looking.store(true, Ordering::Release);
        let reader = Arc::clone(&store);
        let read = tokio::task::spawn_blocking(move || {
            let index = reader.index()?;
            let upto = index.min(scanned.saturating_add(scan));
            if upto <= scanned {
                return anyhow::Ok((scanned, None, false));
            }
            Ok((upto, reader.first_local_commit_after(scanned, upto)?.map(|_| upto), upto < index))
        })
        .await;
        looking.store(false, Ordering::Release);
        more = false;
        if let Ok(Ok((upto, found, rest))) = read {
            scanned = upto;
            sender.send_modify(|cut| {
                cut.scanned = upto;
                if let Some(index) = found {
                    cut.newest_local = cut.newest_local.max(index);
                }
            });
            // The rest of a long run is read at once, with no commit to wait for.
            more = rest;
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

/// How a refresher paces its folds: the pause after a fold that a fresh read or a claim this
/// node wrote cannot cut below, and the full pause otherwise.
#[derive(Clone, Copy, Debug)]
pub struct Pace {
    pub floor: Duration,
    pub full: Duration,
}

/// Pause after a fold that took `folded`. A fresh read ends it once it has lasted as long as the
/// fold, and at least the floor, so folding stays under about half a core however often readers
/// ask. A claim this node commits after `through`, an index the fold has surely read (a claim it
/// folded after that costs one more fold, never a missed one), ends it once it has lasted twice
/// the fold, and at least the floor, so a cheap fold under a stream of local writes takes at most
/// a third of a core. Otherwise it lasts the full pause, or the fold's length if longer: claims
/// other nodes wrote never cut it, so a busy replication stream folds no more often.
///
/// The scanner runs on the blocking pool, but the pause never waits on it: a busy pool or a slow
/// look only costs the shortcut, never the timer or a fresh read.
pub async fn pause_after_fold(
    local: &mut LocalCommits,
    fresh: impl Future<Output = ()>,
    pace: Pace,
    folded: Duration,
    through: u64,
) -> PauseEnd {
    let started = tokio::time::Instant::now();
    let full = started + folded.max(pace.full);
    let fresh_floor = started + folded.max(pace.floor);
    let local_floor = (started + (folded * 2).max(pace.floor)).min(full);
    tokio::pin!(fresh);
    let mut fresh_wanted = false;
    let mut local_found = false;
    loop {
        let floor = match (fresh_wanted, local_found) {
            (true, _) => Some(fresh_floor),
            (false, true) => Some(local_floor),
            (false, false) => None,
        };
        tokio::select! {
            () = tokio::time::sleep_until(full) => return PauseEnd::Full,
            () = async { tokio::time::sleep_until(floor.unwrap_or(full)).await }, if floor.is_some() => {
                return if fresh_wanted { PauseEnd::FreshRead } else { PauseEnd::LocalClaim };
            }
            () = &mut fresh, if !fresh_wanted => fresh_wanted = true,
            () = local.after(through), if !local_found => local_found = true,
        }
    }
}
