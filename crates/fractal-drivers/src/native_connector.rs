//! Supervisor-captured native connector custody. Session labels select a generation; only
//! kernel process generations establish its wrapper, provider, and connector roles.
//! Records survive wrapper exec/adoption. Missing or incomplete records grant no custody.
use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};
use std::{fs, io::Read as _, path::{Path, PathBuf}};

use crate::claude_process::{Process, process};

const FILE: &str = "native-connector-process.json";
const LOCK: &str = "native-connector-process.lock";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Delivery,
    Wrapper,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Launch {
    version: u8,
    harness: String,
    session: String,
    sequence: u64,
    wrapper: Process,
    provider: Option<Process>,
    #[serde(default)]
    connector: Option<Process>,
}

/// An immutable capture of a launch record and the exact connector process generation.
#[derive(Clone, Debug)]
pub struct Witness {
    dir: PathBuf,
    launch: Launch,
    peer: Process,
    role: Role,
}

fn supported(harness: &str) -> bool {
    matches!(harness, "omp" | "pi" | "codex" | "opencode")
}

fn lock(dir: &Path, create: bool) -> Result<crate::flock::FileLock> {
    use crate::flock::{self, FileLock, Mode, Open, Wait};
    let file = flock::open(&dir.join(LOCK), if create { Open::Create } else { Open::Existing })?;
    FileLock::hold(file, Mode::Exclusive, Wait::Now)?
        .context("native connector launch record is busy")
}

fn load(dir: &Path) -> Result<Launch> {
    let file = fs::File::open(dir.join(FILE))?;
    anyhow::ensure!(file.metadata()?.is_file(), "native connector record is not a file");
    let mut bytes = Vec::new();
    file.take(4097).read_to_end(&mut bytes)?;
    anyhow::ensure!(bytes.len() <= 4096, "native connector record exceeds byte limit");
    Ok(serde_json::from_slice(&bytes)?)
}

fn store(dir: &Path, launch: &Launch) -> Result<()> {
    crate::harness_state::write_json_atomic(&dir.join(FILE), launch, dir, ".native-connector-process")
}

/// Presence only: malformed, incomplete, or stale records still require refusal by capture.
pub fn recorded(dir: &Path) -> Result<bool> {
    Ok(dir.join(FILE).try_exists()?)
}

/// Locate a launch's physical wrapper; callers must independently select its terminal relation.
/// This lookup alone grants no connector role.
pub fn wrapper(dir: &Path) -> Result<Process> {
    Ok(load(dir)?.wrapper)
}

/// Read the trusted bootstrap incarnation before the provider publishes its observed state.
/// The expected physical wrapper is selected by the daemon, never by the connector.
pub fn incarnation(dir: &Path, expected_wrapper: Process, harness: &str) -> Result<(String, u64)> {
    let _guard = lock(dir, false)?;
    let launch = load(dir)?;
    anyhow::ensure!(launch.version == 1 && supported(harness) && launch.harness == harness
        && launch.wrapper == expected_wrapper && process(expected_wrapper.pid)? == expected_wrapper,
        "native connector bootstrap wrapper generation changed");
    Ok((launch.session, launch.sequence))
}

/// Begin only at a fresh supervisor launch, never in a requesting connector or adoption path.
pub(crate) fn begin(dir: &Path, harness: &str, session: &str, sequence: u64) -> Result<()> {
    anyhow::ensure!(supported(harness) && !session.is_empty(), "unsupported native connector launch");
    fs::create_dir_all(dir)?;
    let _guard = lock(dir, true)?;
    store(dir, &Launch {
        version: 1,
        harness: harness.into(),
        session: session.into(),
        sequence,
        wrapper: process(std::process::id())?,
        provider: None,
        connector: None,
    })
}

/// Complete the record with the supervisor's actual spawn result, not requester context.
pub(crate) fn spawned(dir: &Path, harness: &str, session: &str, sequence: u64, pid: u32) -> Result<()> {
    let _guard = lock(dir, false)?;
    let mut launch = load(dir)?;
    anyhow::ensure!(launch.harness == harness && launch.session == session && launch.sequence == sequence,
        "native connector launch was superseded");
    anyhow::ensure!(process(std::process::id())? == launch.wrapper && launch.provider.is_none(),
        "native connector wrapper generation changed or provider already recorded");
    let provider = process(pid)?;
    anyhow::ensure!(provider.parent == launch.wrapper.pid && provider.pid != launch.wrapper.pid,
        "native connector provider is not the spawned direct child");
    launch.provider = Some(provider);
    store(dir, &launch)
}

fn establish_connector(
    launch: &Launch, provider_peer_pid: u32, connector_pid: u32, harness: &str,
    session: &str, sequence: u64, read: impl Fn(u32) -> Result<Process>,
) -> Result<Process> {
    anyhow::ensure!(launch.version == 1 && matches!(harness, "omp" | "pi")
        && launch.harness == harness && launch.session == session && launch.sequence == sequence,
        "native connector registration does not match launch");
    anyhow::ensure!(read(launch.wrapper.pid)? == launch.wrapper,
        "native connector wrapper generation is not live");
    let provider = launch.provider.context("native connector provider launch is incomplete")?;
    anyhow::ensure!(provider.pid == provider_peer_pid && provider.parent == launch.wrapper.pid
        && read(provider_peer_pid)? == provider,
        "native connector registration requires the exact provider");
    let connector = read(connector_pid)?;
    anyhow::ensure!(connector.pid != provider.pid && connector.parent == provider.pid,
        "native connector registration requires a direct provider child");
    if let Some(captured) = launch.connector.filter(|captured| *captured != connector) {
        let gone = match read(captured.pid) {
            Ok(current) => current.birth != captured.birth,
            Err(error) => error.downcast_ref::<std::io::Error>().is_some_and(|error| {
                error.kind() == std::io::ErrorKind::NotFound
                    || error.raw_os_error() == Some(libc::ESRCH)
            }),
        };
        anyhow::ensure!(gone, "native connector previous generation is live or unavailable");
    }
    Ok(connector)
}

fn registration_process(pid: u32) -> Result<Process> {
    process(pid).map_err(|error| {
        if let Ok(pid) = i32::try_from(pid)
            && pid > 1
        {
            // SAFETY: signal zero only probes existence; it delivers no signal. This second
            // kernel observation distinguishes a departed process from unavailable metadata.
            let result = unsafe { libc::kill(pid, 0) };
            let absence = std::io::Error::last_os_error();
            if result == -1 && absence.raw_os_error() == Some(libc::ESRCH) {
                return absence.into();
            }
        }
        error
    })
}

/// Register the child the provider itself launched. `provider_peer_pid` must come from the
/// authenticated local transport peer, never from registration payload or inherited context.
/// An identical publication is idempotent. Only the provider may replace a provably departed
/// connector generation; live or indeterminate predecessors retain exclusive custody.
pub fn register_connector(
    dir: &Path, provider_peer_pid: u32, connector_pid: u32, harness: &str,
    session: &str, sequence: u64,
) -> Result<()> {
    let _guard = lock(dir, false)?;
    let mut launch = load(dir)?;
    let connector = establish_connector(
        &launch, provider_peer_pid, connector_pid, harness, session, sequence, registration_process,
    )?;
    // Recheck every captured generation before making registration visible.
    anyhow::ensure!(establish_connector(
        &launch, provider_peer_pid, connector_pid, harness, session, sequence, registration_process,
    )? == connector, "native connector generation changed during registration");
    launch.connector = Some(connector);
    store(dir, &launch)
}

/// Recover ownership for observation only. A missing legacy launch witness may reuse the
/// canonical incarnation's sequence, but must never claim a new generation or mint custody.
pub(crate) fn observer_sequence(dir: &Path, harness: &str, session: &str) -> Result<u64> {
    if !recorded(dir)? {
        let observed = crate::harness_state::read(
            &crate::harness_state::harness_state_path(dir), None,
        ).context("native connector canonical session record is missing")?;
        anyhow::ensure!(observed.harness.as_deref() == Some(harness)
            && observed.evidence_incarnation.as_deref() == Some(session),
            "native connector canonical session generation changed");
        return observed.ownership_sequence.context("native connector canonical ownership is unavailable");
    }
    let _guard = lock(dir, false)?;
    let launch = load(dir)?;
    anyhow::ensure!(launch.version == 1 && launch.harness == harness && launch.session == session
        && launch.wrapper == process(std::process::id())?, "native connector launch generation changed");
    Ok(launch.sequence)
}

// Keep all daemon-selected expectations explicit at this one proof boundary.
#[allow(clippy::too_many_arguments)]
fn prove(launch: &Launch, expected_wrapper: Process, peer_pid: u32, harness: &str,
    role: Role, session: &str, sequence: u64, read: impl Fn(u32) -> Result<Process>) -> Result<Process> {
    anyhow::ensure!(launch.version == 1 && supported(harness) && launch.harness == harness
        && launch.session == session && launch.sequence == sequence && launch.wrapper == expected_wrapper
        && expected_wrapper.pid > 1, "native connector launch does not match current session");
    anyhow::ensure!(read(expected_wrapper.pid)? == expected_wrapper, "native connector wrapper generation is not live");
    let peer = read(peer_pid)?;
    if role == Role::Wrapper || matches!(harness, "codex" | "opencode") {
        // The trusted wrapper owns startup title publication before spawn and terminal
        // publication after provider exit. Its role depends only on its exact live generation.
        anyhow::ensure!(peer == expected_wrapper, "native connector role requires the exact wrapper");
    } else {
        let provider = launch.provider.context("native connector provider launch is incomplete")?;
        anyhow::ensure!(provider.parent == expected_wrapper.pid && read(provider.pid)? == provider,
            "native connector provider generation is not live");
        anyhow::ensure!(peer.pid != provider.pid && peer.parent == provider.pid
            && launch.connector == Some(peer),
            "native delivery connector is not the registered provider child");
    }
    Ok(peer)
}

/// Capture custody against a daemon-selected physical wrapper and current harness incarnation.
/// Neither the connector's argv nor inherited environment contributes to this proof.
pub fn capture(dir: &Path, expected_wrapper: Process, peer_pid: u32, harness: &str,
    role: Role, session: &str, sequence: u64) -> Result<Witness> {
    let _guard = lock(dir, false)?;
    let launch = load(dir)?;
    let peer = prove(&launch, expected_wrapper, peer_pid, harness, role, session, sequence, process)?;
    let witness = Witness { dir: dir.into(), launch, peer, role };
    witness.validate_processes(process)?;
    anyhow::ensure!(load(dir)? == witness.launch, "native connector launch changed during capture");
    Ok(witness)
}

impl Witness {
    fn validate_processes(&self, read: impl Fn(u32) -> Result<Process>) -> Result<()> {
        let peer = prove(&self.launch, self.launch.wrapper, self.peer.pid, &self.launch.harness,
            self.role, &self.launch.session, self.launch.sequence, read)?;
        anyhow::ensure!(peer == self.peer, "native connector peer generation changed");
        Ok(())
    }

    fn validate_launch(&self, launch: &Launch, read: impl Fn(u32) -> Result<Process>) -> Result<()> {
        anyhow::ensure!(*launch == self.launch, "native connector launch record changed");
        self.validate_processes(read)
    }

    /// Recheck the captured record and all captured kernel generations before publication.
    pub fn revalidate(&self) -> Result<()> {
        let _guard = lock(&self.dir, false)?;
        self.validate_launch(&load(&self.dir)?, process)
    }
}

#[cfg(test)]
#[path = "native_connector_tests.rs"]
mod tests;
