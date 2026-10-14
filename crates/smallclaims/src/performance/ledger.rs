//! Test builds only (`test-support`): the exact CPU charged to chosen kinds between two marks.
//!
//! `with_cpu` charges a span's thread CPU when the span ends, so a meter read at an instant
//! misses the part of a running span before it and counts all of a span that ended after it
//! began. This ledger records each span of a watched kind as it opens and closes, and at each
//! mark reads the CPU clock of every open span's thread, so the CPU between two marks is exact
//! for spans that cross either mark. A span's own start and end CPU are read under the same lock
//! as the marks, so the order of every read is the order the ledger records: no span's CPU before
//! a mark is counted after it. It is off until `start`, and only spans of the kinds `start` names
//! are kept: a bounded record for one measured interval. Another thread's CPU clock is read
//! through Linux's `pthread_getcpuclockid`; elsewhere every charge is `Unknown`.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicPtr, AtomicU64, Ordering};

/// More spans than this between `start` and the last mark make the charge lossy. A regime of the
/// load runs a few hundred reconcile passes; each span costs about a hundred bytes plus a sample
/// per mark it was open at.
const SPAN_LIMIT: usize = 100_000;
/// Marks after this many return `None`: the load takes two per regime.
const MARK_LIMIT: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mark {
    generation: u64,
    index: usize,
}

/// Why a charge cannot stand for the CPU between its marks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    /// Every span of the kind between the marks is counted exactly.
    Complete,
    /// An open span's thread clock could not be read at a mark.
    Incomplete,
    /// Spans were dropped at the limit.
    Lossy,
    /// The ledger was stopped or started again, or never started, between the marks; a mark from
    /// another ledger; or a platform where it keeps no spans.
    #[default]
    Unknown,
}

/// The CPU charged to one kind between two marks.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Charge {
    pub cpu_ns: u64,
    /// Spans that ran between the marks.
    pub spans: u64,
    /// Spans open at the first mark, and at the second.
    pub open_at_start: u64,
    pub open_at_end: u64,
    /// A mark reads its open spans' clocks one after another, not at one instant: the longer of
    /// the two marks' reads, in ns. A span's share near a mark is uncertain by at most this.
    #[serde(default)]
    pub mark_spread_ns: u64,
    pub status: Status,
}

impl Charge {
    /// A charge that stands for nothing: no ledger, or no marks to bound it.
    pub fn unknown() -> Self {
        Self::default()
    }
}

struct Span {
    kind: String,
    clock: libc::clockid_t,
    start_ns: u64,
    /// Marks taken before it opened, and before it closed (`None` while open).
    opened_after: usize,
    closed_after: Option<usize>,
    end_ns: u64,
    /// Its thread's CPU at each mark taken while it was open; `None` where unreadable.
    samples: BTreeMap<usize, Option<u64>>,
}

#[derive(Default)]
struct State {
    generation: u64,
    kinds: Vec<String>,
    spans: Vec<Span>,
    open: Vec<usize>,
    marks: usize,
    /// How long each mark took to read its open spans' clocks, in ns.
    spreads: Vec<u64>,
    lost: u64,
    limit: usize,
}

static LEDGER: Mutex<Option<State>> = Mutex::new(None);

/// Every ledger `start` makes gets a generation no other has had, across `stop`s, so a mark or a
/// span handle from an earlier ledger never matches a later one.
static GENERATIONS: AtomicU64 = AtomicU64::new(0);

/// The kinds the ledger watches, null while it is off: every `with_cpu` reads this one atomic,
/// and only a span of a watched kind takes the ledger's lock. It points into `LISTS`.
static WATCHED: AtomicPtr<&'static [&'static str]> = AtomicPtr::new(std::ptr::null_mut());

/// Each distinct list of watched kinds, boxed once and kept for the process, so a pointer `open`
/// read from `WATCHED` stays valid without freeing what another thread may still be reading.
/// Starting again with the same kinds reuses its box; at most `LIST_LIMIT` lists exist, and a
/// `start` with one more leaves the ledger off.
static LISTS: Mutex<Vec<&'static &'static [&'static str]>> = Mutex::new(Vec::new());
const LIST_LIMIT: usize = 16;

/// Keep a ledger of the spans of `kinds` from now, forgetting any earlier one.
pub fn start(kinds: &'static [&'static str]) {
    let mut ledger = LEDGER.lock().unwrap();
    let list = {
        let mut lists = LISTS.lock().unwrap();
        match lists.iter().copied().find(|&list| *list == kinds) {
            Some(list) => Some(list),
            None if lists.len() < LIST_LIMIT => {
                let list: &'static &'static [&'static str] = Box::leak(Box::new(kinds));
                lists.push(list);
                Some(list)
            }
            None => None,
        }
    };
    // Refuse to start rather than reuse a generation or keep another list.
    let generation = next_generation(&GENERATIONS);
    let (Some(list), Some(generation)) = (list, generation) else {
        *ledger = None;
        WATCHED.store(std::ptr::null_mut(), Ordering::Release);
        return;
    };
    *ledger = Some(State {
        generation,
        kinds: kinds.iter().map(|kind| (*kind).to_owned()).collect(),
        limit: SPAN_LIMIT,
        ..State::default()
    });
    WATCHED.store(std::ptr::from_ref(list).cast_mut(), Ordering::Release);
}

/// The next generation `counter` allocates, or `None` from then on once it is exhausted: the
/// counter stops at its maximum rather than wrap to a generation already issued.
fn next_generation(counter: &AtomicU64) -> Option<u64> {
    counter
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |issued| issued.checked_add(1))
        .ok()
        .map(|issued| issued + 1)
}

/// Stop keeping the ledger and drop what it holds.
pub fn stop() {
    let mut ledger = LEDGER.lock().unwrap();
    WATCHED.store(std::ptr::null_mut(), Ordering::Release);
    *ledger = None;
}

/// Read every open span's thread CPU now.
pub fn mark() -> Option<Mark> {
    let mut ledger = LEDGER.lock().unwrap();
    let state = ledger.as_mut()?;
    if state.marks >= MARK_LIMIT {
        return None;
    }
    let index = state.marks;
    state.marks += 1;
    let began = std::time::Instant::now();
    for &span in &state.open {
        let span = &mut state.spans[span];
        let sample = clock_ns(span.clock);
        span.samples.insert(index, sample);
    }
    state.spreads.push(began.elapsed().as_nanos() as u64);
    Some(Mark { generation: state.generation, index })
}

/// The CPU charged to `kind` between `from` and `to`.
pub fn charge(kind: &str, from: Mark, to: Mark) -> Charge {
    let unknown = Charge::unknown();
    if cfg!(not(target_os = "linux")) {
        return unknown;
    }
    let ledger = LEDGER.lock().unwrap();
    let Some(state) = ledger.as_ref() else {
        return unknown;
    };
    if from.generation != state.generation
        || to.generation != state.generation
        || from.index >= to.index
        || to.index >= state.marks
        || !state.kinds.iter().any(|watched| watched == kind)
    {
        return unknown;
    }
    let mut charge = Charge {
        cpu_ns: 0,
        spans: 0,
        open_at_start: 0,
        open_at_end: 0,
        mark_spread_ns: state.spreads[from.index].max(state.spreads[to.index]),
        status: if state.lost > 0 { Status::Lossy } else { Status::Complete },
    };
    let mut incomplete = false;
    for span in state.spans.iter().filter(|span| span.kind == kind) {
        // Opened after the second mark, or closed before the first: none of it is between.
        if span.opened_after > to.index || span.closed_after.is_some_and(|closed| closed <= from.index) {
            continue;
        }
        let open_at_start = span.opened_after <= from.index;
        let open_at_end = span.closed_after.is_none_or(|closed| closed > to.index);
        let begin = if open_at_start { span.samples.get(&from.index).copied().flatten() } else { Some(span.start_ns) };
        let end = if open_at_end { span.samples.get(&to.index).copied().flatten() } else { Some(span.end_ns) };
        charge.spans += 1;
        charge.open_at_start += u64::from(open_at_start);
        charge.open_at_end += u64::from(open_at_end);
        // A thread's CPU clock never runs backward; a pair that does is not a measurement.
        match (begin, end) {
            (Some(begin), Some(end)) if begin <= end => charge.cpu_ns += end - begin,
            _ => incomplete = true,
        }
    }
    if incomplete && charge.status == Status::Complete {
        charge.status = Status::Incomplete;
    }
    charge
}

/// A span of a watched kind opening on this thread; `None` when the ledger is off or does not
/// watch `kind`. Its start CPU is read under the ledger's lock.
#[cfg(not(target_os = "linux"))]
pub(super) fn open(_kind: &str) -> Option<(u64, usize)> {
    None
}

#[cfg(target_os = "linux")]
pub(super) fn open(kind: &str) -> Option<(u64, usize)> {
    let watched = WATCHED.load(Ordering::Acquire);
    // SAFETY: a non-null pointer points into `LISTS`, whose boxes are never freed.
    if watched.is_null() || !unsafe { *watched }.contains(&kind) {
        return None;
    }
    let mut ledger = LEDGER.lock().unwrap();
    let state = ledger.as_mut()?;
    if !state.kinds.iter().any(|watched| watched == kind) {
        return None;
    }
    if state.spans.len() >= state.limit {
        state.lost += 1;
        return None;
    }
    let mut clock: libc::clockid_t = 0;
    // SAFETY: pthread_self names this live thread; clock receives its CPU clock id.
    let start_ns = (unsafe { libc::pthread_getcpuclockid(libc::pthread_self(), &mut clock) } == 0)
        .then(|| clock_ns(clock))
        .flatten();
    let Some(start_ns) = start_ns else {
        state.lost += 1;
        return None;
    };
    let index = state.spans.len();
    state.spans.push(Span {
        kind: kind.to_owned(),
        clock,
        start_ns,
        opened_after: state.marks,
        closed_after: None,
        end_ns: 0,
        samples: BTreeMap::new(),
    });
    state.open.push(index);
    Some((state.generation, index))
}

/// The span `open` returned closing, on the thread that opened it. Its end CPU is read under the
/// ledger's lock. A handle from another ledger, or one already closed, changes nothing.
pub(super) fn close((generation, index): (u64, usize)) {
    let mut ledger = LEDGER.lock().unwrap();
    let Some(state) = ledger.as_mut().filter(|state| state.generation == generation) else {
        return;
    };
    let marks = state.marks;
    let Some(span) = state.spans.get_mut(index).filter(|span| span.closed_after.is_none()) else {
        return;
    };
    // An unreadable end leaves the span open-ended: a charge that needs it is incomplete.
    match clock_ns(span.clock) {
        Some(end_ns) => {
            span.end_ns = end_ns;
            span.closed_after = Some(marks);
        }
        None => state.lost += 1,
    }
    state.open.retain(|&open| open != index);
}

fn clock_ns(clock: libc::clockid_t) -> Option<u64> {
    let mut time = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: time points to a valid timespec; clock is an open span's live thread's CPU clock.
    (unsafe { libc::clock_gettime(clock, &mut time) } == 0).then(|| {
        (time.tv_sec as u64).saturating_mul(1_000_000_000).saturating_add(time.tv_nsec as u64)
    })
}

#[cfg(all(test, target_os = "linux"))]
#[path = "ledger_tests.rs"]
mod tests;

#[cfg(all(test, not(target_os = "linux")))]
mod unsupported {
    #[test]
    fn every_charge_is_unknown_where_no_span_is_kept() {
        super::start(&["task reconcile-pass"]);
        let from = super::mark().unwrap();
        let to = super::mark().unwrap();
        assert_eq!(super::charge("task reconcile-pass", from, to).status, super::Status::Unknown);
        super::stop();
    }
}
