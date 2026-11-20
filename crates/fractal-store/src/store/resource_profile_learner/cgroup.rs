//! Read existing execution cgroups before retirement removes their counters. This records
//! physical observations, not an assertion that daemon/shared-server work is contained.
use std::path::Path;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use super::MemoryEvents;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IoDeviceStat {
    pub major: u32,
    pub minor: u32,
    pub read_bytes: u64,
    pub write_bytes: u64,
    pub read_operations: u64,
    pub write_operations: u64,
    pub discard_bytes: Option<u64>,
    pub discard_operations: Option<u64>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Observation {
    pub memory_peak_bytes: Option<u64>,
    pub memory_events: Option<MemoryEvents>,
    /// None: no delegated IO accounting; Some(empty): accounting exists with no IO yet.
    pub io_stat: Option<Vec<IoDeviceStat>>,
    /// Sample again after observer exit and before any destructive retirement. None cannot
    /// prove an untruncated execution, even when a later retirement proves tree absence.
    pub populated: Option<bool>,
}

fn optional_file(path: &Path) -> Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("reading cgroup counter {}", path.display())),
    }
}

pub fn read(cgroup: &Path) -> Result<Observation> {
    let populated_before = optional_file(&cgroup.join("cgroup.events"))?
        .map(|value| parse_populated(&value)).transpose()?;
    let memory_peak_bytes = optional_file(&cgroup.join("memory.peak"))?
        .map(|value| value.trim().parse()).transpose()?;
    let memory_events = optional_file(&cgroup.join("memory.events"))?
        .map(|value| parse_memory_events(&value)).transpose()?;
    let io_stat = optional_file(&cgroup.join("io.stat"))?
        .map(|value| parse_io_stat(&value)).transpose()?;
    let populated_after = optional_file(&cgroup.join("cgroup.events"))?
        .map(|value| parse_populated(&value)).transpose()?;
    let populated = stable_population(populated_before, populated_after);
    Ok(Observation { memory_peak_bytes, memory_events, io_stat, populated })
}

// Reading "empty" only after the counters could miss a final peak written during the snapshot.
// Unknown on either side cannot certify that every counter was observed after natural quiescence.
fn stable_population(before: Option<bool>, after: Option<bool>) -> Option<bool> {
    match (before, after) {
        (Some(false), Some(false)) => Some(false),
        (Some(true), _) | (_, Some(true)) => Some(true),
        _ => None,
    }
}

pub fn parse_populated(value: &str) -> Result<bool> {
    let mut populated = None;
    for line in value.lines() {
        let mut fields = line.split_ascii_whitespace();
        if fields.next() != Some("populated") { continue; }
        ensure!(populated.is_none(), "duplicate cgroup populated counter");
        populated = Some(match fields.next() {
            Some("0") => false,
            Some("1") => true,
            _ => anyhow::bail!("invalid cgroup populated counter"),
        });
        ensure!(fields.next().is_none(), "malformed cgroup populated counter");
    }
    populated.context("missing cgroup populated counter")
}

pub fn parse_memory_events(value: &str) -> Result<MemoryEvents> {
    let mut counts = [None; 5];
    for line in value.lines() {
        let mut fields = line.split_ascii_whitespace();
        let slot = match fields.next() {
            Some("high") => 0, Some("max") => 1, Some("oom") => 2,
            Some("oom_kill") => 3, Some("oom_group_kill") => 4,
            _ => continue,
        };
        ensure!(counts[slot].is_none(), "duplicate memory event counter");
        counts[slot] = Some(fields.next().context("missing memory event count")?.parse::<u64>()?);
        ensure!(fields.next().is_none(), "malformed memory event counter");
    }
    Ok(MemoryEvents {
        high: counts[0].context("missing high event count")?,
        max: counts[1].context("missing max event count")?,
        oom: counts[2].context("missing oom event count")?,
        oom_kill: counts[3].context("missing oom_kill event count")?,
        // Older kernels do not expose the group-kill extension; the required oom_kill
        // counter still accounts for the kills. Absence here does not synthesize no OOM.
        oom_group_kill: counts[4].unwrap_or(0),
    })
}

pub fn parse_io_stat(value: &str) -> Result<Vec<IoDeviceStat>> {
    let mut devices = Vec::new();
    for line in value.lines().filter(|line| !line.trim().is_empty()) {
        let mut fields = line.split_ascii_whitespace();
        let (major, minor) = fields.next().context("missing IO device")?
            .split_once(':').context("invalid IO device number")?;
        let major = major.parse::<u32>()?;
        let minor = minor.parse::<u32>()?;
        ensure!(!devices.iter().any(|device: &IoDeviceStat|
            device.major == major && device.minor == minor), "duplicate IO device");
        let mut counts = [None; 6];
        for field in fields {
            let (name, count) = field.split_once('=').context("malformed IO counter")?;
            let slot = match name {
                "rbytes" => 0, "wbytes" => 1, "rios" => 2, "wios" => 3,
                "dbytes" => 4, "dios" => 5, _ => continue,
            };
            ensure!(counts[slot].is_none(), "duplicate IO counter");
            counts[slot] = Some(count.parse::<u64>()?);
        }
        devices.push(IoDeviceStat {
            major, minor,
            read_bytes: counts[0].context("missing IO read bytes")?,
            write_bytes: counts[1].context("missing IO write bytes")?,
            read_operations: counts[2].context("missing IO read operations")?,
            write_operations: counts[3].context("missing IO write operations")?,
            discard_bytes: counts[4], discard_operations: counts[5],
        });
    }
    Ok(devices)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retained_custody_oom_evidence_is_not_lost() {
        let events = parse_memory_events("low 0\nhigh 0\nmax 49\noom 1\noom_kill 1\noom_group_kill 0\n").unwrap();
        assert_eq!((events.max, events.oom, events.oom_kill), (49, 1, 1));
    }

    #[test]
    fn device_counters_preserve_byte_traffic_not_allocated_growth() {
        let input = "8:0 rbytes=11624448 wbytes=95952896 rios=2838 wios=23426 dbytes=0 dios=0\n8:16 rbytes=33652553842688 wbytes=3539582767104 rios=762369947 wios=254211128 dbytes=0 dios=0\n";
        let devices = parse_io_stat(input).unwrap();
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[1].write_bytes, 3539582767104);
        assert_eq!(devices[1].minor, 16);
        assert!(parse_io_stat("").unwrap().is_empty());
    }

    #[test]
    fn missing_or_duplicate_required_fields_are_not_zero_counters() {
        assert!(parse_memory_events("high 0\nmax 0\noom 0\n").is_err());
        assert!(parse_memory_events("high 0\nhigh 0\nmax 0\noom 0\noom_kill 0\n").is_err());
        assert!(parse_io_stat("8:0 wbytes=0 rios=0 wios=0").is_err());
        assert!(parse_io_stat("8:0 rbytes=0 wbytes=0 wbytes=0 rios=0 wios=0").is_err());
    }

    #[test]
    fn optional_missing_io_is_distinct_from_an_empty_device_list() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("memory.peak"), "268435456\n").unwrap();
        std::fs::write(directory.path().join("memory.events"),
            "high 0\nmax 49\noom 1\noom_kill 1\noom_group_kill 0\n").unwrap();
        let missing = read(directory.path()).unwrap();
        assert_eq!(missing.memory_peak_bytes, Some(268435456));
        assert!(missing.io_stat.is_none());
        std::fs::write(directory.path().join("io.stat"), "").unwrap();
        assert_eq!(read(directory.path()).unwrap().io_stat, Some(vec![]));
    }

    #[test]
    fn population_proof_distinguishes_empty_live_and_missing_counters() {
        assert!(!parse_populated("populated 0\nfrozen 0\n").unwrap());
        assert!(parse_populated("populated 1\nfrozen 0\n").unwrap());
        for value in ["frozen 0\n", "populated 2\n", "populated 0\npopulated 1\n"] {
            assert!(parse_populated(value).is_err());
        }
    }

    #[test]
    fn quiescence_must_bracket_every_counter_read() {
        assert_eq!(stable_population(Some(false), Some(false)), Some(false));
        for (before, after) in [
            (Some(true), Some(false)), (Some(false), Some(true)),
            (Some(true), Some(true)), (Some(true), None), (None, Some(true)),
        ] {
            assert_eq!(stable_population(before, after), Some(true));
        }
        for (before, after) in [(None, None), (None, Some(false)), (Some(false), None)] {
            assert_eq!(stable_population(before, after), None);
        }
    }
}
