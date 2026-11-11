//! Pure readback parsing; absent or malformed counters are unknown, not zero.
use super::JobScopePlan;
use crate::{Coverage, Covered};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScopeLimit { Value(u64), Unlimited }
/// Order is memory.high, memory.max, memory.swap.max, pids.max (see `files`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScopeReadback(pub [Covered<ScopeLimit>; 4]);
impl ScopeReadback {
    pub fn matches(&self, plan: &JobScopePlan) -> [Covered<bool>; 4] {
        std::array::from_fn(|i| Covered {
            value: self.0[i].value.map(|v| v == ScopeLimit::Value(plan.files()[i].1)),
            coverage: self.0[i].coverage.clone(),
        })
    }
}
pub fn unknown<T>(reason: impl Into<String>) -> Covered<T> {
    Covered { value: None, coverage: Coverage::Unknown { reason: reason.into() } }
}
fn measured<T>(value: T) -> Covered<T> {
    Covered { value: Some(value), coverage: Coverage::Measured }
}
pub fn parse_limit(text: &str) -> Covered<ScopeLimit> {
    if text.trim() == "max" { return measured(ScopeLimit::Unlimited); }
    match text.trim().parse() {
        Ok(value) => measured(ScopeLimit::Value(value)),
        Err(_) => unknown("invalid cgroup limit"),
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScopeSample<T> {
    pub host: String,
    pub unix_time_ms: u128,
    pub reading: Covered<T>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawScopeUsage {
    pub cpu_usage_usec: ScopeSample<u64>,
    pub memory_peak: ScopeSample<u64>,
    /// Raw kernel counters retained without conflating distinct event kinds/devices.
    pub memory_events: ScopeSample<String>,
    pub io_stat: ScopeSample<String>,
}
pub fn parse_usage_number(text: &str, key: Option<&str>) -> Covered<u64> {
    let value = match key {
        None => Some(text.trim()),
        Some(key) => text.lines().find_map(|line| {
            let mut fields = line.split_whitespace();
            (fields.next() == Some(key)).then(|| fields.next()).flatten()
        }),
    };
    match value.and_then(|v| v.parse().ok()) {
        Some(value) => measured(value),
        None => unknown("missing or invalid usage counter"),
    }
}
/// Preserve raw memory.events/io.stat, including an empty but successfully read io.stat.
pub fn parse_raw_usage(text: &str) -> Covered<String> { measured(text.to_owned()) }
