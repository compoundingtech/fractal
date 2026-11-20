//! What this host is, and what it can give its seats' lanes (docs/st3/seat-lanes.md).
use crate::lane::{Capacity, GIB};

/// Finite work shares the native lane accountant; this policy adds mount/IO fit limits.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct FinitePolicy {
    pub disk_floor_bytes: u64,
    /// Paths identify mounts; requests on the same filesystem use its configured floor.
    pub disk_floors: std::collections::BTreeMap<std::path::PathBuf, u64>,
    /// None permits unrestricted heavy IO; constrained hosts default to one slot.
    pub heavy_io_slots: Option<u32>,
    /// The execution host owns declared seeds; callers select keys, not known-key bytes.
    pub profiles: std::collections::BTreeMap<String, crate::ResourceProfileSeed>,
}

impl Default for FinitePolicy {
    fn default() -> Self {
        Self {
            disk_floor_bytes: 2 << 30,
            disk_floors: std::collections::BTreeMap::new(),
            heavy_io_slots: Some(1),
            profiles: std::collections::BTreeMap::new(),
        }
    }
}

/// The host's online CPUs and installed memory: the machine, not the caller's own quota, which a
/// lane or a cgroup limit would shrink.
pub fn size() -> Option<Capacity> {
    // SAFETY: sysconf reads a system value and has no preconditions.
    let cores = unsafe { libc::sysconf(libc::_SC_NPROCESSORS_ONLN) };
    let cores = u32::try_from(cores).ok().filter(|cores| *cores > 0)?;
    Some(Capacity { cores, memory: installed_memory_bytes()? })
}

/// One host memory observation, including whether pressure withheld eligible ARC credit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HostMemoryObservation {
    pub outside_bytes: u64,
    pub psi_blocked: bool,
}

/// Host memory unavailable to lanes, excluding reclaimable ZFS ARC and live lane usage.
/// `app-lanes.slice` has hierarchical counters for both finite members and laned seats: read it
/// once, never add its children again. Admission charges their declared capacities separately.
/// Missing or malformed ARC/pressure data gives no ARC credit; unavailable required host data
/// returns `None`.
pub fn outside_lane_memory() -> Option<HostMemoryObservation> {
    let meminfo = std::fs::read_to_string("/proc/meminfo").ok()?;
    let arcstats = std::fs::read_to_string("/proc/spl/kstat/zfs/arcstats").ok();
    let pressure = std::fs::read_to_string("/proc/pressure/memory").ok();
    let lane_stat = optional_stat(
        &crate::lane::app_slice_cgroup()?.join(crate::lane::LANES_SLICE).join("memory.stat"),
    )?;
    outside_memory(&meminfo, arcstats.as_deref(), lane_stat.as_deref(), pressure.as_deref())
}

/// ARC credit applied by the same native outside-memory formula, for read-only diagnostics.
pub fn arc_credit_bytes() -> Option<u64> {
    let meminfo = std::fs::read_to_string("/proc/meminfo").ok()?;
    let arcstats = optional_stat(std::path::Path::new("/proc/spl/kstat/zfs/arcstats"))?;
    let pressure = std::fs::read_to_string("/proc/pressure/memory").ok();
    let without_arc = outside_memory(&meminfo, None, None, pressure.as_deref())?;
    let with_arc = outside_memory(&meminfo, arcstats.as_deref(), None, pressure.as_deref())?;
    Some(without_arc.outside_bytes.saturating_sub(with_arc.outside_bytes))
}

fn optional_stat(path: &std::path::Path) -> Option<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(stat) => Some(Some(stat)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Some(None),
        Err(_) => None,
    }
}

/// Parse one host observation. Only ARC above its minimum and reported evictable is eligible
/// for credit, and only while memory PSI `full avg60` is below one percent.
pub fn outside_memory(
    meminfo: &str,
    arcstats: Option<&str>,
    lane_stat: Option<&str>,
    pressure: Option<&str>,
) -> Option<HostMemoryObservation> {
    let total = meminfo_bytes(meminfo, "MemTotal:")?;
    let available = meminfo_bytes(meminfo, "MemAvailable:")?;
    let eligible = arcstats.and_then(arc_reclaimable).unwrap_or(0);
    let psi_blocked = eligible > 0 && !pressure.and_then(full_avg60).is_some_and(|avg| avg < 1.0);
    let reclaimable = if psi_blocked { 0 } else { eligible };
    let lane_memory = lane_stat.map_or(0, unreclaimable);
    Some(HostMemoryObservation {
        outside_bytes: total
            .saturating_sub(available)
            .saturating_sub(reclaimable)
            .saturating_sub(lane_memory),
        psi_blocked,
    })
}

/// ARC kstat counters are bytes, unlike `/proc/meminfo`'s KiB fields.
fn arc_reclaimable(stat: &str) -> Option<u64> {
    let mut counters = [None; 6];
    for line in stat.lines() {
        let mut fields = line.split_whitespace();
        let index = match fields.next() {
            Some("size") => 0,
            Some("c_min") => 1,
            Some("mru_evictable_data") => 2,
            Some("mru_evictable_metadata") => 3,
            Some("mfu_evictable_data") => 4,
            Some("mfu_evictable_metadata") => 5,
            _ => continue,
        };
        if counters[index].is_some() || fields.next()? != "4" {
            return None;
        }
        counters[index] = Some(fields.next()?.parse::<u64>().ok()?);
        if fields.next().is_some() {
            return None;
        }
    }
    let [size, minimum, mru_data, mru_metadata, mfu_data, mfu_metadata] = counters;
    let evictable =
        mru_data?.saturating_add(mru_metadata?).saturating_add(mfu_data?).saturating_add(mfu_metadata?);
    Some(size?.saturating_sub(minimum?).min(evictable))
}

fn full_avg60(pressure: &str) -> Option<f64> {
    let mut fields = pressure
        .lines()
        .find(|line| line.split_whitespace().next() == Some("full"))?
        .split_whitespace();
    fields.next()?;
    let mut avg60 = None;
    for key in ["avg10=", "avg60=", "avg300="] {
        let value = fields.next()?.strip_prefix(key)?.parse::<f64>().ok()?;
        if !value.is_finite() || value < 0.0 {
            return None;
        }
        if key == "avg60=" {
            avg60 = Some(value);
        }
    }
    fields.next()?.strip_prefix("total=")?.parse::<u64>().ok()?;
    if fields.next().is_some() {
        return None;
    }
    avg60
}

/// `anon` plus `kernel` from a cgroup's `memory.stat`.
pub(crate) fn unreclaimable(stat: &str) -> u64 {
    stat.lines()
        .filter_map(|line| line.split_once(' '))
        .filter(|(key, _)| matches!(*key, "anon" | "kernel"))
        .filter_map(|(_, value)| value.trim().parse::<u64>().ok())
        .sum()
}

/// What a host of `size` can give its lanes: the same formula on every host, less a reserve for the
/// daemon and the system, `max(2, cores/8)` cores and `max(4G, memory/8)`.
pub fn for_lanes(size: Capacity) -> Capacity {
    Capacity {
        cores: size.cores.saturating_sub((size.cores / 8).max(2)),
        memory: size.memory.saturating_sub((size.memory / 8).max(4 * GIB)),
    }
}

#[cfg(target_os = "linux")]
pub fn installed_memory_bytes() -> Option<u64> {
    meminfo_bytes(&std::fs::read_to_string("/proc/meminfo").ok()?, "MemTotal:")
}

fn meminfo_bytes(meminfo: &str, key: &str) -> Option<u64> {
    let mut fields = meminfo
        .lines()
        .find(|line| line.split_whitespace().next() == Some(key))?
        .split_whitespace();
    if fields.next()? != key {
        return None;
    }
    let kib: u64 = fields.next()?.parse().ok()?;
    if fields.next()? != "kB" || fields.next().is_some() {
        return None;
    }
    kib.checked_mul(1024).filter(|bytes| key != "MemTotal:" || *bytes > 0)
}

#[cfg(target_os = "macos")]
pub fn installed_memory_bytes() -> Option<u64> {
    let output = std::process::Command::new("/usr/sbin/sysctl")
        .args(["-n", "hw.memsize"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    std::str::from_utf8(&output.stdout)
        .ok()?
        .trim()
        .parse()
        .ok()
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn installed_memory_bytes() -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOW_PRESSURE: &str =
        "some avg10=4.00 avg60=3.00 avg300=2.00 total=100\n\
         full avg10=2.00 avg60=0.99 avg300=1.50 total=50\n";
    const HIGH_PRESSURE: &str = "full avg10=0.00 avg60=2.00 avg300=0.00 total=50\n";
    const ARC: &str = "size 4 1000\nc_min 4 100\n\
                       mru_evictable_data 4 100\nmru_evictable_metadata 4 20\n\
                       mfu_evictable_data 4 200\nmfu_evictable_metadata 4 30\n";

    fn observation(outside_bytes: u64, psi_blocked: bool) -> Option<HostMemoryObservation> {
        Some(HostMemoryObservation { outside_bytes, psi_blocked })
    }

    #[test]
    fn lane_memory_counts_only_anon_and_kernel_once() {
        // The parent counter already includes finite member and seat descendants.
        let parent = "anon 100\nfile 5000\nkernel 20\nkernel_stack 7\nshmem 3\n";
        assert_eq!(unreclaimable(parent), 120);
        assert_eq!(
            outside_memory("MemTotal: 1024 kB\nMemAvailable: 512 kB\n", None, Some(parent), None),
            observation(512 * 1024 - 120, false)
        );
    }

    #[test]
    fn fictional_host_only_fits_fifteen_gib_with_bounded_arc_credit() {
        // 125 GiB host; about 14.6 GiB available, 34.7 GiB ARC, 3.9 GiB minimum;
        // fictional 20 GiB evictable and 8 GiB live lanes. Only bounded credit fits another 15 GiB.
        let meminfo = "MemTotal: 131072000 kB\nMemAvailable: 15309210 kB\n";
        let arcstats = "13 1 0x01 96 26112 123 456\nname type data\n\
                        size 4 37258870784\nc_min 4 4187593114\n\
                        mru_evictable_data 4 5368709120\nmru_evictable_metadata 4 5368709120\n\
                        mfu_evictable_data 4 5368709120\nmfu_evictable_metadata 4 5368709120\n";
        let lanes = "anon 7516192768\nfile 90000000000\nkernel 1073741824\n";
        let raw = outside_memory(meminfo, Some(arcstats), Some(lanes), Some(HIGH_PRESSURE)).unwrap();
        let credited =
            outside_memory(meminfo, Some(arcstats), Some(lanes), Some(LOW_PRESSURE)).unwrap();
        assert_eq!(raw, HostMemoryObservation { outside_bytes: 109_951_162_368, psi_blocked: true });
        assert_eq!(
            credited,
            HostMemoryObservation { outside_bytes: 88_476_325_888, psi_blocked: false }
        );
        let budget = for_lanes(Capacity { cores: 32, memory: 125 * GIB }).memory;
        assert!(raw.outside_bytes + 8 * GIB + 15 * GIB > budget);
        assert!(credited.outside_bytes + 8 * GIB + 15 * GIB <= budget);
    }

    #[test]
    fn arc_credit_is_bounded_by_evictable_bytes_and_bytes_above_minimum() {
        let meminfo = "MemTotal: 1024 kB\nMemAvailable: 256 kB\n";
        assert_eq!(
            outside_memory(meminfo, Some(ARC), None, Some(LOW_PRESSURE)),
            observation(768 * 1024 - 350, false)
        );
        let minimum_bound = ARC.replace("c_min 4 100", "c_min 4 900");
        assert_eq!(
            outside_memory(meminfo, Some(&minimum_bound), None, Some(LOW_PRESSURE)),
            observation(768 * 1024 - 100, false)
        );
        let overflowing_sum = "size 4 1000\nc_min 4 100\n\
                               mru_evictable_data 4 18446744073709551615\n\
                               mru_evictable_metadata 4 1\nmfu_evictable_data 4 1\n\
                               mfu_evictable_metadata 4 1\n";
        assert_eq!(
            outside_memory(meminfo, Some(overflowing_sum), None, Some(LOW_PRESSURE)),
            observation(768 * 1024 - 900, false)
        );
    }

    #[test]
    fn psi_withholds_credit_at_one_percent_or_when_unavailable() {
        let meminfo = "MemTotal: 1024 kB\nMemAvailable: 256 kB\n";
        for pressure in [
            None,
            Some(HIGH_PRESSURE),
            Some("full avg10=0.00 avg60=1.00 avg300=0.00 total=0\n"),
            Some("some avg10=0.00 avg60=0.00 avg300=0.00 total=0\n"),
            Some("full avg10=0.00 avg300=0.00 total=0\n"),
            Some("full avg10=0.00 avg60=x avg300=0.00 total=0\n"),
            Some("full avg10=0.00 avg60=NaN avg300=0.00 total=0\n"),
            Some("full avg10=0.00 avg60=inf avg300=0.00 total=0\n"),
            Some("full avg10=0.00 avg60=-0.01 avg300=0.00 total=0\n"),
            Some("full avg10=0.00 avg60=0.00 avg300=0.00 total=x\n"),
        ] {
            assert_eq!(
                outside_memory(meminfo, Some(ARC), None, pressure),
                observation(768 * 1024, true),
                "{pressure:?}"
            );
        }
        assert_eq!(
            outside_memory(meminfo, Some(ARC), None, Some(LOW_PRESSURE)),
            observation(768 * 1024 - 350, false)
        );
    }

    #[test]
    fn absent_or_malformed_arc_never_makes_host_usage_unknown_or_psi_blocked() {
        let meminfo = "MemTotal: 1024 kB\nMemAvailable: 256 kB\n";
        assert_eq!(outside_memory(meminfo, None, None, None), observation(768 * 1024, false));
        assert_eq!(
            outside_memory(meminfo, None, Some("anon 100\nkernel 20\n"), None),
            observation(768 * 1024 - 120, false)
        );
        for key in [
            "size",
            "c_min",
            "mru_evictable_data",
            "mru_evictable_metadata",
            "mfu_evictable_data",
            "mfu_evictable_metadata",
        ] {
            let missing = ARC
                .lines()
                .filter(|line| line.split_whitespace().next() != Some(key))
                .collect::<Vec<_>>()
                .join("\n");
            let malformed = ARC.replace(&format!("{key} 4 "), &format!("{key} 4 x"));
            let wrong_type = ARC.replace(&format!("{key} 4 "), &format!("{key} 3 "));
            for stat in [&missing, &malformed, &wrong_type] {
                for pressure in [None, Some(LOW_PRESSURE), Some(HIGH_PRESSURE)] {
                    assert_eq!(
                        outside_memory(meminfo, Some(stat), None, pressure),
                        observation(768 * 1024, false),
                        "{stat}"
                    );
                }
            }
        }
        for stat in [format!("{ARC}size 4 1000\n"), ARC.replace("size 4 1000", "size 4 1000 extra")] {
            assert_eq!(
                outside_memory(meminfo, Some(&stat), None, Some(LOW_PRESSURE)),
                observation(768 * 1024, false)
            );
        }
    }

    #[test]
    fn arc_below_its_minimum_is_not_reclaimable_and_outside_never_underflows() {
        let meminfo = "MemTotal: 1024 kB\nMemAvailable: 256 kB\n";
        let arcstats = ARC.replace("c_min 4 100", "c_min 4 2000");
        assert_eq!(
            outside_memory(meminfo, Some(&arcstats), None, None),
            observation(768 * 1024, false)
        );
        assert_eq!(
            outside_memory(meminfo, None, Some("anon 1048576\nkernel 1\n"), None),
            observation(0, false)
        );
        assert_eq!(
            outside_memory(
                "MemTotal: 1 kB\nMemAvailable: 1 kB\n", Some(ARC), None, Some(LOW_PRESSURE)
            ),
            observation(0, false)
        );
        assert_eq!(
            outside_memory("MemTotal: 1 kB\nMemAvailable: 2 kB\n", None, None, None),
            observation(0, false)
        );
        let no_evictable = ARC.replace(" 4 100\n", " 4 0\n")
            .replace(" 4 20\n", " 4 0\n")
            .replace(" 4 200\n", " 4 0\n")
            .replace(" 4 30\n", " 4 0\n");
        assert_eq!(
            outside_memory(meminfo, Some(&no_evictable), None, Some(HIGH_PRESSURE)),
            observation(768 * 1024, false)
        );
    }

    #[test]
    fn missing_or_malformed_required_host_fields_are_not_zero_usage() {
        for meminfo in [
            "MemTotal: 1024 kB\n",
            "MemAvailable: 512 kB\n",
            "MemTotal: 1024 kB\nMemAvailable: x kB\n",
            "MemTotal: 1024 kB\nMemAvailable: 512 MB\n",
        ] {
            assert_eq!(outside_memory(meminfo, None, None, None), None, "{meminfo}");
        }
        assert_eq!(
            outside_memory("MemTotal: 1 kB\nMemAvailable: 0 kB\n", None, None, None),
            observation(1024, false)
        );
    }

    #[test]
    fn every_host_keeps_the_same_reserve_for_itself() {
        let lanes = |cores, memory: u64| for_lanes(Capacity { cores, memory: memory * GIB });
        assert_eq!(lanes(16, 64), Capacity { cores: 14, memory: 56 * GIB });
        assert_eq!(lanes(32, 96), Capacity { cores: 28, memory: 84 * GIB });
        assert_eq!(lanes(8, 16), Capacity { cores: 6, memory: 12 * GIB });
        assert_eq!(lanes(2, 2), Capacity { cores: 0, memory: 0 });
        let here = size().expect("a test host has a size");
        assert!(here.cores >= 1 && here.memory >= 256 << 20);
    }

    #[test]
    fn parses_installed_memory_without_using_free_memory_or_accepting_bad_units() {
        assert_eq!(
            meminfo_bytes("MemFree: 5 kB\nMemTotal: 1048576 kB\n", "MemTotal:"),
            Some(1 << 30)
        );
        for input in [
            "MemFree: 1048576 kB",
            "MemTotal: x kB",
            "MemTotal: 0 kB",
            "MemTotal: 1024 MB",
            "MemTotal: 18446744073709551615 kB",
            "MemTotal: 1 kB extra",
        ] {
            assert_eq!(meminfo_bytes(input, "MemTotal:"), None, "{input}");
        }
    }
}
