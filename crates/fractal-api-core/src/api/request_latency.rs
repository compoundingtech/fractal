//! Completed response-envelope timings. Action buckets are subsets of route totals.
//!
//! Each path also keeps rolling 1-minute, 5-minute and 1-hour windows, and each latency objective
//! in `slo/objectives.kdl` keeps the same windows over all its paths.

use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};

use axum::http::Method;
use serde_json::{Value, json};

const ROUTES: usize = 256;
const RECENT: usize = 512;

#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
enum WorkAction {
    Claim,
    Renew,
    Progress,
    Complete,
    Fail,
    Release,
    Extend,
}

impl WorkAction {
    fn classify(method: &Method, route: &str, path: &str) -> Option<Self> {
        if method != Method::POST {
            return None;
        }
        if route == "/v1/work/extend/{*subject}" {
            return Some(Self::Extend);
        }
        if route != "/v1/work/{action}/{*subject}" {
            return None;
        }
        let (action, _) = path.strip_prefix("/v1/work/")?.split_once('/')?;
        // Axum decodes action path parameters. Bound decoding to the longest known
        // name (eight bytes), even when every byte is percent encoded.
        if action.len() > 24 {
            return None;
        }
        match urlencoding::decode(action).ok()?.as_ref() {
            "claim" => Some(Self::Claim),
            "renew" => Some(Self::Renew),
            "progress" => Some(Self::Progress),
            "complete" => Some(Self::Complete),
            "fail" => Some(Self::Fail),
            "release" => Some(Self::Release),
            "extend" => Some(Self::Extend),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Claim => "claim",
            Self::Renew => "renew",
            Self::Progress => "progress",
            Self::Complete => "complete",
            Self::Fail => "fail",
            Self::Release => "release",
            Self::Extend => "extend",
        }
    }

    fn route(self) -> &'static str {
        match self {
            Self::Claim => "/v1/work/claim/{*subject}",
            Self::Renew => "/v1/work/renew/{*subject}",
            Self::Progress => "/v1/work/progress/{*subject}",
            Self::Complete => "/v1/work/complete/{*subject}",
            Self::Fail => "/v1/work/fail/{*subject}",
            Self::Release => "/v1/work/release/{*subject}",
            Self::Extend => "/v1/work/extend/{*subject}",
        }
    }
}

const AGENTS: &str = "/v1/client/agents";

/// What an agents roster read asks for. A fresh first page waits, by design, for the refresher
/// to publish a roster at or after its own cut; min_cut names a fixed host-local cut.
/// Other first pages answer from a publication at once.
/// Each keeps its own sample so a latency target can count the designed wait separately
/// without dropping it from the route total.
#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
enum AgentsRead {
    FirstPage,
    Fresh,
    Continuation,
}

impl AgentsRead {
    fn classify(method: &Method, route: &str, query: Option<&str>) -> Option<Self> {
        if method != Method::GET || route != AGENTS {
            return None;
        }
        let mut fresh = false;
        for (name, value) in query.unwrap_or_default().split('&').filter_map(|pair| pair.split_once('=')) {
            match name {
                // A continuation answers from its first page's publication and never waits.
                "cursor" => return Some(Self::Continuation),
                "fresh" => fresh |= value == "true",
                "min_cut" => fresh |= value.parse::<u64>().is_ok(),
                _ => {}
            }
        }
        Some(if fresh { Self::Fresh } else { Self::FirstPage })
    }

    fn name(self) -> &'static str {
        match self {
            Self::FirstPage => "first-page",
            Self::Fresh => "fresh",
            Self::Continuation => "continuation",
        }
    }
}

/// Whether the request asked the agents list to be fresh: it waits for one roster refresh by
/// design. A person still waits on it, so it has its own target rather than none.
pub fn is_fresh_agents_read(method: &Method, route: &str, query: Option<&str>) -> bool {
    AgentsRead::classify(method, route, query) == Some(AgentsRead::Fresh)
}

/// What a request waits for beyond its own work, which decides the target it counts toward.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Wait {
    /// Nothing: it counts toward the target of its own path.
    No,
    /// A change it asked to wait for, so its time is mostly that wait: it counts toward no target.
    LongPoll,
    /// A fresh agents list: it counts toward the target of its `(fresh)` path.
    Fresh,
}

/// Explicit bulk operations, independent of interactive latency objectives. Shared
/// page routes are not exports: only the dedicated route proves the operation.
#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
enum ExportOperation {
    Backup,
    AuthorityProof,
    Replication,
}

impl ExportOperation {
    fn classify(method: &Method, route: &str) -> Option<Self> {
        match (method.as_str(), route) {
            ("GET", "/v1/backup") => Some(Self::Backup),
            ("POST", "/v1/agents/rename-id/proof/export" | "/v1/missions/rename-id/proof/export") => {
                Some(Self::AuthorityProof)
            }
            ("POST", "/v1/internal/replication/export" | "/v1/internal/replication/shard/{shard}/export") => {
                Some(Self::Replication)
            }
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Backup => "backup-export",
            Self::AuthorityProof => "authority-proof-export",
            Self::Replication => "replication-export",
        }
    }
}

#[derive(Default)]
struct Sample {
    count: u64,
    recent_ms: VecDeque<u64>,
}

impl Sample {
    fn record(&mut self, elapsed: Duration) {
        self.count = self.count.saturating_add(1);
        if self.recent_ms.len() == RECENT {
            self.recent_ms.pop_front();
        }
        self.recent_ms
            .push_back(elapsed.as_millis().min(u64::MAX as u128) as u64);
    }

    fn snapshot(&self, route: &str, scope: &str) -> Value {
        let mut sorted = self.recent_ms.iter().copied().collect::<Vec<_>>();
        sorted.sort_unstable();
        let percentile = |percent: usize| {
            sorted
                .get(((sorted.len() * percent).div_ceil(100)).saturating_sub(1))
                .copied()
                .unwrap_or_default()
        };
        json!({
            "scope": scope,
            "route": route,
            "count": self.count,
            "recent_count": sorted.len(),
            "p50_ms": percentile(50),
            "p99_ms": percentile(99),
            "max_ms": sorted.last().copied().unwrap_or_default(),
            "duration_scope": "response-envelope",
            "population": "completed-responses",
        })
    }
}

/// Where an agents roster read's time goes: the refresher's fold, from a previous publication
/// or from the log when none was kept, and its wait for roster admission; and a fresh read's
/// wait for a publication at its cut and the page it then builds.
#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub enum RosterStage {
    Refresh,
    RefreshCold,
    RefreshAdmission,
    FreshWait,
    FreshPage,
}

impl RosterStage {
    fn name(self) -> &'static str {
        match self {
            Self::Refresh => "refresh",
            Self::RefreshCold => "refresh-cold",
            Self::RefreshAdmission => "refresh-admission",
            Self::FreshWait => "fresh-wait",
            Self::FreshPage => "fresh-page",
        }
    }

    fn population(self) -> &'static str {
        match self {
            Self::Refresh | Self::RefreshCold | Self::RefreshAdmission => "refresher-folds",
            Self::FreshWait | Self::FreshPage => "fresh-reads",
        }
    }
}

/// Where a current-work read's time goes when the work list is published: a fresh read's wait
/// for a publication at its cut, a page served from a publication, and a page whose actor's
/// order in that publication its read built. Descriptive only: no target of its own.
#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub enum WorkStage {
    FreshWait,
    Page,
    ActorBuild,
}

impl WorkStage {
    fn name(self) -> &'static str {
        match self {
            Self::FreshWait => "fresh-wait",
            Self::Page => "page",
            Self::ActorBuild => "actor-build",
        }
    }

    fn population(self) -> &'static str {
        match self {
            Self::FreshWait => "fresh-reads",
            Self::Page | Self::ActorBuild => "first-pages",
        }
    }
}

/// The client current-work route, which work stages are reported beside.
const CLIENT_WORK: &str = "/v1/client/work";

#[derive(Default)]
pub struct Meter {
    routes: BTreeMap<String, Sample>,
    // Independent of the general route cap: all seven known actions retain a
    // complete denominator even if the route table has already filled.
    work_actions: BTreeMap<WorkAction, Sample>,
    agents_reads: BTreeMap<AgentsRead, Sample>,
    // Bounded by the dedicated operation enum, not the arbitrary route table cap.
    exports: BTreeMap<ExportOperation, Sample>,
    roster_stages: BTreeMap<RosterStage, Sample>,
    work_stages: BTreeMap<WorkStage, Sample>,
    /// Windows by path: `GET /route`, `stream COLLECTION`, or a long poll's route.
    paths: BTreeMap<String, smallclaims::windows::Series>,
    /// Windows by target position in `slo/objectives.kdl`: served here, then from another machine.
    targets: Vec<[smallclaims::windows::Series; 2]>,
    /// The member's CPU by closed minute: the daemon's and its replication workers'.
    cpu: smallclaims::windows::cpu_minutes::CpuMinutes,
    // Member-local diagnostics only; native objectives replace these with graph summaries.
    client_intervals: BTreeMap<(String, String, Option<String>), smallclaims::windows::IntervalSeries>,
}

/// Owned in-memory evidence. Finish only after releasing the shared request meter:
/// attaching native budgets and store evidence can wait on ledger SQLite I/O.
pub struct WindowSnapshot {
    now: Instant,
    targets: Vec<(usize, Value)>,
    cpu: Value,
    client: Vec<Value>,
    paths: Option<Vec<Value>>,
}

impl WindowSnapshot {
    pub fn finish(self) -> Value {
        let mut rows = self.targets.into_iter().map(|(index, mut row)| {
            row["budget"] = crate::slo::latency_budget(index, false, self.now);
            if row.get("remote_p99_ms").is_some() {
                row["remote_budget"] = crate::slo::latency_budget(index, true, self.now);
            }
            row
        }).collect::<Vec<_>>();
        rows.extend(crate::slo::store_report(self.cpu));
        rows.extend(self.client);
        let mut report = json!({"targets": rows});
        if let Some(paths) = self.paths {
            report["paths"] = json!(paths);
        }
        report
    }
}

/// How a completed sample is counted in the windows. Build it with [`Timed::resolve`] before
/// taking the meter's lock: the key and the target lookup allocate and scan.
pub struct Timed {
    key: String,
    target: Option<usize>,
    remote: bool,
}

impl Timed {
    /// `path` is `GET /v1/client/now`, or `stream agents` for a socket subscription's first
    /// snapshot. `remote` says it read from another machine that owns it, through this daemon.
    /// What it `wait`ed for sets its key: a long poll is kept under `(long poll)` and counts
    /// toward no target, and a fresh agents list under `(fresh)` and its own target.
    pub fn resolve(path: &str, remote: bool, wait: Wait) -> Self {
        let key = match wait {
            Wait::No => path.to_owned(),
            Wait::LongPoll => format!("{path} (long poll)"),
            Wait::Fresh => format!("{path} (fresh)"),
        };
        let target = match wait {
            Wait::LongPoll => None,
            Wait::No | Wait::Fresh => crate::slo::targets().index_of(&key),
        };
        Self {
            key,
            target,
            remote,
        }
    }
}

impl Meter {
    pub fn record(
        &mut self,
        method: &Method,
        route: &str,
        path: &str,
        query: Option<&str>,
        elapsed: Duration,
    ) {
        if self.routes.len() < ROUTES || self.routes.contains_key(route) {
            self.routes
                .entry(route.to_owned())
                .or_default()
                .record(elapsed);
        }
        if let Some(action) = WorkAction::classify(method, route, path) {
            self.work_actions.entry(action).or_default().record(elapsed);
        }
        if let Some(read) = AgentsRead::classify(method, route, query) {
            self.agents_reads.entry(read).or_default().record(elapsed);
        }
        if let Some(operation) = ExportOperation::classify(method, route) {
            self.exports.entry(operation).or_default().record(elapsed);
        }
    }

    /// The member's CPU by closed minute, which the minute task and the worker's reports feed.
    pub fn cpu_minutes(&mut self) -> &mut smallclaims::windows::cpu_minutes::CpuMinutes {
        &mut self.cpu
    }

    /// Count one roster stage's duration.
    pub fn record_roster_stage(&mut self, stage: RosterStage, elapsed: Duration) {
        self.roster_stages.entry(stage).or_default().record(elapsed);
    }

    /// Count one published current-work stage's duration.
    pub fn record_work_stage(&mut self, stage: WorkStage, elapsed: Duration) {
        self.work_stages.entry(stage).or_default().record(elapsed);
    }

    /// Count one sample in its path's windows and its target's.
    pub fn time(&mut self, now: Instant, timed: Timed, elapsed: Duration) {
        let targets = crate::slo::targets();
        let Timed {
            key,
            target,
            remote,
        } = timed;
        let over = target.is_some_and(|target| elapsed > targets.latency[target].target(remote));
        if let Some(series) = self.paths.get_mut(key.as_str()) {
            series.record(now, elapsed, over);
        } else if self.paths.len() < ROUTES {
            self.paths
                .entry(key)
                .or_default()
                .record(now, elapsed, over);
        }
        if let Some(target) = target {
            if self.targets.is_empty() {
                self.targets
                    .resize_with(targets.latency.len(), Default::default);
            }
            self.targets[target][usize::from(remote)].record(now, elapsed, over);
            crate::slo::note_latency(target, remote, now, over);
        }
    }

    /// Snapshot every target and sampled path without consulting the native ledger.
    pub fn windows_snapshot(&mut self, now: Instant) -> WindowSnapshot {
        let mut snapshot = self.targets_snapshot(now);
        snapshot.paths = Some(self.path_rows(now));
        snapshot
    }

    /// Snapshot the collector's in-memory evidence; finish after dropping the meter guard.
    pub fn targets_snapshot(&mut self, now: Instant) -> WindowSnapshot {
        let targets = crate::slo::targets();
        let empty = smallclaims::windows::Series::default();
        let rows = targets
            .latency
            .iter()
            .enumerate()
            .filter(|(_, latency)| !latency.paths.iter().any(|p| p.starts_with("client/ios/")))
            .map(|(index, latency)| {
                let windows = |remote: bool| {
                    self.targets
                        .get(index)
                        .map_or(&empty, |pair| &pair[usize::from(remote)])
                        .snapshot(now)
                };
                let mut row = json!({
                    "name": latency.name,
                    "about": latency.about,
                    "p99_ms": latency.p99_ms,
                    "windows": windows(false),
                });
                if let Some(remote) = latency.remote_p99_ms {
                    row["remote_p99_ms"] = json!(remote);
                    row["remote_windows"] = windows(true);
                }
                (index, row)
            })
            .collect::<Vec<_>>();
        let wall = crate::api::client_now_ms() as u64;
        let mut client = Vec::new();
        for latency in targets.latency.iter().filter(|t| t.paths.iter().any(|p| p.starts_with("client/ios/"))) {
            let windows = self.client_windows(wall, &latency.name, false);
            client.push(json!({"name":latency.name,"about":latency.about,"p99_ms":latency.p99_ms,
                "population":"client-observed-latencies", "windows":windows}));
        }
        for share in &targets.share {
            client.push(json!({"name":share.name,"about":share.about,"min_percent":share.min_percent,
                "population":"client-observed-foreground-ms","windows":self.client_windows(wall,&share.name,true)}));
        }
        WindowSnapshot { now, targets: rows, cpu: self.cpu.report(wall), client, paths: None }
    }

    fn path_rows(&self, now: Instant) -> Vec<Value> {
        let targets = crate::slo::targets();
        let mut paths = self
            .paths
            .iter()
            .filter(|(_, series)| !series.is_empty(now))
            .map(|(path, series)| {
                json!({
                    "path": path,
                    "target": targets.for_path(path).map(|target| target.name.as_str()),
                    "windows": series.snapshot(now),
                })
            })
            .collect::<Vec<_>>();
        let wall = crate::api::client_now_ms() as u64;
        for ((target,carrier,path),series) in &self.client_intervals {
            let share = target == "ios-live-share";
            paths.push(json!({"path":format!("client/ios/{}", target.strip_prefix("ios-").unwrap_or(target)),
                "target":target,"carrier":carrier,"carrier_path":path,
                "population":if share {"client-observed-foreground-ms"} else {"client-observed-latencies"},
                "windows":series.snapshot(wall,share)}));
        }
        paths
    }

    pub fn can_report(&self, now_ms: u64, samples: &[super::client_observations::Validated]) -> bool {
        samples.iter().filter(|s| Self::live_minute(now_ms, &s.minute)).all(|s| self.client_intervals.get(&s.key).is_none_or(|series| series.can_record(&s.minute)))
    }

    pub fn report(&mut self, now_ms: u64, samples: &[super::client_observations::Validated]) {
        for s in samples {
            if Self::live_minute(now_ms, &s.minute) {
                self.client_intervals.entry(s.key.clone()).or_default().record(now_ms,&s.minute);
            }
        }
    }

    fn live_minute(now_ms: u64, minute: &smallclaims::windows::MinuteSummary) -> bool {
        minute.start_ms >= now_ms.saturating_sub(3_600_000) && minute.start_ms.saturating_add(60_000) <= now_ms
    }

    fn client_windows(&self, now_ms: u64, target: &str, share: bool) -> Value {
        let mut total = smallclaims::windows::IntervalSeries::default();
        // At most 9 carrier/path combinations per target and 60 minutes per series.
        for ((name,_,_),series) in &self.client_intervals {
            if name == target {
                total.absorb(series);
            }
        }
        total.snapshot(now_ms,share)
    }


    pub fn snapshot(&self) -> Vec<Value> {
        self.routes
            .iter()
            .map(|(route, sample)| sample.snapshot(route, "route"))
            .chain(self.work_actions.iter().map(|(action, sample)| {
                let mut row = sample.snapshot(action.route(), "work-action");
                row["method"] = json!("POST");
                row["action"] = json!(action.name());
                row
            }))
            .chain(self.agents_reads.iter().map(|(read, sample)| {
                let mut row = sample.snapshot(AGENTS, "agents-read");
                row["method"] = json!("GET");
                row["read"] = json!(read.name());
                row
            }))
            .chain(self.roster_stages.iter().map(|(stage, sample)| {
                let mut row = sample.snapshot(AGENTS, "agents-roster");
                row["stage"] = json!(stage.name());
                row["duration_scope"] = json!("roster-stage");
                row["population"] = json!(stage.population());
                row
            }))
            .chain(self.work_stages.iter().map(|(stage, sample)| {
                let mut row = sample.snapshot(CLIENT_WORK, "work-list");
                row["stage"] = json!(stage.name());
                row["duration_scope"] = json!("work-stage");
                row["population"] = json!(stage.population());
                row
            }))
            .chain(self.exports.iter().map(|(operation, sample)| {
                let mut row = sample.snapshot(operation.name(), "export-operation");
                row["operation_class"] = json!(operation.name());
                row
            }))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn organic_terminal_first_screen_counts_but_plural_listing_does_not() {
        let now = Instant::now();
        let mut meter = Meter::default();
        meter.time(now, Timed::resolve("stream terminals", false, Wait::No), Duration::from_secs(5));
        let listing = meter.targets_snapshot(now).finish();
        let attach = listing["targets"].as_array().unwrap().iter()
            .find(|row| row["name"] == "terminal-attach").unwrap();
        assert_eq!(attach["windows"]["1h"]["count"], 0);
        assert!(crate::slo::targets().for_path("stream terminals").is_none());

        // The collections socket records this singular key only after a screen send succeeds.
        meter.time(now, Timed::resolve("stream terminal", false, Wait::No), Duration::from_millis(120));
        meter.time(now, Timed::resolve("GET /v1/client/terminals/{id}/screen", false, Wait::No), Duration::from_millis(80));
        let measured = meter.windows_snapshot(now).finish();
        let attach = measured["targets"].as_array().unwrap().iter()
            .find(|row| row["name"] == "terminal-attach").unwrap();
        assert_eq!(attach["windows"]["1h"]["count"], 2);
        assert_eq!(attach["windows"]["1h"]["over_target"], 0);
        assert_eq!(attach["windows"]["1h"]["max_ms"], 120.0);
    }

    #[test]
    fn dedicated_exports_keep_separate_classes_without_reclassifying_reads() {
        let mut meter = Meter::default();
        for (method, route, duration) in [
            (Method::GET, "/v1/backup", 4000),
            (Method::POST, "/v1/agents/rename-id/proof/export", 2000),
            (Method::POST, "/v1/missions/rename-id/proof/export", 3000),
            (Method::POST, "/v1/internal/replication/export", 500),
            (Method::POST, "/v1/internal/replication/shard/{shard}/export", 700),
            (Method::GET, "/v1/client/sessions/{*id}", 900),
            // A mailbox page can serve either an ordinary listing or an export.
            (Method::GET, "/v1/messages/page", 600),
            // A similarly named route or the wrong method is not an export.
            (Method::GET, "/v1/agents/rename-id/proof/export", 100),
            (Method::POST, "/v1/internal/replication/receive", 800),
        ] {
            meter.record(&method, route, route, None, Duration::from_millis(duration));
        }
        let rows = meter.snapshot();
        let operations = rows.iter().filter(|row| row["scope"] == "export-operation").collect::<Vec<_>>();
        assert_eq!(operations.len(), 3);
        for (class, count, max_ms) in [
            ("backup-export", 1, 4000),
            ("authority-proof-export", 2, 3000),
            ("replication-export", 2, 700),
        ] {
            let row = operations.iter().find(|row| row["operation_class"] == class).unwrap();
            assert_eq!(row["count"], count);
            assert_eq!(row["max_ms"], max_ms);
        }
        let targets = crate::slo::targets();
        assert_eq!(targets.for_path("GET /v1/client/sessions/{*id}").unwrap().name, "person-read");
        for route in ["GET /v1/backup", "POST /v1/agents/rename-id/proof/export",
            "POST /v1/internal/replication/export", "GET /v1/messages/page"] {
            assert!(targets.for_path(route).is_none(), "{route}");
        }
    }

    #[test]
    fn export_classes_survive_a_full_route_table() {
        let mut meter = Meter::default();
        for index in 0..ROUTES {
            let route = format!("/unrelated/{index}");
            meter.record(&Method::GET, &route, &route, None, Duration::ZERO);
        }
        meter.record(&Method::GET, "/v1/backup", "/v1/backup", None, Duration::from_secs(2));
        let rows = meter.snapshot();
        assert!(!rows.iter().any(|row| row["scope"] == "route" && row["route"] == "/v1/backup"));
        let export = rows.iter().find(|row| row["operation_class"] == "backup-export").unwrap();
        assert_eq!(export["count"], 1);
        assert_eq!(export["max_ms"], 2000);
    }

    #[test]
    fn http_completion_never_waits_for_blocked_ledger_reports() {
        use std::sync::mpsc;

        const CHILD: &str = "FRACTAL_METER_LEDGER_IO_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "api::request_latency::tests::http_completion_never_waits_for_blocked_ledger_reports", "--nocapture"])
                .env(CHILD, "1")
                .output().unwrap();
            assert!(output.status.success(), "isolated ledger regression failed:\n{}\n{}",
                String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
            assert!(String::from_utf8_lossy(&output.stdout).contains("blocked-writer projection regression completed"),
                "isolated child did not execute the regression:\n{}", String::from_utf8_lossy(&output.stdout));
            return;
        }

        // Both globals are isolated so this ledger cannot affect another test.
        std::thread_local! {
            static BLOCKED: std::cell::RefCell<Option<mpsc::Sender<()>>> =
                const { std::cell::RefCell::new(None) };
        }
        let root = tempfile::tempdir().unwrap();
        let state = crate::api::tests_shared::state(root.path());
        let app = crate::api::router(state.clone());
        let path = root.path().join("native-budget.sqlite");
        crate::slo::install_budget_ledger(&path, "node").unwrap();
        smallclaims::windows::budgets::set_busy_handler_for_tests(Some(|attempt| {
            if attempt == 0 {
                BLOCKED.with(|sender| { let _ = sender.borrow().as_ref().unwrap().send(()); });
            }
            std::thread::sleep(Duration::from_millis(1));
            true
        })).unwrap();

        for (name, render) in [
            ("collector", crate::api::request_latency_target_windows as fn() -> Value),
            ("diagnostics", crate::api::request_latency_windows as fn() -> Value),
        ] {
            let baseline = render();
            let as_of = baseline["targets"][0]["budget"]["as_of_unix_ms"].clone();
            let blocker = rusqlite::Connection::open(&path).unwrap();
            blocker.execute_batch("BEGIN EXCLUSIVE").unwrap();
            let (entered, blocked) = mpsc::channel();
            let writer = std::thread::spawn(move || {
                BLOCKED.with(|sender| *sender.borrow_mut() = Some(entered));
                crate::slo::checkpoint_budgets()
            });
            // A real checkpoint owns LEDGER and SQLite acknowledges its actual wait.
            let entered_io = blocked.recv_timeout(Duration::from_secs(5));
            let (reported, reports) = mpsc::channel();
            let reporting = std::thread::spawn(move || {
                let _ = reported.send(render());
            });
            let (projected, projected_rows) = mpsc::channel();
            let projection = std::thread::spawn({
                let state = state.clone();
                move || {
                    // Match the frozen assistant caller's pinned store-read boundary.
                    let rows = state.store.read_snapshot(|_| {
                        crate::api::operations::current_slo_rows(&state.store, &state.state_dir)
                    });
                    let _ = projected.send(rows);
                }
            });
            let (native_sent, native_reports) = mpsc::channel();
            let native = std::thread::spawn(move || {
                let targets = crate::slo::targets();
                let target = &targets.latency[0];
                let event_stream = format!("latency/{}/local/{}", target.name, target.p99_ms);
                let gauge_stream = format!("database/{}/{}",
                    targets.database.max_live_gb, targets.database.max_growth_mb_per_day);
                let event = smallclaims::windows::budgets::report(&event_stream);
                let gauge = smallclaims::windows::budgets::gauge_report(&gauge_stream,
                    smallclaims::windows::budgets::unix_ms(), 2 * 60 * 60 * 1000, 1.0);
                let _ = native_sent.send((event, gauge));
            });
            let (sent, completed) = mpsc::channel();
            let request = std::thread::spawn({
                let app = app.clone();
                move || {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all().build().unwrap();
                    let response = runtime.block_on(
                        crate::api::tests_shared_client_v0_tests::get_request(app, "/v1/health"));
                    let _ = sent.send(response);
                }
            });
            let response = completed.recv_timeout(Duration::from_secs(1));
            let report = reports.recv_timeout(Duration::from_secs(1));
            let current_rows = projected_rows.recv_timeout(Duration::from_secs(1));
            let native_result = native_reports.recv_timeout(Duration::from_secs(1));
            let writer_still_blocked = !writer.is_finished();
            // Never assert a timeout while a worker can still be stranded on LEDGER.
            let rollback = blocker.execute_batch("ROLLBACK");
            drop(blocker);
            let written = writer.join();
            let reported = reporting.join();
            let projected = projection.join();
            let native_joined = native.join();
            let requested = request.join();
            rollback.unwrap();
            written.unwrap().unwrap();
            reported.unwrap();
            projected.unwrap();
            native_joined.unwrap();
            requested.unwrap();
            assert!(entered_io.is_ok(), "{name} checkpoint never reached SQLite busy wait: {entered_io:?}");
            assert!(writer_still_blocked, "{name} checkpoint finished with disk still locked");
            assert_eq!(response.expect("HTTP completion waited for ledger I/O").0, axum::http::StatusCode::OK);
            let report = report.expect("report wrapper waited for the blocked native ledger writer");
            let current_rows = current_rows.expect("current_slo_rows waited for the blocked native ledger writer").unwrap();
            assert!(!current_rows.is_empty());
            assert!(current_rows.iter().all(|row| row["id"].is_string()));
            for id in [crate::slo::targets().latency[0].name.as_str(), "database", "database-archive", "transaction/read"] {
                let row = current_rows.iter().find(|row| row["id"] == id)
                    .unwrap_or_else(|| panic!("current_slo_rows omitted native objective {id}"));
                assert_eq!(row["kind"], "objective");
                assert_eq!(row["status"], "no-data");
                assert!(row["remaining_percent"].is_null());
            }
            assert!(as_of.is_u64(), "installed native projection omitted its as-of timestamp");
            let (event, gauge) = native_result.expect("public report APIs waited for the blocked native ledger writer");
            for evidence in [event.unwrap(), gauge.unwrap()] {
                assert_eq!(evidence["as_of_unix_ms"], as_of);
                assert!(evidence["stale"].is_boolean());
                assert_eq!(evidence["available"], !evidence["stale"].as_bool().unwrap());
                assert!(evidence["windows"].is_object());
            }

            let rows = report["targets"].as_array().unwrap();
            let targets = crate::slo::targets();
            let server = targets.latency.iter()
                .filter(|latency| !latency.paths.iter().any(|path| path.starts_with("client/ios/")))
                .collect::<Vec<_>>();
            for (row, target) in rows.iter().zip(&server) {
                assert_eq!(row["name"], target.name);
                assert_eq!(row["budget"]["as_of_unix_ms"], as_of);
                assert_eq!(row["budget"]["available"], !row["budget"]["stale"].as_bool().unwrap());
                assert!(row["windows"].is_object());
                if target.remote_p99_ms.is_some() {
                    assert_eq!(row["remote_budget"]["as_of_unix_ms"], as_of);
                    assert_eq!(row["remote_budget"]["available"], !row["remote_budget"]["stale"].as_bool().unwrap());
                    assert!(row["remote_windows"].is_object());
                }
            }
            let store = &rows[server.len()..server.len() + 6];
            assert_eq!(store.iter().map(|row| row["name"].as_str().unwrap()).collect::<Vec<_>>(),
                ["sql-statement", "transaction", "writer-hold", "cpu", "database", "database-archive"]);
            assert_eq!(store[4]["budget"]["as_of_unix_ms"], as_of);
            assert_eq!(store[4]["budget"]["available"], !store[4]["budget"]["stale"].as_bool().unwrap());
            assert_eq!(store[5]["kind"], "gauge");
            assert!(store[5]["budget"].is_null());
            assert_eq!(store[5]["integrity"], "not-assessed");
            assert!(store[5]["catalogue_coverage"].is_null());
            let client = targets.latency.iter()
                .filter(|latency| latency.paths.iter().any(|path| path.starts_with("client/ios/")))
                .map(|latency| latency.name.as_str())
                .chain(targets.share.iter().map(|share| share.name.as_str())).collect::<Vec<_>>();
            assert_eq!(rows[server.len() + 6..].iter()
                .map(|row| row["name"].as_str().unwrap()).collect::<Vec<_>>(), client);
            assert_eq!(report.get("paths").is_some(), name == "diagnostics");
        }
        let rows = crate::api::request_latency_snapshot();
        assert_eq!(rows.iter().find(|row| row["scope"] == "route" && row["route"] == "/v1/health")
            .unwrap()["count"], 2);
        println!("blocked-writer projection regression completed");
    }

    const WORK: &str = "/v1/work/{action}/{*subject}";

    fn row(meter: &Meter, scope: &str, route: &str) -> Value {
        meter
            .snapshot()
            .into_iter()
            .find(|row| row["scope"] == scope && row["route"] == route)
            .unwrap()
    }

    #[test]
    fn renew_and_claim_keep_independent_full_duration_samples_and_route_total() {
        let mut meter = Meter::default();
        for (path, duration) in [
            ("/v1/work/renew/work/private-a", 11),
            ("/v1/work/claim/work/private-b", 900),
            ("/v1/work/renew/work/private-c", 27),
        ] {
            meter.record(&Method::POST, WORK, path, None, Duration::from_millis(duration));
        }
        let renew = row(&meter, "work-action", "/v1/work/renew/{*subject}");
        assert_eq!(renew["count"], 2);
        assert_eq!(renew["recent_count"], 2);
        assert_eq!(renew["p99_ms"], 27);
        assert_eq!(renew["method"], "POST");
        assert_eq!(renew["duration_scope"], "response-envelope");
        let claim = row(&meter, "work-action", "/v1/work/claim/{*subject}");
        assert_eq!(claim["count"], 1);
        assert_eq!(claim["p99_ms"], 900);
        let aggregate = row(&meter, "route", WORK);
        assert_eq!(aggregate["count"], 3);
        assert_eq!(aggregate["p99_ms"], 900);
        assert!(
            !serde_json::to_string(&meter.snapshot())
                .unwrap()
                .contains("private")
        );
    }

    #[test]
    fn only_known_post_actions_allocate_static_buckets_including_encoded_parameters() {
        let mut meter = Meter::default();
        for action in [
            "claim", "renew", "progress", "complete", "fail", "release", "extend",
        ] {
            meter.record(
                &Method::POST,
                WORK,
                &format!("/v1/work/{action}/secret"),
                None,
                Duration::ZERO,
            );
        }
        meter.record(
            &Method::POST,
            "/v1/work/extend/{*subject}",
            "/v1/work/extend/secret",
            None,
            Duration::ZERO,
        );
        meter.record(
            &Method::POST,
            WORK,
            "/v1/work/%72%65%6e%65%77/secret",
            None,
            Duration::ZERO,
        );
        for method in [Method::GET, Method::HEAD, Method::PUT] {
            meter.record(&method, WORK, "/v1/work/renew/secret", None, Duration::ZERO);
        }
        for i in 0..1_000 {
            meter.record(
                &Method::POST,
                WORK,
                &format!("/v1/work/unknown-{i}/secret"),
                None,
                Duration::ZERO,
            );
        }
        meter.record(
            &Method::POST,
            "/unmatched",
            "/v1/work/renew/secret",
            None,
            Duration::ZERO,
        );
        assert_eq!(meter.work_actions.len(), 7);
        assert_eq!(
            row(&meter, "work-action", "/v1/work/renew/{*subject}")["count"],
            2
        );
        assert_eq!(
            row(&meter, "work-action", "/v1/work/extend/{*subject}")["count"],
            2
        );
        let json = serde_json::to_string(&meter.snapshot()).unwrap();
        assert!(!json.contains("secret"));
        assert!(!json.contains("unknown-"));
        assert!(!json.contains("%72"));
    }

    #[test]
    fn paths_count_toward_their_target_with_the_remote_target_for_remote_reads() {
        let mut meter = Meter::default();
        let now = Instant::now();
        let time = |meter: &mut Meter, path, remote, long_poll: bool, ms| {
            let wait = if long_poll { Wait::LongPoll } else { Wait::No };
            meter.time(
                now,
                Timed::resolve(path, remote, wait),
                Duration::from_millis(ms),
            );
        };
        time(&mut meter, "GET /v1/client/now", false, false, 50);
        time(&mut meter, "GET /v1/client/now", false, false, 150);
        time(&mut meter, "GET /v1/client/now", false, false, 350);
        time(&mut meter, "stream agents", true, false, 250);
        time(&mut meter, "stream conversation", true, false, 350);
        time(&mut meter, "GET /v1/client/conversations/{id}/changes", false, true, 30_000);
        time(&mut meter, "GET /v1/health", false, false, 1);
        let report = meter.windows_snapshot(now).finish();
        let target = |name: &str| {
            report["targets"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["name"] == name)
                .unwrap()
                .clone()
        };
        let read = target("person-read");
        assert_eq!(read["windows"]["1m"]["count"], 3);
        assert_eq!(read["windows"]["1m"]["over_target"], 1);
        assert_eq!(read["remote_windows"]["5m"]["count"], 2);
        assert_eq!(read["remote_windows"]["5m"]["over_target"], 1);
        assert_eq!(target("write-ack")["windows"]["1h"]["count"], 0);
        for name in ["sql-statement", "transaction", "cpu", "database", "database-archive"] {
            target(name);
        }
        let paths = report["paths"].as_array().unwrap();
        let path = |name: &str| paths.iter().find(|row| row["path"] == name).unwrap();
        assert_eq!(path("GET /v1/client/now")["target"], "person-read");
        assert_eq!(path("GET /v1/health")["target"], Value::Null);
        let poll = path("GET /v1/client/conversations/{id}/changes (long poll)");
        assert_eq!(poll["target"], Value::Null);
        assert_eq!(poll["windows"]["1m"]["over_target"], 0);
        assert_eq!(paths.len(), 5);
    }

    #[test]
    fn a_fresh_agents_read_has_its_own_target_and_counts_toward_no_other() {
        assert!(is_fresh_agents_read(&Method::GET, AGENTS, Some("limit=5&fresh=true")));
        assert!(is_fresh_agents_read(&Method::GET, AGENTS, Some("min_cut=42&fresh=false")));
        assert!(is_fresh_agents_read(&Method::GET, AGENTS, Some("fresh=false&min_cut=42")));
        assert!(!is_fresh_agents_read(&Method::GET, AGENTS, Some("fresh=false")));
        assert!(!is_fresh_agents_read(&Method::GET, AGENTS, None));
        assert!(!is_fresh_agents_read(&Method::GET, AGENTS, Some("fresh=true&cursor=page")));
        assert!(!is_fresh_agents_read(&Method::POST, AGENTS, Some("fresh=true")));
        assert!(!is_fresh_agents_read(&Method::GET, "/v1/client/work", Some("fresh=true")));
        let targets = crate::slo::targets();
        let named = |timed: &Timed| timed.target.map(|index| targets.latency[index].name.as_str());
        let first = Timed::resolve("GET /v1/client/agents", false, Wait::No);
        assert_eq!(named(&first), Some("person-read"));
        let fresh = Timed::resolve("GET /v1/client/agents", false, Wait::Fresh);
        assert_eq!(fresh.key, "GET /v1/client/agents (fresh)");
        assert_eq!(named(&fresh), Some("person-read-fresh"));
        let poll = Timed::resolve("GET /v1/client/agents", false, Wait::LongPoll);
        assert_eq!(poll.key, "GET /v1/client/agents (long poll)");
        assert_eq!(named(&poll), None);
        // Fresh and ordinary reads have separate windows, each with a 300 ms target.
        let mut meter = Meter::default();
        let now = Instant::now();
        meter.time(now, Timed::resolve("GET /v1/client/agents", false, Wait::Fresh), Duration::from_millis(200));
        meter.time(now, Timed::resolve("GET /v1/client/agents", false, Wait::Fresh), Duration::from_millis(400));
        meter.time(now, Timed::resolve("GET /v1/client/agents", false, Wait::No), Duration::from_millis(150));
        // The target is inclusive: exactly 300 ms is not over it (Meter::time uses >).
        meter.time(now, Timed::resolve("GET /v1/client/agents", false, Wait::No), Duration::from_millis(300));
        meter.time(now, Timed::resolve("GET /v1/client/agents", false, Wait::No), Duration::from_millis(350));
        let report = meter.windows_snapshot(now).finish();
        let row = |name: &str| {
            report["targets"].as_array().unwrap().iter().find(|row| row["name"] == name).unwrap().clone()
        };
        assert_eq!(row("person-read-fresh")["windows"]["1m"]["count"], 2);
        assert_eq!(row("person-read-fresh")["windows"]["1m"]["over_target"], 1);
        assert_eq!(row("person-read")["windows"]["1m"]["count"], 3);
        assert_eq!(row("person-read")["windows"]["1m"]["over_target"], 1);
    }

    /// A refresher's publication counts toward `projection-fresh`, and one past 1500 ms is over
    /// it.
    #[test]
    fn publications_count_toward_projection_freshness() {
        let mut meter = Meter::default();
        let now = Instant::now();
        for (collection, ms) in [("missions", 900), ("agents", 1200), ("attention", 1800)] {
            meter.time(
                now,
                Timed::resolve(&format!("publish {collection}"), false, Wait::No),
                Duration::from_millis(ms),
            );
        }
        let report = meter.windows_snapshot(now).finish();
        let fresh = report["targets"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["name"] == "projection-fresh")
            .unwrap()
            .clone();
        assert_eq!(fresh["windows"]["1m"]["count"], 3);
        assert_eq!(fresh["windows"]["1m"]["over_target"], 1);
    }

    #[test]
    fn roster_stages_report_their_own_samples_beside_the_route() {
        let mut meter = Meter::default();
        for (stage, ms) in [
            (RosterStage::Refresh, 40), (RosterStage::Refresh, 60), (RosterStage::RefreshCold, 1700),
            (RosterStage::RefreshAdmission, 2),
            (RosterStage::FreshWait, 90), (RosterStage::FreshPage, 4),
        ] {
            meter.record_roster_stage(stage, Duration::from_millis(ms));
        }
        let stage = |name: &str| meter.snapshot().into_iter()
            .find(|row| row["scope"] == "agents-roster" && row["stage"] == name).unwrap();
        assert_eq!(stage("refresh")["count"], 2);
        assert_eq!(stage("refresh")["max_ms"], 60);
        assert_eq!(stage("refresh")["population"], "refresher-folds");
        assert_eq!(stage("refresh-cold")["count"], 1);
        assert_eq!(stage("refresh-cold")["population"], "refresher-folds");
        assert_eq!(stage("refresh-admission")["p99_ms"], 2);
        assert_eq!(stage("fresh-wait")["p99_ms"], 90);
        assert_eq!(stage("fresh-wait")["population"], "fresh-reads");
        assert_eq!(stage("fresh-page")["duration_scope"], "roster-stage");
        assert_eq!(stage("fresh-page")["route"], AGENTS);
        // Stages are not requests: no route row appears for them.
        assert!(meter.snapshot().iter().all(|row| row["scope"] != "route"));
    }

    #[test]
    fn work_stages_report_their_own_samples_and_leave_the_roster_keys_alone() {
        let mut meter = Meter::default();
        meter.record_roster_stage(RosterStage::FreshWait, Duration::from_millis(90));
        for (stage, ms) in [
            (WorkStage::FreshWait, 700), (WorkStage::Page, 2), (WorkStage::Page, 3), (WorkStage::ActorBuild, 9),
        ] {
            meter.record_work_stage(stage, Duration::from_millis(ms));
        }
        let rows = meter.snapshot();
        let stage = |name: &str| rows.iter()
            .find(|row| row["scope"] == "work-list" && row["stage"] == name).unwrap();
        assert_eq!(stage("fresh-wait")["p99_ms"], 700);
        assert_eq!(stage("fresh-wait")["population"], "fresh-reads");
        assert_eq!(stage("page")["count"], 2);
        assert_eq!(stage("actor-build")["population"], "first-pages");
        assert_eq!(stage("page")["route"], CLIENT_WORK);
        assert_eq!(stage("page")["duration_scope"], "work-stage");
        // The roster's rows keep their keys, and stages are not requests.
        let roster = rows.iter().find(|row| row["scope"] == "agents-roster").unwrap();
        assert_eq!((roster["stage"].clone(), roster["route"].clone()), (json!("fresh-wait"), json!(AGENTS)));
        assert!(rows.iter().all(|row| row["scope"] != "route"));
    }

    #[test]
    fn agents_reads_split_the_designed_fresh_wait_from_the_route_total() {
        let mut meter = Meter::default();
        for (query, duration) in [
            (None, 4),
            (Some("limit=50"), 6),
            (Some("limit=50&fresh=true"), 700),
            (Some("fresh=false"), 5),
            (Some("fresh=true&cursor=page%2Fsecret"), 3),
        ] {
            meter.record(&Method::GET, AGENTS, AGENTS, query, Duration::from_millis(duration));
        }
        meter.record(&Method::HEAD, AGENTS, AGENTS, Some("fresh=true"), Duration::from_millis(9));
        meter.record(&Method::GET, "/v1/client/work", "/v1/client/work", Some("fresh=true"), Duration::ZERO);
        let read = |name: &str| meter.snapshot().into_iter()
            .find(|row| row["scope"] == "agents-read" && row["read"] == name).unwrap();
        assert_eq!(read("first-page")["count"], 3);
        assert_eq!(read("first-page")["p99_ms"], 6);
        assert_eq!(read("fresh")["count"], 1);
        assert_eq!(read("fresh")["p99_ms"], 700);
        assert_eq!(read("continuation")["count"], 1);
        assert_eq!(read("fresh")["route"], AGENTS);
        // The route total keeps every read, the designed wait included.
        let route = row(&meter, "route", AGENTS);
        assert_eq!(route["count"], 6);
        assert_eq!(route["max_ms"], 700);
        assert!(!serde_json::to_string(&meter.snapshot()).unwrap().contains("secret"));
    }

    #[test]
    fn action_denominator_survives_route_capacity_and_retains_last_512_of_all_completions() {
        let mut meter = Meter::default();
        for i in 0..ROUTES {
            meter.record(
                &Method::GET,
                &format!("/known-route-{i}"),
                "",
                None,
                Duration::ZERO,
            );
        }
        for ms in 1..=600 {
            meter.record(
                &Method::POST,
                WORK,
                "/v1/work/renew/secret",
                None,
                Duration::from_millis(ms),
            );
        }
        assert_eq!(meter.routes.len(), ROUTES);
        assert!(!meter.routes.contains_key(WORK));
        let renew = row(&meter, "work-action", "/v1/work/renew/{*subject}");
        assert_eq!(renew["count"], 600);
        assert_eq!(renew["recent_count"], 512);
        assert_eq!(renew["p50_ms"], 344);
        assert_eq!(renew["p99_ms"], 595);
        assert_eq!(renew["max_ms"], 600);
        assert_eq!(meter.snapshot().len(), ROUTES + 1);
    }
    #[test]
    fn client_intervals_batch_disjoint_minutes_and_old_reports_are_history_only() {
        use super::super::client_observations::Validated;
        let key: (String,String,Option<String>)=("ios-connect".into(),"lan".into(),None);
        let sample=|start,count|Validated {key:key.clone(), minute:smallclaims::windows::MinuteSummary {
            start_ms:start,count,over:0,max_ms:100,buckets:vec![(smallclaims::windows::histogram_upper_ms(100),count)]
        }};
        let mut meter=Meter::default();
        meter.report(60_000,&[sample(0,1_000_000_000)]);
        let now=3_660_000;
        assert!(meter.can_report(now,&[sample(0,1)]),"expired slot capacity cannot reject history-only reports");
        meter.report(now,&[sample(0,1)]);
        assert_eq!(meter.client_windows(now,"ios-connect",false)["1h"]["count"],0);
        let batch=[sample(3_540_000,100),sample(3_600_000,200)];
        assert!(meter.can_report(now,&batch));meter.report(now,&batch);
        assert_eq!(meter.client_windows(now,"ios-connect",false)["5m"]["count"],300);
        assert_eq!(meter.client_windows(now,"ios-connect",false)["1m"]["count"],200);
    }

}
