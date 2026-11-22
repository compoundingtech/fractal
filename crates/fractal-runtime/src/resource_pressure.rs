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
        std::time::Duration::from_millis((self.freshness_ms / 4).max(1))
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(!self.rules.is_empty() && self.rules.len() <= 12,
            "PSI brake requires one to twelve signal/window rules");
        ensure!(self.sustained_ms != 0 && self.resume_ms != 0 && self.freshness_ms != 0,
            "PSI brake durations must be positive");
        for (index, rule) in self.rules.iter().enumerate() {
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
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Reason {
    Pressure(Signal),
    UnknownObservation,
    Recovering,
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
    last_observation: Option<u64>,
    recovering_since: Option<u64>,
    tripped: bool,
    trip_signal: Option<Signal>,
}

impl Brake {
    pub fn new(policy: Policy) -> Result<Self> {
        policy.validate()?;
        Ok(Self {
            over_since: vec![None; policy.rules.len()], policy,
            last_observation: None, recovering_since: None,
            tripped: false, trip_signal: None,
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
                && reading.observed_at_ms <= now_ms
                && now_ms - reading.observed_at_ms <= self.policy.freshness_ms
        });
        if !complete {
            self.over_since.fill(None);
            self.recovering_since = None;
            self.tripped = true;
            self.trip_signal = None;
            return Decision { blocked: true, reason: Some(Reason::UnknownObservation) };
        }
        let mut all_below_resume = true;
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
                    self.trip_signal = Some(rule.signal);
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
                self.trip_signal = None;
                self.recovering_since = None;
                self.over_since.fill(None);
                return Decision { blocked: false, reason: None };
            }
        } else {
            self.recovering_since = None;
        }
        Decision { blocked: true, reason: Some(if all_below_resume {
            Reason::Recovering
        } else {
            self.trip_signal.map_or(Reason::Recovering, Reason::Pressure)
        }) }
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

/// Reuse the small kernel-file and parsed-reading buffers across host observations.
#[derive(Default, Debug)]
pub struct Reader {
    memory: String,
    io: String,
    meminfo: String,
    readings: Vec<Reading>,
}

impl Reader {
    pub fn read(&mut self, policy: &Policy, now_ms: u64) -> Result<&[Reading]> {
        let lane = if policy.rules.iter().any(|rule|
            matches!(rule.signal, Signal::MemorySome | Signal::MemoryFull))
        {
            crate::lane::app_slice_cgroup()
                .ok_or_else(|| anyhow::anyhow!("missing lane cgroup"))?
                .join(crate::lane::LANES_SLICE)
        } else { std::path::PathBuf::new() };
        self.read_paths(policy, now_ms, &lane.join("memory.pressure"),
            std::path::Path::new("/proc/pressure/io"),
            std::path::Path::new("/proc/meminfo"))
    }

    fn read_paths(
        &mut self,
        policy: &Policy,
        now_ms: u64,
        memory_path: &std::path::Path,
        io_path: &std::path::Path,
        meminfo_path: &std::path::Path,
    ) -> Result<&[Reading]> {
        use std::io::Read as _;
        self.readings.clear();
        let needs_memory = policy.rules.iter().any(|rule|
            matches!(rule.signal, Signal::MemorySome | Signal::MemoryFull));
        let needs_io = policy.rules.iter().any(|rule|
            matches!(rule.signal, Signal::IoSome | Signal::IoFull));
        let mut shortage = false;
        let mut lane_absent = false;
        if needs_memory {
            self.memory.clear();
            match std::fs::File::open(memory_path) {
                Ok(mut file) => { file.read_to_string(&mut self.memory)?; }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    let cgroup = memory_path.parent().ok_or_else(|| anyhow::anyhow!("missing lane path"))?;
                    match std::fs::metadata(cgroup) {
                        Err(missing) if missing.kind() == std::io::ErrorKind::NotFound => {
                            // The runtime creates this slice only when enforced work launches.
                            // Require its containing hierarchy: an unavailable mount/manager
                            // is not evidence that the lane alone is absent.
                            let parent = cgroup.parent().ok_or_else(|| anyhow::anyhow!("missing lane parent"))?;
                            ensure!(std::fs::metadata(parent)?.is_dir(), "missing lane hierarchy");
                            lane_absent = true;
                        }
                        _ => return Err(error.into()),
                    }
                }
                Err(error) => return Err(error.into()),
            }
            self.meminfo.clear();
            std::fs::File::open(meminfo_path)?.read_to_string(&mut self.meminfo)?;
            let mut available = self.meminfo.lines().filter_map(|line|
                line.strip_prefix("MemAvailable:"));
            let value = available.next().ok_or_else(|| anyhow::anyhow!("missing MemAvailable"))?;
            ensure!(available.next().is_none(), "duplicate MemAvailable");
            let mut fields = value.split_ascii_whitespace();
            let kib: u64 = fields.next().ok_or_else(|| anyhow::anyhow!("missing MemAvailable value"))?.parse()?;
            ensure!(fields.next() == Some("kB") && fields.next().is_none(), "invalid MemAvailable unit");
            let bytes = kib.checked_mul(1024).ok_or_else(|| anyhow::anyhow!("MemAvailable overflow"))?;
            shortage = bytes < policy.memory_available_floor_bytes;
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
            };
            let memory_rule = matches!(rule.signal, Signal::MemoryFull | Signal::MemorySome);
            let pressure = if lane_absent && memory_rule {
                0
            } else {
                let mut matching = source.lines().filter(|line|
                    line.split_ascii_whitespace().next() == Some(name));
                let line = matching.next().ok_or_else(|| anyhow::anyhow!("missing PSI signal"))?;
                ensure!(matching.next().is_none(), "duplicate PSI signal");
                parse_millipercent(line, rule.window)?
            };
            // A real host shortage must trip even when the lane has no current holders.
            // An existing lane's PSI must still parse, even during host shortage.
            let pressure = if shortage && memory_rule {
                100000
            } else { pressure };
            self.readings.push(Reading {
                signal: rule.signal, window: rule.window,
                millipercent: pressure,
                observed_at_ms: now_ms,
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
    origin: std::time::Instant,
    sampled_at: Option<u64>,
    paths: Option<[std::path::PathBuf; 3]>,
}

impl Observer {
    pub fn new(policy: Policy) -> Result<Self> {
        Ok(Self {
            brake: Brake::new(policy.clone())?, policy, reader: Reader::default(),
            origin: std::time::Instant::now(), sampled_at: None, paths: None,
        })
    }

    /// Inject kernel-file locations, including the aggregate finite-lane memory PSI.
    pub fn with_paths(policy: Policy, paths: [std::path::PathBuf; 3]) -> Result<Self> {
        let mut observer = Self::new(policy)?;
        observer.paths = Some(paths);
        Ok(observer)
    }

    pub fn observe(&mut self, policy: &Policy) -> Result<Decision> {
        if &self.policy != policy {
            self.brake = Brake::new(policy.clone())?;
            self.policy = policy.clone();
            self.sampled_at = None;
        }
        let now = u64::try_from(self.origin.elapsed().as_millis()).unwrap_or(u64::MAX);
        if self.sampled_at.is_none_or(|at|
            u128::from(now.saturating_sub(at)) >= self.policy.sample_interval().as_millis())
        {
            self.sampled_at = Some(now);
            let readings = match &self.paths {
                Some([memory, io, meminfo]) =>
                    self.reader.read_paths(&self.policy, now, memory, io, meminfo),
                None => self.reader.read(&self.policy, now),
            };
            return Ok(match readings {
                Ok(readings) => self.brake.observe(readings, now),
                Err(_) => { self.reader.readings.clear(); self.brake.observe(&[], now) },
            });
        }
        Ok(self.brake.observe(&self.reader.readings, now))
    }
}

#[cfg(test)]
#[path = "resource_pressure_tests.rs"]
mod tests;
