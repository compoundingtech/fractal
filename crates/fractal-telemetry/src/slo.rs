//! The daemon's targets, from the objectives it ships in `slo/objectives.kdl`, which is built in
//! and read by the same reader as a declared objective (`fractal_kernel::objective`). The same
//! file is what `st doctor` and CI's daemon_load read. A target describes what we aim for; it
//! gates nothing.

use std::sync::{LazyLock, OnceLock};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail, ensure};
use fractal_kernel::objective::{self, Bound, Indicator, Objective};
use serde::Serialize;
use serde_json::{Value, json};
use smallclaims::windows::StoreWork;

/// The objectives file this daemon was built with.
pub const SOURCE: &str = include_str!("../../../slo/objectives.kdl");

#[derive(Debug)]
pub struct Targets {
    pub latency: Vec<Latency>,
    pub share: Vec<Share>,
    pub statement: Statement,
    pub transaction: Transaction,
    pub writer_hold: WriterHold,
    pub cpu: Cpu,
    pub divergence: Divergence,
    pub database: Database,
}

#[derive(Debug)]
pub struct Latency {
    pub name: String,
    pub about: String,
    pub p99_ms: u64,
    /// The same reads served through this daemon from another machine that owns them.
    pub remote_p99_ms: Option<u64>,
    pub paths: Vec<String>,
}

#[derive(Debug)]
pub struct Share {
    pub name: String,
    pub about: String,
    pub min_percent: u64,
    pub path: String,
}

#[derive(Debug)]
pub struct Statement {
    pub about: String,
    pub p99_ms: u64,
    pub max_ms: u64,
}

#[derive(Debug)]
pub struct Transaction {
    pub about: String,
    pub max_ms: u64,
}

/// Each hold of the store's single writer, and each read transaction past the same bound, by who
/// held it.
#[derive(Debug)]
pub struct WriterHold {
    pub about: String,
    pub max_ms: u64,
}

/// The member's CPU: its daemon process and every replication worker process beside it, over
/// closed UTC minutes. See `smallclaims::windows::cpu_minutes` for what makes a minute idle.
#[derive(Debug)]
pub struct Cpu {
    pub about: String,
    /// The most cores the member may use over the hour.
    pub max_cores: f64,
    /// The most cores the member may use over the hour's idle minutes, on average.
    pub idle_max_cores: f64,
    /// The idle minutes an hour needs before the idle target judges it; with fewer it is unknown.
    pub idle_min_minutes: u64,
    /// The bound daemon_load holds the daemon to under its synthetic load in CI.
    pub load_max_cores: f64,
}

/// Projection divergences, counted fleet-wide from their open faults. A projection is a pure
/// function of the claims, so each one is a bug.
#[derive(Debug)]
pub struct Divergence {
    pub about: String,
    pub max_open: u64,
}

#[derive(Debug)]
pub struct Database {
    pub about: String,
    /// The pages in use, not counting free pages SQLite reuses before the file grows.
    pub max_live_gb: f64,
    /// Net daily growth of the pages in use: what is written less what checkpoints free.
    pub max_growth_mb_per_day: f64,
}

impl Latency {
    /// The target a sample served here (`remote` false) or from another machine misses past.
    pub fn target(&self, remote: bool) -> Duration {
        let ms = if remote {
            self.remote_p99_ms.unwrap_or(self.p99_ms)
        } else {
            self.p99_ms
        };
        Duration::from_millis(ms)
    }
}

/// Read and check an objectives file of the objectives Fractal ships. A missing, unknown or
/// nonsensical child is an error, never a default.
pub fn parse(text: &str) -> Result<Targets> {
    read(text).map(|(_, targets)| targets)
}

fn read(text: &str) -> Result<(Vec<Objective>, Targets)> {
    let objectives =
        objective::parse_document(text).map_err(|error| anyhow!("the objectives file is not valid: {error}"))?;
    let targets = build(&objectives)?;
    check(&targets)?;
    Ok((objectives, targets))
}

/// Put `value` in the one slot its indicator has, named `expected`: `fractal doctor` and the OTLP
/// export read these by name.
fn one<T>(slot: &mut Option<T>, indicator: &str, expected: &str, name: &str, value: T) -> Result<()> {
    ensure!(name == expected, "the {indicator} objective is named {name}, not {expected}");
    ensure!(slot.is_none(), "there are two {indicator} objectives");
    *slot = Some(value);
    Ok(())
}

/// The targets the shipped objectives say. Every one is a single segment; an interval is only
/// ever declared.
fn build(objectives: &[Objective]) -> Result<Targets> {
    let mut latency = Vec::new();
    let mut share = Vec::new();
    let mut statement = None;
    let mut transaction = None;
    let mut writer_hold = None;
    let mut cpu = None;
    let mut divergence = None;
    let mut database = None;
    for objective in objectives {
        let name = objective.name.as_str();
        ensure!(!name.contains('/'), "objective {name} is not one segment: only a declared objective has a namespace");
        let about = objective.about.clone();
        let within = |stat: &str, remote: bool| objective.target(stat, remote).and_then(objective::Target::within_ms);
        let required = |stat: &str| within(stat, false).with_context(|| format!("objective {name} has no {stat} duration"));
        let max = |stat: &str| {
            objective.target(stat, false).and_then(objective::Target::max).with_context(|| format!("objective {name} has no {stat} maximum"))
        };
        let whole = |value: f64, what: &str| -> Result<u64> {
            ensure!(value >= 0.0 && value.fract() == 0.0, "objective {name}'s {what} is not a whole number");
            Ok(value as u64)
        };
        match &objective.indicator {
            Indicator::Latency { paths } => latency.push(Latency {
                name: name.to_owned(),
                about,
                p99_ms: required("p99")?,
                remote_p99_ms: within("p99", true),
                paths: paths.clone(),
            }),
            Indicator::Share { path } => {
                let min_percent = match objective.target("live-share", false).map(|target| target.bound) {
                    Some(Bound::MinPercent(min)) => whole(min, "minimum percent")?,
                    _ => bail!("objective {name} has no live-share minimum"),
                };
                share.push(Share { name: name.to_owned(), about, min_percent, path: path.clone() });
            }
            Indicator::Statement => {
                let value = Statement { about, p99_ms: required("p99")?, max_ms: required("max")? };
                one(&mut statement, "statement", "sql-statement", name, value)?;
            }
            Indicator::Transaction => {
                let value = Transaction { about, max_ms: required("max")? };
                one(&mut transaction, "transaction", "transaction", name, value)?;
            }
            Indicator::WriterHold => {
                let value = WriterHold { about, max_ms: required("max")? };
                one(&mut writer_hold, "writer-hold", "writer-hold", name, value)?;
            }
            Indicator::Cpu { idle_min_minutes, load_max_cores } => {
                let value = Cpu {
                    about,
                    max_cores: max("cores")?,
                    idle_max_cores: max("idle-cores")?,
                    idle_min_minutes: *idle_min_minutes,
                    load_max_cores: *load_max_cores,
                };
                one(&mut cpu, "cpu", "cpu", name, value)?;
            }
            Indicator::Database => {
                let value = Database {
                    about,
                    max_live_gb: max("live-gb")?,
                    max_growth_mb_per_day: max("growth-mb-per-day")?,
                };
                one(&mut database, "database", "database", name, value)?;
            }
            Indicator::Divergence => {
                let value = Divergence { about, max_open: whole(max("open")?, "open maximum")? };
                one(&mut divergence, "divergence", "projection-divergence", name, value)?;
            }
            Indicator::Interval(_)
            | Indicator::Count(_)
            | Indicator::GithubMergedCount(_)
            | Indicator::GithubReadyToMerged(_)
            | Indicator::Ratio(_)
            | Indicator::Gauge(_)
            | Indicator::Native { .. } => {
                bail!("objective {name} is a fleet declaration: Fractal ships native measurement definitions, not claim indicators, GitHub indicators or owner bindings")
            }
        }
    }
    let missing = |indicator: &str| format!("there is no {indicator} objective");
    Ok(Targets {
        latency,
        share,
        statement: statement.with_context(|| missing("statement"))?,
        transaction: transaction.with_context(|| missing("transaction"))?,
        writer_hold: writer_hold.with_context(|| missing("writer-hold"))?,
        cpu: cpu.with_context(|| missing("cpu"))?,
        divergence: divergence.with_context(|| missing("divergence"))?,
        database: database.with_context(|| missing("database"))?,
    })
}

fn check(targets: &Targets) -> Result<()> {
    let mut names = std::collections::BTreeSet::new();
    let mut paths = std::collections::BTreeSet::new();
    for latency in &targets.latency {
        ensure!(
            names.insert(latency.name.clone()),
            "target {} is named twice",
            latency.name
        );
        ensure!(latency.p99_ms > 0, "target {} has no latency", latency.name);
        ensure!(
            latency.remote_p99_ms.is_none_or(|remote| remote >= latency.p99_ms),
            "target {}'s remote latency is below its own",
            latency.name
        );
        ensure!(!latency.paths.is_empty(), "target {} has no paths", latency.name);
        for path in &latency.paths {
            if path.starts_with("client/ios/") {
                ensure!(paths.insert(path.as_str()), "path {path:?} has two targets");
                continue;
            }
            let (method, route) = path
                .split_once(' ')
                .with_context(|| format!("path {path:?} names no method"))?;
            ensure!(
                matches!(
                    method,
                    "GET" | "POST" | "PUT" | "DELETE" | "stream" | "publish"
                ),
                "path {path:?} has an unknown method"
            );
            ensure!(
                matches!(method, "stream" | "publish") || route.starts_with("/v1/"),
                "path {path:?} is not a daemon route"
            );
            ensure!(paths.insert(path.as_str()), "path {path:?} has two targets");
        }
    }
    for share in &targets.share {
        ensure!(names.insert(share.name.clone()), "duplicate share target");
        ensure!(share.min_percent > 0 && share.min_percent <= 100, "invalid share target");
        ensure!(share.path.starts_with("client/ios/") && paths.insert(&share.path), "invalid share path");
    }
    ensure!(
        targets.statement.p99_ms > 0 && targets.statement.max_ms >= targets.statement.p99_ms,
        "the statement targets are inconsistent"
    );
    ensure!(targets.transaction.max_ms > 0, "the transaction target is zero");
    ensure!(targets.writer_hold.max_ms > 0, "the writer-hold target is zero");
    let cpu = &targets.cpu;
    ensure!(
        [cpu.max_cores, cpu.idle_max_cores, cpu.load_max_cores]
            .iter()
            .all(|cores| cores.is_finite() && *cores > 0.0),
        "the CPU targets are not positive numbers of cores"
    );
    ensure!(cpu.idle_max_cores <= cpu.max_cores, "the idle CPU target is above the member's");
    ensure!(
        (1..=smallclaims::windows::cpu_minutes::MINUTES).contains(&cpu.idle_min_minutes),
        "the idle CPU target's minimum is not between 1 and 60 minutes"
    );
    ensure!(
        [targets.database.max_live_gb, targets.database.max_growth_mb_per_day]
            .iter()
            .all(|value| value.is_finite() && *value > 0.0),
        "the database targets are not positive sizes"
    );
    Ok(())
}

/// The shipped objectives and the targets they say. The file is checked by a test, so this cannot
/// fail in a release.
static BUILT_IN: LazyLock<(Vec<Objective>, Targets)> =
    LazyLock::new(|| read(SOURCE).expect("the built-in objectives file is valid"));

/// The objectives Fractal ships, as the objective reader reads them.
pub fn shipped() -> &'static [Objective] {
    &BUILT_IN.0
}

/// The built-in targets.
pub fn targets() -> &'static Targets {
    &BUILT_IN.1
}

/// Stable identities bind bad counters to their original measurement boundary.
static LATENCY_STREAMS: LazyLock<Vec<[String; 2]>> = LazyLock::new(|| {
    targets().latency.iter().map(|target| [
        format!("latency/{}/local/{}", target.name, target.p99_ms),
        format!("latency/{}/remote/{}", target.name, target.remote_p99_ms.unwrap_or(target.p99_ms)),
    ]).collect()
});

fn store_stream(work: StoreWork) -> String {
    let ms = match work {
        StoreWork::Statement => targets().statement.p99_ms,
        StoreWork::WriterHold => targets().writer_hold.max_ms,
        _ => targets().transaction.max_ms,
    };
    format!("{}/{ms}", work.name())
}

/// The daemon supplies a host-local state path and checkpoints this ledger every minute.
/// Existing diagnostic windows are deliberately not used as historical evidence.
pub fn install_budget_ledger(path: &std::path::Path, node: &str) -> Result<()> {
    install();
    let streams = LATENCY_STREAMS.iter().flat_map(|pair| pair.iter().cloned())
        .chain([StoreWork::ReadTransaction, StoreWork::WriteTransaction, StoreWork::WriterHold]
            .into_iter().map(store_stream)).collect();
    smallclaims::windows::budgets::install(path, node, streams)
}

pub fn checkpoint_budgets() -> Result<()> {
    smallclaims::windows::budgets::checkpoint()
}

/// Hot-path counters only: no claim, SQLite statement, or per-event allocation.
pub fn note_latency(index: usize, remote: bool, _now: std::time::Instant, over: bool) {
    if let Some(pair) = LATENCY_STREAMS.get(index) {
        smallclaims::windows::budgets::record(&pair[usize::from(remote)], over);
    }
}

fn evidence(result: Result<Value>) -> Value {
    result.unwrap_or_else(|error| json!({"available": false, "error": error.to_string(), "windows": {}}))
}

pub fn latency_budget(index: usize, remote: bool, _now: std::time::Instant) -> Value {
    LATENCY_STREAMS.get(index).map_or_else(
        || json!({"available": false, "windows": {}}),
        |pair| evidence(smallclaims::windows::budgets::report(&pair[usize::from(remote)])),
    )
}

fn store_windows(work: StoreWork) -> Value {
    let mut windows = smallclaims::windows::store_series(work).snapshot(std::time::Instant::now());
    windows["budget"] = evidence(smallclaims::windows::budgets::report(&store_stream(work)));
    windows
}

fn database_stream() -> String {
    format!("database/{}/{}", targets().database.max_live_gb, targets().database.max_growth_mb_per_day)
}

/// The hourly source is held at most two hours. Its missing growth baseline cannot prove
/// a healthy conjunction, but an already excessive live size is independently known bad.
fn database_sample(size: &DatabaseSize) -> Option<f64> {
    let live = size.live_bytes as f64 / (targets().database.max_live_gb * 1_000_000_000.0);
    size.growth_bytes_per_day.map(|growth| {
        live.max(growth as f64 / (targets().database.max_growth_mb_per_day * 1_000_000.0))
    }).or_else(|| (live > 1.0).then_some(live))
}

fn database_budget() -> Value {
    let mut value = evidence(smallclaims::windows::budgets::gauge_report(
        &database_stream(), smallclaims::windows::budgets::unix_ms(), 2 * 60 * 60 * 1000, 1.0,
    ));
    value["cadence_ms"] = json!(60 * 60 * 1000);
    value
}

impl Targets {
    /// Which target `path` (`GET /v1/client/now`, `stream agents`) counts toward, by position.
    pub fn index_of(&self, path: &str) -> Option<usize> {
        self.latency
            .iter()
            .position(|latency| latency.paths.iter().any(|p| p == path))
    }

    /// The target `path` counts toward.
    pub fn for_path(&self, path: &str) -> Option<&Latency> {
        self.index_of(path).map(|index| &self.latency[index])
    }

    pub fn statement_target(&self) -> Duration {
        Duration::from_millis(self.statement.p99_ms)
    }

    pub fn transaction_target(&self) -> Duration {
        Duration::from_millis(self.transaction.max_ms)
    }

    pub fn writer_hold_target(&self) -> Duration {
        Duration::from_millis(self.writer_hold.max_ms)
    }
}

/// Give the store's windows their targets, once per process.
pub fn install() {
    static INSTALLED: OnceLock<()> = OnceLock::new();
    INSTALLED.get_or_init(|| {
        let targets = targets();
        smallclaims::windows::set_store_target(
            StoreWork::Statement,
            Some(targets.statement_target()),
        );
        for work in [StoreWork::ReadTransaction, StoreWork::WriteTransaction] {
            smallclaims::windows::set_store_target(work, Some(targets.transaction_target()));
        }
        smallclaims::windows::set_store_target(StoreWork::WriterHold, Some(targets.writer_hold_target()));
    });
}

/// One measurement of the store.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DatabaseSize {
    pub measured_at_unix_ms: u128,
    /// The store's pages, including free ones and those committed to the WAL but not yet copied
    /// into the main file.
    pub file_bytes: u64,
    /// The pages in use: the file less its free pages, which SQLite reuses before growing.
    pub live_bytes: u64,
    /// How much `live_bytes` grew a day, from the sample nearest a day ago (or the oldest, when
    /// the store has been sampled for less than a day but at least an hour). Negative when
    /// checkpoints freed more than was written.
    pub growth_bytes_per_day: Option<i64>,
    /// The span the growth was measured over.
    pub growth_span_ms: Option<u128>,
    /// The WAL file's length beside the store.
    pub wal_bytes: u64,
    /// The main file's length. Pages committed to the WAL and not yet copied in are not in it.
    pub main_file_bytes: u64,
    /// What the store takes on disk: the main file's length and the WAL's. Page counts include
    /// pages still in the WAL, so they are not added to it.
    pub physical_bytes: u64,
    /// How much `physical_bytes` grew since the sample nearest exactly a day ago, within half an
    /// hour of it, scaled to a day. Never extrapolated: none until that baseline exists.
    pub physical_growth_bytes_per_day: Option<i64>,
    /// The span the physical growth was measured over, within half an hour of a day.
    pub physical_growth_span_ms: Option<u128>,
}

/// Independently measured archive file lengths, never subtracted from the hot-store gauge.
/// These are capacity measurements, not proof of object integrity or catalogue completeness.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ArchiveBytes {
    pub segment_file_bytes: u64,
    pub index_bytes: u64,
    pub overlay_bytes: u64,
    pub staging_bytes: u64,
    pub orphan_bytes: u64,
    pub backup_bytes: u64,
    pub files: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ArchiveSizeStatus { NotActivated, Available, Unavailable }

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DatabaseArchiveSize {
    pub measured_at_unix_ms: u128,
    /// Hash of the canonical local measurement root, never the private filesystem path.
    pub scope: String,
    pub status: ArchiveSizeStatus,
    pub bytes: Option<ArchiveBytes>,
    /// Segment files plus separately owned index and overlay files.
    pub archive_bytes: Option<u64>,
    /// All files, additionally including staging, orphans and backup-owned copies.
    pub capacity_bytes: Option<u64>,
    pub growth_bytes_per_day: Option<i64>,
    pub growth_span_ms: Option<u128>,
    pub baseline_at_unix_ms: Option<u128>,
    pub error: Option<String>,
}

static DATABASE_ARCHIVE: std::sync::Mutex<Option<DatabaseArchiveSize>> = std::sync::Mutex::new(None);

pub fn note_database_archive_size(size: &DatabaseArchiveSize) {
    *DATABASE_ARCHIVE.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(size.clone());
}

pub fn database_archive_size() -> Option<DatabaseArchiveSize> {
    DATABASE_ARCHIVE.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
}

fn database_archive_row(size: Option<DatabaseArchiveSize>, now: u128) -> Value {
    let fresh = size.as_ref().is_some_and(|size| size.measured_at_unix_ms <= now
        && now - size.measured_at_unix_ms <= 2 * 60 * 60 * 1000);
    json!({
        "name": "database-archive",
        "kind": "gauge",
        "about": "Separately measured local wire-archive bytes and net daily growth; not a hot-store saving or durability proof",
        "fresh": fresh,
        "status": if fresh { size.as_ref().map(|size| size.status) } else { None },
        "size": size,
        "integrity": "not-assessed",
        "catalogue_coverage": Value::Null,
        "budget": Value::Null,
    })
}

/// This member's newest store size, which the daemon measures hourly.
static DATABASE: std::sync::Mutex<Option<DatabaseSize>> = std::sync::Mutex::new(None);

/// Remember the newest store size for its readers.
pub fn note_database_size(size: &DatabaseSize) {
    *DATABASE.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(size.clone());
    if let Err(error) = smallclaims::windows::budgets::record_gauge(
        &database_stream(), size.measured_at_unix_ms.min(u128::from(u64::MAX)) as u64, database_sample(size),
    ) {
        tracing::error!(%error, "persisting native database SLO observation");
    }
}

/// The newest store size the daemon measured: the one sample every reader of the store's size
/// and growth uses, the database target and disk-filling conditions alike.
pub fn database_size() -> Option<DatabaseSize> {
    DATABASE.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
}

/// The store's windows and the member's CPU minutes (`CpuMinutes::report`), and the store's size,
/// with their targets.
pub fn store_report(cpu: Value) -> Vec<Value> {
    let targets = targets();
    let series = store_windows;
    vec![
        json!({
            "name": "sql-statement",
            "about": targets.statement.about,
            "p99_ms": targets.statement.p99_ms,
            "max_ms": targets.statement.max_ms,
            "windows": series(StoreWork::Statement),
        }),
        json!({
            "name": "transaction",
            "about": targets.transaction.about,
            "max_ms": targets.transaction.max_ms,
            "read": series(StoreWork::ReadTransaction),
            "write": series(StoreWork::WriteTransaction),
            "writer_hold": series(StoreWork::WriterHold),
        }),
        writer_hold_row(),
        cpu_row(cpu),
        json!({
            "name": "database",
            "about": targets.database.about,
            "max_live_gb": targets.database.max_live_gb,
            "max_growth_mb_per_day": targets.database.max_growth_mb_per_day,
            "size": database_size().map_or(Value::Null, |size| json!(size)),
            "budget": database_budget(),
        }),
        database_archive_row(database_archive_size(), u128::from(smallclaims::windows::budgets::unix_ms())),
    ]
}

/// The CPU row: the targets beside the minutes' `windows`, `minutes` and `idle`.
fn cpu_row(minutes: Value) -> Value {
    let targets = targets();
    let mut row = json!({
        "name": "cpu",
        "about": targets.cpu.about,
        "max_cores": targets.cpu.max_cores,
        "idle_max_cores": targets.cpu.idle_max_cores,
        "idle_min_minutes": targets.cpu.idle_min_minutes,
    });
    if let (Value::Object(row), Value::Object(minutes)) = (&mut row, minutes) {
        row.extend(minutes);
    }
    row
}

/// How the member's CPU fares against its two targets; a `None` is unknown, never met.
#[derive(Debug, PartialEq)]
pub struct CpuVerdict {
    /// Cores over the hour, when every one of its minutes is held with its CPU known.
    pub hour: Option<f64>,
    pub hour_met: Option<bool>,
    /// Closed minutes the hour holds, of 60.
    pub minutes: u64,
    pub idle_minutes: u64,
    /// Cores over the idle minutes, when the hour is whole and holds enough of them.
    pub idle: Option<f64>,
    pub idle_met: Option<bool>,
}

/// Judge the CPU row: the hour against `max_cores`; the idle minutes against `idle_max_cores`
/// only when the hour is whole (the daemon has been up all of it) and holds at least
/// `idle_min_minutes` of them.
pub fn cpu_verdict(row: &Value) -> CpuVerdict {
    let max = row["max_cores"].as_f64().unwrap_or_default();
    let idle_max = row["idle_max_cores"].as_f64().unwrap_or_default();
    let needed = row["idle_min_minutes"].as_u64().unwrap_or(u64::MAX);
    let hour = row["windows"]["1h"]["cores"].as_f64();
    let minutes = row["minutes"].as_u64().unwrap_or_default();
    let idle_minutes = row["idle"]["minutes"].as_u64().unwrap_or_default();
    let idle = row["idle"]["cores"]
        .as_f64()
        .filter(|_| minutes >= smallclaims::windows::cpu_minutes::MINUTES && idle_minutes >= needed);
    CpuVerdict {
        hour,
        hour_met: hour.map(|cores| cores <= max),
        minutes,
        idle_minutes,
        idle,
        idle_met: idle.map(|cores| cores <= idle_max),
    }
}

/// Productive seats alone made the idle minimum impossible, independently of any overlapping
/// missing telemetry. This is ineligibility, not a passing measurement or a CPU coverage claim.
pub fn cpu_idle_busy(row: &Value) -> bool {
    let minutes = row["minutes"].as_u64();
    let needed = row["idle_min_minutes"].as_u64();
    let busy = row["idle"]["not_idle"]["seat"].as_u64();
    matches!((minutes, needed, busy), (Some(60), Some(needed @ 1..=60), Some(busy)) if busy > 60 - needed)
}

/// `n` cores, as a target or a reading says it.
pub fn cores(n: f64) -> String {
    format!("{n} core{}", if n <= 1.0 { "" } else { "s" })
}

/// Open read snapshots reported beside the writer's holds.
const OPEN_READS: usize = 8;
/// Routes whose read loans are reported beside the writer's holds; the rest are summed.
const LOAN_ROUTES: usize = 8;

/// The writer-hold objective: each hold of the store's single writer against its target, and who
/// held it; each read transaction past that target, and who kept it open.
fn writer_hold_row() -> Value {
    let targets = targets();
    let series = store_windows;
    let mut row = json!({
        "name": "writer-hold",
        "about": targets.writer_hold.about,
        "max_ms": targets.writer_hold.max_ms,
        "windows": series(StoreWork::WriterHold),
        "read_windows": series(StoreWork::ReadTransaction),
        "open_reads": smallclaims::sqlite::open_snapshots_over(targets.writer_hold_target(), OPEN_READS),
        // Every read loan by the route it works for, checked out or with a confirmed snapshot:
        // a loan held past its budget keeps the WAL from recycling even before it reads.
        "read_loans": smallclaims::sqlite::loans_by_route(LOAN_ROUTES),
        "wal": smallclaims::windows::wal::report(),
        // Where a replication snapshot's build spends its time: waiting for the build lock that
        // one build holds at a time, reading the envelope inventory, and hashing its digests.
        "snapshot_build": {
            "wait": series(StoreWork::SnapshotBuildWait),
            "inventory": series(StoreWork::SnapshotInventory),
            "digest": series(StoreWork::SnapshotDigest),
        },
        "threads": process_threads(),
    });
    if let (Value::Object(row), Value::Object(holds)) = (&mut row, smallclaims::windows::holds::report()) {
        row.extend(holds);
    }
    row
}

/// Thread names reported beside the writer's holds, by how many threads have each.
const THREAD_NAMES: usize = 8;

/// This process's threads, in all and by name, from `/proc/self/task`; null where there is none.
/// Callers waiting for the store run on blocking threads, so their count rises with the waits.
fn process_threads() -> Value {
    let Ok(tasks) = std::fs::read_dir("/proc/self/task") else {
        return Value::Null;
    };
    let mut names = std::collections::BTreeMap::<String, u64>::new();
    for task in tasks.flatten() {
        let name = std::fs::read_to_string(task.path().join("comm")).unwrap_or_default();
        *names.entry(name.trim().to_owned()).or_default() += 1;
    }
    let total = names.values().sum::<u64>();
    let mut names = names.into_iter().collect::<Vec<_>>();
    names.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let by_name = names
        .into_iter()
        .take(THREAD_NAMES)
        .map(|(name, count)| json!({"name": name, "count": count}))
        .collect::<Vec<_>>();
    json!({"total": total, "by_name": by_name})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_row_is_a_separate_informational_gauge_and_stale_is_unknown() {
        let size = DatabaseArchiveSize { measured_at_unix_ms: 100, scope: "scope".into(),
            status: ArchiveSizeStatus::Available, bytes: Some(ArchiveBytes::default()),
            archive_bytes: Some(0), capacity_bytes: Some(0), growth_bytes_per_day: None,
            growth_span_ms: None, baseline_at_unix_ms: None, error: None };
        let row = database_archive_row(Some(size.clone()), 100);
        assert_eq!(row["name"], "database-archive");
        assert_eq!(row["kind"], "gauge");
        assert_eq!(row["status"], "available");
        assert!(row["budget"].is_null());
        assert!(row["size"]["growth_bytes_per_day"].is_null());
        assert_eq!(row["integrity"], "not-assessed");
        assert!(row["catalogue_coverage"].is_null());
        for now in [99, 100 + 2 * 60 * 60 * 1000 + 1] {
            let stale = database_archive_row(Some(size.clone()), now);
            assert_eq!(stale["fresh"], false);
            assert!(stale["status"].is_null());
        }
        let missing = database_archive_row(None, 100);
        assert_eq!(missing["fresh"], false);
        assert!(missing["size"].is_null());
    }

    #[test]
    fn unconfigured_archive_is_not_a_zero_byte_success() {
        let size = DatabaseArchiveSize { measured_at_unix_ms: 100, scope: "scope".into(),
            status: ArchiveSizeStatus::NotActivated, bytes: None, archive_bytes: None,
            capacity_bytes: None, growth_bytes_per_day: None, growth_span_ms: None,
            baseline_at_unix_ms: None, error: None };
        let row = database_archive_row(Some(size), 100);
        assert_eq!(row["status"], "not-activated");
        assert!(row["size"]["archive_bytes"].is_null());
        assert!(row["size"]["growth_bytes_per_day"].is_null());
        assert!(row["budget"].is_null());
    }

    #[test]
    fn database_unknown_growth_cannot_prove_health_or_hide_live_breach() {
        let mut size = DatabaseSize {
            measured_at_unix_ms: 0, file_bytes: 0, live_bytes: 0,
            growth_bytes_per_day: None, growth_span_ms: None, wal_bytes: 0,
            main_file_bytes: 0, physical_bytes: 0,
            physical_growth_bytes_per_day: None, physical_growth_span_ms: None,
        };
        assert_eq!(database_sample(&size), None);
        size.live_bytes = (targets().database.max_live_gb * 2_000_000_000.0) as u64;
        assert_eq!(database_sample(&size), Some(2.0));
        size.live_bytes = 0;
        size.growth_bytes_per_day = Some((targets().database.max_growth_mb_per_day * 2_000_000.0) as i64);
        assert_eq!(database_sample(&size), Some(2.0));
        size.growth_bytes_per_day = Some(0);
        assert_eq!(database_sample(&size), Some(0.0));
    }

    #[test]
    fn the_built_in_objectives_file_is_valid_and_names_the_mission_targets() {
        let targets = parse(SOURCE).unwrap();
        assert_eq!(shipped().len(), 18);
        assert!(shipped().iter().all(|objective| !objective.declarable()));
        let read = targets.for_path("GET /v1/client/agents").unwrap();
        assert_eq!((read.p99_ms, read.remote_p99_ms), (300, Some(300)));
        assert_eq!(read.target(true), Duration::from_millis(300));
        for path in [
            "stream agents",
            "stream conversation",
            "GET /v1/client/conversations/{id}/changes",
            "GET /v1/client/work",
            "GET /v1/client/now",
            "GET /v1/client/documents/content",
        ] {
            assert_eq!(targets.for_path(path).unwrap().name, "person-read", "{path}");
        }
        for path in [
            "POST /v1/messages",
            "POST /v1/work/{action}/{*subject}",
            "POST /v1/harness-events",
        ] {
            let write = targets.for_path(path).unwrap();
            assert_eq!((write.name.as_str(), write.p99_ms), ("write-ack", 300), "{path}");
        }
        let fresh = targets.for_path("GET /v1/client/agents (fresh)").unwrap();
        assert_eq!((fresh.name.as_str(), fresh.p99_ms), ("person-read-fresh", 300));
        assert_eq!(fresh.remote_p99_ms, None);
        // A fresh list counts toward its own target only, never the first page's.
        assert_eq!(targets.for_path("GET /v1/client/agents").unwrap().name, "person-read");
        for path in [
            "GET /v1/client/missions",
            "GET /v1/client/missions/{*id}",
            "stream missions",
        ] {
            assert_eq!(targets.for_path(path).unwrap().name, "person-read", "{path}");
        }
        for path in ["publish missions", "publish agents", "publish attention"] {
            let fresh = targets.for_path(path).unwrap();
            assert_eq!((fresh.name.as_str(), fresh.p99_ms), ("projection-fresh", 1500), "{path}");
        }
        let attach = targets.for_path("stream terminal").unwrap();
        assert_eq!(attach.p99_ms, 200);
        assert_eq!((targets.statement.p99_ms, targets.statement.max_ms), (10, 100));
        assert_eq!((targets.transaction.max_ms, targets.writer_hold.max_ms), (300, 300));
        assert_eq!(
            (targets.cpu.max_cores, targets.cpu.idle_max_cores, targets.cpu.idle_min_minutes, targets.cpu.load_max_cores),
            (1.0, 0.5, 10, 2.0)
        );
        assert_eq!((targets.database.max_live_gb, targets.database.max_growth_mb_per_day), (8.0, 300.0));
        assert!(targets.for_path("GET /v1/health").is_none());
    }

    #[test]
    fn every_route_path_in_the_objectives_file_is_a_route_the_daemon_serves() {
        // api.rs's route table and the route families the daemon adds to it (fractal-daemon's `api::router`).
        let source = [
            include_str!("../../fractal-api-core/src/api.rs"),
            include_str!("../../fractal-api-mailbox/src/api/mailbox.rs"),
            include_str!("../../fractal-api-client-v0/src/api/client_v0.rs"),
        ]
        .concat();
        for latency in &targets().latency {
            for path in &latency.paths {
                let Some(route) = path.strip_prefix("GET ").or(path.strip_prefix("POST ")) else {
                    continue;
                };
                // `(fresh)` names the fresh form of a route the daemon serves.
                let route = route.strip_suffix(" (fresh)").unwrap_or(route);
                assert!(
                    source.contains(&format!("\"{route}\"")),
                    "{path} is not routed in api.rs or a route family"
                );
            }
        }
    }

    #[test]
    fn the_request_latency_report_carries_the_writer_hold_objective() {
        install();
        let rows = store_report(json!({}));
        let row = rows.iter().find(|row| row["name"] == "writer-hold").unwrap();
        assert_eq!(row["max_ms"], targets().writer_hold.max_ms);
        for field in ["windows", "read_windows", "open_reads", "holders", "long_holds", "long_reads", "in_progress", "waiting"] {
            assert!(row.get(field).is_some(), "{field} missing from {row}");
        }
        if cfg!(target_os = "linux") {
            let threads = &row["threads"];
            assert!(threads["total"].as_u64().unwrap() >= 1, "{threads}");
            let named = threads["by_name"].as_array().unwrap().iter().map(|n| n["count"].as_u64().unwrap()).sum::<u64>();
            assert!(named <= threads["total"].as_u64().unwrap());
        }
        assert!(row["windows"]["1h"].is_object());
    }

    /// The shipped file without objective `name`.
    fn without(name: &str) -> String {
        let start = SOURCE.find(&format!("\nobjective \"{name}\" {{")).unwrap() + 1;
        let end = start + SOURCE[start..].find("\n}\n").unwrap() + 3;
        format!("{}{}", &SOURCE[..start], &SOURCE[end..])
    }

    fn error(text: &str) -> String {
        parse(text).unwrap_err().to_string()
    }

    #[test]
    fn a_broken_objectives_file_is_refused_not_defaulted() {
        for name in ["sql-statement", "transaction", "writer-hold", "cpu", "database", "projection-divergence"] {
            assert!(error(&without(name)).contains("there is no"), "{name}");
        }
        for (from, to) in [
            ("target \"cores\" max=1", "target \"cores\" max=-1.0"),
            ("target \"cores\" max=1", "target \"cores\" max=0.4"),
            ("target \"idle-cores\" max=0.5", "target \"idle-cores\" max=0.0"),
            ("target \"idle-cores\" max=0.5", "target \"idle-cores\" max=#nan"),
            ("load-max-cores 2", "load-max-cores #inf"),
            ("idle-min-minutes 10", "idle-min-minutes 0"),
            ("idle-min-minutes 10", "idle-min-minutes 61"),
            ("idle-min-minutes 10", "idle-min-minutes 10.5"),
            ("    load-max-cores 2\n", ""),
            ("  target \"idle-cores\" max=0.5\n", ""),
            ("target \"live-gb\" max=8", "target \"live-gb\" max=0.0"),
            ("  indicator \"statement\"\n", "  indicator \"statement\"\n  typo 1\n"),
            ("target \"live-share\" min-percent=99", "target \"live-share\" min-percent=99.5"),
            ("target \"open\" max=0", "target \"open\" max=0.5"),
        ] {
            assert!(SOURCE.contains(from), "{from}");
            assert!(parse(&SOURCE.replace(from, to)).is_err(), "{to}");
        }
        let twice = format!(
            "{SOURCE}\nobjective \"again\" {{\n  about \"x\"\n  indicator \"latency\" {{\n    path \"GET /v1/client/now\"\n  }}\n  target \"p99\" within=\"1ms\"\n}}\n"
        );
        assert!(error(&twice).contains("two targets"));
        // Each statement, transaction, writer-hold, CPU, database and divergence objective is
        // the one of its name; the shipped ones are one segment, and none is an interval.
        assert!(error(&SOURCE.replace("objective \"database\"", "objective \"store-size\"")).contains("named store-size, not database"));
        assert!(error(&SOURCE.replace("objective \"database\"", "objective \"ops/database\"")).contains("not one segment"));
        assert!(error(&SOURCE.replace("objective \"database\"", "objective \"write-ack\"")).contains("declared twice"));
        let interval = format!(
            "{SOURCE}\nobjective \"change-rollout\" {{\n  about \"x\"\n  indicator \"interval\" {{\n    subject-prefix \"custom/acme/change/\"\n    start \"custom.acme.change-ready\"\n    stop \"custom.acme.change-applied\"\n  }}\n  window \"7d\"\n  target \"p95\" within=\"5m\"\n}}\n"
        );
        assert!(error(&interval).contains("is a fleet declaration"), "{}", error(&interval));
    }

}
