//! Process-local admission for launchd RPCs. Health reads never wait for the RPC.
use std::cell::Cell;
use std::io;
use std::process::Output;
use std::sync::{LazyLock, Mutex, TryLockError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;

const INITIAL_BACKOFF: Duration = Duration::from_secs(5);
const MAX_BACKOFF: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Reason {
    LaunchdUnresponsive,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct Health {
    pub reason: Reason,
    pub since_unix_ms: u64,
    pub retry_after_unix_ms: u64,
    pub consecutive_timeouts: u32,
}

struct Failure {
    health: Health,
    retry_at: Instant,
    backoff: Duration,
}

#[derive(Default)]
struct Gate {
    flight: Mutex<()>,
    failure: Mutex<Option<Failure>>,
}

impl Gate {
    fn run(&self, command: impl FnOnce(&dyn Fn()) -> io::Result<Output>, clock: impl Fn() -> (Instant, u64)) -> io::Result<Output> {
        // Never queue worker requests behind a stalled supervisor or start a second child.
        let _flight = match self.flight.try_lock() {
            Ok(flight) => flight,
            Err(TryLockError::WouldBlock) => return Err(io::Error::new(io::ErrorKind::WouldBlock, "launchctl probe already in flight")),
            Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        };
        let (now, _) = clock();
        {
            let failure = self.failure.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(failure) = failure.as_ref().filter(|failure| now < failure.retry_at) {
                return Err(io::Error::new(io::ErrorKind::WouldBlock,
                    format!("launchd-unresponsive since {}; retry after {}", failure.health.since_unix_ms, failure.health.retry_after_unix_ms)));
            }
        }
        let published = Cell::new(false);
        let publish_timeout = || {
            if !published.replace(true) {
                self.record_timeout(clock());
            }
        };
        // The runner publishes before killing/reaping. Keep the flight guard
        // until cleanup returns, even if waitpid takes longer than the deadline.
        let result = command(&publish_timeout);
        if matches!(&result, Err(error) if error.kind() == io::ErrorKind::TimedOut) {
            publish_timeout();
        } else if result.is_ok() {
            // A completed RPC (including service-not-found) proves launchd answered.
            *self.failure.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        }
        result
    }

    fn record_timeout(&self, (now, unix_ms): (Instant, u64)) {
        let mut failure = self.failure.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let (since, count, backoff) = failure.as_ref().map_or((unix_ms, 1, INITIAL_BACKOFF), |previous| {
            (previous.health.since_unix_ms, previous.health.consecutive_timeouts.saturating_add(1),
                previous.backoff.saturating_mul(2).min(MAX_BACKOFF))
        });
        *failure = Some(Failure {
            health: Health { reason: Reason::LaunchdUnresponsive, since_unix_ms: since,
                retry_after_unix_ms: unix_ms.saturating_add(u64::try_from(backoff.as_millis()).unwrap_or(u64::MAX)),
                consecutive_timeouts: count },
            retry_at: now + backoff,
            backoff,
        });
    }

    fn health(&self) -> Option<Health> {
        self.failure.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref().map(|failure| failure.health.clone())
    }
}

static GATE: LazyLock<Gate> = LazyLock::new(Gate::default);
pub(super) fn run(command: impl FnOnce(&dyn Fn()) -> io::Result<Output>) -> io::Result<Output> {
    GATE.run(command, || (Instant::now(), u64::try_from(SystemTime::now().duration_since(UNIX_EPOCH)
        .unwrap_or_default().as_millis()).unwrap_or(u64::MAX)))
}

pub fn snapshot() -> Option<Health> {
    GATE.health()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt as _;
    use std::sync::{Arc, Barrier};

    fn success() -> io::Result<Output> {
        Ok(Output { status: std::process::ExitStatus::from_raw(0), stdout: Vec::new(), stderr: Vec::new() })
    }

    #[test]
    fn concurrent_callers_skip_without_running_a_second_command() {
        let gate = Arc::new(Gate::default());
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let child = {
            let (gate, entered, release) = (gate.clone(), entered.clone(), release.clone());
            std::thread::spawn(move || gate.run(|_| {
                entered.wait();
                release.wait();
                success()
            }, || (Instant::now(), 0)))
        };
        entered.wait();
        for _ in 0..511 {
            let error = gate.run(|_| panic!("second command spawned"), || (Instant::now(), 0)).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        }
        assert_eq!(gate.health(), None); // health reads do not block on the flight
        release.wait();
        child.join().unwrap().unwrap();
    }

    #[test]
    fn timeout_reports_since_and_exponential_capped_backoff_then_recovers() {
        let gate = Gate::default();
        let start = Instant::now();
        let mut elapsed = Duration::ZERO;
        for (index, seconds) in [5, 10, 20, 40, 60, 60].into_iter().enumerate() {
            let now = start + elapsed;
            let unix_ms = 1000 + u64::try_from(elapsed.as_millis()).unwrap();
            let error = gate.run(|_| Err(io::Error::from(io::ErrorKind::TimedOut)), || (now, unix_ms)).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::TimedOut);
            let health = gate.health().unwrap();
            assert_eq!(health.reason, Reason::LaunchdUnresponsive);
            assert_eq!(health.since_unix_ms, 1000);
            assert_eq!(health.consecutive_timeouts, u32::try_from(index + 1).unwrap());
            assert_eq!(health.retry_after_unix_ms, unix_ms + seconds * 1000);
            assert_eq!(serde_json::to_value(&health).unwrap()["reason"], "launchd-unresponsive");
            let before = now + Duration::from_secs(seconds) - Duration::from_nanos(1);
            assert_eq!(gate.run(|_| panic!("backoff spawned a command"), || (before, unix_ms)).unwrap_err().kind(), io::ErrorKind::WouldBlock);
            elapsed += Duration::from_secs(seconds);
        }
        gate.run(|_| success(), || (start + elapsed, 0)).unwrap();
        assert_eq!(gate.health(), None);
        gate.run(|_| Err(io::Error::from(io::ErrorKind::TimedOut)), || (start + elapsed, 200_000)).unwrap_err();
        assert_eq!(gate.health().unwrap().retry_after_unix_ms, 205_000);
    }

    #[test]
    fn timeout_health_precedes_cleanup_without_releasing_single_flight() {
        let gate = Arc::new(Gate::default());
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let now = Instant::now();
        let child = {
            let (gate, entered, release) = (gate.clone(), entered.clone(), release.clone());
            std::thread::spawn(move || gate.run(|publish_timeout| {
                publish_timeout();
                entered.wait();
                release.wait();
                Err(io::Error::from(io::ErrorKind::TimedOut))
            }, || (now, 1000)))
        };
        entered.wait();
        let health = gate.health().unwrap();
        assert_eq!(health.reason, Reason::LaunchdUnresponsive);
        assert_eq!(health.since_unix_ms, 1000);
        assert_eq!(health.consecutive_timeouts, 1);
        let error = gate.run(|_| panic!("cleanup still owns the flight"),
            || (now + MAX_BACKOFF, 61_000)).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        release.wait();
        assert_eq!(child.join().unwrap().unwrap_err().kind(), io::ErrorKind::TimedOut);
        assert_eq!(gate.health().unwrap().consecutive_timeouts, 1);
    }

    #[test]
    fn unwinding_does_not_wedge_admission_or_health_locks() {
        use std::panic::{AssertUnwindSafe, catch_unwind};

        let gate = Gate::default();
        let now = Instant::now();
        assert!(catch_unwind(AssertUnwindSafe(|| {
            let _ = gate.run(|_| panic!("injected command unwind"), || (now, 1000));
        })).is_err());
        assert!(gate.flight.is_poisoned());
        gate.run(|_| success(), || (now, 1000)).unwrap();

        assert!(catch_unwind(AssertUnwindSafe(|| {
            let _failure = gate.failure.lock().unwrap();
            panic!("injected health unwind");
        })).is_err());
        assert!(gate.failure.is_poisoned());
        gate.run(|publish_timeout| {
            publish_timeout();
            Err(io::Error::from(io::ErrorKind::TimedOut))
        }, || (now, 1000)).unwrap_err();
        assert_eq!(gate.health().unwrap().reason, Reason::LaunchdUnresponsive);
        gate.run(|_| success(), || (now + INITIAL_BACKOFF, 6000)).unwrap();
        assert_eq!(gate.health(), None);
    }
}
