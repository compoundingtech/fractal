//! Seat lanes: a CPU and memory cap around a seat's scope (docs/st3/seat-lanes.md).
//!
//! A seat that declares `capacity cores=N memory="SIZE"` starts in a scope whose cgroup holds
//! `CPUQuota` and `MemoryMax`, so the harness and everything it starts share the cap. Only on a Linux host whose config turns lanes on; elsewhere the declaration is logged as
//! not enforced and the seat starts as it does without one.

use std::collections::BTreeMap;
use std::hash::{Hash as _, Hasher as _};
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context as _, Result, bail};

/// The launch environment key that carries a seat's [`Capacity`] from the reconciler to the
/// spawn. The runtime removes it before the harness starts; the reconciler replaces any value a
/// declaration sets, so only a `capacity` declaration selects a lane.
pub const CAPACITY_ENV: &str = "FRACTAL_SEAT_CAPACITY";
/// The older spellings of [`CAPACITY_ENV`], removed with it.
pub const CAPACITY_ENV_ALIASES: [&str; 2] = ["ST_SEAT_CAPACITY", "ST3_SEAT_CAPACITY"];

const GIB: u64 = 1 << 30;
const MIB: u64 = 1 << 20;
/// The smallest memory a lane may declare: below it a harness cannot start.
const MINIMUM_MEMORY: u64 = 256 * MIB;
/// The most cores a lane may declare. The host's own size is admission's question (step 2).
const MAXIMUM_CORES: u32 = 1024;
/// Processes and threads one lane may hold, as `builds.slice` allows on the build hosts.
const TASKS_MAX: u32 = 8192;

/// The slice every lane starts in, inside `app.slice` beside the unlaned seats, so the daemon
/// keeps its weight against them. Its `memory.events` counts every OOM kill a lane made and
/// outlives the lanes, so a host guard can tell the kills lanes make on purpose (here) from any
/// other (the user slice's count less this one).
pub const LANES_SLICE: &str = "app-lanes.slice";

static ENFORCED: AtomicBool = AtomicBool::new(false);

/// A seat lane's size: how many cores of CPU time and how many bytes of memory it may use.
#[derive(Clone, Copy, Debug, serde::Deserialize, Eq, PartialEq, serde::Serialize)]
pub struct Capacity {
    pub cores: u32,
    /// Bytes.
    pub memory: u64,
}

impl Capacity {
    /// The base lane a `capacity` declaration gets for what it leaves out: enough for a harness
    /// thinking, not for a build.
    pub const DEFAULT: Self = Self {
        cores: 1,
        memory: 3 * GIB,
    };

    /// A declared lane, with [`Self::DEFAULT`] for an omitted value.
    pub fn declared(cores: Option<u32>, memory: Option<&str>) -> Result<Self> {
        let cores = cores.unwrap_or(Self::DEFAULT.cores);
        if !(1..=MAXIMUM_CORES).contains(&cores) {
            bail!("capacity cores must be between 1 and {MAXIMUM_CORES}, not {cores}");
        }
        let memory = match memory {
            Some(memory) => parse_memory(memory)?,
            None => Self::DEFAULT.memory,
        };
        if memory < MINIMUM_MEMORY {
            bail!("capacity memory must be at least 256M");
        }
        Ok(Self { cores, memory })
    }

    /// The scope properties that hold this lane. The kernel kills inside a lane that goes over
    /// its memory, and only the process it kills ends (`OOMPolicy=continue`; systemd's default
    /// stops the whole scope, the seat with it). Swap is bounded to a quarter of the lane, so the
    /// excess is killed in the lane instead of moving to the host's swap. No `MemoryHigh`: a
    /// throttle just below the limit stalls the task that reaches it instead of capping it.
    pub(crate) fn properties(&self) -> Vec<String> {
        vec![
            format!("--slice={LANES_SLICE}"),
            format!("--property=CPUQuota={}%", u64::from(self.cores) * 100),
            format!("--property=MemoryMax={}", self.memory),
            format!("--property=MemorySwapMax={}", self.memory / 4),
            "--property=OOMPolicy=continue".to_owned(),
            format!("--property=TasksMax={TASKS_MAX}"),
        ]
    }

    /// Removes the lane a launch environment carries, with every spelling of its key.
    pub(crate) fn take(environment: &mut BTreeMap<String, String>) -> Option<Self> {
        let value = environment.remove(CAPACITY_ENV);
        for alias in CAPACITY_ENV_ALIASES {
            environment.remove(alias);
        }
        let value = value?;
        match value.parse() {
            Ok(capacity) => Some(capacity),
            Err(error) => {
                eprintln!("st3: WARN ignoring an unreadable seat capacity `{value}`: {error:#}");
                None
            }
        }
    }
}

impl std::fmt::Display for Capacity {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "cores={},memory={}", self.cores, self.memory)
    }
}

impl std::str::FromStr for Capacity {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        let mut cores = None;
        let mut memory = None;
        for field in value.split(',') {
            match field.split_once('=') {
                Some(("cores", text)) => cores = Some(text.parse().context("capacity cores")?),
                Some(("memory", text)) => memory = Some(text.parse().context("capacity memory")?),
                _ => bail!("unknown capacity field `{field}`"),
            }
        }
        match (cores, memory) {
            (Some(cores), Some(memory)) => Ok(Self { cores, memory }),
            _ => bail!("a capacity names both cores and memory"),
        }
    }
}

/// `4G`, `512M`, `1T` or a byte count: binary units, as systemd reads them.
pub fn parse_memory(value: &str) -> Result<u64> {
    let (digits, unit) = match value.char_indices().last() {
        Some((index, suffix)) if suffix.is_ascii_alphabetic() => (&value[..index], Some(suffix)),
        _ => (value, None),
    };
    let shift = match unit.map(|unit| unit.to_ascii_uppercase()) {
        None => 0,
        Some('K') => 10,
        Some('M') => 20,
        Some('G') => 30,
        Some('T') => 40,
        Some(_) => bail!("capacity memory `{value}` must end in K, M, G or T"),
    };
    let amount: u64 = digits
        .parse()
        .with_context(|| format!("capacity memory `{value}` must be a whole number with K, M, G or T"))?;
    amount
        .checked_mul(1 << shift)
        .with_context(|| format!("capacity memory `{value}` is too large"))
}

/// Turns lanes on or off for this host (`lanes` in the install config). Applies to launches after
/// this call; the daemon calls it once at start.
pub fn set_enforced(enforced: bool) {
    ENFORCED.store(enforced, Ordering::Relaxed);
}

pub fn enforced() -> bool {
    ENFORCED.load(Ordering::Relaxed)
}

/// The lane a launch gets: its declared capacity, when this host enforces lanes and can.
/// `systemd_254` is whether the user manager is systemd 254 or newer.
pub(crate) fn applied(capacity: Option<Capacity>, unit: &str, systemd_254: bool) -> Option<Capacity> {
    let capacity = capacity?;
    if !enforced() {
        eprintln!("st3: {unit} declares capacity {capacity}, not enforced: lanes are off on this host");
        return None;
    }
    if crate::isolation_mode() == crate::Isolation::Scope && !systemd_254 {
        eprintln!(
            "st3: WARN {unit} declares capacity {capacity}, not enforced: a lane needs systemd 254 or newer \
             (OOMPolicy on a scope)"
        );
        return None;
    }
    if crate::isolation_mode() != crate::Isolation::Scope {
        eprintln!(
            "st3: WARN {unit} declares capacity {capacity}, not enforced: this host has no systemd user scopes \
             (macOS has no per-tree CPU or memory cap for a user process)"
        );
        return None;
    }
    Some(capacity)
}

/// What a laned task's environment adds so its builds stay in the lane. The cargo shim and the
/// test runner on the build hosts run in place under `BUILDS_SLICE_ACTIVE` instead of moving into
/// `builds.slice`, and `scripts/st-test` follows the same rule. The managed sccache server compiles
/// in `builds.slice`, so a lane names a server socket of its own and the client starts that server
/// inside the lane, on the same on-disk cache. A seat that names its own sccache server keeps it.
pub(crate) fn environment(
    unit: &str,
    environment: impl Fn(&str) -> Option<String>,
) -> Vec<(&'static str, String)> {
    let mut added = vec![("BUILDS_SLICE_ACTIVE", "1".to_owned())];
    if environment("SCCACHE_SERVER_UDS").is_none()
        && environment("SCCACHE_SERVER_PORT").is_none()
        && let Some(runtime) = environment("XDG_RUNTIME_DIR").filter(|dir| !dir.is_empty())
    {
        // A unit name is longer than a socket path may be, so the socket is named by its hash.
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        unit.hash(&mut hasher);
        added.push((
            "SCCACHE_SERVER_UDS",
            format!("{runtime}/sccache-lane-{:016x}.sock", hasher.finish()),
        ));
    }
    added
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_declaration_fills_what_it_omits_and_refuses_what_cannot_start() {
        assert_eq!(Capacity::declared(None, None).unwrap(), Capacity::DEFAULT);
        assert_eq!(
            Capacity::declared(Some(2), Some("4G")).unwrap(),
            Capacity { cores: 2, memory: 4 * GIB }
        );
        assert_eq!(Capacity::declared(Some(4), None).unwrap().memory, 3 * GIB);
        assert!(Capacity::declared(Some(0), None).is_err());
        assert!(Capacity::declared(None, Some("100M")).is_err());
        assert!(Capacity::declared(None, Some("4X")).is_err());
        assert!(Capacity::declared(None, Some("4.5G")).is_err());
    }

    #[test]
    fn memory_reads_binary_units() {
        assert_eq!(parse_memory("512M").unwrap(), 512 * MIB);
        assert_eq!(parse_memory("4g").unwrap(), 4 * GIB);
        assert_eq!(parse_memory("1T").unwrap(), 1024 * GIB);
        assert_eq!(parse_memory("1048576").unwrap(), MIB);
        assert!(parse_memory("").is_err());
        assert!(parse_memory("G").is_err());
        assert!(parse_memory("99999999999T").is_err());
    }

    #[test]
    fn the_launch_key_round_trips_and_every_spelling_is_removed() {
        let capacity = Capacity { cores: 2, memory: 4 * GIB };
        let mut environment = BTreeMap::from([
            (CAPACITY_ENV.to_owned(), capacity.to_string()),
            ("ST_SEAT_CAPACITY".to_owned(), "cores=64,memory=1".to_owned()),
            ("PATH".to_owned(), "/bin".to_owned()),
        ]);
        assert_eq!(Capacity::take(&mut environment), Some(capacity));
        assert_eq!(environment.keys().collect::<Vec<_>>(), ["PATH"]);

        let mut aliased = BTreeMap::from([("ST3_SEAT_CAPACITY".to_owned(), capacity.to_string())]);
        assert_eq!(Capacity::take(&mut aliased), None, "only the reconciler's key selects a lane");
        assert!(aliased.is_empty());
    }

    #[test]
    fn properties_cap_cpu_and_memory_and_an_oom_ends_only_its_process() {
        let properties = Capacity { cores: 2, memory: 4 * GIB }.properties();
        assert_eq!(
            properties,
            [
                "--slice=app-lanes.slice",
                "--property=CPUQuota=200%",
                "--property=MemoryMax=4294967296",
                "--property=MemorySwapMax=1073741824",
                "--property=OOMPolicy=continue",
                "--property=TasksMax=8192",
            ]
        );
    }

    #[test]
    fn builds_stay_in_the_lane_and_a_declared_sccache_server_is_kept() {
        let runtime = |key: &str| (key == "XDG_RUNTIME_DIR").then(|| "/run/user/1000".to_owned());
        let added = environment("st3-a-very-long-seat-name.scope", runtime);
        assert_eq!(added[0], ("BUILDS_SLICE_ACTIVE", "1".to_owned()));
        let (key, socket) = &added[1];
        assert_eq!(*key, "SCCACHE_SERVER_UDS");
        assert!(socket.starts_with("/run/user/1000/sccache-lane-") && socket.len() < 108);
        assert_ne!(environment("st3-another.scope", runtime)[1], added[1]);

        let own = |key: &str| (key == "SCCACHE_SERVER_UDS").then(|| "/elsewhere.sock".to_owned());
        assert_eq!(environment("st3-a.scope", own), [("BUILDS_SLICE_ACTIVE", "1".to_owned())]);
    }
}
