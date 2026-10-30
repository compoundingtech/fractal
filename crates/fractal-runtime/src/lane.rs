//! Seat lanes: a CPU and memory cap around a seat (docs/st3/seat-lanes.md).
//!
//! A seat that declares `capacity cores=N memory="SIZE"` gets a slice of its own,
//! `app-lanes-<seat>.slice`, that holds `CPUQuota` and `MemoryMax`. Each launch of its harness is
//! a scope inside it, so the harness and everything it starts share the cap, and a job the seat
//! starts as a unit of its own in the slice outlives a restart of the harness. The slice ends when
//! the seat stops for good. Only on a Linux host whose config turns lanes on; elsewhere the
//! declaration is logged as not enforced and the seat starts as it does without one.

use std::collections::BTreeMap;
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

/// Held by a test while it relies on the switch, which is process-wide.
#[cfg(test)]
pub(crate) static TEST_SWITCH: std::sync::Mutex<()> = std::sync::Mutex::new(());

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

    /// The slice properties that hold this lane. Swap is bounded to a quarter of the lane, so the
    /// excess is killed in the lane instead of moving to the host's swap. No `MemoryHigh`: a
    /// throttle just below the limit stalls the task that reaches it instead of capping it.
    pub(crate) fn slice_properties(&self) -> Vec<String> {
        vec![
            format!("CPUQuota={}%", u64::from(self.cores) * 100),
            format!("MemoryMax={}", self.memory),
            format!("MemorySwapMax={}", self.memory / 4),
            format!("TasksMax={TASKS_MAX}"),
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

/// A seat's slice: `app-lanes-<seat>_<digest>.slice`, inside [`LANES_SLICE`]. A dash in a slice
/// name nests it, so the seat's own dashes, like its other unsafe bytes, become underscores for
/// reading; the digest of the whole runtime id keeps two seats that read alike apart, and is the
/// same on every build, so a seat finds its slice again after an upgrade.
pub fn seat_slice(runtime_id: &str) -> String {
    let seat: String = crate::isolate::sanitize(runtime_id)
        .chars()
        .map(|character| if character == '-' { '_' } else { character })
        .collect();
    format!("app-lanes-{seat}_{}.slice", &digest(runtime_id)[..12])
}

fn digest(value: &str) -> String {
    use sha2::Digest as _;
    format!("{:x}", sha2::Sha256::digest(value.as_bytes()))
}

/// What this process knows of one seat's slice, from the first launch prepared in it until its
/// end. Its lock is held while the slice is capped for a launch and while it is ended, so an end
/// never runs halfway through a launch's preparation. Only a seat with a lane has one.
#[derive(Default)]
struct Slot {
    /// Launches prepared in the slice.
    launches: u64,
    /// The launch count an end was asked at: the end acts only while no launch came after it.
    end_at: Option<u64>,
    /// The lane ended and its slot left the map; a launch that waited on its lock takes a new one.
    removed: bool,
}

type Slots = BTreeMap<String, std::sync::Arc<std::sync::Mutex<Slot>>>;

static SLOTS: std::sync::Mutex<Slots> = std::sync::Mutex::new(BTreeMap::new());

fn slots() -> std::sync::MutexGuard<'static, Slots> {
    SLOTS.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// A lane a launch starts in: the seat's capped slice.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Lane {
    pub(crate) slice: String,
}

impl Lane {
    /// What the launch's scope adds: it starts in the seat's slice, and an OOM kill inside it ends
    /// only the process the kernel killed (`OOMPolicy=continue`; systemd's default stops the whole
    /// scope, the seat with it).
    pub(crate) fn scope_arguments(&self) -> Vec<String> {
        vec![
            format!("--slice={}", self.slice),
            "--property=OOMPolicy=continue".to_owned(),
        ]
    }
}

/// The lane a launch of `runtime_id` starts in: its declared capacity, when this host enforces
/// lanes and can. The seat's slice holds the capacity before anything starts in it, so a laned
/// harness never runs uncapped; a slice that cannot be capped refuses the launch.
pub(crate) fn prepare(capacity: Option<Capacity>, runtime_id: &str) -> Result<Option<Lane>> {
    let Some(capacity) = capacity else {
        return Ok(None);
    };
    if !enforced() {
        eprintln!("st3: {runtime_id} declares capacity {capacity}, not enforced: lanes are off on this host");
        return Ok(None);
    }
    if crate::isolation_mode() != crate::Isolation::Scope {
        eprintln!(
            "st3: WARN {runtime_id} declares capacity {capacity}, not enforced: this host has no systemd user scopes \
             (macOS has no per-tree CPU or memory cap for a user process)"
        );
        return Ok(None);
    }
    if !crate::isolate::systemd_254() {
        eprintln!(
            "st3: WARN {runtime_id} declares capacity {capacity}, not enforced: a lane needs systemd 254 or newer \
             (OOMPolicy on a scope)"
        );
        return Ok(None);
    }
    let slice = seat_slice(runtime_id);
    loop {
        let slot = slots().entry(slice.clone()).or_default().clone();
        let mut slot = slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if slot.removed {
            continue;
        }
        // An end asked before this launch no longer applies to the slice.
        slot.launches += 1;
        slot.end_at = None;
        let output = crate::isolate::systemctl()
            .args(["--user", "set-property", "--runtime", &slice])
            .args(capacity.slice_properties())
            .stdin(std::process::Stdio::null())
            .output()
            .with_context(|| format!("cap the lane {slice}"))?;
        if !output.status.success() {
            bail!(
                "cap the lane {slice}: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        return Ok(Some(Lane { slice }));
    }
}

/// Ends the lane of `runtime_id`, when it has one: everything still running in its slice, the jobs
/// the seat started included, then the slice's limits. In the background, since the units in it
/// get their own stop timeouts. For a seat that stopped for good; a restart keeps its lane, and a
/// launch prepared after this call cancels it, so a seat started again at once keeps its new lane.
/// A seat without a lane does nothing here beyond one look for its slice in the cgroup tree.
pub fn end(runtime_id: &str) {
    let slice = seat_slice(runtime_id);
    if !request_end(&slice) {
        return;
    }
    crate::isolate::end_later(format!("lane:{slice}"), move || end_requested(&slice));
}

/// Marks the end of `slice`'s lane as asked. False when the seat has no lane: this process
/// prepared none there, and no slice of that name is left from before it started (a lane prepared
/// before a daemon restart keeps running, its slot gone with the old process).
fn request_end(slice: &str) -> bool {
    if !slots().contains_key(slice) && !slice_cgroup(slice).is_some_and(|cgroup| cgroup.is_dir()) {
        return false;
    }
    loop {
        let slot = slots().entry(slice.to_owned()).or_default().clone();
        let mut slot = slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if slot.removed {
            continue;
        }
        slot.end_at = Some(slot.launches);
        return true;
    }
}

/// Where `slice` sits in the cgroup tree: under `app-lanes.slice` of the user manager this process
/// runs in.
fn slice_cgroup(slice: &str) -> Option<std::path::PathBuf> {
    let own = std::fs::read_to_string("/proc/self/cgroup").ok()?;
    let own = own.lines().find_map(|line| line.strip_prefix("0::"))?;
    let parts: Vec<&str> = own.split('/').filter(|part| !part.is_empty()).collect();
    let manager = parts.iter().position(|part| part.starts_with("user@") && part.ends_with(".service"))?;
    let mut path = std::path::PathBuf::from("/sys/fs/cgroup");
    path.extend(&parts[..=manager]);
    path.extend(["app.slice", LANES_SLICE, slice]);
    Some(path)
}

/// Ends `slice` if its last end request still stands: no launch was prepared after it. Its slot
/// goes with it.
fn end_requested(slice: &str) -> Result<()> {
    let Some(slot) = slots().get(slice).cloned() else {
        return Ok(());
    };
    let mut slot = slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if slot.removed || slot.end_at.take() != Some(slot.launches) {
        return Ok(());
    }
    stop_slice(slice)?;
    slot.removed = true;
    slots().remove(slice);
    Ok(())
}

fn stop_slice(slice: &str) -> Result<()> {
    let loaded = crate::isolate::systemctl()
        .args(["--user", "show", "--property=LoadState", "--value", slice])
        .output()
        .with_context(|| format!("read the state of the lane {slice}"))?;
    anyhow::ensure!(
        loaded.status.success(),
        "read the state of the lane {slice}: {}",
        String::from_utf8_lossy(&loaded.stderr).trim()
    );
    // A slice that is not loaded has nothing left to end.
    if String::from_utf8_lossy(&loaded.stdout).trim() != "loaded" {
        return Ok(());
    }
    for verb in ["stop", "revert"] {
        let output = crate::isolate::systemctl()
            .args(["--user", verb, slice])
            .stdin(std::process::Stdio::null())
            .output()
            .with_context(|| format!("{verb} the lane {slice}"))?;
        anyhow::ensure!(
            output.status.success(),
            "{verb} the lane {slice}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

/// Asks for the end of `runtime_id`'s lane without running it; the test then decides when it runs.
#[cfg(test)]
pub(crate) fn request_end_for_test(runtime_id: &str) -> impl FnOnce() -> Result<()> {
    let slice = seat_slice(runtime_id);
    request_end(&slice);
    move || end_requested(&slice)
}

/// Whether this process holds a slot for `runtime_id`'s lane.
#[cfg(test)]
pub(crate) fn prepared_for_test(runtime_id: &str) -> bool {
    slots().contains_key(&seat_slice(runtime_id))
}

/// Drops this process's slot for `runtime_id`'s lane, as a daemon restart does.
#[cfg(test)]
pub(crate) fn forget_for_test(runtime_id: &str) {
    slots().remove(&seat_slice(runtime_id));
}

/// What a laned task's environment adds so its builds stay in the lane. The cargo shim and the
/// test runner on the build hosts run in place under `BUILDS_SLICE_ACTIVE` instead of moving into
/// `builds.slice` (`scripts/st-test` reads its own cgroup's limits instead). The host's sccache
/// server compiles in `builds.slice`, so a lane names a server socket of its own and the client
/// starts that server inside the lane, on the same on-disk cache. A seat that names its own sccache
/// server keeps it; one it only inherited from the daemon (`seat` holds the daemon's own value) does
/// not count, since that is the host's server. The lane's socket is the client's first choice
/// either way: sccache reads `SCCACHE_SERVER_UDS` before `SCCACHE_SERVER_PORT`.
/// `FRACTAL_LANE_SLICE` names the slice, for a job the seat starts as its own unit there
/// (`systemd-run --user --slice=...`) so that it outlives a restart of the harness. `seat` reads the
/// task's environment and `host` the daemon's.
pub(crate) fn environment(
    lane: &Lane,
    seat: impl Fn(&str) -> Option<String>,
    host: impl Fn(&str) -> Option<String>,
) -> Vec<(&'static str, String)> {
    let mut added = vec![
        ("BUILDS_SLICE_ACTIVE", "1".to_owned()),
        ("FRACTAL_LANE_SLICE", lane.slice.clone()),
    ];
    let named = |key: &str| seat(key).filter(|value| host(key).as_ref() != Some(value));
    // The client takes a socket before a port, so a port the seat names is its server only while
    // no socket is set at all; an inherited socket would win over it.
    let own_server = match seat("SCCACHE_SERVER_UDS") {
        Some(_) => named("SCCACHE_SERVER_UDS").is_some(),
        None => named("SCCACHE_SERVER_PORT").is_some(),
    };
    if !own_server
        && let Some(runtime) = seat("XDG_RUNTIME_DIR")
            .or_else(|| host("XDG_RUNTIME_DIR"))
            .filter(|dir| !dir.is_empty())
    {
        // A slice name can be longer than a socket path may be, so the socket is named by its digest.
        added.push((
            "SCCACHE_SERVER_UDS",
            format!("{runtime}/sccache-lane-{}.sock", &digest(&lane.slice)[..16]),
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
    fn the_slice_caps_cpu_and_memory_and_an_oom_ends_only_its_process() {
        assert_eq!(
            Capacity { cores: 2, memory: 4 * GIB }.slice_properties(),
            [
                "CPUQuota=200%",
                "MemoryMax=4294967296",
                "MemorySwapMax=1073741824",
                "TasksMax=8192",
            ]
        );
        let lane = Lane { slice: seat_slice("fleet.example.seat-1") };
        assert_eq!(
            lane.scope_arguments(),
            [format!("--slice={}", lane.slice), "--property=OOMPolicy=continue".to_owned()]
        );
    }

    #[test]
    fn a_seat_slice_nests_only_under_the_lanes_slice_and_is_the_seats_alone() {
        // A dash nests a slice, so the seat's own become underscores; other unsafe bytes do too.
        let slice = seat_slice("fleet.factory.seat-lanes.01-a");
        assert!(slice.starts_with("app-lanes-fleet.factory.seat_lanes.01_a_"), "{slice}");
        assert!(slice.ends_with(".slice") && slice.matches('-').count() == 2, "{slice}");
        // Seats that read alike once their bytes are made safe still get slices of their own.
        for (one, other) in [("a-b", "a_b"), ("a b/c", "a_b_c"), ("x.y", "x.y ")] {
            assert_ne!(seat_slice(one), seat_slice(other), "{one} and {other}");
        }
        // The same on every build, so a restart (or an upgrade) finds its seat's slice again.
        assert_eq!(seat_slice("same"), "app-lanes-same_0967115f2813.slice");
    }

    #[test]
    fn a_seat_without_a_lane_does_no_lane_work() {
        let id = "fleet.example.unlaned";
        assert_eq!(prepare(None, id).unwrap(), None);
        end(id);
        assert!(!prepared_for_test(id), "a seat without a lane holds no slot");
        assert!(!request_end(&seat_slice(id)), "and has no end to queue");
    }

    #[test]
    fn builds_stay_in_the_lane_and_a_declared_sccache_server_is_kept() {
        let runtime = |key: &str| (key == "XDG_RUNTIME_DIR").then(|| "/run/user/1000".to_owned());
        let nothing = |_: &str| None;
        let lane = Lane { slice: seat_slice("fleet.example.a-very-long-seat-name") };
        let added = environment(&lane, runtime, nothing);
        assert_eq!(added[0], ("BUILDS_SLICE_ACTIVE", "1".to_owned()));
        assert_eq!(added[1], ("FRACTAL_LANE_SLICE", lane.slice.clone()));
        let (key, socket) = &added[2];
        assert_eq!(*key, "SCCACHE_SERVER_UDS");
        assert!(socket.starts_with("/run/user/1000/sccache-lane-") && socket.len() < 108);
        assert_eq!(environment(&lane, runtime, nothing)[2], added[2], "a restart reuses the seat's server");
        assert_eq!(environment(&lane, nothing, runtime)[2], added[2], "the daemon's runtime directory serves too");
        let other = Lane { slice: seat_slice("fleet.example.other") };
        assert_ne!(environment(&other, runtime, nothing)[2], added[2]);

        let server = |value: &'static str| {
            move |key: &str| match key {
                "XDG_RUNTIME_DIR" => Some("/run/user/1000".to_owned()),
                "SCCACHE_SERVER_UDS" => Some(value.to_owned()),
                _ => None,
            }
        };
        assert_eq!(environment(&lane, server("/elsewhere.sock"), nothing).len(), 2, "the seat's own server");
        // The host's server, only inherited from the daemon, compiles outside the lane.
        assert_eq!(environment(&lane, server("/host.sock"), server("/host.sock"))[2], added[2]);
        assert_eq!(environment(&lane, server("/elsewhere.sock"), server("/host.sock")).len(), 2);
        let port = |key: &str| (key == "SCCACHE_SERVER_PORT").then(|| "4226".to_owned());
        assert_eq!(environment(&lane, |key| runtime(key).or_else(|| port(key)), port)[2], added[2]);
        assert_eq!(environment(&lane, |key| runtime(key).or_else(|| port(key)), nothing).len(), 2, "the seat's own port");
        // A port of the seat's own does not win over a socket inherited from the daemon: the client
        // takes the socket first, so the lane's replaces it.
        let both = |key: &str| server("/host.sock")(key).or_else(|| port(key));
        assert_eq!(environment(&lane, both, server("/host.sock"))[2], added[2]);
    }
}
