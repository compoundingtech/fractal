//! Inactive helpers: unit 7 must apply and verify these before its first process.
//! The caller supplies a dedicated cgroup directory; no launch path calls this module.
use fractal_jobs::{
    Covered,
    scope::{JobScopePlan, observation::*},
};
use std::{
    fs, io,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

/// Writes are not atomic: an error leaves a partially configured scope, never launchable.
pub fn apply(directory: &Path, plan: &JobScopePlan) -> io::Result<ScopeReadback> {
    for (file, value) in plan.files() {
        fs::write(directory.join(file), value.to_string())?;
    }
    Ok(readback(directory))
}
fn read<T>(directory: &Path, file: &str, parse: impl FnOnce(&str) -> Covered<T>) -> Covered<T> {
    match fs::read_to_string(directory.join(file)) {
        Ok(text) => parse(&text),
        Err(error) => unknown(format!("{file}: {error}")),
    }
}
pub fn readback(directory: &Path) -> ScopeReadback {
    ScopeReadback(
        ["memory.high", "memory.max", "memory.swap.max", "pids.max"]
            .map(|file| read(directory, file, parse_limit)),
    )
}
fn sample<T>(
    directory: &Path,
    host: &str,
    file: &str,
    parse: impl FnOnce(&str) -> Covered<T>,
) -> io::Result<ScopeSample<T>> {
    let reading = read(directory, file, parse);
    let unix_time_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_millis();
    Ok(ScopeSample {
        host: host.to_owned(),
        unix_time_ms,
        reading,
    })
}
/// Call at start and end for CPU deltas; these are raw counters, not interval totals.
/// Each individual read gets its own host timestamp, including unknown reads.
pub fn read_usage(directory: &Path, host: &str) -> io::Result<RawScopeUsage> {
    Ok(RawScopeUsage {
        cpu_usage_usec: sample(directory, host, "cpu.stat", |s| {
            parse_usage_number(s, Some("usage_usec"))
        })?,
        memory_peak: sample(directory, host, "memory.peak", |s| {
            parse_usage_number(s, None)
        })?,
        memory_events: sample(directory, host, "memory.events", parse_raw_usage)?,
        io_stat: sample(directory, host, "io.stat", parse_raw_usage)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use fractal_jobs::Coverage;

    fn plan() -> JobScopePlan {
        JobScopePlan::new(14 << 30, 16 << 30, 128, 14 << 30, 16 << 30).unwrap()
    }
    #[test]
    fn fixture_apply_match_mismatch_and_unknown() {
        let dir = tempfile::tempdir().unwrap();
        let plan = plan();
        let observed = apply(dir.path(), &plan).unwrap();
        assert!(
            observed
                .matches(&plan)
                .iter()
                .all(|v| v.value == Some(true))
        );
        fs::write(dir.path().join("memory.max"), "max").unwrap();
        let observed = readback(dir.path());
        assert_eq!(observed.0[1].value, Some(ScopeLimit::Unlimited));
        assert_eq!(observed.matches(&plan)[1].value, Some(false));
        fs::remove_file(dir.path().join("memory.high")).unwrap();
        fs::write(dir.path().join("pids.max"), "malformed").unwrap();
        let observed = readback(dir.path());
        for i in [0, 3] {
            assert_eq!(observed.0[i].value, None);
            assert!(matches!(observed.0[i].coverage, Coverage::Unknown { .. }));
            assert_eq!(observed.matches(&plan)[i].value, None);
        }
    }
    #[test]
    fn raw_usage_and_missing_reads() {
        let dir = tempfile::tempdir().unwrap();
        let missing = read_usage(dir.path(), "fixture-host").unwrap();
        assert_eq!(missing.cpu_usage_usec.reading.value, None);
        assert_eq!(missing.memory_peak.reading.value, None);
        assert_eq!(missing.memory_events.reading.value, None);
        assert_eq!(missing.io_stat.reading.value, None);
        for file in ["cpu.stat", "memory.peak", "memory.events", "io.stat"] {
            fs::write(
                dir.path().join(file),
                match file {
                    "cpu.stat" => "user_usec 3\nusage_usec 42\nsystem_usec 2\n",
                    "memory.peak" => "4096\n",
                    "memory.events" => "oom 0\noom_kill 0\n",
                    _ => "259:0 rbytes=10 wbytes=20 rios=1 wios=2\n",
                },
            )
            .unwrap();
        }
        let usage = read_usage(dir.path(), "fixture-host").unwrap();
        assert_eq!(usage.cpu_usage_usec.reading.value, Some(42));
        assert_eq!(usage.memory_peak.reading.value, Some(4096));
        assert_eq!(
            usage.memory_events.reading.value.as_deref(),
            Some("oom 0\noom_kill 0\n")
        );
        assert!(usage.io_stat.reading.value.unwrap().contains("rbytes=10"));
        assert_eq!(usage.cpu_usage_usec.host, "fixture-host");
        assert!(usage.cpu_usage_usec.unix_time_ms >= missing.cpu_usage_usec.unix_time_ms);
        assert!(matches!(
            missing.io_stat.reading.coverage,
            Coverage::Unknown { .. }
        ));
        assert_eq!(
            parse_usage_number("usage_usec bad", Some("usage_usec")).value,
            None
        );
    }
}
