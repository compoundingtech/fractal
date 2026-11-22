//! Pure host-pressure brake. The host observation loop supplies fresh kernel PSI readings;
//! elapsed time alone never constitutes a healthy observation or admission grant.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

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

impl Window {
    fn duration_ms(self) -> u64 {
        match self { Self::Avg10 => 10000, Self::Avg60 => 60000, Self::Avg300 => 300000 }
    }
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
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Reading {
    pub signal: Signal,
    pub window: Window,
    pub millipercent: u32,
    /// Same monotonic clock domain as the caller's now_ms, never a wall-clock timestamp.
    pub observed_at_ms: u64,
    /// Fresh host MemAvailable is below the independent configured floor.
    pub memory_shortage: bool,
    pub memory_available_bytes: Option<u64>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum ObservationCause {
    MissingOrInvalid,
    ObservationGap,
    StaleDecision,
    NotYetSampled,
    Io { os_error: Option<i32> },
    InvalidMemoryFloor,
}

impl ObservationCause {
    fn from_error(error: &anyhow::Error) -> Self {
        if error.is::<InvalidMemoryFloor>() {
            Self::InvalidMemoryFloor
        } else if let Some(error) = error.downcast_ref::<std::io::Error>() {
            Self::Io { os_error: error.raw_os_error() }
        } else {
            Self::MissingOrInvalid
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Reason {
    /// PSI values use milli-percent; MemoryAvailable values use bytes.
    Pressure { signal: Signal, value: u64, threshold: u64 },
    /// Remaining healthy observation time, in milliseconds.
    Recovering { remaining: u64 },
    UnknownObservation { cause: ObservationCause },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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
            self.trip_pressure.unwrap_or(recovering)
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


fn allocstall_total(source: &str) -> Result<u64> {
    ensure!(source.ends_with('\n'), "truncated vmstat");
    let mut total = 0u64;
    let mut found = false;
    for (index, line) in source.lines().enumerate() {
        let mut fields = line.split_ascii_whitespace();
        let Some(key) = fields.next().filter(|key| key.starts_with("allocstall_")) else { continue; };
        ensure!(key.len() > "allocstall_".len(), "missing allocstall zone");
        ensure!(!source.lines().take(index).any(|previous|
            previous.split_ascii_whitespace().next() == Some(key)), "duplicate allocstall counter");
        let count: u64 = fields.next().ok_or_else(|| anyhow::anyhow!("missing allocstall value"))?.parse()?;
        ensure!(fields.next().is_none(), "invalid allocstall counter");
        total = total.checked_add(count).ok_or_else(|| anyhow::anyhow!("allocstall overflow"))?;
        found = true;
    }
    ensure!(found, "missing allocstall counters");
    Ok(total)
}

fn signal_pressure(source: &str, name: &str, window: Window) -> Result<u32> {
    let mut matches = source.lines().filter(|line|
        line.split_ascii_whitespace().next() == Some(name));
    let line = matches.next().ok_or_else(|| anyhow::anyhow!("missing PSI signal"))?;
    ensure!(matches.next().is_none(), "duplicate PSI signal");
    parse_millipercent(line, window)
}

#[derive(Clone, Copy)]
struct KernelPaths<'a> {
    /// None means lane placement is not applicable in this cgroup layout.
    memory_path: Option<&'a std::path::Path>,
    io_path: &'a std::path::Path,
    meminfo_path: &'a std::path::Path,
    root_memory_path: &'a std::path::Path,
    vmstat_path: &'a std::path::Path,
}

/// Reuse the small kernel-file and parsed-reading buffers across host observations.
#[derive(Default, Debug)]
pub struct Reader {
    memory: String,
    io: String,
    meminfo: String,
    root_memory: String,
    vmstat: String,
    own_cgroup: String,
    warned_lane_layout: bool,
    lane_layout_not_applicable: bool,
    previous_allocstall: Option<u64>,
    last_allocstall_at_ms: Option<u64>,
    readings: Vec<Reading>,
}

impl Reader {
    fn new(policy: &Policy, meminfo_path: &std::path::Path) -> Result<Self> {
        let mut reader = Self::default();
        if policy.memory_available_floor_bytes > 0 {
            use std::io::Read as _;
            std::fs::File::open(meminfo_path)?.read_to_string(&mut reader.meminfo)?;
            validate_memory_floor(policy, &reader.meminfo)?;
        }
        Ok(reader)
    }

    pub fn read(&mut self, policy: &Policy, now_ms: u64) -> Result<&[Reading]> {
        self.read_layout(policy, now_ms, std::path::Path::new("/proc/self/cgroup"),
            std::path::Path::new("/sys/fs/cgroup"), KernelPaths {
            memory_path: None,
            io_path: std::path::Path::new("/proc/pressure/io"),
            meminfo_path: std::path::Path::new("/proc/meminfo"),
            root_memory_path: std::path::Path::new("/proc/pressure/memory"),
            vmstat_path: std::path::Path::new("/proc/vmstat"),
        })
    }

    fn read_layout(
        &mut self,
        policy: &Policy,
        now_ms: u64,
        placement_path: &std::path::Path,
        cgroup_root: &std::path::Path,
        paths: KernelPaths<'_>,
    ) -> Result<&[Reading]> {
        use std::io::Read as _;
        self.lane_layout_not_applicable = false;
        let mut lane = None;
        if policy.rules.iter().any(|rule| matches!(rule.signal, Signal::MemorySome | Signal::MemoryFull)) {
            self.own_cgroup.clear();
            std::fs::File::open(placement_path)?.read_to_string(&mut self.own_cgroup)?;
            ensure!(!self.own_cgroup.trim().is_empty(), "empty cgroup placement");
            let mut unified = self.own_cgroup.lines().filter_map(|line| line.strip_prefix("0::"));
            let placement = unified.next().ok_or_else(|| anyhow::anyhow!("missing unified cgroup placement"))?;
            ensure!(unified.next().is_none(), "duplicate unified cgroup placement");
            let path = std::path::Path::new(placement);
            ensure!(path.is_absolute() && path.components().all(|part|
                matches!(part, std::path::Component::RootDir | std::path::Component::Normal(_))),
                "malformed unified cgroup placement");
            lane = crate::lane::user_manager_cgroup_in(placement, cgroup_root)
                .map(|manager| manager.join("app.slice").join(crate::lane::LANES_SLICE).join("memory.pressure"));
            self.lane_layout_not_applicable = lane.is_none();
            if lane.is_none() && !self.warned_lane_layout {
                self.warned_lane_layout = true;
                tracing::warn!(cgroup = %self.own_cgroup.trim(),
                    "finite lane PSI is not applicable to this cgroup layout; using qualified root PSI and the memory floor");
            }
        }
        self.read_paths(policy, now_ms, KernelPaths { memory_path: lane.as_deref(), ..paths })
    }

    fn read_paths(
        &mut self,
        policy: &Policy,
        now_ms: u64,
        paths: KernelPaths<'_>,
    ) -> Result<&[Reading]> {
        use std::io::Read as _;
        let KernelPaths { memory_path, io_path, meminfo_path, root_memory_path, vmstat_path } = paths;
        self.readings.clear();
        let needs_memory = policy.rules.iter().any(|rule|
            matches!(rule.signal, Signal::MemorySome | Signal::MemoryFull));
        let needs_io = policy.rules.iter().any(|rule|
            matches!(rule.signal, Signal::IoSome | Signal::IoFull));
        let mut shortage = false;
        let mut available = None;
        let mut lane_absent = memory_path.is_none();
        if policy.memory_available_floor_bytes > 0 {
            self.meminfo.clear();
            std::fs::File::open(meminfo_path)?.read_to_string(&mut self.meminfo)?;
            validate_memory_floor(policy, &self.meminfo)?;
            available = Some(meminfo_bytes(&self.meminfo, "MemAvailable:")?);
            shortage = available.is_some_and(|bytes| bytes < policy.memory_available_floor_bytes);
        }
        if needs_memory {
            self.memory.clear();
            if let Some(memory_path) = memory_path {
                match std::fs::File::open(memory_path) {
                    Ok(mut file) => { file.read_to_string(&mut self.memory)?; }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        let cgroup = memory_path.parent().ok_or_else(|| anyhow::anyhow!("missing lane path"))?;
                        match std::fs::metadata(cgroup) {
                            Err(missing) if missing.kind() == std::io::ErrorKind::NotFound => {
                                // Only the lane itself may be absent, not its containing hierarchy.
                                let parent = cgroup.parent().ok_or_else(|| anyhow::anyhow!("missing lane parent"))?;
                                ensure!(std::fs::metadata(parent)?.is_dir(), "missing lane hierarchy");
                                lane_absent = true;
                            }
                            _ => return Err(error.into()),
                        }
                    }
                    Err(error) => return Err(error.into()),
                }
            }
            self.root_memory.clear();
            std::fs::File::open(root_memory_path)?.read_to_string(&mut self.root_memory)?;
            self.vmstat.clear();
            std::fs::File::open(vmstat_path)?.read_to_string(&mut self.vmstat)?;
            let current = allocstall_total(&self.vmstat)?;
            // memory.high uses try_to_free_mem_cgroup_pages. ALLOCSTALL is accounted
            // in do_try_to_free_pages only when !cgroup_reclaim(sc) (Linux v6.18
            // mm/vmscan.c), so it distinguishes global reclaim from that memcg path.
            // pgscan_direct includes memcg reclaim; kswapd also runs for routine
            // cache/ARC churn. Neither scan counter alone proves host distress.
            // ALLOCSTALL counts reclaim entry, not each sample spent stalled. Keep
            // evidence for the PSI window plus the brake's sustained interval so
            // pressure starting within that window can trip on a later sample.
            match self.previous_allocstall {
                Some(previous) if current > previous => self.last_allocstall_at_ms = Some(now_ms),
                Some(previous) if current == previous => {},
                _ => self.last_allocstall_at_ms = None, // First sample/reset is lane-only.
            }
            self.previous_allocstall = Some(current);
        }
        if needs_io {
            self.io.clear();
            std::fs::File::open(io_path)?.read_to_string(&mut self.io)?;
        }
        for rule in &policy.rules {
            let (source, name) = match rule.signal {
                Signal::MemoryFull => (&self.memory, "full"),
                Signal::MemorySome => (&self.memory, "some"),
                Signal::IoFull => (&self.io, "full"),
                Signal::IoSome => (&self.io, "some"),
                Signal::MemoryAvailable => unreachable!("memory floor is not a PSI rule"),
            };
            let memory_rule = matches!(rule.signal, Signal::MemoryFull | Signal::MemorySome);
            let pressure = if lane_absent && memory_rule {
                0
            } else { signal_pressure(source, name, rule.window)? };
            // Always parse root PSI, even without a recent qualifying event.
            let pressure = if memory_rule {
                let root = signal_pressure(&self.root_memory, "full", rule.window)?;
                let qualified = self.last_allocstall_at_ms.and_then(|at| now_ms.checked_sub(at))
                    .is_some_and(|age| age <= rule.window.duration_ms().saturating_add(policy.sustained_ms));
                if qualified { pressure.max(root) } else { pressure }
            } else { pressure };
            self.readings.push(Reading {
                signal: rule.signal, window: rule.window,
                millipercent: pressure,
                observed_at_ms: now_ms,
                memory_shortage: shortage,
                memory_available_bytes: available,
            });
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
    paths: Option<[std::path::PathBuf; 5]>,
}

impl Observer {
    /// Placement for a well-formed layout where finite-lane PSI is not applicable.
    pub fn lane_psi_not_applicable_placement(&self) -> Option<&str> {
        self.reader.lane_layout_not_applicable.then(|| self.reader.own_cgroup.trim())
    }

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

    /// Inject lane PSI, root IO, meminfo, root memory PSI and vmstat locations, in order.
    pub fn with_paths(policy: Policy, paths: [std::path::PathBuf; 5]) -> Result<Self> {
        let reader = Reader::new(&policy, &paths[2])?;
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
            self.cached
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
            self.reader.previous_allocstall = None;
            self.reader.last_allocstall_at_ms = None;
        }
        let now = u64::try_from(self.origin.elapsed().as_millis()).unwrap_or(u64::MAX);
        // The daemon interval owns cadence. A second throttle here would discard a
        // slightly early tick and turn scheduling jitter into a freshness gap.
        self.sampled_at = Some(now);
        let readings = match &self.paths {
            Some([memory, io, meminfo, root_memory, vmstat]) =>
                self.reader.read_paths(&self.policy, now, KernelPaths {
                    memory_path: Some(memory), io_path: io, meminfo_path: meminfo,
                    root_memory_path: root_memory, vmstat_path: vmstat,
                }),
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
