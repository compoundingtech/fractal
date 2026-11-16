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
}

impl Default for FinitePolicy {
    fn default() -> Self {
        Self {
            disk_floor_bytes: 2 << 30,
            disk_floors: std::collections::BTreeMap::new(),
            heavy_io_slots: Some(1),
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

/// Host memory unavailable to lanes, excluding reclaimable ZFS ARC and live lane usage.
/// `app-lanes.slice` has hierarchical counters for both finite members and laned seats: read it
/// once, never add its children again. Admission charges their declared capacities separately.
/// An absent ARC or lane slice contributes zero; unreadable or malformed host data returns `None`.
pub fn outside_lane_memory() -> Option<u64> {
    let meminfo = std::fs::read_to_string("/proc/meminfo").ok()?;
    let arcstats = optional_stat(std::path::Path::new("/proc/spl/kstat/zfs/arcstats"))?;
    let lane_stat = optional_stat(
        &crate::lane::app_slice_cgroup()?.join(crate::lane::LANES_SLICE).join("memory.stat"),
    )?;
    outside_memory(&meminfo, arcstats.as_deref(), lane_stat.as_deref())
}

/// ARC credit applied by the same native outside-memory formula, for read-only diagnostics.
pub fn arc_credit_bytes() -> Option<u64> {
    let meminfo = std::fs::read_to_string("/proc/meminfo").ok()?;
    let arcstats = optional_stat(std::path::Path::new("/proc/spl/kstat/zfs/arcstats"))?;
    let without_arc = outside_memory(&meminfo, None, None)?;
    let with_arc = outside_memory(&meminfo, arcstats.as_deref(), None)?;
    Some(without_arc.saturating_sub(with_arc))
}

fn optional_stat(path: &std::path::Path) -> Option<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(stat) => Some(Some(stat)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Some(None),
        Err(_) => None,
    }
}

/// Parse one host observation. Optional ARC and hierarchical lane counters are absent on hosts
/// without ZFS or live lanes; required malformed host counters make the observation unavailable.
pub fn outside_memory(meminfo: &str, arcstats: Option<&str>, lane_stat: Option<&str>) -> Option<u64> {
    let total = meminfo_bytes(meminfo, "MemTotal:")?;
    let available = meminfo_bytes(meminfo, "MemAvailable:")?;
    let reclaimable = match arcstats {
        Some(stat) => arc_reclaimable(stat)?,
        None => 0,
    };
    let lane_memory = lane_stat.map_or(0, unreclaimable);
    Some(total.saturating_sub(available).saturating_sub(reclaimable).saturating_sub(lane_memory))
}

/// ARC's `size` and `c_min` are bytes, unlike `/proc/meminfo`'s KiB fields.
fn arc_reclaimable(stat: &str) -> Option<u64> {
    let mut size = None;
    let mut minimum = None;
    for line in stat.lines() {
        let mut fields = line.split_whitespace();
        let field = match fields.next() {
            Some("size") => &mut size,
            Some("c_min") => &mut minimum,
            _ => continue,
        };
        if fields.next()? != "4" {
            return None;
        }
        *field = Some(fields.next()?.parse::<u64>().ok()?);
        if fields.next().is_some() {
            return None;
        }
    }
    Some(size?.saturating_sub(minimum?))
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

    #[test]
    fn lane_memory_counts_only_anon_and_kernel_once() {
        // The parent counter already includes finite member and seat descendants.
        let parent = "anon 100\nfile 5000\nkernel 20\nkernel_stack 7\nshmem 3\n";
        assert_eq!(unreclaimable(parent), 120);
        assert_eq!(
            outside_memory("MemTotal: 1024 kB\nMemAvailable: 512 kB\n", None, Some(parent)),
            Some(512 * 1024 - 120)
        );
    }

    #[test]
    fn fictional_host_excludes_available_memory_arc_above_minimum_and_live_lanes() {
        // Approximately 125 GiB total, 14.6 GiB available, 34.7 GiB ARC and 3.9 GiB c_min.
        let meminfo = "MemTotal: 131072000 kB\nMemAvailable: 15309210 kB\n";
        let arcstats = "13 1 0x01 96 26112 123 456\nname type data\n\
                        size 4 37258870784\nc_min 4 4187593114\n";
        let lanes = "anon 7516192768\nfile 90000000000\nkernel 1073741824\n";
        assert_eq!(arc_reclaimable(arcstats), Some(33_071_277_670));
        assert_eq!(
            outside_memory(meminfo, Some(arcstats), Some(lanes)),
            Some(125 * GIB - 15_309_210 * 1024 - 33_071_277_670 - 8 * GIB)
        );
    }

    #[test]
    fn hosts_without_zfs_or_active_lanes_still_account_for_all_other_memory() {
        let meminfo = "MemTotal: 1024 kB\nMemAvailable: 256 kB\n";
        assert_eq!(outside_memory(meminfo, None, None), Some(768 * 1024));
        assert_eq!(outside_memory(meminfo, None, Some("anon 100\nkernel 20\n")), Some(768 * 1024 - 120));
    }

    #[test]
    fn arc_below_its_minimum_is_not_reclaimable_and_outside_never_underflows() {
        let meminfo = "MemTotal: 1024 kB\nMemAvailable: 256 kB\n";
        let arcstats = "size 4 100\nc_min 4 200\n";
        assert_eq!(arc_reclaimable(arcstats), Some(0));
        assert_eq!(outside_memory(meminfo, Some(arcstats), None), Some(768 * 1024));
        assert_eq!(outside_memory(meminfo, None, Some("anon 1048576\nkernel 1\n")), Some(0));
        assert_eq!(outside_memory(meminfo, Some("size 4 1048576\nc_min 4 0\n"), None), Some(0));
        assert_eq!(outside_memory("MemTotal: 1 kB\nMemAvailable: 2 kB\n", None, None), Some(0));
    }

    #[test]
    fn missing_or_malformed_required_host_fields_are_not_zero_usage() {
        for meminfo in [
            "MemTotal: 1024 kB\n",
            "MemAvailable: 512 kB\n",
            "MemTotal: 1024 kB\nMemAvailable: x kB\n",
            "MemTotal: 1024 kB\nMemAvailable: 512 MB\n",
        ] {
            assert_eq!(outside_memory(meminfo, None, None), None, "{meminfo}");
        }
        for arcstats in [
            "size 4 100\n",
            "c_min 4 10\n",
            "size 4 x\nc_min 4 10\n",
            "size 4 100\nc_min 4 x\n",
            "size 3 100\nc_min 4 10\n",
            "size 4 100 extra\nc_min 4 10\n",
        ] {
            assert_eq!(arc_reclaimable(arcstats), None, "{arcstats}");
        }
        assert_eq!(outside_memory("MemTotal: 1 kB\nMemAvailable: 0 kB\n", None, None), Some(1024));
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
