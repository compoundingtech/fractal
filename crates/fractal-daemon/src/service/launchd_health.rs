//! Process-local admission for launchd RPCs. Health reads never wait for the RPC.
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
    fn run(&self, command: impl FnOnce() -> io::Result<Output>, clock: impl Fn() -> (Instant, u64)) -> io::Result<Output> {
        // Never queue worker requests behind a stalled supervisor or start a second child.
        let _flight = match self.flight.try_lock() {
            Ok(flight) => flight,
            Err(TryLockError::WouldBlock) => return Err(io::Error::new(io::ErrorKind::WouldBlock, "launchctl probe already in flight")),
            Err(TryLockError::Poisoned(_)) => return Err(io::Error::other("launchctl admission lock poisoned")),
        };
        let (now, _) = clock();
        {
            let failure = self.failure.lock().map_err(|_| io::Error::other("launchctl health lock poisoned"))?;
            if let Some(failure) = failure.as_ref().filter(|failure| now < failure.retry_at) {
                return Err(io::Error::new(io::ErrorKind::WouldBlock,
                    format!("launchd-unresponsive since {}; retry after {}", failure.health.since_unix_ms, failure.health.retry_after_unix_ms)));
            }
        }
        let result = command();
        let (finished, unix_ms) = clock();
        let mut failure = self.failure.lock().map_err(|_| io::Error::other("launchctl health lock poisoned"))?;
        match &result {
            Err(error) if error.kind() == io::ErrorKind::TimedOut => {
                let (since, count, backoff) = failure.as_ref().map_or((unix_ms, 1, INITIAL_BACKOFF), |previous| {
                    (previous.health.since_unix_ms, previous.health.consecutive_timeouts.saturating_add(1),
                        previous.backoff.saturating_mul(2).min(MAX_BACKOFF))
                });
                *failure = Some(Failure {
                    health: Health { reason: Reason::LaunchdUnresponsive, since_unix_ms: since,
                        retry_after_unix_ms: unix_ms.saturating_add(u64::try_from(backoff.as_millis()).unwrap_or(u64::MAX)),
                        consecutive_timeouts: count },
                    retry_at: finished + backoff,
                    backoff,
                });
            }
            // A completed RPC (including service-not-found) proves launchd answered.
            Ok(_) => *failure = None,
            Err(_) => {}
        }
        result
    }

    fn health(&self) -> Option<Health> {
        self.failure.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref().map(|failure| failure.health.clone())
    }
}

static GATE: LazyLock<Gate> = LazyLock::new(Gate::default);
pub(super) fn run(command: impl FnOnce() -> io::Result<Output>) -> io::Result<Output> {
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
            std::thread::spawn(move || gate.run(|| {
                entered.wait();
                release.wait();
                success()
            }, || (Instant::now(), 0)))
        };
        entered.wait();
        for _ in 0..511 {
            let error = gate.run(|| panic!("second command spawned"), || (Instant::now(), 0)).unwrap_err();
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
            let error = gate.run(|| Err(io::Error::from(io::ErrorKind::TimedOut)), || (now, unix_ms)).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::TimedOut);
            let health = gate.health().unwrap();
            assert_eq!(health.reason, Reason::LaunchdUnresponsive);
            assert_eq!(health.since_unix_ms, 1000);
            assert_eq!(health.consecutive_timeouts, u32::try_from(index + 1).unwrap());
            assert_eq!(health.retry_after_unix_ms, unix_ms + seconds * 1000);
            assert_eq!(serde_json::to_value(&health).unwrap()["reason"], "launchd-unresponsive");
            let before = now + Duration::from_secs(seconds) - Duration::from_nanos(1);
            assert_eq!(gate.run(|| panic!("backoff spawned a command"), || (before, unix_ms)).unwrap_err().kind(), io::ErrorKind::WouldBlock);
            elapsed += Duration::from_secs(seconds);
        }
        gate.run(success, || (start + elapsed, 0)).unwrap();
        assert_eq!(gate.health(), None);
        gate.run(|| Err(io::Error::from(io::ErrorKind::TimedOut)), || (start + elapsed, 200_000)).unwrap_err();
        assert_eq!(gate.health().unwrap().retry_after_unix_ms, 205_000);
    }
}
