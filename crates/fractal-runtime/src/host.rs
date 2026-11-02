//! What this host is, and what it can give its seats' lanes (docs/st3/seat-lanes.md).
use crate::lane::{Capacity, GIB};

/// The host's online CPUs and installed memory: the machine, not the caller's own quota, which a
/// lane or a cgroup limit would shrink.
pub fn size() -> Option<Capacity> {
    // SAFETY: sysconf reads a system value and has no preconditions.
    let cores = unsafe { libc::sysconf(libc::_SC_NPROCESSORS_ONLN) };
    let cores = u32::try_from(cores).ok().filter(|cores| *cores > 0)?;
    Some(Capacity { cores, memory: installed_memory_bytes()? })
}

/// What the seats outside lanes hold that the kernel cannot take back: the anonymous and kernel
/// memory (`memory.stat`) of each seat scope directly in the user manager's `app.slice`. Page cache
/// is left out, since it is reclaimed under pressure. A laned seat's scopes are in its lane, which
/// counts at its declared size instead. `None` where there is no cgroup tree to read.
pub fn unlaned_seat_memory() -> Option<u64> {
    Some(scopes_memory(&crate::lane::app_slice_cgroup()?))
}

fn scopes_memory(app_slice: &std::path::Path) -> u64 {
    std::fs::read_dir(app_slice)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.starts_with("st3-") && name.ends_with(".scope")
        })
        .filter_map(|entry| std::fs::read_to_string(entry.path().join("memory.stat")).ok())
        .map(|stat| unreclaimable(&stat))
        .sum()
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
    meminfo_bytes(&std::fs::read_to_string("/proc/meminfo").ok()?)
}

#[cfg(any(target_os = "linux", test))]
fn meminfo_bytes(meminfo: &str) -> Option<u64> {
    let mut fields = meminfo
        .lines()
        .find(|line| line.starts_with("MemTotal:"))?
        .split_whitespace();
    if fields.next()? != "MemTotal:" {
        return None;
    }
    let kib: u64 = fields.next()?.parse().ok()?;
    if fields.next()? != "kB" || fields.next().is_some() {
        return None;
    }
    kib.checked_mul(1024).filter(|bytes| *bytes > 0)
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
    fn only_what_seat_scopes_cannot_give_back_counts() {
        assert_eq!(unreclaimable("anon 100\nfile 5000\nkernel 20\nkernel_stack 7\nshmem 3\n"), 120);
        let app = std::env::temp_dir().join(format!("host-scopes-{}", std::process::id()));
        for (scope, stat) in [
            ("st3-fleet.worker-run-1-0.scope", "anon 1000\nfile 90000\nkernel 24\n"),
            ("st3-pty-server-fleet.worker-2.scope", "anon 300\nkernel 0\n"),
            ("app-git.scope", "anon 777777\n"),
            ("app-lanes.slice", "anon 888888\n"),
        ] {
            std::fs::create_dir_all(app.join(scope)).unwrap();
            std::fs::write(app.join(scope).join("memory.stat"), stat).unwrap();
        }
        assert_eq!(scopes_memory(&app), 1324);
        std::fs::remove_dir_all(&app).unwrap();
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
            meminfo_bytes("MemFree: 5 kB\nMemTotal: 1048576 kB\n"),
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
            assert_eq!(meminfo_bytes(input), None, "{input}");
        }
    }
}
