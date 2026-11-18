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
}

fn optional_file(path: &Path) -> Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("reading cgroup counter {}", path.display())),
    }
}

pub fn read(cgroup: &Path) -> Result<Observation> {
    let memory_peak_bytes = optional_file(&cgroup.join("memory.peak"))?
        .map(|value| value.trim().parse()).transpose()?;
    let memory_events = optional_file(&cgroup.join("memory.events"))?
        .map(|value| parse_memory_events(&value)).transpose()?;
    let io_stat = optional_file(&cgroup.join("io.stat"))?
        .map(|value| parse_io_stat(&value)).transpose()?;
    Ok(Observation { memory_peak_bytes, memory_events, io_stat })
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
    fn dev4_io_devices_preserve_byte_traffic_not_allocated_growth() {
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
}
