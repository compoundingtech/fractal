//! The member's CPU by closed UTC minute, for the CPU targets in `slo/targets.toml`: the daemon's
//! own process CPU and each replication worker's, with what kept each minute from being idle.
//!
//! The daemon reads its own CPU at each minute's boundary. A replication worker sends its
//! cumulative process CPU every [`WORKER_REPORT`], and the newest reading the daemon holds at a
//! boundary stands for the worker's CPU there: a worker's CPU lands in the minute the daemon heard
//! it in, at most one report late, and nothing is lost between minutes. A minute's CPU is known
//! only when the workers the member runs each had a reading at both its boundaries, no older than
//! [`WORKER_FRESH`], from the same process: a worker that restarted, went quiet or whose counter
//! went backwards leaves the minute's CPU unproven.
//!
//! A minute is idle only when that is known: its CPU is known, its seats were checked and none was
//! working, and nothing else was noted in it (the daemon starting, a deploy, a replication
//! catch-up). A minute the ledger does not hold, from before the daemon started or one whose
//! boundary was missed, is neither idle nor busy: it is missing, and the hour is then incomplete.

use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

/// Closed minutes kept, and the targets' window: one hour.
pub const MINUTES: u64 = 60;
/// How often a replication worker sends its process CPU.
pub const WORKER_REPORT: Duration = Duration::from_secs(10);
/// A worker reading older than this at a boundary proves nothing about the worker's CPU there.
pub const WORKER_FRESH: Duration = Duration::from_secs(25);
const MINUTE_MS: u64 = 60_000;

/// What kept a minute from being idle, besides a working seat or CPU that is not known.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Busy {
    /// The daemon started inside it.
    Start,
    /// A deploy was being checked on this member.
    Deploy,
    /// Replication was catching up with a peer.
    CatchUp,
}

impl Busy {
    const ALL: [Busy; 3] = [Busy::Start, Busy::Deploy, Busy::CatchUp];

    fn bit(self) -> u8 {
        1 << self as u8
    }

    pub fn name(self) -> &'static str {
        match self {
            Busy::Start => "start",
            Busy::Deploy => "deploy",
            Busy::CatchUp => "catch-up",
        }
    }
}

/// A replication worker process: its pid and when it started, in Unix milliseconds.
pub type Worker = (u32, u64);

struct Open {
    minute: u64,
    at: Instant,
    daemon_ns: u64,
    /// `None` when a worker the member runs had no fresh reading.
    workers: Option<BTreeMap<Worker, u64>>,
    busy: u8,
}

struct Closed {
    minute: u64,
    /// The daemon's and the workers' CPU over the minute; `None` when a worker's is not known.
    cpu_ns: Option<u64>,
    wall_ns: u64,
    busy: u8,
    /// Whether a seat was working, or not known not to be; `None` until the seats are checked.
    seats_busy: Option<bool>,
}

impl Closed {
    fn idle(&self) -> bool {
        self.cpu_ns.is_some() && self.busy == 0 && self.seats_busy == Some(false)
    }
}

#[derive(Default)]
pub struct CpuMinutes {
    open: Option<Open>,
    closed: VecDeque<Closed>,
    /// Each worker's newest reading and when it arrived.
    workers: BTreeMap<Worker, (u64, Instant)>,
    started: bool,
}

impl CpuMinutes {
    /// A worker's cumulative process CPU, as it reported it at `at`.
    pub fn note_worker(&mut self, worker: Worker, cpu_ns: u64, at: Instant) {
        self.workers.insert(worker, (cpu_ns, at));
        // A worker that stopped reporting is no longer fresh; past a minute it is forgotten.
        self.workers
            .retain(|_, (_, seen)| at.saturating_duration_since(*seen) <= Duration::from_secs(60));
    }

    /// Note what keeps the open minute from being idle.
    pub fn note(&mut self, busy: Busy) {
        if let Some(open) = &mut self.open {
            open.busy |= busy.bit();
        }
    }

    fn fresh_workers(&self, at: Instant, expected: usize) -> Option<BTreeMap<Worker, u64>> {
        let fresh = self
            .workers
            .iter()
            .filter(|(_, (_, seen))| at.saturating_duration_since(*seen) <= WORKER_FRESH)
            .map(|(worker, (cpu, _))| (*worker, *cpu))
            .collect::<BTreeMap<_, _>>();
        (fresh.len() >= expected).then_some(fresh)
    }

    /// The boundary that starts UTC `minute`, read at `at` with the daemon's CPU `daemon_ns`:
    /// close the minute before it and open this one. `expected_workers` is how many replication
    /// workers the member runs. The first boundary is the daemon's start, inside its minute.
    /// Returns the minute it closed, whose seats are then [`settle`](Self::settle)d.
    pub fn boundary(
        &mut self,
        minute: u64,
        at: Instant,
        daemon_ns: u64,
        expected_workers: usize,
    ) -> Option<u64> {
        if self.open.as_ref().is_some_and(|open| open.minute >= minute) {
            return None;
        }
        let workers = self.fresh_workers(at, expected_workers);
        let first = !std::mem::replace(&mut self.started, true);
        let mut closed = None;
        // A skipped boundary (a late tick, a machine asleep) leaves its minutes out.
        if let Some(open) = self.open.take().filter(|open| open.minute + 1 == minute) {
            let workers_ns = match (&open.workers, &workers) {
                (Some(from), Some(to)) if from.keys().eq(to.keys()) => from
                    .values()
                    .zip(to.values())
                    .try_fold(0_u64, |sum, (from, to)| to.checked_sub(*from).map(|ns| sum.saturating_add(ns))),
                _ => None,
            };
            self.closed.push_back(Closed {
                minute: open.minute,
                cpu_ns: workers_ns.zip(daemon_ns.checked_sub(open.daemon_ns)).map(|(w, d)| w.saturating_add(d)),
                wall_ns: u64::try_from(at.saturating_duration_since(open.at).as_nanos()).unwrap_or(u64::MAX),
                busy: open.busy,
                seats_busy: None,
            });
            closed = Some(open.minute);
        }
        self.open = Some(Open {
            minute,
            at,
            daemon_ns,
            workers,
            busy: if first { Busy::Start.bit() } else { 0 },
        });
        while self.closed.front().is_some_and(|old| old.minute + MINUTES < minute) {
            self.closed.pop_front();
        }
        closed
    }

    /// Whether a seat on this member was working in closed `minute`, or not known not to be;
    /// `None` when that could not be checked.
    pub fn settle(&mut self, minute: u64, seats_busy: Option<bool>) {
        if let Some(closed) = self.closed.iter_mut().rev().find(|closed| closed.minute == minute) {
            closed.seats_busy = seats_busy;
        }
    }

    /// The closed minutes of the hour before `now_unix_ms`'s minute: the cores over the last
    /// minute, five and sixty, each `null` unless every minute in it is held with its CPU known;
    /// how many minutes the hour holds; and its idle minutes, their cores and why the others were
    /// not idle (a minute may have several reasons).
    pub fn report(&self, now_unix_ms: u64) -> Value {
        let now = now_unix_ms / MINUTE_MS;
        let hour = self
            .closed
            .iter()
            .filter(|closed| closed.minute + MINUTES >= now && closed.minute < now)
            .collect::<Vec<_>>();
        let cores = |minutes: &[&Closed]| {
            let cpu = minutes.iter().map(|closed| closed.cpu_ns.unwrap_or(0) as f64).sum::<f64>();
            let wall = minutes.iter().map(|closed| closed.wall_ns as f64).sum::<f64>();
            (wall > 0.0).then(|| (cpu / wall * 1000.0).round() / 1000.0)
        };
        let mut windows = serde_json::Map::new();
        for (name, span) in [("1m", 1), ("5m", 5), ("1h", MINUTES)] {
            let known = hour
                .iter()
                .filter(|closed| closed.minute + span >= now && closed.cpu_ns.is_some())
                .copied()
                .collect::<Vec<_>>();
            let count = known.len() as u64;
            windows.insert(
                name.into(),
                json!({"cores": if count == span { cores(&known) } else { None }, "minutes": count}),
            );
        }
        let idle = hour.iter().filter(|closed| closed.idle()).copied().collect::<Vec<_>>();
        let mut not_idle = serde_json::Map::new();
        let count = |test: &dyn Fn(&Closed) -> bool| hour.iter().filter(|closed| test(closed)).count();
        not_idle.insert("seat".into(), json!(count(&|closed| closed.seats_busy == Some(true))));
        for busy in Busy::ALL {
            not_idle.insert(busy.name().into(), json!(count(&|closed| closed.busy & busy.bit() != 0)));
        }
        not_idle.insert(
            "unknown".into(),
            json!(count(&|closed| closed.cpu_ns.is_none() || closed.seats_busy.is_none())),
        );
        json!({
            "windows": windows,
            "minutes": hour.len(),
            "idle": {"minutes": idle.len(), "cores": cores(&idle), "not_idle": not_idle},
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORKER: Worker = (41, 1_000);
    const SECOND: u64 = 1_000_000_000;

    /// A ledger that closed `minutes` quiet minutes from minute 100, the daemon using a tenth of
    /// a core and its worker a tenth more. The first, the daemon's start, is not idle.
    fn quiet(minutes: u64) -> (CpuMinutes, Instant) {
        let start = Instant::now();
        let mut ledger = CpuMinutes::default();
        for k in 0..=minutes {
            let at = start + Duration::from_secs(60 * k);
            ledger.note_worker(WORKER, 6 * SECOND * k, at);
            if let Some(closed) = ledger.boundary(100 + k, at, 6 * SECOND * k, 1) {
                ledger.settle(closed, Some(false));
            }
        }
        (ledger, start)
    }

    fn report(ledger: &CpuMinutes, minute: u64) -> Value {
        ledger.report(minute * MINUTE_MS + 5)
    }

    #[test]
    fn the_numerator_is_the_daemon_and_its_workers() {
        let (ledger, _) = quiet(61);
        let report = report(&ledger, 161);
        for window in ["1m", "5m", "1h"] {
            assert_eq!(report["windows"][window]["cores"], 0.2, "{window}: {report}");
        }
        assert_eq!(report["windows"]["1h"]["minutes"], 60);
        assert_eq!(report["minutes"], 60);
        assert_eq!(report["idle"]["minutes"], 60);
        assert_eq!(report["idle"]["cores"], 0.2);
    }

    #[test]
    fn a_minute_is_idle_only_when_everything_about_it_is_known() {
        let (mut ledger, start) = quiet(3);
        let mut minute = 103;
        // Close the open minute: `readings` are the workers that report, and how many seconds
        // before the boundary; `seats` is what checking its seats found.
        let mut close = |ledger: &mut CpuMinutes, readings: &[(Worker, u64)], seats: Option<bool>, busy: Option<Busy>| {
            if let Some(busy) = busy {
                ledger.note(busy);
            }
            minute += 1;
            let at = start + Duration::from_secs(60 * (minute - 100));
            for (worker, before) in readings {
                let seen = at - Duration::from_secs(*before);
                ledger.note_worker(*worker, 6 * SECOND * (minute - 100) - before * SECOND / 10, seen);
            }
            let closed = ledger.boundary(minute, at, 6 * SECOND * (minute - 100), 1).unwrap();
            ledger.settle(closed, seats);
        };
        let restarted: Worker = (42, 2_000);
        close(&mut ledger, &[(WORKER, 0)], Some(false), None); // 103: idle
        close(&mut ledger, &[(WORKER, 0)], Some(true), None); // 104: a seat was working
        close(&mut ledger, &[(WORKER, 0)], None, None); // 105: the seats could not be checked
        close(&mut ledger, &[(WORKER, 0)], Some(false), Some(Busy::CatchUp)); // 106
        close(&mut ledger, &[(WORKER, 0)], Some(false), Some(Busy::Deploy)); // 107
        close(&mut ledger, &[(WORKER, 30)], Some(false), None); // 108: its end has no fresh reading
        close(&mut ledger, &[(WORKER, 0)], Some(false), None); // 109: nor its start
        close(&mut ledger, &[(WORKER, 0)], Some(false), None); // 110: idle
        // The worker restarts 10 seconds before 112's boundary: two processes at that boundary.
        close(&mut ledger, &[(WORKER, 20), (restarted, 10)], Some(false), None); // 111
        close(&mut ledger, &[(restarted, 0)], Some(false), None); // 112
        close(&mut ledger, &[(restarted, 0)], Some(false), None); // 113: idle again
        let report = report(&ledger, 114);
        let not_idle = &report["idle"]["not_idle"];
        assert_eq!(report["minutes"], 14);
        // Minutes 101, 102, 103, 110 and 113; 100 is the daemon's start.
        assert_eq!(report["idle"]["minutes"], 5, "{report}");
        assert_eq!(not_idle["start"], 1);
        assert_eq!(not_idle["seat"], 1);
        assert_eq!(not_idle["catch-up"], 1);
        assert_eq!(not_idle["deploy"], 1);
        // 105's seats, 108 and 109 around the stale reading, and 111 and 112 across the restart.
        assert_eq!(not_idle["unknown"], 5, "{report}");
        // A window over a minute whose CPU is not known has no cores.
        assert_eq!(report["windows"]["5m"]["cores"], Value::Null);
        assert_eq!(report["windows"]["5m"]["minutes"], 2);
        assert_eq!(report["windows"]["1m"]["cores"], 0.2);
    }

    #[test]
    fn a_worker_counter_that_went_backwards_proves_nothing() {
        let (mut ledger, start) = quiet(2);
        let at = start + Duration::from_secs(180);
        ledger.note_worker(WORKER, SECOND, at);
        let closed = ledger.boundary(103, at, 18 * SECOND, 1).unwrap();
        ledger.settle(closed, Some(false));
        let report = report(&ledger, 103);
        assert_eq!(report["idle"]["minutes"], 1, "{report}");
        assert_eq!(report["idle"]["not_idle"]["unknown"], 1);
    }

    #[test]
    fn a_member_without_workers_needs_none_and_one_with_a_worker_needs_its_reading() {
        let start = Instant::now();
        let mut alone = CpuMinutes::default();
        let mut expecting = CpuMinutes::default();
        for k in 0..=2 {
            let at = start + Duration::from_secs(60 * k);
            if let Some(closed) = alone.boundary(100 + k, at, 3 * SECOND * k, 0) {
                alone.settle(closed, Some(false));
            }
            if let Some(closed) = expecting.boundary(100 + k, at, 3 * SECOND * k, 1) {
                expecting.settle(closed, Some(false));
            }
        }
        assert_eq!(report(&alone, 102)["windows"]["1m"]["cores"], 0.05);
        assert_eq!(report(&expecting, 102)["windows"]["1m"]["cores"], Value::Null);
        assert_eq!(report(&expecting, 102)["idle"]["minutes"], 0);
    }

    #[test]
    fn a_missed_boundary_leaves_its_minute_out_and_the_hour_incomplete() {
        let (mut ledger, start) = quiet(2);
        // Minute 102's boundary was missed: it is not in the ledger, and 103 opens fresh.
        let at = start + Duration::from_secs(240);
        ledger.note_worker(WORKER, 24 * SECOND, at);
        assert_eq!(ledger.boundary(104, at, 24 * SECOND, 1), None);
        let at = start + Duration::from_secs(300);
        ledger.note_worker(WORKER, 30 * SECOND, at);
        let closed = ledger.boundary(105, at, 30 * SECOND, 1).unwrap();
        ledger.settle(closed, Some(false));
        let report = report(&ledger, 105);
        assert_eq!(report["minutes"], 3, "100, 101 and 104");
        assert_eq!(report["windows"]["1h"]["cores"], Value::Null);
        assert_eq!(report["windows"]["1m"]["cores"], 0.2);
        // A boundary seen twice changes nothing.
        assert_eq!(ledger.boundary(105, at, 30 * SECOND, 1), None);
        // An unsettled minute is not idle.
        let at = start + Duration::from_secs(360);
        ledger.note_worker(WORKER, 36 * SECOND, at);
        ledger.boundary(106, at, 36 * SECOND, 1).unwrap();
        assert_eq!(ledger.report(106 * MINUTE_MS)["idle"]["minutes"], 2);
    }

    #[test]
    fn the_ledger_keeps_one_hour() {
        let (ledger, _) = quiet(200);
        assert!(ledger.closed.len() as u64 <= MINUTES + 1);
        assert_eq!(report(&ledger, 300)["minutes"], 60);
    }
}
