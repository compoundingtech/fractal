//! Pure host-pressure brake. The host observation loop supplies fresh kernel PSI readings;
//! elapsed time alone never constitutes a healthy observation or admission grant.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Signal {
    MemoryFull,
    MemorySome,
    IoFull,
    IoSome,
    /// Diagnostic-only signal, measured in bytes rather than milli-percent.
    MemoryAvailable,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Window {
    Avg10,
    Avg60,
    Avg300,
}


#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Rule {
    pub signal: Signal,
    pub window: Window,
    /// Thousandths of one percent: 1000 = 1%, 100000 = 100%.
    pub threshold_millipercent: u32,
    pub resume_millipercent: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct Policy {
    pub rules: Vec<Rule>,
    pub sustained_ms: u64,
    pub resume_ms: u64,
    pub freshness_ms: u64,
    /// Absolute host MemAvailable floor; a shortage counts as memory pressure.
    pub memory_available_floor_bytes: u64,
    /// Empty selects the service, system.slice and interactive app.slice automatically.
    pub protected_cgroups: Vec<PathBuf>,
    /// Job-containing subtrees, excluded whole from observation. Basenames are
    /// canonical systemd slice names resolved beneath the discovered
    /// user-manager root; absolute paths are exact and mount-relative.
    pub job_parent_cgroups: Vec<PathBuf>,
    /// Additional excluded subtrees, resolved exactly like job parents.
    /// Explicit lists replace the defaults: include `app-agent-seats.slice` (or
    /// its absolute cgroup path) to isolate self-throttled seats. Validation
    /// warns, but permits existing configurations, when that entry is omitted.
    pub excluded_cgroups: Vec<PathBuf>,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            rules: vec![
                Rule { signal: Signal::MemoryFull, window: Window::Avg60,
                    threshold_millipercent: 1000, resume_millipercent: 500 },
                Rule { signal: Signal::IoSome, window: Window::Avg60,
                    threshold_millipercent: 25000, resume_millipercent: 10000 },
            ],
            sustained_ms: 60000,
            resume_ms: 120000,
            freshness_ms: 10000,
            memory_available_floor_bytes: 2 << 30,
            protected_cgroups: Vec::new(),
            job_parent_cgroups: vec!["gate.slice".into(), "app-lanes.slice".into(),
                "app-capped.slice".into(), "app-capped-heavy.slice".into(),
                "app-batch.slice".into(), "app-batch-heavy.slice".into()],
            excluded_cgroups: vec!["app-lanes.slice".into(), "gate.slice".into(),
                "app-agent-seats.slice".into()],
        }
    }
}

impl Policy {
    /// Leave room for scheduling jitter without weakening the freshness contract.
    pub fn sample_interval(&self) -> std::time::Duration {
        std::time::Duration::from_millis((self.freshness_ms / 2).max(1))
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(!self.rules.is_empty() && self.rules.len() <= 12,
            "PSI brake requires one to twelve signal/window rules");
        ensure!(self.sustained_ms != 0 && self.resume_ms != 0 && self.freshness_ms != 0,
            "PSI brake durations must be positive");
        for path in self.protected_cgroups.iter()
            .chain(&self.excluded_cgroups)
            .chain(&self.job_parent_cgroups) {
            ensure!(!path.as_os_str().is_empty() && path.components().all(|part|
                matches!(part, std::path::Component::RootDir | std::path::Component::Normal(_))),
                "cgroup paths must stay within the cgroup mount");
        }
        ensure!(self.excluded_cgroups.iter().all(|path|
            path.is_absolute() || path.file_name() == Some(path.as_os_str())),
            "relative cgroup exclusions must be basenames");
        ensure!(self.job_parent_cgroups.iter().all(|path|
            path.is_absolute() || path.file_name() == Some(path.as_os_str())),
            "relative job parent cgroups must be basenames");
        for path in self.job_parent_cgroups.iter().chain(&self.excluded_cgroups)
            .filter(|path| !path.is_absolute()) {
            ensure!(path.to_str().is_some_and(|name| slice_segments(name)
                .is_some_and(|segments| segments.len() <= MAX_CGROUPS as usize)),
                "job parent and excluded cgroup basenames must be canonical systemd slice names");
        }
        ensure!(self.job_parent_cgroups.len() + self.excluded_cgroups.len() <= MAX_CGROUPS as usize,
            "too many configured job parent and excluded cgroups");
        for path in self.job_parent_cgroups.iter().chain(&self.excluded_cgroups)
            .filter(|path| path.is_absolute()) {
            ensure!(path.components().count() <= MAX_CGROUPS as usize,
                "configured cgroup paths must stay within the enumeration bound");
        }
        if !self.protected_cgroups.is_empty()
            && self.job_parent_cgroups.iter().chain(&self.excluded_cgroups)
                .any(|path| !path.is_absolute())
        {
            ensure!(self.protected_cgroups.iter().all(|path|
                !user_manager_roots(std::slice::from_ref(path)).is_empty()),
                "relative job-parent and exclusion names require manager-qualified protected roots; use explicit user-manager/app.slice roots or absolute job-parent and exclusion paths for ancestor roots");
        }
        for (index, rule) in self.rules.iter().enumerate() {
            ensure!(rule.signal != Signal::MemoryAvailable,
                "memory-available uses memoryAvailableFloorBytes, not a PSI rule");
            ensure!(rule.threshold_millipercent <= 100000
                && rule.resume_millipercent < rule.threshold_millipercent,
                "PSI resume threshold must be below trip threshold, both within 0..100%");
            ensure!(!self.rules[..index].iter().any(|other|
                other.signal == rule.signal && other.window == rule.window),
                "duplicate PSI signal/window rule");
        }
        if !self.excluded_cgroups.iter().any(|path|
            path.file_name().is_some_and(|name| name == "app-agent-seats.slice"))
        {
            tracing::warn!("excluded_cgroups omits app-agent-seats.slice; include the seat subtree when overriding defaults to keep self-throttled seats out of host admission pressure");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Reading {
    pub signal: Signal,
    pub window: Window,
    pub millipercent: u32,
    /// Same monotonic clock domain as the caller's now_ms, never a wall-clock timestamp.
    pub observed_at_ms: u64,
    /// Fresh host MemAvailable is below the independent configured floor.
    pub memory_shortage: bool,
    pub memory_available_bytes: Option<u64>,
    pub cgroup: Option<Arc<str>>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum ObservationCause {
    MissingOrInvalid,
    ObservationGap,
    StaleDecision,
    NotYetSampled,
    Io {
        os_error: Option<i32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<Arc<str>>,
    },
    TooManyCgroups { path: Arc<str>, limit: u32 },
    TooManyCgroupEntries { path: Arc<str>, limit: u32 },
    InvalidPressure { path: Arc<str> },
    InvalidMemoryFloor,
}

impl ObservationCause {
    fn from_error(error: &anyhow::Error) -> Self {
        if let Some(error) = error.downcast_ref::<ObservationError>() {
            return error.cause.clone();
        }
        if error.is::<InvalidMemoryFloor>() {
            Self::InvalidMemoryFloor
        } else if let Some(error) = error.downcast_ref::<std::io::Error>() {
            Self::Io { os_error: error.raw_os_error(), path: None }
        } else {
            Self::MissingOrInvalid
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Reason {
    /// PSI values use milli-percent; MemoryAvailable values use bytes.
    Pressure {
        signal: Signal, value: u64, threshold: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cgroup: Option<Arc<str>>,
    },
    /// Remaining healthy observation time, in milliseconds.
    Recovering { remaining: u64 },
    UnknownObservation { cause: ObservationCause },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Decision {
    pub blocked: bool,
    pub reason: Option<Reason>,
}

#[derive(Debug)]
pub struct Brake {
    policy: Policy,
    over_since: Vec<Option<u64>>,
    floor_over_since: Option<u64>,
    last_observation: Option<u64>,
    recovering_since: Option<u64>,
    tripped: bool,
    trip_pressure: Option<Reason>,
}

impl Brake {
    pub fn new(policy: Policy) -> Result<Self> {
        policy.validate()?;
        Ok(Self {
            over_since: vec![None; policy.rules.len()], policy,
            last_observation: None, recovering_since: None, floor_over_since: None,
            tripped: false, trip_pressure: None,
        })
    }

    /// A fresh healthy initial observation is sufficient; recovery hysteresis applies after
    /// pressure/unknown data has actually closed admission, not as an artificial startup delay.
    pub fn observe(&mut self, readings: &[Reading], now_ms: u64) -> Decision {
        let gap = self.last_observation.is_some_and(|previous|
            now_ms < previous || now_ms - previous > self.policy.freshness_ms);
        self.last_observation = Some(now_ms);
        let complete = !gap && self.policy.rules.iter().all(|rule| {
            let mut matches = readings.iter().filter(|reading|
                reading.signal == rule.signal && reading.window == rule.window);
            let Some(reading) = matches.next() else { return false; };
            matches.next().is_none() && reading.millipercent <= 100000
                && (!reading.memory_shortage || reading.memory_available_bytes.is_some())
                && reading.observed_at_ms <= now_ms
                && now_ms - reading.observed_at_ms <= self.policy.freshness_ms
        });
        if !complete {
            return self.unknown(if gap {
                ObservationCause::ObservationGap
            } else {
                ObservationCause::MissingOrInvalid
            });
        }
        let shortage = self.policy.memory_available_floor_bytes > 0
            && readings.iter().any(|reading| reading.memory_shortage);
        if shortage {
            let since = self.floor_over_since.get_or_insert(now_ms);
            if now_ms - *since >= self.policy.sustained_ms {
                self.tripped = true;
                self.trip_pressure = Some(Reason::Pressure {
                    signal: Signal::MemoryAvailable,
                    value: readings.iter().find_map(|reading| reading.memory_available_bytes)
                        .expect("validated memory shortage has an available-byte observation"),
                    threshold: self.policy.memory_available_floor_bytes,
                    cgroup: None,
                });
            }
        } else {
            self.floor_over_since = None;
        }
        let mut all_below_resume = !shortage;
        for (index, rule) in self.policy.rules.iter().enumerate() {
            // Complete unique readings were checked above; no allocation or map reconstruction.
            let Some(reading) = readings.iter().find(|reading|
                reading.signal == rule.signal && reading.window == rule.window) else {
                unreachable!("validated PSI reading disappeared");
            };
            all_below_resume &= reading.millipercent < rule.resume_millipercent;
            if reading.millipercent >= rule.threshold_millipercent {
                let since = self.over_since[index].get_or_insert(now_ms);
                if now_ms - *since >= self.policy.sustained_ms {
                    self.tripped = true;
                    self.trip_pressure = Some(Reason::Pressure {
                        signal: rule.signal, value: u64::from(reading.millipercent),
                        threshold: u64::from(rule.threshold_millipercent),
                        cgroup: reading.cgroup.clone(),
                    });
                }
            } else {
                self.over_since[index] = None;
            }
        }
        if !self.tripped {
            return Decision { blocked: false, reason: None };
        }
        if all_below_resume {
            let since = self.recovering_since.get_or_insert(now_ms);
            if now_ms - *since >= self.policy.resume_ms {
                self.tripped = false;
                self.trip_pressure = None;
                self.recovering_since = None;
                self.over_since.fill(None);
                return Decision { blocked: false, reason: None };
            }
        } else {
            self.recovering_since = None;
        }
        let recovering = Reason::Recovering {
            remaining: self.policy.resume_ms.saturating_sub(
                self.recovering_since.map_or(0, |since| now_ms - since)),
        };
        Decision { blocked: true, reason: Some(if all_below_resume {
            recovering
        } else {
            self.trip_pressure.clone().unwrap_or(recovering)
        }) }
    }

    fn unknown(&mut self, cause: ObservationCause) -> Decision {
        self.over_since.fill(None);
        self.floor_over_since = None;
        self.recovering_since = None;
        self.tripped = true;
        self.trip_pressure = None;
        Decision { blocked: true, reason: Some(Reason::UnknownObservation { cause }) }
    }
}

/// Parse one kernel PSI line without floating-point thresholds or percentage rounding down.
/// Out-of-range/malformed observations remain errors; callers must not replace them with zero.
pub fn parse_millipercent(line: &str, window: Window) -> Result<u32> {
    let field = match window { Window::Avg10 => "avg10=", Window::Avg60 => "avg60=", Window::Avg300 => "avg300=" };
    let mut fields = line.split_ascii_whitespace().filter_map(|value| value.strip_prefix(field));
    let value = fields.next().ok_or_else(|| anyhow::anyhow!("PSI window is absent"))?;
    ensure!(fields.next().is_none(), "duplicate PSI window");
    let (integer, fraction) = value.split_once('.').unwrap_or((value, ""));
    ensure!(!integer.is_empty() && integer.bytes().all(|byte| byte.is_ascii_digit())
        && fraction.len() <= 3 && fraction.bytes().all(|byte| byte.is_ascii_digit()),
        "invalid PSI percentage");
    let whole = integer.parse::<u32>()?;
    ensure!(whole <= 100, "PSI percentage exceeds 100%");
    let decimal = if fraction.is_empty() { 0 } else { fraction.parse::<u32>()? };
    let multiplier = match fraction.len() { 0 | 3 => 1, 1 => 100, 2 => 10, _ => unreachable!() };
    let result = whole * 1000 + decimal * multiplier;
    ensure!(result <= 100000, "PSI percentage exceeds 100%");
    Ok(result)
}

#[derive(Debug)]
struct InvalidMemoryFloor { floor: u64, total: u64 }

impl std::fmt::Display for InvalidMemoryFloor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "psiBrake.memoryAvailableFloorBytes ({}) must be below MemTotal ({})",
            self.floor, self.total)
    }
}
impl std::error::Error for InvalidMemoryFloor {}

fn meminfo_bytes(source: &str, key: &str) -> Result<u64> {
    let mut matches = source.lines().filter_map(|line| line.strip_prefix(key));
    let value = matches.next().ok_or_else(|| anyhow::anyhow!("missing {key}"))?;
    ensure!(matches.next().is_none(), "duplicate {key}");
    let mut fields = value.split_ascii_whitespace();
    let kib: u64 = fields.next().ok_or_else(|| anyhow::anyhow!("missing {key} value"))?.parse()?;
    ensure!(fields.next() == Some("kB") && fields.next().is_none(), "invalid {key} unit");
    kib.checked_mul(1024).ok_or_else(|| anyhow::anyhow!("{key} overflow"))
}
fn validate_memory_floor(policy: &Policy, meminfo: &str) -> Result<()> {
    let total = meminfo_bytes(meminfo, "MemTotal:")?;
    if policy.memory_available_floor_bytes >= total {
        return Err(InvalidMemoryFloor { floor: policy.memory_available_floor_bytes, total }.into());
    }
    Ok(())
}



fn signal_pressure(source: &str, name: &str, window: Window) -> Result<u32> {
    let mut matches = source.lines().filter(|line|
        line.split_ascii_whitespace().next() == Some(name));
    let line = matches.next().ok_or_else(|| anyhow::anyhow!("missing PSI signal"))?;
    ensure!(matches.next().is_none(), "duplicate PSI signal");
    parse_millipercent(line, window)
}

const MAX_CGROUPS: u32 = 256;
const MAX_CGROUP_ENTRIES: u32 = 4096;

/// Shared enumeration bounds across one observation's whole frontier walk.
#[derive(Default)]
struct Enumeration { directories: u32, entries: u32 }

#[derive(Debug)]
struct ObservationError {
    cause: ObservationCause,
}

impl std::fmt::Display for ObservationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "protected cgroup observation failed: {:?}", self.cause)
    }
}
impl std::error::Error for ObservationError {}

fn io_error(path: &Path, error: std::io::Error) -> anyhow::Error {
    ObservationError { cause: ObservationCause::Io {
        os_error: error.raw_os_error(), path: Some(path.to_string_lossy().into()),
    } }.into()
}

fn read_file(path: &Path, buffer: &mut String) -> Result<()> {
    use std::io::Read as _;
    buffer.clear();
    std::fs::File::open(path).and_then(|mut file| file.read_to_string(buffer))
        .map_err(|error| io_error(path, error))?;
    Ok(())
}

fn rooted(root: &Path, path: &Path) -> PathBuf {
    root.join(path.strip_prefix("/").unwrap_or(path))
}

/// Canonical systemd slice hierarchy: `app-capped-heavy.slice` nests inside
/// `app-capped.slice`, which nests inside `app.slice`. Segment names are
/// non-empty runs of ASCII letters, digits and underscores joined by '-'.
fn slice_segments(name: &str) -> Option<Vec<String>> {
    let stem = name.strip_suffix(".slice")?;
    let segments = stem.split('-').map(str::to_owned).collect::<Vec<_>>();
    (segments.iter().all(|segment| !segment.is_empty()
        && segment.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')))
        .then_some(segments)
}

/// One configured job-parent or exclusion entry resolves to a cgroup path:
/// basenames name canonical slices beneath the user-manager root, absolute
/// paths are exact and mount-relative.
fn configured_path(entry: &Path, manager: Option<&Path>, root: &Path) -> Option<PathBuf> {
    if entry.is_absolute() {
        return Some(rooted(root, entry));
    }
    let mut path = manager?.to_path_buf();
    let mut nested = String::new();
    for segment in slice_segments(entry.to_str()?)? {
        if !nested.is_empty() { nested.push('-'); }
        nested.push_str(&segment);
        path.push(format!("{nested}.slice"));
    }
    Some(path)
}

/// Resolve each discovered manager independently; multiple logged-in users may
/// have identically named job slices in different cgroup subtrees.
fn user_manager_roots(protected: &[PathBuf]) -> Vec<PathBuf> {
    let mut managers = Vec::new();
    for path in protected {
        let mut prefix = PathBuf::new();
        let mut found = false;
        for part in path.components() {
            prefix.push(part.as_os_str());
            let Some(name) = part.as_os_str().to_str() else { continue; };
            if name.strip_prefix("user@").and_then(|name| name.strip_suffix(".service"))
                .is_some_and(|uid| !uid.is_empty() && uid.bytes().all(|byte| byte.is_ascii_digit()))
            {
                managers.push(prefix);
                found = true;
                break;
            }
        }
        if !found && path.file_name().is_some_and(|name| name == "app.slice")
            && let Some(parent) = path.parent()
        {
            managers.push(parent.to_path_buf());
        }
    }
    managers.sort();
    managers.dedup();
    managers
}

/// A discovered optional slice is skipped when absent and rediscovered each
/// sample; an explicit protected root absent fails closed instead.
fn push_if_present(protected: &mut Vec<PathBuf>, path: PathBuf) -> Result<()> {
    match std::fs::metadata(&path) {
        Ok(_) => protected.push(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
        Err(error) => return Err(io_error(&path, error)),
    }
    Ok(())
}

/// Buffers are reused; each observation rediscovers the current cgroup children.
#[derive(Default, Debug)]
pub struct Reader {
    memory: String,
    io: String,
    meminfo: String,
    placement: String,
    readings: Vec<Reading>,
}

impl Reader {
    fn new(policy: &Policy, meminfo_path: &Path) -> Result<Self> {
        let mut reader = Self::default();
        if policy.memory_available_floor_bytes > 0 {
            read_file(meminfo_path, &mut reader.meminfo)?;
            validate_memory_floor(policy, &reader.meminfo)?;
        }
        Ok(reader)
    }

    pub fn read(&mut self, policy: &Policy, now_ms: u64) -> Result<&[Reading]> {
        self.read_layout(policy, now_ms, Path::new("/proc/self/cgroup"),
            Path::new("/sys/fs/cgroup"), Path::new("/proc/meminfo"))
    }

    /// ARC eligibility uses the same configured frontier, independent of brake rules.
    pub fn memory_full_avg60(&mut self, policy: &Policy) -> Result<u32> {
        self.memory_full_avg60_layout(policy, Path::new("/proc/self/cgroup"),
            Path::new("/sys/fs/cgroup"), Path::new("/proc/meminfo"))
    }

    fn memory_full_avg60_layout(&mut self, policy: &Policy, placement: &Path,
        root: &Path, meminfo: &Path) -> Result<u32>
    {
        let rules = [Rule { signal: Signal::MemoryFull, window: Window::Avg60,
            threshold_millipercent: 1000, resume_millipercent: 500 }];
        let readings = self.read_layout_rules(policy, &rules, 0, placement, root, meminfo)?;
        ensure!(!readings[0].memory_shortage, "host memory below admission floor");
        Ok(readings[0].millipercent)
    }

    fn protected(&mut self, policy: &Policy, placement_path: &Path, root: &Path) -> Result<Vec<PathBuf>> {
        if !policy.protected_cgroups.is_empty() {
            return Ok(policy.protected_cgroups.iter().map(|path| rooted(root, path)).collect());
        }
        read_file(placement_path, &mut self.placement)?;
        let mut placements = self.placement.lines().filter_map(|line| line.strip_prefix("0::"));
        let own = placements.next().ok_or_else(|| anyhow::anyhow!("missing unified cgroup placement"))?;
        ensure!(placements.next().is_none(), "duplicate unified cgroup placement");
        let path = Path::new(own);
        ensure!(path.is_absolute() && path.components().all(|part|
            matches!(part, std::path::Component::RootDir | std::path::Component::Normal(_))),
            "malformed unified cgroup placement");
        // A delegated service may contain the process in a child cgroup.
        let service_depth = path.components().enumerate()
            .filter(|(_, part)| part.as_os_str().to_string_lossy().ends_with(".service"))
            .map(|(index, _)| index + 1).last();
        let service: PathBuf = path.components().take(service_depth.unwrap_or(usize::MAX)).collect();
        let mut protected = vec![rooted(root, &service), root.join("system.slice")];
        if let Some(manager) = crate::lane::user_manager_cgroup_in(own, root) {
            protected.push(manager.join("app.slice"));
            push_if_present(&mut protected, manager.join("interactive.slice"))?;
        } else {
            // A system service has no user-manager ancestor. Discover user
            // slices at depth one, then probe their conventional manager paths.
            let users = root.join("user.slice");
            match std::fs::metadata(&users) {
                Ok(_) => {
                    for user in Self::children(&users, &mut Enumeration::default())? {
                        let Some(name) = user.file_name().and_then(|name| name.to_str()) else { continue; };
                        let Some(uid) = name.strip_prefix("user-").and_then(|name| name.strip_suffix(".slice"))
                            .filter(|uid| !uid.is_empty() && uid.bytes().all(|byte| byte.is_ascii_digit())) else { continue; };
                        for slice in ["app.slice", "interactive.slice"] {
                            push_if_present(&mut protected,
                                user.join(format!("user@{uid}.service/{slice}")))?;
                        }
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
                Err(error) => return Err(io_error(&users, error)),
            }
            if protected.len() == 2 {
                // Top-level app.slice and interactive.slice also support
                // non-user-manager layouts. app.slice fails closed when
                // absent; the optional interactive.slice is rediscovered
                // each sample.
                protected.push(root.join("app.slice"));
                push_if_present(&mut protected, root.join("interactive.slice"))?;
            }
        }
        protected.sort();
        protected.dedup();
        Ok(protected)
    }

    fn children(path: &Path, bounds: &mut Enumeration) -> Result<Vec<PathBuf>> {
        let entries = std::fs::read_dir(path).map_err(|error| io_error(path, error))?;
        let mut children = Vec::new();
        for entry in entries {
            // Bound raw snapshot entries as well as retained cgroup
            // directories; control files count only toward the raw limit.
            bounds.entries += 1;
            if bounds.entries > MAX_CGROUP_ENTRIES {
                return Err(ObservationError { cause: ObservationCause::TooManyCgroupEntries {
                    path: path.to_string_lossy().into(), limit: MAX_CGROUP_ENTRIES,
                } }.into());
            }
            let entry = entry.map_err(|error| io_error(path, error))?;
            let child = entry.path();
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => {
                    bounds.directories += 1;
                    if bounds.directories > MAX_CGROUPS {
                        return Err(ObservationError { cause: ObservationCause::TooManyCgroups {
                            path: path.to_string_lossy().into(), limit: MAX_CGROUPS,
                        } }.into());
                    }
                    children.push(child);
                }
                Ok(_) => {},
                Err(error) => {
                    if matches!(std::fs::metadata(&child),
                        Err(missing) if missing.kind() == std::io::ErrorKind::NotFound) {
                        continue;
                    }
                    return Err(io_error(&child, error));
                }
            }
        }
        children.sort();
        Ok(children)
    }

    // A child from an enumeration snapshot may disappear before either PSI read.
    // Only absence of the cgroup itself permits skipping it, never absence of PSI.
    fn sample_cgroup(&mut self, rules: &[Rule], root: &Path, path: &Path, vanished_ok: bool) -> Result<()> {
        let needs_memory = rules.iter().any(|rule|
            matches!(rule.signal, Signal::MemorySome | Signal::MemoryFull));
        let needs_io = rules.iter().any(|rule|
            matches!(rule.signal, Signal::IoSome | Signal::IoFull));
        let result = (|| {
            if needs_memory { read_file(&path.join("memory.pressure"), &mut self.memory)?; }
            if needs_io { read_file(&path.join("io.pressure"), &mut self.io)?; }
            Ok(())
        })();
        if let Err(error) = result {
            if vanished_ok && matches!(std::fs::metadata(path),
                Err(missing) if missing.kind() == std::io::ErrorKind::NotFound) {
                return Ok(());
            }
            return Err(error);
        }
        let mut identity = None;
        for (rule, reading) in rules.iter().zip(&mut self.readings) {
            let (source, name) = match rule.signal {
                Signal::MemoryFull => (&self.memory, "full"),
                Signal::MemorySome => (&self.memory, "some"),
                Signal::IoFull => (&self.io, "full"),
                Signal::IoSome => (&self.io, "some"),
                Signal::MemoryAvailable => unreachable!("memory floor is not a PSI rule"),
            };
            let file = path.join(if matches!(rule.signal, Signal::MemoryFull | Signal::MemorySome) {
                "memory.pressure"
            } else { "io.pressure" });
            let value = signal_pressure(source, name, rule.window).map_err(|_| ObservationError {
                cause: ObservationCause::InvalidPressure { path: file.to_string_lossy().into() },
            })?;
            if reading.cgroup.is_none() || value > reading.millipercent {
                let identity = identity.get_or_insert_with(|| Arc::<str>::from(
                    format!("/{}", path.strip_prefix(root).unwrap_or(path).display())));
                reading.millipercent = value;
                reading.cgroup = Some(identity.clone());
            }
        }
        Ok(())
    }

    /// One level of the split frontier. Configured job-parent and excluded
    /// endpoints are skipped whole; descent continues only along configured
    /// paths; every other child is sampled directly.
    fn sample_frontier(&mut self, rules: &[Rule], root: &Path, children: &[PathBuf],
        protected: &[PathBuf], excluded: &[PathBuf], bounds: &mut Enumeration,
    ) -> Result<()> {
        for child in children {
            if excluded.iter().any(|job| child.starts_with(job)) { continue; }
            if protected.iter().any(|path| path == child) {
                // An independently protected child is observed by its own plan.
                continue;
            }
            if excluded.iter().any(|job| job.starts_with(child)) {
                let descendants = match Self::children(child, bounds) {
                    Ok(descendants) => descendants,
                    Err(_) if matches!(std::fs::metadata(child),
                        Err(missing) if missing.kind() == std::io::ErrorKind::NotFound) => continue,
                    Err(error) => return Err(error),
                };
                self.sample_frontier(rules, root, &descendants, protected, excluded, bounds)?;
                continue;
            }
            self.sample_cgroup(rules, root, child, true)?;
        }
        Ok(())
    }

    fn read_layout(&mut self, policy: &Policy, now_ms: u64, placement_path: &Path,
        cgroup_root: &Path, meminfo_path: &Path) -> Result<&[Reading]>
    {
        self.read_layout_rules(policy, &policy.rules, now_ms, placement_path, cgroup_root, meminfo_path)
    }

    fn read_layout_rules(&mut self, policy: &Policy, rules: &[Rule], now_ms: u64,
        placement_path: &Path, cgroup_root: &Path, meminfo_path: &Path) -> Result<&[Reading]>
    {
        let mut available = None;
        if policy.memory_available_floor_bytes > 0 {
            read_file(meminfo_path, &mut self.meminfo)?;
            validate_memory_floor(policy, &self.meminfo)?;
            available = Some(meminfo_bytes(&self.meminfo, "MemAvailable:")?);
        }
        self.readings.clear();
        self.readings.extend(rules.iter().map(|rule| Reading {
            signal: rule.signal, window: rule.window, millipercent: 0, observed_at_ms: now_ms,
            memory_shortage: available.is_some_and(|bytes| bytes < policy.memory_available_floor_bytes),
            memory_available_bytes: available, cgroup: None,
        }));
        let protected = self.protected(policy, placement_path, cgroup_root)?;
        // Job parents and excluded subtrees resolve lexically from the
        // configured names: a missing endpoint still splits its protected
        // ancestors, so a removed job can never resurface through a
        // contaminated ancestor aggregate.
        let managers = user_manager_roots(&protected);
        ensure!(!managers.is_empty()
            || policy.job_parent_cgroups.iter().chain(&policy.excluded_cgroups)
                .all(|path| path.is_absolute()),
            "relative job-parent and exclusion names cannot be resolved without a user manager");
        let mut excluded = Vec::new();
        for entry in policy.job_parent_cgroups.iter().chain(&policy.excluded_cgroups) {
            if entry.is_absolute() {
                excluded.push(rooted(cgroup_root, entry));
            } else {
                for manager in &managers {
                    if let Some(path) = configured_path(entry, Some(manager), cgroup_root) {
                        excluded.push(path);
                    }
                }
            }
        }
        excluded.sort();
        excluded.dedup();
        let mut bounds = Enumeration::default();
        let mut groups = Vec::with_capacity(protected.len());
        for path in protected {
            // A configured job parent or excluded subtree is never observed,
            // even when it is itself protected.
            if excluded.iter().any(|job| path.starts_with(job)) { continue; }
            groups.push(path);
        }
        for path in &groups {
            if excluded.iter().any(|job| job.starts_with(path)) {
                // Split along the configured paths: observe the remaining
                // immediate children, never this contaminated ancestor.
                let children = Self::children(path, &mut bounds)?;
                self.sample_frontier(rules, cgroup_root, &children, &groups, &excluded, &mut bounds)?;
            } else {
                // No configured path lives inside; observe it directly.
                self.sample_cgroup(rules, cgroup_root, path, false)?;
            }
        }
        Ok(&self.readings)
    }
}

/// Shared by the daemon sampler and admission callers; never admits by elapsed time alone.
pub struct Observer {
    policy: Policy,
    brake: Brake,
    reader: Reader,
    origin: tokio::time::Instant,
    sampled_at: Option<u64>,
    cached: Decision,
    paths: Option<[PathBuf; 3]>,
}

impl Observer {
    pub fn new(policy: Policy) -> Result<Self> {
        let reader = Reader::new(&policy, std::path::Path::new("/proc/meminfo"))?;
        Ok(Self {
            brake: Brake::new(policy.clone())?, policy, reader,
            origin: tokio::time::Instant::now(), sampled_at: None, paths: None,
            cached: Decision { blocked: true, reason: Some(Reason::UnknownObservation {
                cause: ObservationCause::NotYetSampled,
            }) },
        })
    }

    /// Inject unified placement, cgroup-v2 mount and meminfo locations.
    pub fn with_paths(policy: Policy, placement_path: PathBuf, cgroup_root: PathBuf,
        meminfo_path: PathBuf) -> Result<Self>
    {
        let reader = Reader::new(&policy, &meminfo_path)?;
        let paths = [placement_path, cgroup_root, meminfo_path];
        Ok(Self {
            brake: Brake::new(policy.clone())?, policy, reader,
            origin: tokio::time::Instant::now(), sampled_at: None, paths: Some(paths),
            cached: Decision { blocked: true, reason: Some(Reason::UnknownObservation {
                cause: ObservationCause::NotYetSampled,
            }) },
        })
    }

    /// Read the sampler's cached decision without kernel I/O or advancing hysteresis.
    /// A stopped sampler cannot leave admission open after its freshness deadline.
    pub fn decision(&self) -> Decision {
        let now = u64::try_from(self.origin.elapsed().as_millis()).unwrap_or(u64::MAX);
        if self.sampled_at.is_some_and(|at| now.saturating_sub(at) > self.policy.freshness_ms) {
            Decision { blocked: true, reason: Some(Reason::UnknownObservation {
                cause: ObservationCause::StaleDecision,
            }) }
        } else {
            self.cached.clone()
        }
    }

    /// Sample kernel observations; only the independent sampler may advance the brake.
    pub fn observe(&mut self, policy: &Policy) -> Result<Decision> {
        if &self.policy != policy {
            self.brake = Brake::new(policy.clone())?;
            self.policy = policy.clone();
            self.sampled_at = None;
            self.cached = Decision { blocked: true, reason: Some(Reason::UnknownObservation {
                cause: ObservationCause::NotYetSampled,
            }) };
        }
        let now = u64::try_from(self.origin.elapsed().as_millis()).unwrap_or(u64::MAX);
        // The daemon interval owns cadence. A second throttle here would discard a
        // slightly early tick and turn scheduling jitter into a freshness gap.
        self.sampled_at = Some(now);
        let readings = match &self.paths {
            Some([placement, root, meminfo]) =>
                self.reader.read_layout(&self.policy, now, placement, root, meminfo),
            None => self.reader.read(&self.policy, now),
        };
        self.cached = match readings {
            Ok(readings) => self.brake.observe(readings, now),
            Err(error) => {
                self.reader.readings.clear();
                self.brake.last_observation = Some(now);
                let decision = self.brake.unknown(ObservationCause::from_error(&error));
                if error.is::<InvalidMemoryFloor>() {
                    self.sampled_at = None;
                    self.cached = decision;
                    return Err(error);
                }
                decision
            }
        };
        Ok(self.decision())
    }
}

#[cfg(test)]
#[path = "resource_pressure_tests.rs"]
mod tests;
