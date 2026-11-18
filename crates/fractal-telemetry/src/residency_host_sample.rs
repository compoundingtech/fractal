//! On-demand host sampling, with no timer, claim writer or daemon registration.
//! Linux private memory is Private_Clean + Private_Dirty + Private_Hugetlb,
//! never RSS. Other platforms and unreadable measurements remain absent.
use std::{fs, path::Path};

/// The projection adapter must provide a consistent, caught-up cut. It is not
/// implemented here: the wake-demand reader does not yet expose held silent mail.
#[derive(Clone, Copy, Debug)]
pub enum HeldSilent {
    Unavailable,
    CaughtUp { oldest_accepted_ms: Option<i64> },
}
impl HeldSilent {
    pub fn age_seconds(self, now_ms: i64) -> Option<f64> {
        match self {
            Self::CaughtUp { oldest_accepted_ms: Some(at) } if at <= now_ms =>
                Some(now_ms.saturating_sub(at) as f64 / 1000.),
            // An empty mailbox is not an age-zero sample; neither is a lagging cut.
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Memory {
    pub host_available_bytes: Option<u64>,
    pub private_bytes: Option<u64>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Coverage { Complete, Partial, Unavailable }
impl Coverage {
    pub const fn label(self) -> &'static str {
        match self { Self::Complete => "complete", Self::Partial => "partial", Self::Unavailable => "unavailable" }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoryDelta {
    pub host_available_delta_bytes: Option<i64>,
    pub private_reclaimed_bytes: Option<i64>,
}
impl MemoryDelta {
    /// Pair the same host/process set around one operation; a missing process or
    /// unreadable endpoint makes that measure absent, not partial reclamation.
    pub fn between(before: Memory, after: Memory) -> Self {
        fn difference(a: Option<u64>, b: Option<u64>) -> Option<i64> {
            i64::try_from(i128::from(a?) - i128::from(b?)).ok()
        }
        Self {
            host_available_delta_bytes: difference(after.host_available_bytes, before.host_available_bytes),
            private_reclaimed_bytes: difference(before.private_bytes, after.private_bytes),
        }
    }
    pub fn coverage(self) -> Coverage {
        match (self.host_available_delta_bytes, self.private_reclaimed_bytes) {
            (Some(_), Some(_)) => Coverage::Complete,
            (None, None) => Coverage::Unavailable,
            _ => Coverage::Partial,
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct HostSample {
    pub held_silent_age_seconds: Option<f64>,
    pub asleep_seats: u64,
    pub memory: Memory,
}

/// Disabled means no sampling and no filesystem I/O. The caller owns the five
/// minute enabled-member schedule and the authoritative asleep-seat count.
pub fn sample(enabled: bool, now_ms: i64, held: HeldSilent, asleep_seats: u64, pids: &[u32]) -> Option<HostSample> {
    enabled.then(|| HostSample {
        held_silent_age_seconds: held.age_seconds(now_ms),
        asleep_seats,
        memory: memory(pids),
    })
}
pub fn memory(pids: &[u32]) -> Memory {
    if cfg!(target_os = "linux") { read_memory(Path::new("/proc"), pids) }
    else { Memory::default() }
}
fn kib_field(text: &str, key: &str) -> Option<u64> {
    let mut lines = text.lines().filter_map(|line| line.split_once(':'))
        .filter(|(name, _)| *name == key);
    let (_, value) = lines.next()?;
    if lines.next().is_some() { return None; }
    let mut parts = value.split_whitespace();
    let value = parts.next()?.parse::<u64>().ok()?;
    if parts.next()? != "kB" || parts.next().is_some() { return None; }
    value.checked_mul(1024)
}
fn private_bytes(text: &str) -> Option<u64> {
    kib_field(text, "Private_Clean")?.checked_add(kib_field(text, "Private_Dirty")?)?
        .checked_add(kib_field(text, "Private_Hugetlb")?)
}
fn read_memory(proc: &Path, pids: &[u32]) -> Memory {
    let host_available_bytes = fs::read_to_string(proc.join("meminfo")).ok()
        .and_then(|text| kib_field(&text, "MemAvailable"));
    // No observed process set is unknown, not zero private bytes. Duplicate PIDs
    // would double-count; reject that input instead of reporting a false benefit.
    let private_bytes = (!pids.is_empty() && !pids.iter().enumerate().any(|(i, pid)| pids[..i].contains(pid)))
        .then(|| pids.iter().try_fold(0u64, |total, pid| {
            total.checked_add(private_bytes(&fs::read_to_string(proc.join(pid.to_string()).join("smaps_rollup")).ok()?)?)
        })).flatten();
    Memory { host_available_bytes, private_bytes }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unreadable_and_unsupported_measurements_are_absent() {
        assert_eq!(read_memory(Path::new("/nonexistent-residency-sensor"), &[1]), Memory::default());
        if !cfg!(target_os = "linux") { assert_eq!(memory(&[1]), Memory::default()); }
        assert!(sample(false, 10, HeldSilent::Unavailable, 2, &[1]).is_none());
    }
    #[test]
    fn parses_private_not_resident_bytes_and_rejects_bad_coverage() {
        let text = "Rss: 10000 kB\nPrivate_Clean: 2 kB\nPrivate_Dirty: 3 kB\nPrivate_Hugetlb: 4 kB\n";
        assert_eq!(private_bytes(text), Some(9 * 1024));
        assert_eq!(private_bytes("Private_Clean: 0 kB\n"), None);
        for bad in ["MemAvailable: 1 B", "MemAvailable: -1 kB", "MemAvailable: 1 kB\nMemAvailable: 2 kB"] {
            assert_eq!(kib_field(bad, "MemAvailable"), None);
        }
    }
    #[test]
    fn age_requires_caught_up_projection_and_a_real_timestamp() {
        assert_eq!(HeldSilent::Unavailable.age_seconds(10), None);
        assert_eq!(HeldSilent::CaughtUp { oldest_accepted_ms: None }.age_seconds(10), None);
        assert_eq!(HeldSilent::CaughtUp { oldest_accepted_ms: Some(2000) }.age_seconds(1000), None);
        assert_eq!(HeldSilent::CaughtUp { oldest_accepted_ms: Some(1000) }.age_seconds(2500), Some(1.5));
    }
    #[test]
    fn memory_deltas_require_both_endpoints() {
        let before = Memory { host_available_bytes: Some(100), private_bytes: Some(80) };
        let after = Memory { host_available_bytes: Some(150), private_bytes: Some(20) };
        let delta = MemoryDelta::between(before, after);
        assert_eq!(delta.host_available_delta_bytes, Some(50));
        assert_eq!(delta.private_reclaimed_bytes, Some(60));
        assert_eq!(delta.coverage(), Coverage::Complete);
        assert_eq!(MemoryDelta::between(before, Memory::default()).coverage(), Coverage::Unavailable);
        assert_eq!(MemoryDelta::between(before, Memory { private_bytes: None, ..after }).coverage(), Coverage::Partial);
    }
}
