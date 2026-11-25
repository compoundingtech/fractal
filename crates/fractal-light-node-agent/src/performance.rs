//! Query-driven self accounting. No Store, task, timer, or child-process sampler.
//!
//! Compare `process_cpu_ns` and `accounting_uptime_ns` only across snapshots with
//! the same `process_instance_id`. Their delta ratio is CPU cores consumed (100
//! times that ratio is percent of one core, not percent of the host). The CPU
//! counter includes every thread of this process, including uninstrumented work;
//! it excludes children and the home process. Accounting uptime begins at this
//! Meter's construction, not at OS process creation. Unix start time is a
//! diagnostic label only; wall-clock changes never enter the delta denominator.
use serde_json::{Value, json};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// Fixed labels keep memory and snapshot size independent of traffic and peers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(usize)]
pub enum Work {
    SessionWake,
    SessionConnect,
    SessionReceive,
    SessionSend,
    NativeObserve,
    NativeStop,
    NativeRecover,
    SessionDisconnect,
    SessionReconnectWait,
    LocalRequest,
    NativeCancel,
}

impl Work {
    pub const ALL: [Self; 11] = [
        Self::SessionWake,
        Self::SessionConnect,
        Self::SessionReceive,
        Self::SessionSend,
        Self::NativeObserve,
        Self::NativeStop,
        Self::NativeRecover,
        Self::SessionDisconnect,
        Self::SessionReconnectWait,
        Self::LocalRequest,
        Self::NativeCancel,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::SessionWake => "session_wake",
            Self::SessionConnect => "session_connect",
            Self::SessionReceive => "session_receive",
            Self::SessionSend => "session_send",
            Self::NativeObserve => "native_observe",
            Self::NativeStop => "native_stop",
            Self::NativeRecover => "native_recover",
            Self::SessionDisconnect => "session_disconnect",
            Self::SessionReconnectWait => "session_reconnect_wait",
            Self::LocalRequest => "local_request",
            Self::NativeCancel => "native_cancel",
        }
    }
}

/// CPU sampling failures are never represented by a fabricated zero counter.
#[derive(Debug)]
pub enum Error {
    UnsupportedPlatform,
    ClockRead { clock: &'static str, source: std::io::Error },
    InvalidClockValue { clock: &'static str },
    CounterOverflow { counter: &'static str },
    ClockRegressed { clock: &'static str },
    BirthBeforeUnixEpoch(std::time::SystemTimeError),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedPlatform => f.write_str("own-process CPU accounting is unsupported on this platform"),
            Self::ClockRead { clock, source } => write!(f, "reading {clock}: {source}"),
            Self::InvalidClockValue { clock } => write!(f, "invalid value from {clock}"),
            Self::CounterOverflow { counter } => write!(f, "{counter} exceeds integer nanosecond range"),
            Self::ClockRegressed { clock } => write!(f, "{clock} regressed"),
            Self::BirthBeforeUnixEpoch(source) => write!(f, "accounting birth predates Unix epoch: {source}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::ClockRead { source, .. } => Some(source),
            Self::BirthBeforeUnixEpoch(source) => Some(source),
            _ => None,
        }
    }
}

struct Birth {
    instance_id: uuid::Uuid,
    unix_ns: u64,
    instant: Instant,
}


#[derive(Default)]
struct Counts {
    count: AtomicU64,
    cpu_ns: AtomicU64,
    cpu_samples: AtomicU64,
    cpu_errors: AtomicU64,
}

/// Counters and accounting identity are lifetime totals for this meter. Construct
/// one at runtime startup and share it via Arc with consumers. A fresh Meter
/// deliberately starts a new comparison identity, even in the same process.
/// Snapshots read independent atomic totals, not a transactional event history.
pub struct Meter {
    birth: Birth,
    cpu_baseline_ns: u64,
    cpu_baseline_uptime_before_ns: u64,
    cpu_baseline_uptime_after_ns: u64,
    work: [Counts; Work::ALL.len()],
}

impl Meter {
    pub fn new() -> Result<Self, Error> {
        let instant = Instant::now();
        let unix_ns = duration_ns(
            SystemTime::now().duration_since(UNIX_EPOCH).map_err(Error::BirthBeforeUnixEpoch)?,
            "accounting_started_unix_ns",
        )?;
        let before = duration_ns(instant.elapsed(), "accounting_uptime_ns")?;
        // Fail startup explicitly if the physical counter is unavailable.
        let cpu_baseline_ns = process_cpu_ns()?;
        let after = duration_ns(instant.elapsed(), "accounting_uptime_ns")?;
        Ok(Self {
            birth: Birth { instance_id: uuid::Uuid::new_v4(), unix_ns, instant },
            cpu_baseline_ns,
            cpu_baseline_uptime_before_ns: before,
            cpu_baseline_uptime_after_ns: after,
            work: std::array::from_fn(|_| Counts::default()),
        })
    }

    /// Record an actual event, not an anticipated wake or a polling iteration.
    /// Saturates at u64::MAX rather than wrapping into a smaller lifetime count.
    pub fn record(&self, work: Work) {
        add(&self.work[work as usize].count, 1);
    }

    /// Execute synchronous work exactly once and sample its current-thread CPU.
    /// The result always survives sampling failure. Never pass an async future:
    /// this measures the closure invocation, not future polls or awaited work.
    /// Nested scopes overlap, so per-work CPU totals must not be summed into a
    /// process total. Failed/unmeasured samples have explicit counts, not zero
    /// CPU observations. Panicking closures are not completed CPU samples.
    pub fn measure_sync<T>(&self, work: Work, operation: impl FnOnce() -> T) -> (T, Result<(), Error>) {
        self.record(work);
        let started = thread_cpu_ns();
        let result = operation();
        let ended = thread_cpu_ns();
        let sample = started.and_then(|start| {
            ended.and_then(|end| end.checked_sub(start).ok_or(Error::ClockRegressed {
                clock: "CLOCK_THREAD_CPUTIME_ID",
            }))
        });
        let counts = &self.work[work as usize];
        let status = match sample {
            Ok(cpu) => {
                add(&counts.cpu_ns, cpu);
                add(&counts.cpu_samples, 1);
                Ok(())
            }
            Err(error) => {
                add(&counts.cpu_errors, 1);
                Err(error)
            }
        };
        (result, status)
    }

    pub fn snapshot(&self) -> Result<Value, Error> {
        // Bracket the physical read so consumers can see sample uncertainty.
        let before = duration_ns(self.birth.instant.elapsed(), "accounting_uptime_ns")?;
        let cpu = process_cpu_ns()?;
        let after = duration_ns(self.birth.instant.elapsed(), "accounting_uptime_ns")?;
        let work = Work::ALL.map(|kind| {
            let counts = &self.work[kind as usize];
            json!({
                "kind": kind.name(),
                "count": counts.count.load(Ordering::Relaxed),
                "thread_cpu_ns": counts.cpu_ns.load(Ordering::Relaxed),
                "thread_cpu_samples": counts.cpu_samples.load(Ordering::Relaxed),
                "thread_cpu_errors": counts.cpu_errors.load(Ordering::Relaxed),
            })
        });
        Ok(json!({
            "schema_version": 1,
            "process_instance_id": self.birth.instance_id.to_string(),
            "pid": std::process::id(),
            "accounting_started_unix_ns": self.birth.unix_ns,
            "process_cpu_baseline_ns": self.cpu_baseline_ns,
            "cpu_baseline_uptime_before_ns": self.cpu_baseline_uptime_before_ns,
            "cpu_baseline_uptime_after_ns": self.cpu_baseline_uptime_after_ns,
            "accounting_uptime_ns": after,
            "cpu_sample_uptime_before_ns": before,
            "cpu_sample_uptime_after_ns": after,
            "process_cpu_ns": cpu,
            "process_cpu_clock": "CLOCK_PROCESS_CPUTIME_ID",
            "process_cpu_scope": "all threads of this process; excludes children and home process",
            "uptime_scope": "monotonic time since this Meter construction, not OS process creation",
            "delta_cpu_cores": "delta(process_cpu_ns) / delta(accounting_uptime_ns), same process_instance_id only",
            "work": work,
            "work_scope": "observed instrumented categories only, not exhaustive work; per-meter lifetime saturating counters; concurrent rows are not transactional; nested synchronous CPU scopes overlap",
        }))
    }
}

fn add(counter: &AtomicU64, value: u64) {
    let _ = counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |old| {
        Some(old.saturating_add(value))
    });
}

fn duration_ns(duration: std::time::Duration, counter: &'static str) -> Result<u64, Error> {
    duration.as_nanos().try_into().map_err(|_| Error::CounterOverflow { counter })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn process_cpu_ns() -> Result<u64, Error> {
    clock_ns(libc::CLOCK_PROCESS_CPUTIME_ID, "CLOCK_PROCESS_CPUTIME_ID")
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn thread_cpu_ns() -> Result<u64, Error> {
    clock_ns(libc::CLOCK_THREAD_CPUTIME_ID, "CLOCK_THREAD_CPUTIME_ID")
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn process_cpu_ns() -> Result<u64, Error> {
    Err(Error::UnsupportedPlatform)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn thread_cpu_ns() -> Result<u64, Error> {
    Err(Error::UnsupportedPlatform)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[allow(unsafe_code)] // Narrow OS CPU-clock boundary; see the pointer invariant below.
fn clock_ns(clock: libc::clockid_t, name: &'static str) -> Result<u64, Error> {
    let mut time = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: time is a valid writable timespec. Callers select only the current
    // process or current thread clocks; no PID or foreign process is accepted.
    if unsafe { libc::clock_gettime(clock, &mut time) } != 0 {
        return Err(Error::ClockRead { clock: name, source: std::io::Error::last_os_error() });
    }
    timespec_ns(time.tv_sec, time.tv_nsec, name)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn timespec_ns(seconds: i64, nanos: i64, clock: &'static str) -> Result<u64, Error> {
    if seconds < 0 || !(0..1_000_000_000).contains(&nanos) {
        return Err(Error::InvalidClockValue { clock });
    }
    (seconds as u64).checked_mul(1_000_000_000)
        .and_then(|seconds| seconds.checked_add(nanos as u64))
        .ok_or(Error::CounterOverflow { counter: clock })
}

#[cfg(test)]
#[path = "performance_tests.rs"]
mod tests;
