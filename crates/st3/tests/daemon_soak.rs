#![cfg(target_os = "linux")]
//! A daemon under a steady stream of requests stops growing.
//!
//! A real `st3 up` process (its own address space, so the figure is the daemon's and not the
//! test's) serves rounds of the same request mix from concurrent clients. After each round the
//! test reads the daemon's resident memory from /proc. Memory may climb while caches fill and the
//! allocator settles; it must not keep climbing. The first third of the rounds is warm-up. The
//! median of the last third may exceed the median of the middle third by only [`allowance`].
//!
//! The rule is a pure function ([`levels_off`]) with its own tests, so the assertion is proven to
//! fail on a leak and pass on a plateau without a daemon.
//!
//! Sensitivity: a leak is caught when it adds more than the allowance over the last two thirds of
//! the rounds, that is about `allowance / (REQUESTS_PER_ROUND * ROUNDS * 2 / 3)` per request.

use std::fs::File;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use serde_json::Value;
use st3::client::Client;
use tokio::sync::Semaphore;

use crate::daemon_bench::claim_input;

/// Rounds of the same mix. Memory is read once after each.
const ROUNDS: usize = 24;
/// Requests in a round.
const REQUESTS_PER_ROUND: usize = 1_500;
/// Requests in flight at once.
const CLIENTS: usize = 8;
/// A few of every ten requests write a claim; the rest read what a seat or a person reads.
const WRITES_PER_TEN: usize = 4;

/// What the last third of the rounds may add over the middle third: 5% of the middle's level, but
/// never under 32 MiB, which is more than the allocator's usual settling between rounds.
fn allowance(middle_kib: u64) -> u64 {
    (middle_kib / 20).max(32 * 1024)
}

fn median(values: &[u64]) -> u64 {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    sorted[sorted.len() / 2]
}

/// `Ok` when the series of per-round resident sizes (KiB) levels off, else what grew.
fn levels_off(series_kib: &[u64]) -> Result<(), String> {
    assert!(series_kib.len() >= 6, "a plateau needs at least six rounds");
    let third = series_kib.len() / 3;
    let middle = median(&series_kib[third..2 * third]);
    let last = median(&series_kib[series_kib.len() - third..]);
    let allowed = allowance(middle);
    if last > middle + allowed {
        return Err(format!(
            "resident memory kept climbing: median {} MiB over the last {third} rounds against {} MiB over the {third} before, \
             {} MiB more than the {} MiB allowed",
            last / 1024,
            middle / 1024,
            (last - middle) / 1024,
            allowed / 1024
        ));
    }
    Ok(())
}

#[test]
fn a_steady_level_passes_and_is_not_fooled_by_the_noise_of_a_round() {
    // 2 GiB warm-up to a plateau of about 2.4 GiB with +-60 MiB noise per round.
    let series = (0..24u64)
        .map(|round| {
            let level = 2_400 * 1024 - (2_400 * 1024 / (round + 1)).min(900 * 1024);
            level + if round % 2 == 0 { 60 * 1024 } else { 0 }
        })
        .collect::<Vec<_>>();
    assert_eq!(levels_off(&series), Ok(()));
}

#[test]
fn a_climb_that_never_stops_fails() {
    // 1 GiB, then 30 MiB more every round: 17 rounds later it is 500 MiB up.
    let series = (0..24u64).map(|round| 1024 * 1024 + round * 30 * 1024).collect::<Vec<_>>();
    let error = levels_off(&series).unwrap_err();
    assert!(error.contains("kept climbing"), "{error}");
}

#[test]
fn a_small_leak_under_the_allowance_is_not_called_a_climb() {
    let series = (0..24u64).map(|round| 1024 * 1024 + round * 1024).collect::<Vec<_>>();
    assert_eq!(levels_off(&series), Ok(()));
}

struct SoakDaemon {
    root: PathBuf,
    child: Child,
}

impl SoakDaemon {
    fn start(root: &Path) -> Self {
        let bin = root.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        // No PTYs, no GitHub and no user manager: the daemon serves requests and nothing else.
        for (name, source) in [
            ("pty", "#!/bin/sh\nprintf '[]\\n'\n"),
            ("gh", "#!/bin/sh\nexit 1\n"),
            ("systemctl", "#!/bin/sh\nexit 1\n"),
        ] {
            let path = bin.join(name);
            std::fs::write(&path, source).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let log = File::create(root.join("daemon.log")).unwrap();
        let child = st3::test_support::command(test_bin!("st3-fixture"))
            .env_clear()
            .env("HOME", root)
            .env(
                "PATH",
                format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
            )
            .env("XDG_CONFIG_HOME", root.join("config"))
            .env("XDG_STATE_HOME", root.join("state"))
            .env("XDG_RUNTIME_DIR", root.join("runtime"))
            .current_dir(root)
            .args(["up", "--node", "soak", "--socket"])
            .arg(root.join("daemon.sock"))
            .arg("--client-gateway-socket")
            .arg(root.join("client.sock"))
            .stdin(Stdio::null())
            .stdout(log.try_clone().unwrap())
            .stderr(log)
            .spawn()
            .unwrap();
        let mut daemon = Self {
            root: root.to_owned(),
            child,
        };
        let deadline = Instant::now() + Duration::from_secs(30);
        while std::os::unix::net::UnixStream::connect(root.join("daemon.sock")).is_err() {
            assert!(
                daemon.child.try_wait().unwrap().is_none(),
                "the daemon exited: {}",
                daemon.log()
            );
            assert!(Instant::now() < deadline, "the daemon did not listen: {}", daemon.log());
            std::thread::sleep(Duration::from_millis(50));
        }
        daemon
    }

    fn socket(&self) -> PathBuf {
        self.root.join("daemon.sock")
    }

    fn log(&self) -> String {
        std::fs::read_to_string(self.root.join("daemon.log")).unwrap_or_default()
    }

    /// The daemon's resident memory, anonymous part and kernel high-water mark, in KiB.
    fn memory(&self) -> Memory {
        let status = std::fs::read_to_string(format!("/proc/{}/status", self.child.id())).unwrap();
        let field = |name: &str| -> u64 {
            status
                .lines()
                .find_map(|line| line.strip_prefix(name)?.trim().strip_suffix("kB")?.trim().parse().ok())
                .unwrap_or_else(|| panic!("no {name} in the daemon's status:\n{status}"))
        };
        Memory {
            rss: field("VmRSS:"),
            anon: field("RssAnon:"),
            peak: field("VmHWM:"),
        }
    }
}

impl Drop for SoakDaemon {
    fn drop(&mut self) {
        if std::thread::panicking() {
            eprintln!("soak daemon log: {}", self.log());
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[derive(Clone, Copy, Debug)]
struct Memory {
    rss: u64,
    anon: u64,
    peak: u64,
}

/// One request of the mix: a claim a driver would post, or a read a seat or a person makes.
async fn request(client: &Client, turn: usize, process: u32) -> Result<(), String> {
    let seat = format!("agent/bench/live-{}", turn % 7);
    let encoded = urlencoding::encode(&seat).into_owned();
    let get = |path: String| async move {
        client
            .get::<Value>(&path)
            .await
            .map(drop)
            .map_err(|error| format!("GET {path}: {error:#}"))
    };
    if turn % 10 < WRITES_PER_TEN {
        let claim = claim_input("runtime.observed", &format!("soak-{process}-{turn}"), turn, "");
        return client
            .post::<_, Value>("/v1/claims", &claim)
            .await
            .map(drop)
            .map_err(|error| format!("POST /v1/claims: {error:#}"));
    }
    match turn % 6 {
        0 => get(format!("/v1/messages/page?include_closed=false&limit=100&to={encoded}")).await,
        1 => get(format!("/v1/status?subject={encoded}")).await,
        2 => get(format!("/v1/work?actor={encoded}")).await,
        3 => get("/v1/client/now".into()).await,
        4 => get("/v1/client/agents".into()).await,
        _ => get("/v1/client/messages".into()).await,
    }
}

async fn round(socket: &Path, turns: &AtomicUsize) -> Result<(), String> {
    let client = Arc::new(Client::unix(socket));
    let slots = Arc::new(Semaphore::new(CLIENTS));
    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..REQUESTS_PER_ROUND {
        let permit = slots.clone().acquire_owned().await.unwrap();
        let client = client.clone();
        let turn = turns.fetch_add(1, Ordering::Relaxed);
        tasks.spawn(async move {
            let outcome = request(&client, turn, std::process::id()).await;
            drop(permit);
            outcome
        });
    }
    while let Some(done) = tasks.join_next().await {
        done.map_err(|error| error.to_string())??;
    }
    Ok(())
}

fn load_average() -> f64 {
    let mut averages = [0f64; 3];
    // SAFETY: getloadavg fills at most the three doubles it is given.
    let read = unsafe { libc::getloadavg(averages.as_mut_ptr(), 3) };
    if read >= 1 { averages[0] } else { f64::NAN }
}

#[test]
fn a_daemon_under_steady_requests_levels_off() {
    if st3::test_support::supervise_test() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let daemon = SoakDaemon::start(root.path());
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let turns = AtomicUsize::new(0);
    let started = Instant::now();
    let mut series = Vec::new();
    eprintln!("daemon soak: {ROUNDS} rounds of {REQUESTS_PER_ROUND} requests, {CLIENTS} at once");
    eprintln!("round  requests  rss_MiB  anon_MiB  peak_MiB  seconds  load1");
    let before = daemon.memory();
    eprintln!("start         0  {:7}  {:8}  {:8}  {:7.1}  {:5.2}", before.rss / 1024, before.anon / 1024, before.peak / 1024, 0.0, load_average());
    for number in 1..=ROUNDS {
        if let Err(error) = runtime.block_on(round(&daemon.socket(), &turns)) {
            panic!("round {number}: {error}\n{}", daemon.log());
        }
        let memory = daemon.memory();
        eprintln!(
            "{number:5}  {:8}  {:7}  {:8}  {:8}  {:7.1}  {:5.2}",
            turns.load(Ordering::Relaxed),
            memory.rss / 1024,
            memory.anon / 1024,
            memory.peak / 1024,
            started.elapsed().as_secs_f64(),
            load_average()
        );
        series.push(memory.anon);
    }
    if let Err(error) = levels_off(&series) {
        panic!("{error}");
    }
}
