//! Process-local systemd state with loaded-only property queries.
//!
//! Absence is evidence only in a complete, admitted snapshot. Invalidating after a
//! control operation does not bypass the two-second observation admission bound:
//! callers get an error until a fresh observation can be admitted. Commands and
//! refreshes share one flight; failures back off from two to sixty seconds.
use anyhow::{Context, Result, anyhow, ensure};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Condvar, LazyLock, Mutex};
use std::process::{Command, Output};
use std::time::{Duration, Instant, SystemTime};

const REFRESH: Duration = Duration::from_secs(2);
const DEADLINE: Duration = Duration::from_secs(3);
const FILTERS: &[&str] = &[
    "fractal*.service", "smalltalk*.service", "st3*.service",
    "fractal*.scope", "smalltalk*.scope", "st3*.scope", "st2-*.scope",
    "fractal*.slice", "smalltalk*.slice", "st3*.slice", "app-lanes*.slice",
];
const PROPERTIES: &str = "--property=Id,LoadState,ActiveState,SubState,UnitFileState,MainPID,ControlGroup";

#[derive(Debug, Clone)]
pub struct Unit {
    pub load: String,
    pub active: String,
    pub sub: String,
    /// Empty when unavailable (for example an unloaded unit or transient scope).
    pub unit_file_state: String,
    /// Zero when unavailable, including unloaded rows preserved from enumeration.
    pub main_pid: u32,
    /// Absolute cgroup filesystem path, not systemd's manager-relative path.
    pub control_group: Option<PathBuf>,
}

#[derive(Debug)]
pub struct Snapshot {
    pub units: BTreeMap<String, Unit>,
    /// Generation witnesses pinned when loaded service MainPIDs were observed.
    /// Missing witnesses preserve status but cannot authorize a worker.
    pub service_identities: BTreeMap<String, crate::PeerProcessIdentity>,
    /// Start of enumeration, conservative for creation/retirement evidence.
    pub captured_at: Instant,
}

impl Snapshot {
    /// Validate the admission-time witness, never recapture a cached numeric PID.
    pub fn worker_identity(&self, name: &str) -> Result<crate::PeerProcessIdentity> {
        let unit = self.units.get(name).context("systemd worker service is absent")?;
        ensure!(name.ends_with(".service") && unit.load != "not-found" && unit.main_pid != 0,
            "systemd worker service has no loaded MainPID");
        let identity = self.service_identities.get(name)
            .context("systemd worker service has no admission-time generation witness")?;
        ensure!(identity.pid() == unit.main_pid, "systemd worker witness differs from observed MainPID");
        identity.validate()?;
        Ok(identity.clone())
    }
}

#[derive(Debug, Clone)]
pub struct SystemdUnresponsive {
    pub since: SystemTime,
    pub message: String,
}

impl std::fmt::Display for SystemdUnresponsive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "systemd-unresponsive since {:?}: {}", self.since, self.message)
    }
}
impl std::error::Error for SystemdUnresponsive {}

#[derive(Debug, Clone, Default)]
pub struct Health {
    pub unresponsive: Option<SystemdUnresponsive>,
}

trait Clock: Send + Sync {
    fn now(&self) -> Instant;
    fn wall(&self) -> SystemTime;
}
struct RealClock;
impl Clock for RealClock {
    fn now(&self) -> Instant { Instant::now() }
    fn wall(&self) -> SystemTime { SystemTime::now() }
}
trait Runner: Send + Sync {
    fn run(&self, command: &mut Command, failed: &dyn Fn(&str)) -> io::Result<Output>;
    // Fake command runners must not inspect host processes named by fixtures.
    fn capture_service_identity(&self, _pid: u32) -> Option<crate::PeerProcessIdentity> { None }
}
struct RealRunner;
impl Runner for RealRunner {
    fn run(&self, command: &mut Command, failed: &dyn Fn(&str)) -> io::Result<Output> {
        bounded_output(command, failed)
    }
    fn capture_service_identity(&self, pid: u32) -> Option<crate::PeerProcessIdentity> {
        crate::PeerProcessIdentity::capture_pid(pid).ok()
    }
}

#[derive(Default)]
struct State {
    flight: bool,
    generation: u64,
    snapshot: Option<Arc<Snapshot>>,
    stale: bool,
    next_refresh: Option<Instant>,
    retry_at: Option<Instant>,
    failures: u32,
    health: Health,
}
struct Monitor<C, R> {
    clock: C,
    runner: R,
    state: Mutex<State>,
    changed: Condvar,
}

// The unlocked runner may unwind. Its owner must release admission even then,
// and a poisoned state lock must not permanently disable future observations.
struct Flight<'a> {
    state: &'a Mutex<State>,
    changed: &'a Condvar,
    active: bool,
}
impl Flight<'_> {
    fn finish(&mut self, state: &mut State) {
        state.flight = false;
        self.active = false;
        self.changed.notify_all();
    }
}
impl Drop for Flight<'_> {
    fn drop(&mut self) {
        if self.active {
            let mut state = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            state.flight = false;
            state.stale = true;
            self.changed.notify_all();
        }
    }
}
impl<C: Clock, R: Runner> Monitor<C, R> {
    fn new(clock: C, runner: R) -> Self {
        Self { clock, runner, state: Mutex::new(State::default()), changed: Condvar::new() }
    }

    fn lock_state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn invalidate(&self) {
        let mut state = self.lock_state();
        state.stale = true;
        state.generation = state.generation.wrapping_add(1);
    }

    // Publish before potentially blocking kill/reap. Waiting readers wake and fail
    // closed rather than waiting on cleanup or using a stale absence result.
    fn publish_failure(&self, message: &str) {
        let mut state = self.lock_state();
        if state.retry_at.is_none() {
            state.failures = state.failures.saturating_add(1);
            let seconds = (2_u64 << state.failures.saturating_sub(1).min(5)).min(60);
            state.retry_at = Some(self.clock.now() + Duration::from_secs(seconds));
        }
        if state.health.unresponsive.is_none() {
            state.health.unresponsive = Some(SystemdUnresponsive {
                since: self.clock.wall(), message: message.to_owned(),
            });
        }
        state.stale = true;
        self.changed.notify_all();
    }

    fn admission_error(&self, state: &State) -> Option<anyhow::Error> {
        if state.retry_at.is_some_and(|at| self.clock.now() < at) ||
            (state.flight && state.health.unresponsive.is_some()) {
            return Some(state.health.unresponsive.clone().map_or_else(
                || anyhow!("systemd refresh is backing off"), anyhow::Error::new));
        }
        None
    }

    fn snapshot(&self) -> Result<Arc<Snapshot>> {
        let mut state = self.lock_state();
        loop {
            if let Some(error) = self.admission_error(&state) { return Err(error); }
            if state.next_refresh.is_some_and(|at| self.clock.now() < at) {
                if !state.stale && let Some(snapshot) = &state.snapshot {
                    return Ok(Arc::clone(snapshot));
                }
                return Err(anyhow!("systemd snapshot invalidated; fresh observation not yet admitted"));
            }
            if !state.flight { break; }
            state = self.changed.wait(state).unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        state.flight = true;
        state.retry_at = None;
        let generation = state.generation;
        drop(state);
        let mut flight = Flight { state: &self.state, changed: &self.changed, active: true };
        let captured_at = self.clock.now();
        let result = self.refresh(captured_at).map(Arc::new);
        if let Err(error) = &result { self.publish_failure(&error.to_string()); }
        let mut state = self.lock_state();
        if let Ok(snapshot) = &result {
            state.snapshot = Some(Arc::clone(snapshot));
            state.next_refresh = Some(self.clock.now() + REFRESH);
            state.stale = state.generation != generation;
            state.retry_at = None;
            state.failures = 0;
            state.health = Health::default();
        }
        flight.finish(&mut state);
        if state.stale && result.is_ok() {
            return Err(anyhow!("systemd snapshot invalidated during refresh"));
        }
        result
    }

    fn snapshot_after_admission(&self) -> Result<Arc<Snapshot>> {
        self.snapshot_after_admission_with(|state, delay| {
            self.changed.wait_timeout(state, delay)
                .unwrap_or_else(std::sync::PoisonError::into_inner).0
        })
    }

    fn snapshot_after_admission_with(
        &self,
        mut wait: impl for<'a> FnMut(
            std::sync::MutexGuard<'a, State>, Duration,
        ) -> std::sync::MutexGuard<'a, State>,
    ) -> Result<Arc<Snapshot>> {
        loop {
            match self.snapshot() {
                Ok(snapshot) => return Ok(snapshot),
                Err(error) => {
                    let state = self.lock_state();
                    // Only a stale successful observation's admission boundary
                    // is waitable here. Transport/backoff errors stay prompt.
                    if self.admission_error(&state).is_some() { return Err(error); }
                    if state.stale && let Some(at) = state.next_refresh {
                        let delay = at.saturating_duration_since(self.clock.now());
                        if !delay.is_zero() {
                            // Condvar releases the state lock during the wait;
                            // no flight or subprocess is owned by this caller.
                            drop(wait(state, delay));
                        } else {
                            drop(state);
                        }
                        continue;
                    }
                    return Err(error);
                },
            }
        }
    }

    fn run(&self, command: &mut Command) -> io::Result<Output> {
        let mut state = self.lock_state();
        loop {
            if let Some(error) = self.admission_error(&state) {
                return Err(io::Error::other(error));
            }
            if !state.flight { break; }
            state = self.changed.wait(state).unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        state.flight = true;
        state.retry_at = None;
        drop(state);
        let mut flight = Flight { state: &self.state, changed: &self.changed, active: true };
        let result = self.runner.run(command, &|message| self.publish_failure(message));
        // A completed nonzero exit is a manager response, not unresponsiveness.
        // Callers interpret command status; only transport/deadline failures gate.
        if let Err(error) = &result { self.publish_failure(&error.to_string()); }
        let mut state = self.lock_state();
        if result.is_ok() {
            state.retry_at = None;
            state.failures = 0;
            state.health = Health::default();
        }
        flight.finish(&mut state);
        result
    }

    fn refresh(&self, captured_at: Instant) -> Result<Snapshot> {
        let mut list = crate::isolate::systemctl();
        list.args(["--user", "list-units", "--all", "--full", "--no-pager", "--output=json"])
            .args(FILTERS).env("LC_ALL", "C");
        let output = self.runner.run(&mut list, &|message| self.publish_failure(message))?;
        ensure!(output.status.success(), "list loaded systemd units: {}: {}", output.status,
            String::from_utf8_lossy(&output.stderr));
        let mut units = listed_units(&output.stdout)?;
        // Generation-terminal proof is stronger than a manager row pending GC.
        // Never re-query or publish those historical scopes, loaded or not-found.
        units.retain(|name, _| !crate::isolate::has_ended(name));
        let names: Vec<String> = units.iter()
            .filter(|(_, unit)| unit.load != "not-found")
            .map(|(name, _)| name.clone()).collect();
        let mut service_identities = BTreeMap::new();
        // Bound argument sizes while never asking show to resolve an unknown name.
        for batch in names.chunks(128) {
            let mut show = crate::isolate::systemctl();
            show.args(["--user", "show", "--no-pager", PROPERTIES]).args(batch)
                .env("LC_ALL", "C");
            let output = self.runner.run(&mut show, &|message| self.publish_failure(message))?;
            ensure!(output.status.success(), "read loaded systemd properties: {}: {}",
                output.status, String::from_utf8_lossy(&output.stderr));
            let properties = parse_properties(&output.stdout, batch)?;
            for (name, unit) in &properties {
                if name.ends_with(".service") && unit.main_pid != 0
                    && let Some(identity) = self.runner.capture_service_identity(unit.main_pid)
                {
                    service_identities.insert(name.clone(), identity);
                }
            }
            units.extend(properties);
        }
        Ok(Snapshot { units, service_identities, captured_at })
    }
}

fn monitor() -> &'static Monitor<RealClock, RealRunner> {
    static MONITOR: LazyLock<Monitor<RealClock, RealRunner>> =
        LazyLock::new(|| Monitor::new(RealClock, RealRunner));
    &MONITOR
}
pub fn snapshot() -> Result<Arc<Snapshot>> { monitor().snapshot() }
/// For explicit control-failure verification, wait for stale-success admission
/// without bypassing refresh bounds. Transport/backoff errors return promptly.
pub fn snapshot_after_admission() -> Result<Arc<Snapshot>> { monitor().snapshot_after_admission() }
/// Mark observation stale after actual creation/control changes, never per lookup.
pub fn invalidate() { monitor().invalidate(); }
pub fn health() -> Health { monitor().lock_state().health.clone() }
/// Execute a control command under the same flight/deadline/backoff as observation.
/// Outputs are never cached. Callers own exit-status handling and invalidation.
pub fn output(command: &mut Command) -> io::Result<Output> { monitor().run(command) }
#[derive(Deserialize)]
struct ListedUnit { unit: String, load: String, active: String, sub: String }
fn listed_units(bytes: &[u8]) -> Result<BTreeMap<String, Unit>> {
    let rows: Vec<ListedUnit> = serde_json::from_slice(bytes).context("parse systemd list-units JSON")?;
    let mut units = BTreeMap::new();
    for row in rows {
        ensure!(!row.unit.is_empty() && !row.unit.contains(['/', '\n', '\0']) &&
            !row.unit.starts_with('-'), "invalid listed systemd unit name");
        ensure!(valid_load(&row.load) && !row.active.is_empty() && !row.sub.is_empty(),
            "incomplete or unknown listed systemd state for {}", row.unit);
        // A not-found unit can still be active after its file is removed. Keep
        // list-units' state for conflict detection, but never show that name.
        // Loaded rows must receive complete properties before publication.
        ensure!(units.insert(row.unit, Unit {
            load: row.load, active: row.active, sub: row.sub,
            unit_file_state: String::new(), main_pid: 0, control_group: None,
        }).is_none(), "duplicate systemd enumeration entry");
    }
    Ok(units)
}

fn valid_load(load: &str) -> bool {
    matches!(load, "stub" | "loaded" | "not-found" | "bad-setting" | "error" | "merged" | "masked")
}

fn parse_properties(bytes: &[u8], expected: &[String]) -> Result<BTreeMap<String, Unit>> {
    let text = std::str::from_utf8(bytes).context("systemd properties are not UTF-8")?;
    let expected: BTreeSet<&str> = expected.iter().map(String::as_str).collect();
    let mut units = BTreeMap::new();
    for block in text.split("\n\n").filter(|block| !block.trim().is_empty()) {
        let mut fields = BTreeMap::new();
        for line in block.lines() {
            let (key, value) = line.split_once('=').context("malformed systemd property")?;
            ensure!(fields.insert(key, value).is_none(), "duplicate systemd property {key}");
        }
        let required = |key| fields.get(key).copied().with_context(|| format!("missing systemd property {key}"));
        let id = required("Id")?;
        ensure!(expected.contains(id), "unrequested systemd unit {id}");
        let load = required("LoadState")?;
        let active = required("ActiveState")?;
        let sub = required("SubState")?;
        // A unit enumerated as loaded can be collected before show runs. Its
        // complete current state is an observation, not a transport failure.
        ensure!(valid_load(load) && !active.is_empty() && !sub.is_empty(),
            "unit has incomplete or unknown properties: {id}");
        let main_pid = if id.ends_with(".service") {
            required("MainPID")?.parse().context("invalid systemd MainPID")?
        } else {
            fields.get("MainPID").copied().unwrap_or("0").parse().context("invalid systemd MainPID")?
        };
        let group = required("ControlGroup")?;
        let control_group = if group.is_empty() { None } else {
            let path = Path::new(group);
            ensure!(path.is_absolute() && path.components().all(|component|
                matches!(component, Component::RootDir | Component::Normal(_))), "invalid systemd cgroup path");
            Some(Path::new("/sys/fs/cgroup").join(path.strip_prefix("/")?))
        };
        ensure!(units.insert(id.to_owned(), Unit { load: load.to_owned(), active: active.to_owned(),
            sub: sub.to_owned(), unit_file_state: fields.get("UnitFileState").copied().unwrap_or("").to_owned(),
            main_pid, control_group }).is_none(), "duplicate systemd unit {id}");
    }
    ensure!(units.len() == expected.len(), "incomplete systemd properties batch");
    Ok(units)
}

trait Invocation {
    fn poll(&mut self) -> io::Result<Option<Output>>;
    fn kill(&mut self) -> io::Result<()>;
    fn reap(&mut self) -> io::Result<()>;
}

// Publish before cleanup; retain the flight until kill and reap have completed.
fn supervise(
    child: &mut impl Invocation,
    elapsed: impl Fn() -> Duration,
    pause: impl Fn(),
    failed: &dyn Fn(&str),
) -> io::Result<Output> {
    let result = (|| loop {
        if elapsed() >= DEADLINE {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "systemctl exceeded 3s invocation deadline"));
        }
        if let Some(output) = child.poll()? { return Ok(output); }
        pause();
    })();
    if let Err(error) = &result {
        failed(&error.to_string());
        let _ = child.kill();
        if let Err(cleanup) = child.reap() {
            return Err(io::Error::new(cleanup.kind(),
                format!("{error}; systemctl child could not be reaped: {cleanup}")));
        }
    }
    result
}

fn retry_interrupted<T>(mut operation: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    loop {
        match operation() {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {},
            result => return result,
        }
    }
}

trait ReapChild {
    fn kill(&mut self) -> io::Result<()>;
    fn wait(&mut self) -> io::Result<()>;
}
impl ReapChild for std::process::Child {
    fn kill(&mut self) -> io::Result<()> { std::process::Child::kill(self) }
    fn wait(&mut self) -> io::Result<()> { std::process::Child::wait(self).map(|_| ()) }
}

// Acquire immediately after spawn, before pipes/setup/callbacks can unwind.
// The owner remains inside the runner, so its cleanup precedes flight release.
struct ReapOnDrop<C: ReapChild> {
    child: C,
    reaped: bool,
}
impl<C: ReapChild> ReapOnDrop<C> {
    fn kill(&mut self) -> io::Result<()> {
        retry_interrupted(|| self.child.kill())
    }
    fn reap(&mut self) -> io::Result<()> {
        retry_interrupted(|| self.child.wait())?;
        self.reaped = true;
        Ok(())
    }
}
impl<C: ReapChild> Drop for ReapOnDrop<C> {
    fn drop(&mut self) {
        if !self.reaped {
            let _ = self.kill();
            let _ = self.reap();
        }
    }
}

#[cfg(unix)]
fn bounded_output(command: &mut Command, failed: &dyn Fn(&str)) -> io::Result<Output> {
    use std::os::fd::AsRawFd;
    use std::process::Stdio;
    let started = Instant::now();
    let mut owner = ReapOnDrop {
        child: command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?,
        reaped: false,
    };
    let stdout = owner.child.stdout.take().expect("piped stdout");
    let stderr = owner.child.stderr.take().expect("piped stderr");
    for fd in [stdout.as_raw_fd(), stderr.as_raw_fd()] {
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags == -1 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
            let error = io::Error::last_os_error();
            failed(&error.to_string());
            // The owner kills/reaps on this return and if publication unwinds.
            return Err(error);
        }
    }
    let mut invocation = ChildInvocation {
        owner, stdout, stderr, out: Vec::new(), err: Vec::new(),
        status: None, out_done: false, err_done: false,
    };
    supervise(&mut invocation, || started.elapsed(),
        || std::thread::sleep(Duration::from_millis(1)), failed)
}

#[cfg(unix)]
struct ChildInvocation {
    owner: ReapOnDrop<std::process::Child>,
    stdout: std::process::ChildStdout,
    stderr: std::process::ChildStderr,
    out: Vec<u8>,
    err: Vec<u8>,
    status: Option<std::process::ExitStatus>,
    out_done: bool,
    err_done: bool,
}

#[cfg(unix)]
impl Invocation for ChildInvocation {
    fn poll(&mut self) -> io::Result<Option<Output>> {
        // Bound work between deadline checks, including continuously writing children.
        fn drain(reader: &mut impl io::Read, bytes: &mut Vec<u8>) -> io::Result<bool> {
            let mut buffer = [0_u8; 65536];
            match reader.read(&mut buffer) {
                Ok(0) => Ok(true),
                Ok(count) => {
                    if bytes.len() + count > 16 * 1024 * 1024 {
                        return Err(io::Error::other("systemctl output exceeds 16 MiB"));
                    }
                    bytes.extend_from_slice(&buffer[..count]);
                    Ok(false)
                },
                Err(error) if matches!(error.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted) => Ok(false),
                Err(error) => Err(error),
            }
        }
        if !self.out_done { self.out_done = drain(&mut self.stdout, &mut self.out)?; }
        if !self.err_done { self.err_done = drain(&mut self.stderr, &mut self.err)?; }
        if self.status.is_none() {
            self.status = self.owner.child.try_wait()?;
            if self.status.is_some() { self.owner.reaped = true; }
        }
        if let Some(status) = self.status && self.out_done && self.err_done {
            return Ok(Some(Output { status, stdout: std::mem::take(&mut self.out),
                stderr: std::mem::take(&mut self.err) }));
        }
        Ok(None)
    }
    fn kill(&mut self) -> io::Result<()> { self.owner.kill() }
    fn reap(&mut self) -> io::Result<()> { self.owner.reap() }
}
#[cfg(not(unix))]
fn bounded_output(_: &mut Command, _: &dyn Fn(&str)) -> io::Result<Output> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "systemd requires Unix"))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::os::unix::process::ExitStatusExt;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct FakeClock {
        base: Instant,
        milliseconds: AtomicUsize,
    }
    impl FakeClock {
        fn new() -> Self { Self { base: Instant::now(), milliseconds: AtomicUsize::new(0) } }
        fn advance(&self, milliseconds: usize) { self.milliseconds.fetch_add(milliseconds, Ordering::SeqCst); }
    }
    impl Clock for FakeClock {
        fn now(&self) -> Instant {
            self.base + Duration::from_millis(self.milliseconds.load(Ordering::SeqCst) as u64)
        }
        fn wall(&self) -> SystemTime { SystemTime::UNIX_EPOCH + self.now().duration_since(self.base) }
    }
    fn success(text: &str) -> Output {
        Output { status: std::process::ExitStatus::from_raw(0), stdout: text.as_bytes().to_vec(), stderr: Vec::new() }
    }
    fn properties(name: &str) -> String {
        format!("Id={name}\nLoadState=loaded\nActiveState=active\nSubState=running\nMainPID=123\nControlGroup=/user.slice/{name}\n")
    }
    #[test]
    fn lookup_preserves_unit_file_state_without_inventing_enablement() {
        let expected = vec!["fractal.service".to_owned()];
        for state in ["linked", "linked-runtime", "enabled", "enabled-runtime", ""] {
            let text = format!("{}UnitFileState={state}\n", properties("fractal.service"));
            let units = parse_properties(text.as_bytes(), &expected).unwrap();
            assert_eq!(units["fractal.service"].unit_file_state, state);
        }
        let units = parse_properties(properties("fractal.service").as_bytes(), &expected).unwrap();
        assert!(units["fractal.service"].unit_file_state.is_empty());
    }
    struct FakeRunner {
        calls: Mutex<Vec<Vec<String>>>,
        outputs: Mutex<VecDeque<io::Result<Output>>>,
    }
    impl FakeRunner {
        fn new(outputs: Vec<io::Result<Output>>) -> Self {
            Self { calls: Mutex::new(Vec::new()), outputs: Mutex::new(outputs.into()) }
        }
        fn count(&self) -> usize { self.calls.lock().unwrap().len() }
    }
    impl Runner for FakeRunner {
        fn run(&self, command: &mut Command, _: &dyn Fn(&str)) -> io::Result<Output> {
            self.calls.lock().unwrap().push(command.get_args().map(|arg| arg.to_string_lossy().into_owned()).collect());
            self.outputs.lock().unwrap().pop_front().expect("unexpected command")
        }
    }
    const LIST: &str = r#"[{"unit":"fractal.service","load":"loaded","active":"active","sub":"running"}]"#;

    #[test]
    fn thousand_lookups_per_second_and_historical_absence_share_one_refresh() {
        let monitor = Monitor::new(FakeClock::new(), FakeRunner::new(vec![
            Ok(success(LIST)), Ok(success(&properties("fractal.service"))),
            Ok(success(LIST)), Ok(success(&properties("fractal.service"))),
        ]));
        for index in 0..2000 {
            let snapshot = monitor.snapshot().unwrap();
            for historical in 0..10 {
                assert!(!snapshot.units.contains_key(&format!("st3-old-{index}-{historical}.scope")));
            }
            monitor.clock.advance(1);
        }
        assert_eq!(monitor.runner.count(), 2);
        monitor.snapshot().unwrap();
        assert_eq!(monitor.runner.count(), 4);
        let calls = monitor.runner.calls.lock().unwrap();
        assert!(calls[0].iter().any(|arg| arg == "--output=json"));
        assert_eq!(calls[1].last().unwrap(), "fractal.service");
        assert!(!calls.iter().flatten().any(|arg| arg.starts_with("st3-old-")));
    }

    #[test]
    fn invalidation_never_bypasses_admission_or_returns_stale_absence() {
        let monitor = Monitor::new(FakeClock::new(), FakeRunner::new(vec![
            Ok(success("[]")), Ok(success("[]")),
        ]));
        monitor.snapshot().unwrap();
        for _ in 0..2000 {
            monitor.invalidate();
            assert!(monitor.snapshot().is_err());
            monitor.clock.advance(1);
        }
        assert_eq!(monitor.runner.count(), 1);
        monitor.snapshot().unwrap();
        assert_eq!(monitor.runner.count(), 2);
    }

    #[test]
    fn failures_back_off_exponentially_to_sixty_seconds_without_stale_success() {
        let outputs = (0..8).map(|_| Err(io::Error::new(io::ErrorKind::TimedOut, "wedged"))).collect();
        let monitor = Monitor::new(FakeClock::new(), FakeRunner::new(outputs));
        let mut calls = 0;
        for seconds in [2, 4, 8, 16, 32, 60, 60, 60] {
            assert!(monitor.snapshot().is_err());
            calls += 1;
            let since = monitor.state.lock().unwrap().health.unresponsive.as_ref().unwrap().since;
            assert_eq!(since, SystemTime::UNIX_EPOCH);
            for _ in 0..1000 { monitor.invalidate(); assert!(monitor.snapshot().is_err()); }
            monitor.clock.advance(seconds * 1000 - 1);
            assert!(monitor.snapshot().is_err());
            assert_eq!(monitor.runner.count(), calls);
            monitor.clock.advance(1);
        }
    }

    #[test]
    fn controls_are_never_cached_and_share_failure_backoff() {
        let monitor = Monitor::new(FakeClock::new(), FakeRunner::new(vec![
            Ok(success("")), Ok(success("")),
            Err(io::Error::new(io::ErrorKind::TimedOut, "wedged")),
        ]));
        let mut command = Command::new("unused-systemctl");
        monitor.run(&mut command).unwrap();
        monitor.run(&mut command).unwrap();
        assert_eq!(monitor.runner.count(), 2);
        assert!(monitor.run(&mut command).is_err());
        assert!(monitor.run(&mut command).is_err());
        assert!(monitor.snapshot().is_err());
        assert_eq!(monitor.runner.count(), 3);
    }

    #[test]
    fn concurrent_callers_have_only_one_flight_including_controls() {
        struct BlockingRunner {
            entered: std::sync::mpsc::Sender<()>,
            release: Mutex<std::sync::mpsc::Receiver<()>>,
            calls: AtomicUsize,
        }
        impl Runner for BlockingRunner {
            fn run(&self, _: &mut Command, _: &dyn Fn(&str)) -> io::Result<Output> {
                let call = self.calls.fetch_add(1, Ordering::SeqCst);
                if call == 0 {
                    self.entered.send(()).unwrap();
                    self.release.lock().unwrap().recv().unwrap();
                }
                Ok(success("[]"))
            }
        }
        let (entered, arrival) = std::sync::mpsc::channel();
        let (release, released) = std::sync::mpsc::channel();
        let monitor = Monitor::new(FakeClock::new(), BlockingRunner {
            entered, release: Mutex::new(released), calls: AtomicUsize::new(0),
        });
        std::thread::scope(|scope| {
            let first = scope.spawn(|| monitor.snapshot().unwrap());
            arrival.recv().unwrap();
            let ready = Arc::new(std::sync::Barrier::new(33));
            let others: Vec<_> = (0..32).map(|_| {
                let ready = Arc::clone(&ready);
                let monitor = &monitor;
                scope.spawn(move || { ready.wait(); monitor.snapshot().unwrap() })
            }).collect();
            ready.wait();
            assert_eq!(monitor.runner.calls.load(Ordering::SeqCst), 1);
            release.send(()).unwrap();
            let first = first.join().unwrap();
            for other in others { assert!(Arc::ptr_eq(&first, &other.join().unwrap())); }
        });
        assert_eq!(monitor.runner.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn timeout_publishes_then_kills_and_reaps_without_sleep() {
        struct HungInvocation { events: Arc<Mutex<Vec<&'static str>>> }
        impl Invocation for HungInvocation {
            fn poll(&mut self) -> io::Result<Option<Output>> { Ok(None) }
            fn kill(&mut self) -> io::Result<()> { self.events.lock().unwrap().push("kill"); Ok(()) }
            fn reap(&mut self) -> io::Result<()> { self.events.lock().unwrap().push("reap"); Ok(()) }
        }
        let events = Arc::new(Mutex::new(Vec::new()));
        let clock = std::cell::Cell::new(Duration::ZERO);
        let error = supervise(&mut HungInvocation { events: Arc::clone(&events) },
            || clock.get(), || clock.set(clock.get() + Duration::from_secs(1)),
            &|_| events.lock().unwrap().push("publish")).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert_eq!(*events.lock().unwrap(), ["publish", "kill", "reap"]);
    }

    #[test]
    fn incomplete_or_diagnostic_reads_never_become_absence() {
        let expected = vec!["fractal.service".to_owned()];
        for bytes in [b"".as_slice(), b"diagnostic only", b"[]", b"{}"] {
            assert!(parse_properties(bytes, &expected).is_err());
        }
        let complete = properties("fractal.service");
        for key in ["Id", "LoadState", "ActiveState", "SubState", "MainPID", "ControlGroup"] {
            let incomplete = complete.lines().filter(|line| !line.starts_with(&format!("{key}="))).collect::<Vec<_>>().join("\n");
            assert!(parse_properties(incomplete.as_bytes(), &expected).is_err(), "{key}");
        }
        for state in ["", "unknown"] {
            let text = complete.replace("LoadState=loaded", &format!("LoadState={state}"));
            assert!(parse_properties(text.as_bytes(), &expected).is_err());
        }
        for text in ["", "diagnostic only", "{}", "[{}]"] {
            assert!(listed_units(text.as_bytes()).is_err());
        }
        assert!(parse_properties(complete.as_bytes(), &["other.service".to_owned()]).is_err());
        assert!(parse_properties(complete.replace("/user.slice/", "/../").as_bytes(), &expected).is_err());
        let parsed = parse_properties(complete.as_bytes(), &expected).unwrap();
        assert_eq!(parsed["fractal.service"].control_group.as_deref(), Some(Path::new("/sys/fs/cgroup/user.slice/fractal.service")));
    }

    #[test]
    fn missing_batch_member_is_error_and_scope_pid_is_optional() {
        let text = properties("st3-work.scope").replace("MainPID=123\n", "");
        assert_eq!(parse_properties(text.as_bytes(), &["st3-work.scope".to_owned()]).unwrap()["st3-work.scope"].main_pid, 0);
        assert!(parse_properties(text.as_bytes(), &["st3-work.scope".to_owned(), "st3-gone.scope".to_owned()]).is_err());
    }

    #[test]
    fn failed_refresh_never_returns_cached_or_partial_absence() {
        let monitor = Monitor::new(FakeClock::new(), FakeRunner::new(vec![
            Ok(success(LIST)), Ok(success(&properties("fractal.service"))),
            Ok(success(LIST)), Ok(success("")),
        ]));
        assert!(monitor.snapshot().unwrap().units.contains_key("fractal.service"));
        monitor.clock.advance(2000);
        assert!(monitor.snapshot().is_err());
        for _ in 0..1000 { assert!(monitor.snapshot().is_err()); }
        assert_eq!(monitor.runner.count(), 4);
    }

    #[test]
    fn batch_queries_only_names_enumerated_loaded() {
        let rows: Vec<_> = (0..257).map(|index| serde_json::json!({
            "unit": format!("st3-work-{index:03}.scope"), "load": "loaded",
            "active": "active", "sub": "running",
        })).chain(std::iter::once(serde_json::json!({
            "unit": "st3-not-loaded.scope", "load": "not-found",
            "active": "inactive", "sub": "dead",
        }))).collect();
        let mut outputs = vec![Ok(success(&serde_json::to_string(&rows).unwrap()))];
        let names: Vec<_> = (0..257).map(|index| format!("st3-work-{index:03}.scope")).collect();
        for batch in names.chunks(128) {
            outputs.push(Ok(success(&batch.iter().map(|name| properties(name)).collect::<Vec<_>>().join("\n"))));
        }
        let monitor = Monitor::new(FakeClock::new(), FakeRunner::new(outputs));
        assert_eq!(monitor.snapshot().unwrap().units.len(), 258);
        let calls = monitor.runner.calls.lock().unwrap();
        assert_eq!(calls.len(), 4);
        for batch in &calls[1..] {
            assert!(batch.len() <= 132);
            for name in &batch[4..] { assert!(names.contains(name)); }
            assert!(!batch.iter().any(|name| name == "st3-not-loaded.scope"));
        }
    }

    #[test]
    fn health_is_published_before_cleanup_releases_the_flight() {
        struct FailureBeforeCleanup;
        impl Runner for FailureBeforeCleanup {
            fn run(&self, _: &mut Command, failed: &dyn Fn(&str)) -> io::Result<Output> {
                failed("wedged");
                Err(io::Error::new(io::ErrorKind::TimedOut, "wedged"))
            }
        }
        let monitor = Monitor::new(FakeClock::new(), FailureBeforeCleanup);
        // Model a refresh in cleanup: readers may not wait for that flight.
        monitor.state.lock().unwrap().flight = true;
        monitor.publish_failure("wedged");
        assert!(monitor.snapshot().unwrap_err().downcast_ref::<SystemdUnresponsive>().is_some());
        monitor.clock.advance(60000);
        assert!(monitor.snapshot().is_err());
        assert!(monitor.run(&mut Command::new("unused-systemctl")).is_err());
    }

    #[test]
    fn completed_nonzero_control_does_not_block_absence_verification() {
        let mut refused = success("");
        refused.status = std::process::ExitStatus::from_raw(1 << 8);
        let monitor = Monitor::new(FakeClock::new(), FakeRunner::new(vec![
            Err(io::Error::new(io::ErrorKind::TimedOut, "wedged")),
            Ok(refused), Ok(success("[]")),
        ]));
        assert!(monitor.run(&mut Command::new("unused-systemctl")).is_err());
        assert!(monitor.lock_state().health.unresponsive.is_some());
        monitor.clock.advance(2000);
        assert!(!monitor.run(&mut Command::new("unused-systemctl")).unwrap().status.success());
        assert!(monitor.lock_state().health.unresponsive.is_none());
        assert_eq!(monitor.lock_state().failures, 0);
        assert!(monitor.lock_state().retry_at.is_none());
        assert!(monitor.snapshot().unwrap().units.is_empty());
        assert_eq!(monitor.runner.count(), 3);
    }

    #[test]
    fn runner_unwind_and_poisoned_state_do_not_wedge_admission() {
        struct PanicOnceRunner(AtomicUsize);
        impl Runner for PanicOnceRunner {
            fn run(&self, _: &mut Command, _: &dyn Fn(&str)) -> io::Result<Output> {
                assert_ne!(self.0.fetch_add(1, Ordering::SeqCst), 0, "injected runner panic");
                Ok(success("[]"))
            }
        }
        let monitor = Monitor::new(FakeClock::new(), PanicOnceRunner(AtomicUsize::new(0)));
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| monitor.snapshot())).is_err());
        assert!(!monitor.lock_state().flight);
        assert!(monitor.snapshot().unwrap().units.is_empty());
        let control = Monitor::new(FakeClock::new(), PanicOnceRunner(AtomicUsize::new(0)));
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(||
            control.run(&mut Command::new("unused-systemctl")))).is_err());
        assert!(!control.lock_state().flight);
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _state = control.state.lock().unwrap();
            panic!("injected state poison");
        })).is_err());
        control.invalidate();
        assert!(control.snapshot().unwrap().units.is_empty());
        control.run(&mut Command::new("unused-systemctl")).unwrap();
    }

    #[test]
    fn cleanup_owner_reaps_on_unwind_and_retries_interrupted_wait() {
        struct InterruptedChild {
            events: Arc<Mutex<Vec<&'static str>>>,
            waits: usize,
        }
        impl ReapChild for InterruptedChild {
            fn kill(&mut self) -> io::Result<()> {
                self.events.lock().unwrap().push("kill");
                Ok(())
            }
            fn wait(&mut self) -> io::Result<()> {
                self.events.lock().unwrap().push("wait");
                self.waits += 1;
                if self.waits < 3 {
                    return Err(io::Error::new(io::ErrorKind::Interrupted, "injected interruption"));
                }
                Ok(())
            }
        }
        let events = Arc::new(Mutex::new(Vec::new()));
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _owner = ReapOnDrop {
                child: InterruptedChild { events: Arc::clone(&events), waits: 0 },
                reaped: false,
            };
            panic!("injected setup/callback unwind");
        }));
        assert!(result.is_err());
        assert_eq!(*events.lock().unwrap(), ["kill", "wait", "wait", "wait"]);
    }

    #[test]
    fn terminal_scopes_still_enumerated_loaded_never_reach_properties() {
        const TERMINAL: &str = "st3-monitor-known-terminal-generation.scope";
        crate::isolate::mark_ended(TERMINAL);
        let rows = serde_json::json!([
            { "unit": TERMINAL, "load": "loaded", "active": "inactive", "sub": "dead" },
            { "unit": "fractal.service", "load": "loaded", "active": "active", "sub": "running" },
        ]);
        let listed = serde_json::to_string(&rows).unwrap();
        let monitor = Monitor::new(FakeClock::new(), FakeRunner::new(vec![
            Ok(success(&listed)), Ok(success(&properties("fractal.service"))),
            Ok(success(&listed)), Ok(success(&properties("fractal.service"))),
        ]));
        for _ in 0..2 {
            let snapshot = monitor.snapshot().unwrap();
            assert!(!snapshot.units.contains_key(TERMINAL));
            assert!(snapshot.units.contains_key("fractal.service"));
            monitor.clock.advance(2000);
        }
        let calls = monitor.runner.calls.lock().unwrap();
        assert_eq!(calls.len(), 4);
        assert!(!calls.iter().flatten().any(|argument| argument == TERMINAL));
    }

    #[test]
    fn unloaded_active_service_is_preserved_without_property_query() {
        let rows = serde_json::json!([
            { "unit": "fractal-unloaded.service", "load": "not-found", "active": "active", "sub": "running" },
            { "unit": "fractal-stopped.service", "load": "not-found", "active": "inactive", "sub": "dead" },
            { "unit": "st3-loaded.scope", "load": "loaded", "active": "active", "sub": "running" },
        ]);
        let monitor = Monitor::new(FakeClock::new(), FakeRunner::new(vec![
            Ok(success(&serde_json::to_string(&rows).unwrap())),
            Ok(success(&properties("st3-loaded.scope"))),
        ]));
        let snapshot = monitor.snapshot().unwrap();
        let unloaded = &snapshot.units["fractal-unloaded.service"];
        assert_eq!(unloaded.load, "not-found");
        assert_eq!(unloaded.active, "active");
        assert_eq!(unloaded.sub, "running");
        assert_eq!(unloaded.main_pid, 0);
        assert!(unloaded.control_group.is_none());
        assert_eq!(snapshot.units["fractal-stopped.service"].active, "inactive");
        assert_eq!(snapshot.units["st3-loaded.scope"].main_pid, 123);
        let calls = monitor.runner.calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(&calls[1][4..], ["st3-loaded.scope"]);
    }

    #[test]
    fn duplicate_enumeration_across_loaded_and_unloaded_rows_is_rejected() {
        let rows = serde_json::json!([
            { "unit": "fractal.service", "load": "loaded", "active": "active", "sub": "running" },
            { "unit": "fractal.service", "load": "not-found", "active": "active", "sub": "running" },
        ]);
        let monitor = Monitor::new(FakeClock::new(), FakeRunner::new(vec![
            Ok(success(&serde_json::to_string(&rows).unwrap())),
        ]));
        assert!(monitor.snapshot().is_err());
        assert_eq!(monitor.runner.count(), 1);
    }

    #[test]
    fn collection_between_list_and_show_preserves_complete_current_state() {
        let gone = properties("st3-work.scope")
            .replace("LoadState=loaded", "LoadState=not-found")
            .replace("ActiveState=active", "ActiveState=inactive")
            .replace("SubState=running", "SubState=dead")
            .replace("MainPID=123", "MainPID=0")
            .replace("ControlGroup=/user.slice/st3-work.scope", "ControlGroup=");
        let rows = serde_json::json!([
            {"unit":"st3-work.scope","load":"loaded","active":"active","sub":"running"},
            {"unit":"fractal.service","load":"loaded","active":"active","sub":"running"},
        ]);
        let monitor = Monitor::new(FakeClock::new(), FakeRunner::new(vec![
            Ok(success(&rows.to_string())),
            Ok(success(&format!("{}\n{gone}", properties("fractal.service")))),
        ]));
        let snapshot = monitor.snapshot().unwrap();
        let unit = &snapshot.units["st3-work.scope"];
        assert_eq!(unit.load, "not-found");
        assert_eq!(unit.active, "inactive");
        assert!(unit.control_group.is_none());
        assert_eq!(snapshot.units["fractal.service"].load, "loaded");
        assert!(monitor.lock_state().health.unresponsive.is_none());
        assert_eq!(monitor.runner.count(), 2);
        assert!(monitor.snapshot().is_ok());
        assert_eq!(monitor.runner.count(), 2, "collection needs no command retry");
    }

    #[test]
    fn explicit_verification_waits_for_admission_without_sleep_or_extra_refresh() {
        let monitor = Monitor::new(FakeClock::new(), FakeRunner::new(vec![
            Ok(success("[]")), Ok(success("[]")),
        ]));
        monitor.snapshot().unwrap();
        monitor.invalidate();
        let waits = AtomicUsize::new(0);
        let snapshot = monitor.snapshot_after_admission_with(|mut state, delay| {
            waits.fetch_add(1, Ordering::SeqCst);
            assert_eq!(delay, REFRESH);
            assert_eq!(monitor.runner.count(), 1);
            monitor.clock.advance(delay.as_millis() as usize);
            // Further invalidation during the wait cannot change admission.
            state.generation = state.generation.wrapping_add(1);
            state.stale = true;
            state
        }).unwrap();
        assert!(snapshot.units.is_empty());
        assert_eq!(waits.load(Ordering::SeqCst), 1);
        assert_eq!(monitor.runner.count(), 2);
    }

    #[test]
    fn explicit_verification_never_waits_through_transport_backoff() {
        let monitor = Monitor::new(FakeClock::new(), FakeRunner::new(vec![
            Err(io::Error::new(io::ErrorKind::TimedOut, "wedged")),
        ]));
        assert!(monitor.snapshot_after_admission_with(|_, _| panic!("must not wait")).is_err());
        assert!(monitor.snapshot_after_admission_with(|_, _| panic!("must not wait")).is_err());
        assert_eq!(monitor.runner.count(), 1);
    }

    #[test]
    fn missing_service_capture_preserves_status_but_denies_authority() {
        let monitor = Monitor::new(FakeClock::new(), FakeRunner::new(vec![
            Ok(success(LIST)), Ok(success(&properties("fractal.service"))),
        ]));
        let snapshot = monitor.snapshot().unwrap();
        assert_eq!(snapshot.units["fractal.service"].main_pid, 123);
        assert!(snapshot.service_identities.is_empty());
        assert!(snapshot.worker_identity("fractal.service").is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn systemd_identity_waiting_child() {
        if std::env::var_os("FRACTAL_SYSTEMD_IDENTITY_WAITING_CHILD").is_some() {
            use std::io::Read as _;
            std::io::stdin().read_exact(&mut [0_u8]).unwrap();
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn exited_admission_witness_is_rejected_before_first_authority_lookup() {
        use std::io::Write as _;
        use std::process::Stdio;
        struct CapturingRunner { commands: FakeRunner, captures: AtomicUsize }
        impl Runner for CapturingRunner {
            fn run(&self, command: &mut Command, failed: &dyn Fn(&str)) -> io::Result<Output> {
                self.commands.run(command, failed)
            }
            fn capture_service_identity(&self, pid: u32) -> Option<crate::PeerProcessIdentity> {
                self.captures.fetch_add(1, Ordering::SeqCst);
                Some(crate::PeerProcessIdentity::capture_pid(pid).unwrap())
            }
        }
        let mut owner = ReapOnDrop {
            child: Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "systemd::tests::systemd_identity_waiting_child"])
                .env("FRACTAL_SYSTEMD_IDENTITY_WAITING_CHILD", "1")
                .stdin(Stdio::piped()).stdout(Stdio::null()).spawn().unwrap(),
            reaped: false,
        };
        let service_properties = properties("fractal.service")
            .replace("MainPID=123", &format!("MainPID={}", owner.child.id()));
        let monitor = Monitor::new(FakeClock::new(), CapturingRunner {
            commands: FakeRunner::new(vec![Ok(success(LIST)), Ok(success(&service_properties))]),
            captures: AtomicUsize::new(0),
        });
        let snapshot = monitor.snapshot().unwrap();
        assert!(snapshot.service_identities.contains_key("fractal.service"));
        assert_eq!(monitor.runner.captures.load(Ordering::SeqCst), 1);
        // End the admitted generation before the very first authority lookup.
        owner.child.stdin.take().unwrap().write_all(b"x").unwrap();
        owner.reap().unwrap();
        assert!(snapshot.worker_identity("fractal.service").is_err());
        assert!(monitor.snapshot().unwrap().worker_identity("fractal.service").is_err());
        assert_eq!(monitor.runner.captures.load(Ordering::SeqCst), 1);
    }
}
